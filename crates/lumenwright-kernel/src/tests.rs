//! Kernel tests on hand-built scenes (no game assets needed).

#![allow(clippy::field_reassign_with_default)]

use crate::math::{V3, v3};
use crate::scene::{
    ALPHA_CUTOUT, ALPHA_OPAQUE, BRICK_WORDS, EMITTER_GRID_HEADER, EMITTER_SAMPLING_ALL,
    EMITTER_SAMPLING_NONE, EMITTER_SAMPLING_ONE, EMITTER_SAMPLING_ONE_BLOCK, EMPTY,
    FLAG_FRONT_ONLY, GpuEmitter, GpuModel, GpuQuad, OV_CELL_WORDS, Params, SceneView, UNIFORM,
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
    emitters: Vec<GpuEmitter>,
    emitter_grid: Vec<u32>,
    sky: Vec<f32>,
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
            emitters: vec![GpuEmitter::default()],
            emitter_grid: vec![0; EMITTER_GRID_HEADER as usize],
            sky: Vec::new(),
        }
    }

    fn view(&self) -> SceneView<'_> {
        SceneView {
            params: &self.params,
            grid: &self.grid,
            models: &self.models,
            quads: &self.quads,
            atlas: &self.atlas,
            emitters: &self.emitters,
            emitter_grid: &self.emitter_grid,
            sky: &self.sky,
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
    // The Preetham sky for a sun at altitude 45 degrees, azimuth 45 degrees
    // (`lumenwright::render::sky::Preetham::new`).
    s.params.sky_sun = [0.5, std::f32::consts::FRAC_1_SQRT_2, 0.5, 1.0];
    s.params.sky_zenith = PREETHAM_45_ZENITH;
    s.params.perez_a = PREETHAM_PEREZ[0];
    s.params.perez_b = PREETHAM_PEREZ[1];
    s.params.perez_c = PREETHAM_PEREZ[2];
    s.params.perez_d = PREETHAM_PEREZ[3];
    s.params.perez_e = PREETHAM_PEREZ[4];
    s.params.sun_dir = [0.0, -1.0, 0.0, 1.0];
    let n = 128;
    s.params.misc_f[2] = n as f32;
    let mut sky = vec![0.0; ((n + 1) * (n + 1) * 3) as usize];
    crate::integrator::build_sky_cache(&s.view(), n, &mut sky);
    s.sky = sky;
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

/// The emittance of the test lamp.
const LAMP_EMITTANCE: f32 = 0.5;
/// The emitter intensity of the lamp scenes.
const LAMP_INTENSITY: f32 = 13.0;

/// A 16³ room floor of white cubes at y = 0 and a lamp: a block at (7, 4, 7) whose model
/// is one white 1×1 quad at its bottom facing down, emittance [`LAMP_EMITTANCE`]. It is
/// emitter 0 in the emitter grid, one 16-block cell. With `decoy`, a second emitter
/// replaces the floor cube at (2, 0, 2): a lamp quad facing up at the cube's bottom,
/// below the floor's top, which the floor never sees; it shares the cell.
fn lamp_scene(strategy: u32, decoy: bool) -> TestScene {
    let mut quads: Vec<GpuQuad> = cube(ALPHA_OPAQUE).to_vec();
    let mut lamp = quad(
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.0, 1.0),
        v3(1.0, 0.0, 0.0),
        ALPHA_OPAQUE,
    );
    lamp.ev[3] = LAMP_EMITTANCE;
    quads.push(lamp);
    let mut up = quad(
        v3(0.0, 0.0, 0.0),
        v3(1.0, 0.0, 0.0),
        v3(0.0, 0.0, 1.0),
        ALPHA_OPAQUE,
    );
    up.ev[3] = LAMP_EMITTANCE;
    quads.push(up);
    // White texels only: the second atlas texel is transparent.
    for q in &mut quads {
        q.uv[2] = 1.0;
    }
    let model = |quad_offset| GpuModel {
        quad_offset,
        quad_count: if quad_offset == 0 { 6 } else { 1 },
        flags: 0,
        _pad: 0,
    };
    let models = vec![GpuModel::default(), model(0), model(6), model(7)];
    let mut brick = vec![0u32; BRICK_WORDS as usize];
    let mut put = |x: usize, y: usize, z: usize, m: u32| {
        let local = (y * 16 + z) * 16 + x;
        brick[local >> 1] |= m << ((local & 1) * 16);
    };
    for z in 0..16usize {
        for x in 0..16usize {
            if !(decoy && x == 2 && z == 2) {
                put(x, 0, z, 1);
            }
        }
    }
    put(7, 4, 7, 2);
    if decoy {
        put(2, 0, 2, 3);
    }
    let mut grid = vec![0u32]; // top: brick 0
    let off_bricks = grid.len() as u32;
    grid.append(&mut brick);
    let off_ov_top = grid.len() as u32;
    grid.push(EMPTY);
    let off_ov_cells = grid.len() as u32;
    let off_ov_quads = grid.len() as u32;
    grid.push(0);
    let mut emitters = vec![GpuEmitter {
        pos: [7.0, 4.0, 7.0, f32::from_bits(6)],
        faces: [1, 0, 0, 0],
    }];
    if decoy {
        emitters.push(GpuEmitter {
            pos: [2.0, 0.0, 2.0, f32::from_bits(7)],
            faces: [1, 0, 0, 0],
        });
    }
    let mut emitter_grid = vec![16, 1, 1, 1, 0, 0, 0, 0];
    emitter_grid.extend([0, emitters.len() as u32]);
    emitter_grid.extend(0..emitters.len() as u32);
    let mut params = Params::default();
    params.grid_dims = [1, 1, 1, 1];
    params.off = [0, off_bricks, off_ov_top, off_ov_cells];
    params.off2 = [off_ov_quads, 2, 1, quads.len() as u32];
    params.misc_f = [LAMP_INTENSITY, 1e-3, 0.0, 0.0];
    params.misc_u = [7, 1, 4, emitters.len() as u32];
    params.emitter = [strategy, 0, 0, 0];
    params.frame = [8, 8, 0, 3];
    params.sun_dir = [0.0, -1.0, 0.0, 1.0];
    TestScene {
        params,
        grid,
        models,
        quads,
        atlas: vec![0xFFFF_FFFF, 0x0000_0000],
        emitters,
        emitter_grid,
        sky: Vec::new(),
    }
}

