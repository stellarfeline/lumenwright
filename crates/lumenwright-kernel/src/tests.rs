//! Kernel tests on hand-built scenes (no game assets needed).

#![allow(clippy::field_reassign_with_default)]

use crate::math::{V3, v3};
use crate::scene::{
    ALPHA_CUTOUT, ALPHA_OPAQUE, BRICK_WORDS, EMPTY, FLAG_FRONT_ONLY, GpuModel, GpuQuad,
    OV_CELL_WORDS, Params, SceneView, UNIFORM,
};
use crate::trace::{is_see_through, see_through_hit, trace};

fn quad(p0: V3, eu: V3, ev: V3, flags: u32) -> GpuQuad {
    GpuQuad {
        p0: [p0.x, p0.y, p0.z, f32::from_bits(flags)],
        eu: [eu.x, eu.y, eu.z, f32::from_bits(0xFFFF_FFFF)],
        ev: [ev.x, ev.y, ev.z, 0.0],
        // Sample a 2x1 atlas: texel 0 opaque white, texel 1 transparent.
        uv: [0.0, 0.0, 2.0, 0.0],
        uvt: [0.0, 1.0, 0.0, 0.0],
    }
}

/// Unit cube faces in block-local space, outward winding (normal = ev × eu).
fn cube(flags: u32) -> [GpuQuad; 6] {
    let (x, y, z) = (v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0));
    [
        quad(v3(1.0, 1.0, 0.0), -x, -y, flags), // north
        quad(v3(0.0, 1.0, 1.0), x, -y, flags),  // south
        quad(v3(1.0, 1.0, 1.0), -z, -y, flags), // east
        quad(v3(0.0, 1.0, 0.0), z, -y, flags),  // west
        quad(v3(0.0, 1.0, 0.0), x, z, flags),   // up
        quad(v3(0.0, 0.0, 1.0), x, -z, flags),  // down
    ]
}

struct TestScene {
    params: Params,
    grid: Vec<u32>,
    models: Vec<GpuModel>,
    quads: Vec<GpuQuad>,
    atlas: Vec<u32>,
}

impl TestScene {
    /// 3×1×1 bricks: brick 0 empty, brick 1 holds one cube at local (4,5,5),
    /// brick 2 is uniform cubes. An entity quad sits in brick 0 when `entity`.
    fn new(entity: bool) -> TestScene {
        let mut quads: Vec<GpuQuad> = cube(ALPHA_OPAQUE).to_vec();
        let models = vec![
            GpuModel::default(),
            GpuModel {
                quad_offset: 0,
                quad_count: 6,
                flags: 0,
                _pad: 0,
            },
        ];
        let nb = 3u32;
        let mut top = vec![EMPTY, 0, UNIFORM | 1];
        let mut brick = vec![0u32; BRICK_WORDS as usize];
        let local = ((5 * 16 + 5) * 16 + 4) as usize;
        brick[local >> 1] |= 1 << ((local & 1) * 16);
        let mut ov_top = vec![EMPTY; nb as usize];
        let mut ov_cells = Vec::new();
        let mut ov_quads = Vec::new();
        if entity {
            // A vertical quad facing -X at x = 8.5 spanning y,z 5..6 (cell (8,5,5)).
            quads.push(quad(
                v3(8.5, 6.0, 5.0),
                v3(0.0, 0.0, 1.0),
                v3(0.0, -1.0, 0.0),
                ALPHA_CUTOUT,
            ));
            ov_top[0] = 0;
            ov_cells = vec![0u32; OV_CELL_WORDS as usize];
            let cell = ((5 * 16 + 5) * 16 + 8) as usize;
            ov_cells[cell * 2] = 0;
            ov_cells[cell * 2 + 1] = 1;
            ov_quads.push(6);
        }
        let mut grid = Vec::new();
        let off_top = grid.len() as u32;
        grid.append(&mut top);
        let off_bricks = grid.len() as u32;
        grid.append(&mut brick);
        let off_ov_top = grid.len() as u32;
        grid.append(&mut ov_top);
        let off_ov_cells = grid.len() as u32;
        grid.append(&mut ov_cells);
        let off_ov_quads = grid.len() as u32;
        grid.append(&mut ov_quads);
        grid.push(0);
        let mut params = Params::default();
        params.grid_dims = [3, 1, 1, nb];
        params.off = [off_top, off_bricks, off_ov_top, off_ov_cells];
        params.off2 = [off_ov_quads, 2, 1, quads.len() as u32];
        params.misc_f = [1.0, 1e-3, 0.0, 0.0];
        TestScene {
            params,
            grid,
            models,
            quads,
            atlas: vec![0xFFFF_FFFF, 0x0000_0000],
        }
    }

