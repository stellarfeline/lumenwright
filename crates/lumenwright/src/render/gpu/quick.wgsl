// Quick mode raster shaders. Mirror of render/cpu/raster.rs, render/texture.rs and render/sky.rs.

struct Uniforms {
    view_proj: mat4x4<f32>,
    sky_color: vec4<f32>,   // w = FogSkyEnd
    fog_color: vec4<f32>,   // w = render-distance fog start
    fog_dist: vec4<f32>,    // x = render fog end, y/z = environmental fog start/end, w = star add
    sunrise: vec4<f32>,
    flags: vec4<f32>,       // x = sky disc, y = glow, z = stars
    sun: array<vec4<f32>, 4>,
    moon: array<vec4<f32>, 4>,
    star_rot: array<vec4<f32>, 3>,
    fan: array<vec4<f32>, 18>,
    cam_forward: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_misc: vec4<f32>,    // tan(fov/2), aspect, width, height
    atlas_size: vec4<u32>,  // width, height, mip levels, mip rows
}

@group(0) @binding(0) var<uniform> U: Uniforms;
@group(0) @binding(1) var<storage, read> atlas: array<u32>;
@group(0) @binding(2) var<storage, read> mips: array<u32>;
@group(0) @binding(3) var<storage, read> stars: array<u32>;

const PI: f32 = 3.14159265358979;
const MAX_ANISOTROPY: f32 = 4.0;
const STAR_BINS_AZ: u32 = 128u;
const STAR_BINS_EL: u32 = 64u;
const RIM_SPHERICAL: f32 = 512.24994;

fn unpack(c: u32) -> vec4<f32> {
    return vec4<f32>(f32(c & 0xFFu), f32((c >> 8u) & 0xFFu), f32((c >> 16u) & 0xFFu), f32(c >> 24u)) / 255.0;
}

// Mirror of render/texture.rs.
fn nearest(uv: vec2<f32>) -> vec4<f32> {
    let w = i32(U.atlas_size.x);
    let h = i32(U.atlas_size.y);
    let x = clamp(i32(floor(uv.x * f32(w))), 0, w - 1);
    let y = clamp(i32(floor(uv.y * f32(h))), 0, h - 1);
    return unpack(atlas[u32(y * w + x)]);
}

fn is_mipmapped(uv: vec2<f32>) -> bool {
    return U.atlas_size.z > 0u && uv.y * f32(U.atlas_size.y) < f32(U.atlas_size.w);
}

fn texel(l: u32, x: i32, y: i32) -> vec4<f32> {
    let w = i32(U.atlas_size.x >> l);
    let h = i32(U.atlas_size.y >> l);
    let i = u32(clamp(y, 0, h - 1) * w + clamp(x, 0, w - 1));
    if (l == 0u) {
        return unpack(atlas[i]);
    }
    var o = 0u;
    for (var k = 1u; k < l; k++) {
        o += (U.atlas_size.x >> k) * (U.atlas_size.y >> k);
    }
    return unpack(mips[o + i]);
}

fn bilinear(uv: vec2<f32>, l: u32) -> vec4<f32> {
    let size = vec2<f32>(f32(U.atlas_size.x >> l), f32(U.atlas_size.y >> l));
    let t = uv * size - 0.5;
    let f = floor(t);
    let fr = t - f;
    let x = i32(f.x);
    let y = i32(f.y);
    let top = mix(texel(l, x, y), texel(l, x + 1, y), fr.x);
    let bottom = mix(texel(l, x, y + 1), texel(l, x + 1, y + 1), fr.x);
    return mix(top, bottom, fr.y);
}

fn trilinear(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let lo = floor(lod);
    let fr = lod - lo;
    let l = u32(lo);
    let a = bilinear(uv, l);
    if (fr > 0.0 && l < U.atlas_size.z) {
        return mix(a, bilinear(uv, l + 1u), fr);
    }
    return a;
}

fn sample_grad(uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(f32(U.atlas_size.x), f32(U.atlas_size.y));
    let px = length(du * size);
    let py = length(dv * size);
    let pmax = max(px, py);
    let pmin = min(px, py);
    let n = clamp(ceil(pmax / max(pmin, 1e-8)), 1.0, MAX_ANISOTROPY);
    let lod = clamp(log2(max(pmax / n, 1e-8)), 0.0, f32(U.atlas_size.z));
    var axis = dv;
    if (px >= py) {
        axis = du;
    }
    var acc = vec4<f32>(0.0);
    let taps = u32(n);
    for (var i = 0u; i < taps; i++) {
        let off = (f32(i) + 0.5) / n - 0.5;
        acc += trilinear(uv + axis * off, lod);
    }
    return acc / n;
}