/// Chunky's emitter sampling at a floor point under the lamp, averaged over many draws,
/// against its expectation: π × intensity × emittance × ∫ |cos θ| / max(d², 1) dA over
/// the lamp (the colour is white and nothing is in the way), integrated in `f64`.
fn floor_emitter_light(s: &TestScene, draws: u32) -> f64 {
    let v = s.view();
    let pos = v3(7.3, 1.0, 7.6);
    let n = v3(0.0, 1.0, 0.0);
    let mut rng = crate::rng::Rng::new(3, 0, 11);
    let mut sum = 0.0f64;
    for _ in 0..draws {
        let c = crate::integrator::sample_emitters(&v, pos, pos + n * 1e-3, n, &mut rng);
        sum += c.y as f64;
    }
    sum / draws as f64
}

fn floor_emitter_expectation() -> f64 {
    let (px, py, pz) = (7.3f64, 1.001f64, 7.6f64);
    let k = 400;
    let mut acc = 0.0;
    for i in 0..k {
        for j in 0..k {
            let (x, z) = (
                7.0 + (i as f64 + 0.5) / k as f64,
                7.0 + (j as f64 + 0.5) / k as f64,
            );
            let d = [x - px, 4.0 - py, z - pz];
            let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            acc += (d[1] / d2.sqrt()) / d2.max(1.0) / (k * k) as f64;
        }
    }
    std::f64::consts::PI * LAMP_INTENSITY as f64 * LAMP_EMITTANCE as f64 * acc
}

/// Chunky's emitter sampling (`ONE`, `ONE_BLOCK` and `ALL` alike with one emitter of one
/// face) converges to its expectation; with `NONE` it gives nothing. One standard error
/// at 200 000 draws is far below the 1% bound.
#[test]
fn emitter_sampling_is_chunkys_estimate() {
    let want = floor_emitter_expectation();
    for strategy in [
        EMITTER_SAMPLING_ONE,
        EMITTER_SAMPLING_ONE_BLOCK,
        EMITTER_SAMPLING_ALL,
    ] {
        let got = floor_emitter_light(&lamp_scene(strategy, false), 200_000);
        eprintln!("emitter sampling {strategy}: {got:.5} against {want:.5}");
        assert!(
            ((got - want) / want).abs() < 0.01,
            "{strategy}: {got} vs {want}"
        );
    }
    assert_eq!(
        floor_emitter_light(&lamp_scene(EMITTER_SAMPLING_NONE, false), 1000),
        0.0
    );
}

