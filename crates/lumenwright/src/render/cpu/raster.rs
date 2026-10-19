//! Tile-binned software rasterizer for quick mode.
//!
//! Matches the WGSL quick shader: nearest texel fetch, alpha test at 0.5 for the
//! opaque pass, alpha blending for the translucent pass, vanilla fog.

use glam::{Vec3, Vec4};
use rayon::prelude::*;

use crate::render::QuickFrame;

const TILE: u32 = 32;

#[derive(Clone, Copy)]
struct ClipVert {
    clip: Vec4,
    uv: [f32; 2],
    color: Vec4,
    pos: Vec3,
}

/// A screen-space triangle ready for rasterization.
#[derive(Clone, Copy)]
struct Tri {
    /// Screen positions (pixels), with z = ndc depth, w = 1/clip.w.
    v: [Vec4; 3],
    /// Attributes pre-divided by clip w: uv(2), color(4), pos(3).
    a: [[f32; 9]; 3],
    area_inv: f32,
    translucent: bool,
    bbox: [i32; 4],
}

fn lerp_vert(a: &ClipVert, b: &ClipVert, t: f32) -> ClipVert {
    ClipVert {
        clip: a.clip + (b.clip - a.clip) * t,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
        color: a.color + (b.color - a.color) * t,
        pos: a.pos + (b.pos - a.pos) * t,
    }
}

/// Clips a triangle against the near plane (clip.z >= 0), producing 0-2 triangles.
fn clip_near(v: [ClipVert; 3], out: &mut Vec<[ClipVert; 3]>) {
    let inside = |c: &ClipVert| c.clip.z >= 0.0;
    let n = v.iter().filter(|c| inside(c)).count();
    if n == 3 {
        out.push(v);
        return;
    }
    if n == 0 {
        return;
    }
    let mut poly: Vec<ClipVert> = Vec::with_capacity(4);
    for i in 0..3 {
        let a = &v[i];
        let b = &v[(i + 1) % 3];
        if inside(a) {
            poly.push(*a);
        }
        if inside(a) != inside(b) {
            let t = a.clip.z / (a.clip.z - b.clip.z);
            poly.push(lerp_vert(a, b, t));
        }
    }
    for i in 1..poly.len() - 1 {
        out.push([poly[0], poly[i], poly[i + 1]]);
    }
}

fn setup(v: &[ClipVert; 3], w: f32, h: f32, translucent: bool) -> Option<Tri> {
    let mut sv = [Vec4::ZERO; 3];
    let mut attrs = [[0f32; 9]; 3];
    for i in 0..3 {
        let c = v[i].clip;
        if c.w <= 1e-6 {
            return None;
        }
        let iw = 1.0 / c.w;
        let ndc = c.truncate() * iw;
        sv[i] = Vec4::new((ndc.x * 0.5 + 0.5) * w, (0.5 - ndc.y * 0.5) * h, ndc.z, iw);
        let a = &v[i];
        attrs[i] = [
            a.uv[0] * iw,
            a.uv[1] * iw,
            a.color.x * iw,
            a.color.y * iw,
            a.color.z * iw,
            a.color.w * iw,
            a.pos.x * iw,
            a.pos.y * iw,
            a.pos.z * iw,
        ];
    }
    // Screen space is y-down, so counter-clockwise (front) triangles have negative area.
    let area =
        (sv[1].x - sv[0].x) * (sv[2].y - sv[0].y) - (sv[1].y - sv[0].y) * (sv[2].x - sv[0].x);
    if area >= 0.0 {
        return None; // back face or degenerate
    }
    let (min_x, max_x) = (
        sv.iter().map(|p| p.x).fold(f32::MAX, f32::min),
        sv.iter().map(|p| p.x).fold(f32::MIN, f32::max),
    );
    let (min_y, max_y) = (
        sv.iter().map(|p| p.y).fold(f32::MAX, f32::min),
        sv.iter().map(|p| p.y).fold(f32::MIN, f32::max),
    );
    let bbox = [
        (min_x.floor() as i32).max(0),
        (min_y.floor() as i32).max(0),
        (max_x.ceil() as i32).min(w as i32 - 1),
        (max_y.ceil() as i32).min(h as i32 - 1),
    ];
    if bbox[0] > bbox[2] || bbox[1] > bbox[3] {
        return None;
    }
    // Reorder to positive area for the edge functions.
    Some(Tri {
        v: [sv[0], sv[2], sv[1]],
        a: [attrs[0], attrs[2], attrs[1]],
        area_inv: 1.0 / -area,
        translucent,
        bbox,
    })
}

