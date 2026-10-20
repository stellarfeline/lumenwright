// HQ path tracer. Line-by-line mirror of crates/lumenwright-kernel/src/{trace,integrator}.rs.
// Buffer layouts are defined in crates/lumenwright-kernel/src/scene.rs.

struct Params {
    cam_pos: vec4<f32>,
    cam_fwd: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_sun: vec4<f32>,
    sky_zenith: vec4<f32>,
    perez_a: vec4<f32>,
    perez_b: vec4<f32>,
    perez_c: vec4<f32>,
    perez_d: vec4<f32>,
    perez_e: vec4<f32>,
    water_color: vec4<f32>,
    misc_f: vec4<f32>,
    grid_dims: vec4<u32>,
    off: vec4<u32>,
    off2: vec4<u32>,
    frame: vec4<u32>,
    misc_u: vec4<u32>,
    see_through: vec4<u32>,
    emitter: vec4<u32>,
}

// Quads are read as raw words: the w lanes of p0/eu carry bit-cast integers
// (flags, tint) that must not go through float loads.
struct Quad {
    p0: vec4<u32>,
    eu: vec4<u32>,
    ev: vec4<u32>,
    uv: vec4<f32>,
    uvt: vec4<f32>,
}

struct Emitter {
    pos: vec4<u32>,
    faces: vec4<u32>,
}