    fn view(&self) -> SceneView<'_> {
        SceneView {
            params: &self.params,
            grid: &self.grid,
            models: &self.models,
            quads: &self.quads,
            atlas: &self.atlas,
        }
    }
}

#[test]
fn ray_skips_empty_brick_and_hits_cube() {
    let s = TestScene::new(false);
    let v = s.view();
    // Cube occupies x 20..21 (brick 1, local x 4), y 5..6, z 5..6.
    let h = trace(&v, v3(0.5, 5.5, 5.5), v3(1.0, 0.0, 0.0), 0.0, f32::INFINITY).expect("hit");
    assert!((h.t - 19.5).abs() < 1e-3, "t = {}", h.t);
    assert_eq!(h.quad, 3, "west face");
}

#[test]
fn ray_from_outside_grid_enters_it() {
    let s = TestScene::new(false);
    let v = s.view();
    let h = trace(
        &v,
        v3(-10.0, 5.5, 5.5),
        v3(1.0, 0.0, 0.0),
        0.0,
        f32::INFINITY,
    )
    .expect("hit");
    assert!((h.t - 30.0).abs() < 1e-3, "t = {}", h.t);
}

#[test]
fn ray_reaches_uniform_brick() {
    let s = TestScene::new(false);
    let v = s.view();
    // Miss the single cube (y = 9), then hit the uniform brick 2 at x = 32.
    let h = trace(&v, v3(0.5, 9.5, 5.5), v3(1.0, 0.0, 0.0), 0.0, f32::INFINITY).expect("hit");
    assert!((h.t - 31.5).abs() < 1e-3, "t = {}", h.t);
}

#[test]
fn ray_misses_everything() {
    let s = TestScene::new(false);
    let v = s.view();
    assert!(trace(&v, v3(0.5, 5.5, 5.5), v3(0.0, 1.0, 0.0), 0.0, f32::INFINITY).is_none());
    assert!(
        trace(
            &v,
            v3(0.5, 5.5, 5.5),
            v3(-1.0, 0.0, 0.0),
            0.0,
            f32::INFINITY
        )
        .is_none()
    );
    // tmax stops before the cube.
    assert!(trace(&v, v3(0.5, 5.5, 5.5), v3(1.0, 0.0, 0.0), 0.0, 10.0).is_none());
}

#[test]
fn oblique_rays_hit_cube_top() {
    let s = TestScene::new(false);
    let v = s.view();
    let o = v3(18.0, 9.0, 3.0);
    let target = v3(20.5, 6.0, 5.5);
    let d = (target - o).normalize();
    let h = trace(&v, o, d, 0.0, f32::INFINITY).expect("hit");
    let p = o + d * h.t;
    assert!(
        (p.y - 6.0).abs() < 1e-3 && h.quad == 4,
        "hit {p:?} quad {}",
        h.quad
    );
}