// The client's sampleNearest (terrain.fsh).
fn sample_block(uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(f32(U.atlas_size.x), f32(U.atlas_size.y));
    let pixel = 1.0 / size;
    let t = uv * size;
    let center = round(t) - 0.5;
    let off0 = t - center;
    let screen = sqrt(max(du * du + dv * dv, vec2<f32>(1e-20)));
    let off = clamp((off0 - 0.5) * pixel / screen + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    return sample_grad((center + off) * pixel, du, dv);
}

fn sample_tex(uv: vec2<f32>, du: vec2<f32>, dv: vec2<f32>) -> vec4<f32> {
    if (is_mipmapped(uv)) {
        return sample_block(uv, du, dv);
    }
    return nearest(uv);
}

fn pixel_dir(x: f32, y: f32) -> vec3<f32> {
    let m = U.cam_misc;
    let sx = (2.0 * (x + 0.5) / m.z - 1.0) * m.y * m.x;
    let sy = (1.0 - 2.0 * (y + 0.5) / m.w) * m.x;
    return normalize(U.cam_forward.xyz + U.cam_right.xyz * sx + U.cam_up.xyz * sy);
}

fn linear_fog(v: f32, start: f32, end: f32) -> f32 {
    if (v <= start) {
        return 0.0;
    } else if (v >= end) {
        return 1.0;
    }
    return (v - start) / (end - start);
}

// Mirror of render/sky.rs.
fn sky_disc(d: vec3<f32>, out_c: ptr<function, vec3<f32>>) {
    if (d.y <= 0.0) {
        return;
    }
    let p = d * (16.0 / d.y);
    let step = PI / 4.0;
    let phi = atan2(p.z, p.x);
    let k = clamp(floor((phi + PI) / step), 0.0, 7.0);
    let th0 = -PI + k * step;
    let r0 = vec2<f32>(512.0 * cos(th0), 512.0 * sin(th0));
    let r1 = vec2<f32>(512.0 * cos(th0 + step), 512.0 * sin(th0 + step));
    let det = r0.x * r1.y - r1.x * r0.y;
    let b1 = (p.x * r1.y - r1.x * p.z) / det;
    let b2 = (r0.x * p.z - p.x * r0.y) / det;
    let s = b1 + b2;
    if (s > 1.0) {
        return;
    }
    let end = U.sky_color.w;
    let sph = 16.0 * (1.0 - s) + RIM_SPHERICAL * s;
    let cyl = 16.0 * (1.0 - s) + 512.0 * s;
    let f = max(linear_fog(sph, 0.0, end), linear_fog(cyl, end, end));
    *out_c = mix(U.sky_color.xyz, U.fog_color.xyz, f);
}

fn fan_alpha(d: vec3<f32>) -> f32 {
    let c = U.fan[0].xyz;
    var best_t = 1e30;
    var alpha = 0.0;
    for (var k = 0u; k < 16u; k++) {
        let e1 = U.fan[k + 1u].xyz - c;
        let e2 = U.fan[k + 2u].xyz - c;
        let pv = cross(d, e2);
        let det = dot(e1, pv);
        if (abs(det) < 1e-6) {
            continue;
        }
        let inv = 1.0 / det;
        let tv = -c;
        let u = dot(tv, pv) * inv;
        if (u < 0.0 || u > 1.0) {
            continue;
        }
        let qv = cross(tv, e1);
        let v = dot(d, qv) * inv;
        if (v < 0.0 || u + v > 1.0) {
            continue;
        }
        let t = dot(e2, qv) * inv;
        if (t > 0.0 && t < best_t) {
            best_t = t;
            alpha = (1.0 - u - v) * U.fan[0].w;
        }
    }
    return alpha;
}

fn body(c4: vec4<f32>, ua: vec4<f32>, va: vec4<f32>, r: vec4<f32>, d: vec3<f32>) -> vec4<f32> {
    if (c4.w == 0.0 || r.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    let c = c4.xyz;
    let dn = dot(d, c);
    if (dn <= 0.0) {
        return vec4<f32>(0.0);
    }
    let q = d * (dot(c, c) / dn) - c;
    let lx = dot(q, ua.xyz);
    let lz = dot(q, va.xyz);
    if (abs(lx) >= 1.0 || abs(lz) >= 1.0) {
        return vec4<f32>(0.0);
    }
    let u = r.x + (lx * 0.5 + 0.5) * r.z;
    let v = r.y + (lz * 0.5 + 0.5) * r.w;
    return unpack(atlas[u32(floor(v)) * U.atlas_size.x + u32(floor(u))]);
}

fn star_hits(d: vec3<f32>) -> u32 {
    let q = vec3<f32>(dot(U.star_rot[0].xyz, d), dot(U.star_rot[1].xyz, d), dot(U.star_rot[2].xyz, d));
    let az = atan2(q.z, q.x);
    let el = asin(clamp(q.y, -1.0, 1.0));
    let ia = u32(clamp(i32(floor((az + PI) / (2.0 * PI) * f32(STAR_BINS_AZ))), 0, i32(STAR_BINS_AZ) - 1));
    let ie = u32(clamp(i32(floor((el + PI / 2.0) / PI * f32(STAR_BINS_EL))), 0, i32(STAR_BINS_EL) - 1));
    let b = ie * STAR_BINS_AZ + ia;
    let base = STAR_BINS_AZ * STAR_BINS_EL + 1u;
    var n = 0u;
    for (var e = stars[b]; e < stars[b + 1u]; e++) {
        let o = base + e * 9u;
        let c = vec3<f32>(bitcast<f32>(stars[o]), bitcast<f32>(stars[o + 1u]), bitcast<f32>(stars[o + 2u]));
        let a1 = vec3<f32>(bitcast<f32>(stars[o + 3u]), bitcast<f32>(stars[o + 4u]), bitcast<f32>(stars[o + 5u]));
        let a2 = vec3<f32>(bitcast<f32>(stars[o + 6u]), bitcast<f32>(stars[o + 7u]), bitcast<f32>(stars[o + 8u]));
        let dn = dot(q, c);
        if (dn <= 0.0) {
            continue;
        }
        let p = q * (dot(c, c) / dn) - c;
        if (abs(dot(p, a1)) < 1.0 && abs(dot(p, a2)) < 1.0) {
            n += 1u;
        }
    }
    return n;
}

fn background(d: vec3<f32>) -> vec3<f32> {
    var c = U.fog_color.xyz;
    if (U.flags.x > 0.5) {
        sky_disc(d, &c);
    }
    if (U.flags.y > 0.5) {
        c = mix(c, U.sunrise.xyz, fan_alpha(d));
    }
    let s = body(U.sun[0], U.sun[1], U.sun[2], U.sun[3], d);
    c = min(c + s.xyz * s.w, vec3<f32>(1.0));
    let m = body(U.moon[0], U.moon[1], U.moon[2], U.moon[3], d);
    c = min(c + m.xyz * m.w, vec3<f32>(1.0));
    if (U.flags.z > 0.5) {
        let n = star_hits(d);
        for (var i = 0u; i < n; i++) {
            c = min(c + vec3<f32>(U.fog_dist.w), vec3<f32>(1.0));
        }
    }
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// The client's total_fog_value (fog.glsl).
fn fog_factor(rel: vec3<f32>) -> f32 {
    let sph = length(rel);
    let cyl = max(sqrt(rel.x * rel.x + rel.z * rel.z), abs(rel.y));
    return max(linear_fog(sph, U.fog_dist.y, U.fog_dist.z), linear_fog(cyl, U.fog_color.w, U.fog_dist.x));
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pos: vec3<f32>,
}

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.clip = U.view_proj * vec4<f32>(pos, 1.0);
    o.uv = uv;
    o.color = color;
    o.pos = pos;
    return o;
}

@fragment
fn fs_opaque(i: VsOut) -> @location(0) vec4<f32> {
    let du = dpdxFine(i.uv);
    let dv = dpdyFine(i.uv);
    let tex = sample_tex(i.uv, du, dv);
    if (tex.w < 0.5) {
        discard;
    }
    var rgb = tex.xyz * i.color.xyz;
    rgb = mix(rgb, U.fog_color.xyz, fog_factor(i.pos));
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn fs_translucent(i: VsOut) -> @location(0) vec4<f32> {
    let du = dpdxFine(i.uv);
    let dv = dpdyFine(i.uv);
    let tex = sample_tex(i.uv, du, dv);
    let a = tex.w * i.color.w;
    if (a <= 0.0) {
        discard;
    }
    var rgb = tex.xyz * i.color.xyz;
    rgb = mix(rgb, U.fog_color.xyz, fog_factor(i.pos));
    return vec4<f32>(rgb, a);
}

// Full-screen triangle for the sky background.
@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 1.0, 1.0);
}

@fragment
fn fs_sky(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(background(pixel_dir(frag.x - 0.5, frag.y - 0.5)), 1.0);
}