struct Model {
    quad_offset: u32,
    quad_count: u32,
    flags: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> grid: array<u32>;
@group(0) @binding(2) var<storage, read> models: array<Model>;
@group(0) @binding(3) var<storage, read> quads: array<Quad>;
@group(0) @binding(4) var<storage, read> atlas: array<u32>;
@group(0) @binding(5) var<storage, read_write> accum: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read> emitters: array<Emitter>;
@group(0) @binding(7) var<storage, read> emitter_grid: array<u32>;

const EMPTY: u32 = 0xFFFFFFFFu;
const UNIFORM: u32 = 0x80000000u;
const BRICK_WORDS: u32 = 2048u;
const OV_CELL_WORDS: u32 = 8192u;
const NO_QUAD: u32 = 0xFFFFFFFFu;
const ALPHA_CUTOUT: u32 = 1u;
const ALPHA_TRANSLUCENT: u32 = 2u;
const FLAG_WATER: u32 = 4u;
const FLAG_FRONT_ONLY: u32 = 16u;
const EMITTER_SAMPLING_NONE: u32 = 0u;
const EMITTER_SAMPLING_ONE: u32 = 1u;
const EMITTER_SAMPLING_ONE_BLOCK: u32 = 2u;
const EMITTER_SAMPLING_ALL: u32 = 3u;
const EMITTER_GRID_HEADER: u32 = 8u;
const PI: f32 = 3.14159265358979;
const INF: f32 = 1e30;
const MAX_EVENTS: u32 = 48u;
const MAX_STEPS: u32 = 65536u;

// ---------------------------------------------------------------- rng / utils

var<private> rng_state: u32;

fn pcg_hash(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rnd() -> f32 {
    rng_state = pcg_hash(rng_state);
    return f32(rng_state >> 8u) * (1.0 / 16777216.0);
}

// math::GAMMA and math::gamma_to_linear (Chunky's texture decoding: c^2.2).
const GAMMA: f32 = 2.2;

fn gamma_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(exp(GAMMA * log(max(c, vec3<f32>(1e-30)))), vec3<f32>(0.0), c <= vec3<f32>(0.0));
}

fn unpack_rgba(c: u32) -> vec4<f32> {
    return vec4<f32>(f32(c & 0xFFu), f32((c >> 8u) & 0xFFu), f32((c >> 16u) & 0xFFu), f32(c >> 24u)) / 255.0;
}

fn linear_rgb(c: u32) -> vec3<f32> {
    return gamma_to_linear(unpack_rgba(c).xyz);
}

fn q_p0(q: Quad) -> vec3<f32> { return bitcast<vec3<f32>>(q.p0.xyz); }
fn q_eu(q: Quad) -> vec3<f32> { return bitcast<vec3<f32>>(q.eu.xyz); }
fn q_ev(q: Quad) -> vec3<f32> { return bitcast<vec3<f32>>(q.ev.xyz); }
fn q_flags(q: Quad) -> u32 { return q.p0.w; }
fn q_tint(q: Quad) -> u32 { return q.eu.w; }
fn q_emittance(q: Quad) -> f32 { return bitcast<f32>(q.ev.w); }
fn q_normal(q: Quad) -> vec3<f32> { return cross(q_ev(q), q_eu(q)); }

fn onb_t(n: vec3<f32>) -> vec3<f32> {
    let sign = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    return vec3<f32>(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x);
}

fn onb_b(n: vec3<f32>) -> vec3<f32> {
    let sign = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    return vec3<f32>(b, sign + n.y * n.y * a, -n.y);
}

fn safe_inv(x: f32) -> f32 {
    if (abs(x) < 1e-12) {
        return select(1e12, -1e12, x < 0.0);
    }
    return 1.0 / x;
}

fn sgn(x: f32) -> i32 {
    if (x > 0.0) { return 1; }
    if (x < 0.0) { return -1; }
    return 0;
}

fn next_boundary(o: f32, inv: f32, cell: i32, step: i32) -> f32 {
    if (step == 0) { return INF; }
    let bound = select(cell, cell + 1, step > 0);
    return (f32(bound) - o) * inv;
}

// ---------------------------------------------------------------- traversal

struct Hit {
    t: f32,
    quad: u32,
    texel: u32,
    s: f32,
    tt: f32,
}

fn texel_at(u: f32, v: f32) -> u32 {
    let w = i32(P.off2.y);
    let h = i32(P.off2.z);
    let x = clamp(i32(floor(u)), 0, w - 1);
    let y = clamp(i32(floor(v)), 0, h - 1);
    return atlas[u32(y * w + x)];
}

fn quad_texel(q: Quad, s: f32, t: f32) -> u32 {
    let sc = min(max(s, 0.0005), 0.9995);
    let tc = min(max(t, 0.0005), 0.9995);
    let u = q.uv.x + sc * q.uv.z + tc * q.uvt.x;
    let v = q.uv.y + sc * q.uv.w + tc * q.uvt.y;
    return texel_at(u, v);
}

fn test_quad(qi: u32, o: vec3<f32>, d: vec3<f32>, tmin: f32, best: ptr<function, Hit>) {
    let q = quads[qi];
    let eu = q_eu(q);
    let ev = q_ev(q);
    let n = cross(eu, ev);
    let denom = dot(n, d);
    if (abs(denom) < 1e-12) { return; }
    let p0 = q_p0(q);
    let t = dot(n, p0 - o) / denom;
    if (!(t > tmin && t < (*best).t)) { return; }
    let p = o + d * t - p0;
    let nn = dot(n, n);
    let s = dot(cross(p, ev), n) / nn;
    let tt = dot(cross(eu, p), n) / nn;
    if (s < 0.0 || s > 1.0 || tt < 0.0 || tt > 1.0) { return; }
    if ((q_flags(q) & FLAG_FRONT_ONLY) != 0u && dot(q_normal(q), d) >= 0.0) { return; }
    let tex = quad_texel(q, s, tt);
    let alpha = tex >> 24u;
    let mode = q_flags(q) & 3u;
    if ((mode == ALPHA_CUTOUT && alpha < 128u) || (mode == ALPHA_TRANSLUCENT && alpha == 0u)) { return; }
    (*best) = Hit(t, qi, tex, s, tt);
}

fn is_see_through(qi: u32) -> bool {
    return qi >= P.see_through.x && qi - P.see_through.x < P.see_through.y;
}

fn see_through_hit(o: vec3<f32>, d: vec3<f32>, tmin: f32) -> Hit {
    var best = Hit(INF, NO_QUAD, 0u, 0.0, 0.0);
    for (var i = 0u; i < P.see_through.y; i++) {
        test_quad(P.see_through.x + i, o, d, tmin, &best);
    }
    return best;
}

fn trace(o: vec3<f32>, d: vec3<f32>, tmin: f32, tmax: f32) -> Hit {
    var result = Hit(INF, NO_QUAD, 0u, 0.0, 0.0);
    let dims = P.grid_dims.xyz;
    let size = vec3<f32>(f32(dims.x * 16u), f32(dims.y * 16u), f32(dims.z * 16u));
    let inv = vec3<f32>(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z));
    let ta = (vec3<f32>(0.0) - o) * inv;
    let tb = (size - o) * inv;
    let t_enter = max(max(min(ta.x, tb.x), min(ta.y, tb.y)), min(ta.z, tb.z));
    let t_exit = min(min(max(ta.x, tb.x), max(ta.y, tb.y)), max(ta.z, tb.z));
    var t = max(t_enter, 0.0);
    let t_end = min(t_exit, tmax);
    if (t > t_end) { return result; }