/// Chunky picks one emitter of the cell uniformly and does not divide by the pick's
/// probability (`Grid.sampleEmitterPosition`), so an emitter the floor cannot see in the
/// same cell halves the light `ONE` brings, and `ALL` averages over the cell alike.
#[test]
fn an_unseen_emitter_in_the_cell_halves_the_sampled_light() {
    let want = floor_emitter_expectation() / 2.0;
    for strategy in [EMITTER_SAMPLING_ONE, EMITTER_SAMPLING_ALL] {
        let got = floor_emitter_light(&lamp_scene(strategy, true), 200_000);
        eprintln!("with a decoy, emitter sampling {strategy}: {got:.5} against {want:.5}");
        assert!(
            ((got - want) / want).abs() < 0.02,
            "{strategy}: {got} vs {want}"
        );
    }
}

/// A camera ray that meets an emitter sees its texel colour squared times its emittance
/// times the emitter intensity (Chunky's quadratic emitter mapping). With one bounce the
/// path ends at the next surface, which adds nothing (Chunky's ray depth), and an
/// emitter is not lit by emitter sampling, so the lamp's emitted light is all there is.
#[test]
fn an_emitter_seen_from_the_camera_shows_its_emitted_light() {
    let mut s = lamp_scene(EMITTER_SAMPLING_ONE, false);
    s.params.frame = [1, 1, 0, 1];
    s.params.cam_pos = [7.5, 2.0, 7.5, 0.01];
    s.params.cam_fwd = [0.0, 1.0, 0.0, 1.0];
    s.params.cam_right = [1.0, 0.0, 0.0, 0.0];
    s.params.cam_up = [0.0, 0.0, -1.0, 0.0];
    let v = s.view();
    let want = LAMP_EMITTANCE * LAMP_INTENSITY;
    for i in 0..16 {
        let c = crate::render_sample(&v, 0, 0, i);
        assert!(
            (c.x - want).abs() < 1e-4 && (c.y - want).abs() < 1e-4,
            "{c:?}"
        );
    }
}

#[test]
fn sin_cos_turns_is_within_its_bound_of_f64() {
    use crate::math::{MAX_TURN_ERROR, sin_cos_turns};
    let inside = (0..1u32 << 20).map(|i| i as f32 / (1u32 << 20) as f32);
    let outside = (0..20_000).map(|i| (i as f32 - 10_000.0) * 0.037_f32);
    let mut worst = 0.0f64;
    let mut n = 0;
    for u in inside.chain(outside) {
        let (s, c) = sin_cos_turns(u);
        let a = 2.0 * core::f64::consts::PI * u as f64;
        worst = worst
            .max((s as f64 - a.sin()).abs())
            .max((c as f64 - a.cos()).abs());
        n += 1;
    }
    eprintln!("sin_cos_turns: worst absolute error {worst:.3e} over {n} turns");
    assert_eq!(n, (1 << 20) + 20_000);
    assert!(
        worst <= MAX_TURN_ERROR as f64,
        "worst error {worst:.3e} exceeds {MAX_TURN_ERROR:e}"
    );
    // Quadrant boundaries are exact.
    assert_eq!(sin_cos_turns(0.0), (0.0, 1.0));
    assert_eq!(sin_cos_turns(0.25), (1.0, -0.0));
    assert_eq!(sin_cos_turns(0.5), (-0.0, -1.0));
    assert_eq!(sin_cos_turns(0.75), (-1.0, 0.0));
}

#[test]
fn ln_and_gamma_decoding_are_within_their_bounds_of_f64() {
    use crate::math::{GAMMA, MAX_GAMMA_ERROR, MAX_LN_ERROR, gamma_to_linear, ln};
    let mut worst_ln = 0.0f64;
    let mut n = 0;
    for i in 0..1u32 << 20 {
        let x = (1.0 / 255.0) + (1.0 - 1.0 / 255.0) * i as f32 / ((1u32 << 20) - 1) as f32;
        worst_ln = worst_ln.max((ln(x) as f64 - (x as f64).ln()).abs());
        n += 1;
    }
    // Every byte value a texel channel can hold, as relative error of the decoded value.
    let mut worst_gamma = 0.0f64;
    for k in 1..=255u32 {
        let c = k as f32 / 255.0;
        let exact = (c as f64).powf(GAMMA as f64);
        worst_gamma = worst_gamma.max(((gamma_to_linear(c) as f64 - exact) / exact).abs());
    }
    eprintln!(
        "ln: worst absolute error {worst_ln:.3e} over {n} values; gamma decoding: worst relative \
         error {worst_gamma:.3e} over 255 byte values"
    );
    assert_eq!(n, 1 << 20);
    assert!(worst_ln <= MAX_LN_ERROR as f64, "{worst_ln:.3e}");
    assert!(worst_gamma <= MAX_GAMMA_ERROR as f64, "{worst_gamma:.3e}");
    assert_eq!(gamma_to_linear(0.0), 0.0);
    assert_eq!(gamma_to_linear(1.0), 1.0);
}