#[test]
fn entity_overlay_quads_are_hit_and_alpha_tested() {
    let s = TestScene::new(true);
    let v = s.view();
    // Through the opaque texel (s < 0.5) of the entity quad at x = 8.5.
    let h = trace(
        &v,
        v3(0.5, 5.5, 5.25),
        v3(1.0, 0.0, 0.0),
        0.0,
        f32::INFINITY,
    )
    .expect("hit");
    assert!((h.t - 8.0).abs() < 1e-3, "t = {}", h.t);
    assert_eq!(h.quad, 6);
    // Through the transparent texel (s > 0.5): cutout lets the ray reach the cube.
    let h = trace(
        &v,
        v3(0.5, 5.5, 5.75),
        v3(1.0, 0.0, 0.0),
        0.0,
        f32::INFINITY,
    )
    .expect("hit");
    assert!((h.t - 19.5).abs() < 1e-3, "t = {}", h.t);
}

#[test]
fn front_only_quads_are_met_from_the_front_only() {
    let mut s = TestScene::new(true);
    // The entity quad faces -x.
    s.quads[6].p0[3] = f32::from_bits(ALPHA_CUTOUT | FLAG_FRONT_ONLY);
    let v = s.view();
    let front = trace(&v, v3(0.5, 5.5, 5.25), v3(1.0, 0.0, 0.0), 0.0, 16.0);
    assert_eq!(front.map(|h| h.quad), Some(6));
    let back = trace(
        &v,
        v3(12.0, 5.5, 5.25),
        v3(-1.0, 0.0, 0.0),
        0.0,
        f32::INFINITY,
    );
    assert!(back.is_none(), "{back:?}");
}

#[test]
fn a_see_through_quad_is_found_behind_what_hides_it() {
    let mut s = TestScene::new(true);
    s.params.see_through = [6, 1, 0, 0];
    let v = s.view();
    assert!(is_see_through(&v, 6) && !is_see_through(&v, 5) && !is_see_through(&v, 7));
    // Looking -x from x = 25.5: the cube's east face (x = 21) hides the quad at 8.5.
    let (o, d) = (v3(25.5, 5.5, 5.25), v3(-1.0, 0.0, 0.0));
    let h = trace(&v, o, d, 0.0, f32::INFINITY).expect("hit");
    assert_eq!(h.quad, 2);
    let st = see_through_hit(&v, o, d, h.t).expect("see-through");
    assert_eq!(st.quad, 6);
    assert!((st.t - 17.0).abs() < 1e-3, "t = {}", st.t);
    // Through its transparent texel there is nothing to find.
    assert!(see_through_hit(&v, v3(25.5, 5.5, 5.75), d, h.t).is_none());
    s.params.see_through = [0, 0, 0, 0];
    assert!(see_through_hit(&s.view(), o, d, h.t).is_none());
}

#[test]
fn empty_scene_shows_sky() {
    let mut s = TestScene::new(false);
    s.params.frame = [4, 4, 0, 4];
    s.params.cam_pos = [0.5, 12.0, 0.5, 0.5];
    s.params.cam_fwd = [0.0, 1.0, 0.0, 1.0];
    s.params.cam_right = [1.0, 0.0, 0.0, 0.0];
    s.params.cam_up = [0.0, 0.0, -1.0, 0.0];
    s.params.sky_zenith = [0.2, 0.4, 0.8, 1.0];
    s.params.sky_horizon = [0.5, 0.6, 0.7, 0.0];
    s.params.sun_dir = [0.0, -1.0, 0.0, 1.0];
    let v = s.view();
    for y in 0..4 {
        for x in 0..4 {
            let c = crate::render_sample(&v, x, y, 0);
            assert!(c.z > c.x && c.z > 0.3, "sky should be blue: {c:?}");
        }
    }
}

#[test]
fn rng_matches_reference_values() {
    // Pinned so the WGSL port (and any future change) can be checked against it.
    assert_eq!(crate::rng::pcg_hash(0), 129708002);
    assert_eq!(crate::rng::pcg_hash(1), 2831084092);
}