    let start = o + d * (t + 1e-4);
    let gs = vec3<i32>(i32(dims.x) * 16, i32(dims.y) * 16, i32(dims.z) * 16);
    var cell = vec3<i32>(
        clamp(i32(floor(start.x)), 0, gs.x - 1),
        clamp(i32(floor(start.y)), 0, gs.y - 1),
        clamp(i32(floor(start.z)), 0, gs.z - 1));
    let step = vec3<i32>(sgn(d.x), sgn(d.y), sgn(d.z));
    var next = vec3<f32>(
        next_boundary(o.x, inv.x, cell.x, step.x),
        next_boundary(o.y, inv.y, cell.y, step.y),
        next_boundary(o.z, inv.z, cell.z, step.z));
    let best_t = t_end + 1e-3;

    for (var iter = 0u; iter < MAX_STEPS; iter++) {
        let bc = cell >> vec3<u32>(4u);
        let b = (u32(bc.y) * dims.z + u32(bc.z)) * dims.x + u32(bc.x);
        let entry = grid[P.off.x + b];
        let ov = grid[P.off.z + b];

        if (entry == EMPTY && ov == EMPTY) {
            var t_out = INF;
            var axis = 0;
            for (var a = 0; a < 3; a++) {
                if (step[a] != 0) {
                    let bound = select(bc[a] * 16, (bc[a] + 1) * 16, step[a] > 0);
                    let tt = (f32(bound) - o[a]) * inv[a];
                    if (tt < t_out) {
                        t_out = tt;
                        axis = a;
                    }
                }
            }
            if (t_out > t_end) { break; }
            let pos = o + d * t_out;
            for (var a = 0; a < 3; a++) {
                if (a == axis) {
                    cell[a] = select(bc[a] * 16 - 1, (bc[a] + 1) * 16, step[a] > 0);
                } else {
                    cell[a] = clamp(i32(floor(pos[a])), bc[a] * 16, bc[a] * 16 + 15);
                }
            }
            if (cell[axis] < 0 || cell[axis] >= gs[axis]) { break; }
            next = vec3<f32>(
                next_boundary(o.x, inv.x, cell.x, step.x),
                next_boundary(o.y, inv.y, cell.y, step.y),
                next_boundary(o.z, inv.z, cell.z, step.z));
            t = t_out;
            continue;
        }

        let t_cell_exit = min(next.x, min(next.y, next.z));
        let t_lo = max(t - 1e-4, tmin);
        let t_hi = t_cell_exit + 1e-4;
        let local = u32(((cell.y & 15) * 16 + (cell.z & 15)) * 16 + (cell.x & 15));

        var model = 0u;
        if (entry != EMPTY) {
            if ((entry & UNIFORM) != 0u) {
                model = entry & ~UNIFORM;
            } else {
                let w = grid[P.off.y + entry * BRICK_WORDS + (local >> 1u)];
                model = (w >> ((local & 1u) * 16u)) & 0xFFFFu;
            }
        }
        var cand = Hit(min(t_hi, best_t), NO_QUAD, 0u, 0.0, 0.0);
        if (model != 0u) {
            let m = models[model];
            let lo = o - vec3<f32>(f32(cell.x), f32(cell.y), f32(cell.z));
            for (var i = 0u; i < m.quad_count; i++) {
                test_quad(m.quad_offset + i, lo, d, t_lo, &cand);
            }
        }
        if (ov != EMPTY) {
            let base = P.off.w + ov * OV_CELL_WORDS + local * 2u;
            let off = grid[base];
            let count = grid[base + 1u];
            for (var i = 0u; i < count; i++) {
                let qi = grid[P.off2.x + off + i];
                test_quad(qi, o, d, t_lo, &cand);
            }
        }
        if (cand.quad != NO_QUAD) {
            return cand;
        }

        var axis = 2;
        if (next.x < next.y) {
            if (next.x < next.z) { axis = 0; } else { axis = 2; }
        } else if (next.y < next.z) {
            axis = 1;
        }
        t = next[axis];
        if (t > t_end) { break; }
        cell[axis] += step[axis];
        if (cell[axis] < 0 || cell[axis] >= gs[axis]) { break; }
        next[axis] = next_boundary(o[axis], inv[axis], cell[axis], step[axis]);
    }
    return result;
}