#[test]
fn acos_is_within_its_bound_of_f64() {
    use crate::math::{MAX_ACOS_ERROR, acos};
    let mut worst = 0.0f64;
    let mut n = 0;
    for i in 0..=1u32 << 20 {
        let x = -1.0 + 2.0 * i as f32 / (1u32 << 20) as f32;
        worst = worst.max((acos(x) as f64 - (x as f64).acos()).abs());
        n += 1;
    }
    eprintln!("acos: worst absolute error {worst:.3e} over {n} values");
    assert_eq!(n, (1 << 20) + 1);
    assert!(worst <= MAX_ACOS_ERROR as f64, "{worst:.3e}");
}

#[test]
fn atan2_is_within_its_bound_of_f64() {
    use crate::math::{MAX_ATAN2_ERROR, atan2};
    let mut worst = 0.0f64;
    let mut n = 0;
    for i in 0..1u32 << 16 {
        let a = i as f32 / (1u32 << 16) as f32 * std::f32::consts::TAU;
        for r in [1e-3f32, 1.0, 250.0] {
            let (y, x) = (a.sin() * r, a.cos() * r);
            worst = worst.max((atan2(y, x) as f64 - (y as f64).atan2(x as f64)).abs());
            n += 1;
        }
    }
    for (y, x, want) in [
        (0.0f32, 1.0f32, 0.0f64),
        (1.0, 0.0, std::f64::consts::FRAC_PI_2),
        (0.0, 0.0, 0.0),
    ] {
        assert!((atan2(y, x) as f64 - want).abs() < 1e-7, "atan2({y}, {x})");
    }
    eprintln!("atan2: worst absolute error {worst:.3e} over {n} values");
    assert_eq!(n, 3 << 16);
    assert!(worst <= MAX_ATAN2_ERROR as f64, "{worst:.3e}");
}

/// `lumenwright::render::sky::Preetham::new` for a sun at altitude 45 degrees: the zenith
/// x, y and Y times their normalisation.
const PREETHAM_45_ZENITH: [f32; 4] = [0.296_242_06, 0.307_511_93, 8.306_274, 0.0];
/// The Perez coefficients A to E for x, y and Y at turbidity 2.5 (Chunky's `PreethamSky`).
const PREETHAM_PEREZ: [[f32; 4]; 5] = [
    [-0.307_45, -0.302_55, -1.016_25, 0.0],
    [-0.165_45, -0.228_3, -0.461, 0.0],
    [0.211_5, 0.190_45, 5.268_35, 0.0],
    [-1.059_15, -1.763_95, -2.275_6, 0.0],
    [0.036_95, 0.025_65, 0.202_8, 0.0],
];