#[inline]
fn edge(a: Vec4, b: Vec4, px: f32, py: f32) -> f32 {
    (b.x - a.x) * (py - a.y) - (b.y - a.y) * (px - a.x)
}

#[inline]
fn unpack(c: u32) -> Vec4 {
    Vec4::new(
        (c & 0xFF) as f32,
        ((c >> 8) & 0xFF) as f32,
        ((c >> 16) & 0xFF) as f32,
        (c >> 24) as f32,
    ) / 255.0
}

#[inline]
fn pack(c: Vec3) -> u32 {
    let c = (c.clamp(Vec3::ZERO, Vec3::ONE) * 255.0 + 0.5).as_uvec3();
    c.x | (c.y << 8) | (c.z << 16) | 0xFF00_0000
}

// The open depth comparisons reject a NaN; `RangeInclusive::contains` would not.
#[allow(clippy::manual_range_contains)]
/// Rasterizes a quick-mode frame on the CPU and returns packed RGBA8 pixels
/// (red in the low byte), row major, top row first.
pub fn rasterize(f: &QuickFrame) -> Vec<u32> {
    let (w, h) = (f.width as f32, f.height as f32);
    let mesh = f.mesh;
    let t0 = std::time::Instant::now();

    let verts: Vec<ClipVert> = mesh
        .vertices
        .par_iter()
        .map(|v| {
            let pos = Vec3::from(v.pos);
            ClipVert {
                clip: f.view_proj * pos.extend(1.0),
                uv: v.uv,
                color: Vec4::new(
                    v.color[0] as f32,
                    v.color[1] as f32,
                    v.color[2] as f32,
                    v.color[3] as f32,
                ) / 255.0,
                pos,
            }
        })
        .collect();

    let build = |idx: &[u32], translucent: bool| -> Vec<Tri> {
        idx.par_chunks(3 * 1024)
            .flat_map_iter(|chunk| {
                let mut out = Vec::new();
                let mut clipped = Vec::new();
                for t in chunk.as_chunks::<3>().0 {
                    let tv = [
                        verts[t[0] as usize],
                        verts[t[1] as usize],
                        verts[t[2] as usize],
                    ];
                    // Trivial reject if all vertices are outside one side.
                    let all = |f: &dyn Fn(&Vec4) -> bool| tv.iter().all(|v| f(&v.clip));
                    if all(&|c| c.x > c.w)
                        || all(&|c| c.x < -c.w)
                        || all(&|c| c.y > c.w)
                        || all(&|c| c.y < -c.w)
                        || all(&|c| c.z < 0.0)
                    {
                        continue;
                    }
                    clipped.clear();
                    clip_near(tv, &mut clipped);
                    for c in &clipped {
                        if let Some(tri) = setup(c, w, h, translucent) {
                            out.push(tri);
                        }
                    }
                }
                out
            })
            .collect()
    };
    let mut tris = build(&mesh.opaque, false);
    let n_opaque = tris.len();
    tris.extend(build(&mesh.translucent, true));

    // Bin triangles into tiles (order preserved: opaque first, translucent back-to-front).
    let tiles_x = f.width.div_ceil(TILE);
    let tiles_y = f.height.div_ceil(TILE);
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); (tiles_x * tiles_y) as usize];
    for (i, t) in tris.iter().enumerate() {
        let (x0, y0, x1, y1) = (
            t.bbox[0] as u32 / TILE,
            t.bbox[1] as u32 / TILE,
            t.bbox[2] as u32 / TILE,
            t.bbox[3] as u32 / TILE,
        );
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                bins[(ty * tiles_x + tx) as usize].push(i as u32);
            }
        }
    }

    let atlas = f.atlas_pixels;
    let [aw, ah] = f.atlas_size;
    let sky = &f.sky;
    let fog = Vec3::from_slice(&sky.fog_color[..3]);
    let tile_pixels: Vec<(u32, Vec<u32>)> = (0..tiles_x * tiles_y)
        .into_par_iter()
        .map(|ti| {
            let (tx, ty) = (ti % tiles_x, ti / tiles_x);
            let (x0, y0) = (tx * TILE, ty * TILE);
            let (tw, th) = (TILE.min(f.width - x0), TILE.min(f.height - y0));
            let mut color = vec![0u32; (tw * th) as usize];
            let mut depth = vec![f32::INFINITY; (tw * th) as usize];
            let mut covered = vec![false; (tw * th) as usize];
            for &i in &bins[ti as usize] {
                let t = &tris[i as usize];
                let bx0 = (t.bbox[0] as u32).max(x0);
                let by0 = (t.bbox[1] as u32).max(y0);
                let bx1 = (t.bbox[2] as u32).min(x0 + tw - 1);
                let by1 = (t.bbox[3] as u32).min(y0 + th - 1);
                if bx0 > bx1 || by0 > by1 {
                    continue;
                }
                for py in by0..=by1 {
                    for px in bx0..=bx1 {
                        let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                        let w0 = edge(t.v[1], t.v[2], fx, fy);
                        let w1 = edge(t.v[2], t.v[0], fx, fy);
                        let w2 = edge(t.v[0], t.v[1], fx, fy);
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let (b0, b1, b2) = (w0 * t.area_inv, w1 * t.area_inv, w2 * t.area_inv);
                        let z = b0 * t.v[0].z + b1 * t.v[1].z + b2 * t.v[2].z;
                        let li = ((py - y0) * tw + (px - x0)) as usize;
                        if z >= depth[li] || z < 0.0 || z > 1.0 {
                            continue;
                        }
                        let iw = b0 * t.v[0].w + b1 * t.v[1].w + b2 * t.v[2].w;
                        let pw = 1.0 / iw;
                        let attr =
                            |k: usize| (b0 * t.a[0][k] + b1 * t.a[1][k] + b2 * t.a[2][k]) * pw;
                        let (u, v) = (attr(0), attr(1));
                        let tx = ((u * aw as f32).floor() as i32).clamp(0, aw as i32 - 1) as u32;
                        let tyy = ((v * ah as f32).floor() as i32).clamp(0, ah as i32 - 1) as u32;
                        let tex = unpack(atlas[(tyy * aw + tx) as usize]);
                        let vc = Vec4::new(attr(2), attr(3), attr(4), attr(5));
                        let pos = Vec3::new(attr(6), attr(7), attr(8));
                        let mut rgb = tex.truncate() * vc.truncate();
                        rgb = rgb.lerp(fog, sky.fog_factor(pos));
                        if t.translucent {
                            let a = tex.w * vc.w;
                            if a <= 0.0 {
                                continue;
                            }
                            let dst = if covered[li] {
                                unpack(color[li]).truncate()
                            } else {
                                sky.background(sky.pixel_dir(px as f32, py as f32), atlas, aw)
                            };
                            color[li] = pack(dst.lerp(rgb, a));
                            covered[li] = true;
                        } else {
                            if tex.w < 0.5 {
                                continue;
                            }
                            depth[li] = z;
                            color[li] = pack(rgb);
                            covered[li] = true;
                        }
                    }
                }
            }
            for py in 0..th {
                for px in 0..tw {
                    let li = (py * tw + px) as usize;
                    if !covered[li] {
                        color[li] = pack(sky.background(
                            sky.pixel_dir((x0 + px) as f32, (y0 + py) as f32),
                            atlas,
                            aw,
                        ));
                    }
                }
            }
            (ti, color)
        })
        .collect();

    let mut out = vec![0u32; (f.width * f.height) as usize];
    for (ti, color) in tile_pixels {
        let (tx, ty) = (ti % tiles_x, ti / tiles_x);
        let (x0, y0) = (tx * TILE, ty * TILE);
        let tw = TILE.min(f.width - x0);
        for (i, c) in color.iter().enumerate() {
            let (px, py) = (x0 + i as u32 % tw, y0 + i as u32 / tw);
            out[(py * f.width + px) as usize] = *c;
        }
    }
    log::info!(
        "cpu raster: {} triangles ({} opaque) in {:.2?}",
        tris.len(),
        n_opaque,
        t0.elapsed()
    );
    out
}