// ---------------------------------------------------------------- integrator

// math::acos: Abramowitz and Stegun 4.4.46.
fn acos_as(x: f32) -> f32 {
    let a = clamp(abs(x), 0.0, 1.0);
    let p = 1.5707963
        + a * (-0.2145988
            + a * (0.08897899
                + a * (-0.0501743
                    + a * (0.03089188
                        + a * (-0.017088126 + a * (0.00667009 + a * -0.0012624911))))));
    let r = sqrt(1.0 - a) * p;
    return select(r, PI - r, x < 0.0);
}

// integrator::sky_radiance: Chunky's Preetham sky.
fn sky_radiance(d: vec3<f32>) -> vec3<f32> {
    let cos_theta = max(d.y + P.sky_zenith.w, 0.0);
    let cos_gamma = clamp(dot(d, P.sky_sun.xyz), -1.0, 1.0);
    let gamma = acos_as(cos_gamma);
    let cos2_gamma = cos_gamma * cos_gamma;
    var a = vec3<f32>(0.0);
    if (cos_theta > 0.0) {
        a = P.perez_a.xyz * exp(P.perez_b.xyz / cos_theta);
    }
    let perez = (1.0 + a) * (1.0 + P.perez_c.xyz * exp(P.perez_d.xyz * gamma) + P.perez_e.xyz * cos2_gamma);
    let xyl = P.sky_zenith.xyz * perez;
    if (xyl.y <= 5e-8) {
        return vec3<f32>(0.0);
    }
    let f = xyl.z / xyl.y;
    let cx = xyl.x * f;
    let cy = xyl.z;
    let cz = (1.0 - xyl.x - xyl.y) * f;
    return vec3<f32>(
        2.3706743 * cx - 0.9000405 * cy - 0.4706338 * cz,
        -0.513885 * cx + 1.4253036 * cy + 0.0885814 * cz,
        0.0052982 * cx - 0.0146949 * cy + 1.0093968 * cz) * (0.045 * P.sky_sun.w);
}

fn sun_disk(d: vec3<f32>) -> vec3<f32> {
    if (dot(d, P.sun_dir.xyz) >= P.sun_dir.w) {
        return P.sun_color.xyz * P.misc_f.z;
    }
    return vec3<f32>(0.0);
}