/// A ray that crosses some 250 cells before it meets a front-only face at its cell's
/// entry still meets it: the traversal's cell boundaries must not drift past the margin
/// of the range each cell tests its quads in. Every brick holds a block, so the ray
/// steps cell by cell all the way.
#[test]
fn a_front_face_far_along_the_ray_is_met() {
    let quads: Vec<GpuQuad> = cube(ALPHA_OPAQUE | FLAG_FRONT_ONLY).to_vec();
    let models = vec![
        GpuModel::default(),
        GpuModel {
            quad_offset: 0,
            quad_count: 6,
            flags: 0,
            _pad: 0,
        },
    ];
    // 16 × 1 × 1 bricks. Bricks 0 to 14 hold one cube in a corner no ray passes; brick
    // 15 holds a wall of cubes at x = 250, y and z 0..16.
    let nb = 16u32;
    let mut top = vec![0u32; nb as usize];
    top[15] = 1;
    let mut stray = vec![0u32; BRICK_WORDS as usize];
    let corner = (15 * 16 + 15) * 16;
    stray[corner >> 1] |= 1 << ((corner & 1) * 16);
    let mut wall = vec![0u32; BRICK_WORDS as usize];
    for y in 0..16usize {
        for z in 0..16usize {
            let local = (y * 16 + z) * 16 + (250 - 240);
            wall[local >> 1] |= 1 << ((local & 1) * 16);
        }
    }
    let mut grid = top;
    let off_bricks = grid.len() as u32;
    grid.append(&mut stray);
    grid.append(&mut wall);
    let off_ov_top = grid.len() as u32;
    grid.extend(vec![EMPTY; nb as usize]);
    let off_ov_cells = grid.len() as u32;
    grid.push(0);
    let mut params = Params::default();
    params.grid_dims = [16, 1, 1, nb];
    params.off = [0, off_bricks, off_ov_top, off_ov_cells];
    params.off2 = [off_ov_cells, 2, 1, quads.len() as u32];
    let s = TestScene {
        params,
        grid,
        models,
        quads,
        atlas: vec![0xFFFF_FFFF, 0x0000_0000],
        emitters: vec![GpuEmitter::default()],
        emitter_grid: vec![0; EMITTER_GRID_HEADER as usize],
        sky: Vec::new(),
    };
    let v = s.view();
    let mut missed = 0;
    let mut n = 0;
    for i in 0..64 {
        for j in 0..64 {
            let o = v3(
                0.137 + i as f32 * 0.0131,
                1.1 + i as f32 * 0.03,
                1.3 + j as f32 * 0.05,
            );
            let d = v3(
                1.0,
                0.04 + 0.002 * (j as f32 - 32.0) / 32.0,
                0.03 + 0.002 * (i as f32 - 32.0) / 32.0,
            )
            .normalize();
            match trace(&v, o, d, 0.0, f32::INFINITY) {
                Some(h) if (o + d * h.t).x > 249.99 && (o + d * h.t).x < 250.01 => {}
                _ => missed += 1,
            }
            n += 1;
        }
    }
    assert_eq!(n, 4096);
    assert_eq!(
        missed, 0,
        "{missed} of {n} rays missed the wall 250 blocks away"
    );
}

/// Of two coplanar quads of a model, the later one is met (a grass block's tinted side
/// overlay over its side), whichever way the ray comes.
#[test]
fn a_later_coplanar_quad_is_met() {
    let mut s = TestScene::new(false);
    // Model 1 is the cube's six faces at quads 0..6; add copies of its up and north
    // faces at 6 and 7.
    let (up, north) = (s.quads[4], s.quads[0]);
    s.quads.push(up);
    s.quads.push(north);
    s.models[1].quad_count = 8;
    s.params.off2[3] = s.quads.len() as u32;
    let v = s.view();
    // The cube is at brick 1, local (4, 5, 5): grid (20, 5, 5).
    let mut n = 0;
    for i in 0..16 {
        let f = 0.1 + 0.7 * (i as f32 + 0.5) / 16.0;
        let down = trace(
            &v,
            v3(20.0 + f, 9.0, 5.0 + f),
            v3(0.01, -1.0, 0.02).normalize(),
            0.0,
            100.0,
        );
        assert_eq!(down.map(|h| h.quad), Some(6), "from above at {f}");
        let south = trace(
            &v,
            v3(20.0 + f, 5.0 + f, 1.0),
            v3(0.02, 0.01, 1.0).normalize(),
            0.0,
            100.0,
        );
        assert_eq!(south.map(|h| h.quad), Some(7), "from the north at {f}");
        n += 2;
    }
    assert_eq!(n, 32);
}

/// A scene whose sky is two maps: the lens's 4 × 2 (texel `(i, j)` holds `(i, j, 1)`) and
/// the light's 2 × 2 (`(10 + i, 10 + j, 2)`).
fn map_scene() -> TestScene {
    let mut s = TestScene::new(false);
    s.params.sky_map = [4.0, 2.0, 2.0, 2.0];
    let mut sky = Vec::new();
    for j in 0..2 {
        for i in 0..4 {
            sky.extend([i as f32, j as f32, 1.0]);
        }
    }
    for j in 0..2 {
        for i in 0..2 {
            sky.extend([10.0 + i as f32, 10.0 + j as f32, 2.0]);
        }
    }
    s.sky = sky;
    s
}

/// A unit direction at `turns` of azimuth from +x towards +z and `el` radians up.
fn sky_dir(turns: f32, el: f32) -> V3 {
    let az = turns * std::f32::consts::TAU;
    v3(el.cos() * az.cos(), el.sin(), el.cos() * az.sin())
}