fn cosine_hemisphere(n: vec3<f32>) -> vec3<f32> {
    let r1 = rnd();
    let r2 = rnd();
    let r = sqrt(r1);
    let phi = 2.0 * PI * r2;
    return normalize(onb_t(n) * (r * cos(phi)) + onb_b(n) * (r * sin(phi)) + n * sqrt(max(1.0 - r1, 0.0)));
}

fn sample_sun() -> vec3<f32> {
    let sun = P.sun_dir.xyz;
    let cos_max = P.sun_dir.w;
    let r1 = rnd();
    let r2 = rnd();
    let cos_t = 1.0 - r1 * (1.0 - cos_max);
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let phi = 2.0 * PI * r2;
    return normalize(onb_t(sun) * (sin_t * cos(phi)) + onb_b(sun) * (sin_t * sin(phi)) + sun * cos_t);
}

fn transmittance(sigma: vec3<f32>, t: f32) -> vec3<f32> {
    return exp(-sigma * t);
}

fn shadow(o_in: vec3<f32>, d: vec3<f32>, tmax_in: f32) -> vec3<f32> {
    let eps = P.misc_f.y;
    var o = o_in;
    var tmax = tmax_in;
    var tr = vec3<f32>(1.0);
    for (var i = 0; i < 8; i++) {
        let h = trace(o, d, eps, tmax);
        if (h.quad == NO_QUAD) {
            return tr;
        }
        let q = quads[h.quad];
        let flags = q_flags(q);
        if ((flags & FLAG_WATER) != 0u) {
            if (dot(q_normal(q), d) > 0.0) {
                tr = tr * transmittance(P.water_color.xyz, h.t);
            }
        } else if ((flags & 3u) == ALPHA_TRANSLUCENT) {
            let a = f32(h.texel >> 24u) / 255.0;
            let col = linear_rgb(h.texel) * linear_rgb(q_tint(q));
            tr = tr * mix(vec3<f32>(1.0), col, a);
        } else {
            return vec3<f32>(0.0);
        }
        o = o + d * (h.t + eps);
        tmax -= h.t + eps;
    }
    return tr;
}

fn pick(n: u32, u: f32) -> u32 {
    let i = u32(u * f32(n));
    return select(n - 1u, i, i < n);
}

// integrator::emitter_cell: (start, count) of the emitters near grid-space point p.
fn emitter_cell(p: vec3<f32>) -> vec2<u32> {
    let cell = f32(emitter_grid[0]);
    let size = vec3<i32>(i32(emitter_grid[1]), i32(emitter_grid[2]), i32(emitter_grid[3]));
    let c = vec3<i32>(
        i32(floor(p.x / cell)) - bitcast<i32>(emitter_grid[4]),
        i32(floor(p.y / cell)) - bitcast<i32>(emitter_grid[5]),
        i32(floor(p.z / cell)) - bitcast<i32>(emitter_grid[6]));
    if (any(c < vec3<i32>(0)) || any(c >= size)) {
        return vec2<u32>(0u, 0u);
    }
    let i = u32((c.y * size.z + c.z) * size.x + c.x);
    let at = EMITTER_GRID_HEADER + i * 2u;
    return vec2<u32>(emitter_grid[at], emitter_grid[at + 1u]);
}

// integrator::sample_emitter_face: Chunky's PathTracer.sampleEmitterFace.
fn sample_emitter_face(origin: vec3<f32>, n: vec3<f32>, e: Emitter, face: u32, scaler: f32) -> vec3<f32> {
    let q = quads[e.pos.w + face];
    let s = rnd();
    let t = rnd();
    let at = bitcast<vec3<f32>>(e.pos.xyz) + q_p0(q) + q_eu(q) * s + q_ev(q) * t;
    let to = at - origin;
    if (dot(to, n) <= 0.0) { return vec3<f32>(0.0); }
    let dist = length(to);
    let l = to / dist;
    let eps = P.misc_f.y;
    let h = trace(origin, l, eps, dist + 2.0 * eps);
    if (h.quad == NO_QUAD) { return vec3<f32>(0.0); }
    if (abs(h.t - dist) >= 2.0 * eps) { return vec3<f32>(0.0); }
    let nq = q_normal(q);
    let area = length(nq);
    let cos_l = abs(dot(nq, l)) / area;
    let c = linear_rgb(h.texel) * linear_rgb(q_tint(quads[h.quad]));
    return c * (cos_l / max(dist * dist, 1.0) * area * q_emittance(q) * P.misc_f.x * scaler);
}

// integrator::sample_emitter.
fn sample_emitter(origin: vec3<f32>, n: vec3<f32>, e: Emitter) -> vec3<f32> {
    let faces = e.faces.x;
    if (faces == 0u) { return vec3<f32>(0.0); }
    if (P.emitter.x == EMITTER_SAMPLING_ONE) {
        let f = pick(faces, rnd());
        return sample_emitter_face(origin, n, e, f, 1.0);
    }
    var sum = vec3<f32>(0.0);
    for (var f = 0u; f < faces; f++) {
        sum += sample_emitter_face(origin, n, e, f, 1.0 / f32(faces));
    }
    return sum;
}

// integrator::sample_emitters: Chunky's emitter sampling in doDiffuseReflection.
fn sample_emitters(pos: vec3<f32>, origin: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let sc = emitter_cell(pos);
    let start = sc.x;
    let count = sc.y;
    if (count == 0u) { return vec3<f32>(0.0); }
    let list = EMITTER_GRID_HEADER + emitter_grid[1] * emitter_grid[2] * emitter_grid[3] * 2u + start;
    let strategy = P.emitter.x;
    if (strategy == EMITTER_SAMPLING_ONE || strategy == EMITTER_SAMPLING_ONE_BLOCK) {
        let i = emitter_grid[list + pick(count, rnd())];
        return sample_emitter(origin, n, emitters[i]) * PI;
    }
    if (strategy == EMITTER_SAMPLING_ALL) {
        var sum = vec3<f32>(0.0);
        for (var k = 0u; k < count; k++) {
            let i = emitter_grid[list + k];
            sum += sample_emitter(origin, n, emitters[i]);
        }
        return sum * (PI / f32(count));
    }
    return vec3<f32>(0.0);
}