fn near(a: V3, b: V3, tol: f32) -> bool {
    (a.x - b.x).abs() <= tol && (a.y - b.y).abs() <= tol && (a.z - b.z).abs() <= tol
}

/// Each map is read at its texel centres exactly and bilinearly between them; the
/// azimuth wraps across the seam at +x, the height holds at the outer rows' centres, and
/// the light's map is the one after the lens's in the buffer.
#[test]
fn the_sky_maps_are_read_bilinearly() {
    use crate::integrator::{sky_lens, sky_light};
    use core::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    let s = map_scene();
    let v = s.view();
    let mut checked = 0;
    for j in 0..2 {
        for i in 0..4 {
            let d = sky_dir(
                (i as f32 + 0.5) / 4.0,
                (j as f32 + 0.5) * FRAC_PI_2 - FRAC_PI_2,
            );
            let c = sky_lens(&v, d);
            assert!(
                near(c, v3(i as f32, j as f32, 1.0), 1e-3),
                "({i}, {j}): {c:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 8);
    // Across the seam, half way between the last column and the first.
    let c = sky_lens(&v, sky_dir(0.0, -FRAC_PI_4));
    assert!(near(c, v3(1.5, 0.0, 1.0), 1e-3), "{c:?}");
    // Half way between the rows, and at the zenith and the nadir, held.
    let c = sky_lens(&v, sky_dir(0.125, 0.0));
    assert!(near(c, v3(0.0, 0.5, 1.0), 1e-3), "{c:?}");
    assert!(near(
        sky_lens(&v, v3(0.0, 1.0, 0.0)),
        v3(1.5, 1.0, 1.0),
        1e-3
    ));
    assert!(near(
        sky_lens(&v, v3(0.0, -1.0, 0.0)),
        v3(1.5, 0.0, 1.0),
        1e-3
    ));
    // The light's map.
    let c = sky_light(&v, sky_dir(0.25, FRAC_PI_4));
    assert!(near(c, v3(10.0, 11.0, 2.0), 1e-3), "{c:?}");
    let c = sky_light(&v, sky_dir(0.5, 0.0));
    assert!(near(c, v3(10.5, 10.5, 2.0), 1e-3), "{c:?}");
}

/// Under the maps a camera ray that meets the sky sees the lens's map, and a surface is
/// lit by the light's map alone: a white top face under a uniform light map of 0.3 shows
/// 0.3 in every sample, whatever the lens's map holds, with no sun light.
#[test]
fn the_lens_sees_one_map_and_a_bounce_is_lit_by_the_other() {
    let mut s = TestScene::new(false);
    s.params.sky_map = [4.0, 2.0, 2.0, 2.0];
    s.sky = [vec![5.0f32; 4 * 2 * 3], vec![0.3f32; 2 * 2 * 3]].concat();
    s.params.frame = [1, 1, 0, 1];
    s.params.cam_pos = [40.25, 20.0, 8.25, 1e-3];
    s.params.cam_right = [1.0, 0.0, 0.0, 0.0];
    s.params.sun_dir = [0.0, 1.0, 0.0, 0.9995];
    // Down, at the uniform brick's top (y = 16).
    s.params.cam_fwd = [0.0, -1.0, 0.0, 1.0];
    s.params.cam_up = [0.0, 0.0, -1.0, 0.0];
    for sample in 0..16 {
        let c = crate::render_sample(&s.view(), 0, 0, sample);
        assert!(near(c, v3(0.3, 0.3, 0.3), 1e-4), "{sample}: {c:?}");
    }
    // Up, at the sky.
    s.params.cam_fwd = [0.0, 1.0, 0.0, 1.0];
    s.params.cam_up = [0.0, 0.0, 1.0, 0.0];
    let c = crate::render_sample(&s.view(), 0, 0, 0);
    assert!(near(c, v3(5.0, 5.0, 5.0), 1e-4), "{c:?}");
    // A sun that casts light lights the face too; the same with sun colour 0 does not.
    s.params.cam_fwd = [0.0, -1.0, 0.0, 1.0];
    s.params.cam_up = [0.0, 0.0, -1.0, 0.0];
    s.params.sun_color = [1.0, 1.0, 1.0, 0.03];
    let lit = crate::render_sample(&s.view(), 0, 0, 0);
    assert!(lit.x > 1.0, "{lit:?}");
}