fn render_sample(px: u32, py: u32, sample: u32) -> vec3<f32> {
    let width = P.frame.x;
    let height = P.frame.y;
    let max_bounces = P.frame.w;
    let eps = P.misc_f.y;
    rng_state = pcg_hash((py * width + px) ^ pcg_hash(sample + pcg_hash(P.misc_u.x)));

    let tan_half = P.cam_pos.w;
    let aspect = P.cam_fwd.w;
    let jx = rnd();
    let jy = rnd();
    let sx = (2.0 * (f32(px) + jx) / f32(width) - 1.0) * aspect * tan_half;
    let sy = (1.0 - 2.0 * (f32(py) + jy) / f32(height)) * tan_half;
    var d = normalize(P.cam_fwd.xyz + P.cam_right.xyz * sx + P.cam_up.xyz * sy);
    var o = P.cam_pos.xyz;

    var radiance = vec3<f32>(0.0);
    var beta = vec3<f32>(1.0);
    var bounces = 0u;
    var specular = true;
    var in_water = false;
    var strategy = EMITTER_SAMPLING_NONE;
    if (P.misc_u.w > 0u) { strategy = P.emitter.x; }
    var tmin = 0.0;
    let sigma = P.water_color.xyz;

    for (var ev = 0u; ev < MAX_EVENTS; ev++) {
        var h = trace(o, d, tmin, INF);
        var forced = false;
        if (bounces == 0u && P.see_through.y > 0u && h.quad != NO_QUAD && !is_see_through(h.quad)) {
            let s = see_through_hit(o, d, h.t);
            if (s.quad != NO_QUAD) {
                if ((q_flags(quads[s.quad]) & 3u) == ALPHA_TRANSLUCENT) {
                    if (rnd() < f32(s.texel >> 24u) / 255.0) {
                        h = s;
                        forced = true;
                    }
                } else {
                    h = s;
                }
            }
        }
        if (h.quad == NO_QUAD) {
            if (!in_water) {
                radiance += beta * sky_radiance(d);
                if (specular && bounces == 0u) {
                    radiance += beta * sun_disk(d);
                }
            }
            break;
        }
        if (in_water) {
            beta = beta * transmittance(sigma, h.t);
        }
        let q = quads[h.quad];
        let flags = q_flags(q);
        let pos = o + d * h.t;
        let alpha = f32(h.texel >> 24u) / 255.0;
        let albedo = linear_rgb(h.texel) * linear_rgb(q_tint(q));
        var n = normalize(q_normal(q));
        let front = dot(n, d) < 0.0;
        if (!front) { n = -n; }

        if ((flags & FLAG_WATER) != 0u) {
            let ior = P.water_color.w;
            let eta = select(ior, 1.0 / ior, front);
            let cos_i = -dot(n, d);
            let sin2_t = eta * eta * max(1.0 - cos_i * cos_i, 0.0);
            var r0 = (1.0 - ior) / (1.0 + ior);
            r0 = r0 * r0;
            let m = 1.0 - cos_i;
            let fresnel = r0 + (1.0 - r0) * m * m * m * m * m;
            if (sin2_t >= 1.0 || rnd() < fresnel) {
                d = normalize(d + n * (2.0 * cos_i));
                o = pos + n * eps;
            } else {
                let cos_t = sqrt(1.0 - sin2_t);
                d = normalize(d * eta + n * (eta * cos_i - cos_t));
                o = pos - n * eps;
                in_water = front;
            }
            specular = true;
            tmin = 0.0;
            continue;
        }

        if (!forced && (flags & 3u) == ALPHA_TRANSLUCENT && rnd() >= alpha) {
            beta = beta * mix(vec3<f32>(1.0), albedo, alpha);
            o = pos + d * eps;
            tmin = 0.0;
            continue;
        }

        if (bounces >= max_bounces) { break; }
        let emittance = q_emittance(q) * P.misc_f.x;
        let emits = emittance > 0.0
            && (P.emitter.y == 0u || strategy == EMITTER_SAMPLING_NONE || bounces == 0u);
        if (emits) {
            radiance += beta * (albedo * albedo) * emittance;
        }
        bounces += 1u;

        let origin = pos + n * eps;
        let sun = P.sun_dir.xyz;
        if (dot(n, sun) > 0.0 && P.sun_dir.y > -0.1) {
            let l = sample_sun();
            let cos_l = dot(n, l);
            if (cos_l > 0.0) {
                let tr = shadow(origin, l, INF);
                radiance += beta * albedo * P.sun_color.xyz * tr * cos_l;
            }
        }
        if (!emits && strategy != EMITTER_SAMPLING_NONE) {
            radiance += beta * albedo * sample_emitters(pos, origin, n);
        }

        beta = beta * albedo;
        if (bounces >= 3u) {
            let q_rr = clamp(max(beta.x, max(beta.y, beta.z)), 0.05, 0.95);
            if (rnd() > q_rr) { break; }
            beta = beta / q_rr;
        }
        d = cosine_hemisphere(n);
        o = origin;
        tmin = 0.0;
        specular = false;
    }
    return radiance;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let width = P.frame.x;
    let height = P.frame.y;
    if (gid.x >= width || gid.y >= height) {
        return;
    }
    let idx = gid.y * width + gid.x;
    var acc = accum[idx];
    for (var s = 0u; s < P.misc_u.y; s++) {
        let c = render_sample(gid.x, gid.y, P.frame.z + s);
        if (all(abs(c) < vec3<f32>(1e30))) {
            acc += vec4<f32>(c, 1.0);
        } else {
            acc += vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
    }
    accum[idx] = acc;
}
