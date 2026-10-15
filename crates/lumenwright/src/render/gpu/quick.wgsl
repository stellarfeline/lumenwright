// Quick mode raster shaders. Mirror of render/cpu/raster.rs and render/sky.rs.

struct Uniforms {
    view_proj: mat4x4<f32>,
    sky_color: vec4<f32>,
    fog_color: vec4<f32>,   // w = fog start
    sun_dir: vec4<f32>,     // w = fog end
    sun_rect: vec4<f32>,
    cam_forward: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_misc: vec4<f32>,    // tan(fov/2), aspect, width, height
    atlas_size: vec4<u32>,
}

@group(0) @binding(0) var<uniform> U: Uniforms;
@group(0) @binding(1) var<storage, read> atlas: array<u32>;

fn unpack(c: u32) -> vec4<f32> {
    return vec4<f32>(f32(c & 0xFFu), f32((c >> 8u) & 0xFFu), f32((c >> 16u) & 0xFFu), f32(c >> 24u)) / 255.0;
}

fn fetch(uv: vec2<f32>) -> vec4<f32> {
    let w = i32(U.atlas_size.x);
    let h = i32(U.atlas_size.y);
    let x = clamp(i32(floor(uv.x * f32(w))), 0, w - 1);
    let y = clamp(i32(floor(uv.y * f32(h))), 0, h - 1);
    return unpack(atlas[u32(y * w + x)]);
}

fn pixel_dir(x: f32, y: f32) -> vec3<f32> {
    let m = U.cam_misc;
    let sx = (2.0 * (x + 0.5) / m.z - 1.0) * m.y * m.x;
    let sy = (1.0 - 2.0 * (y + 0.5) / m.w) * m.x;
    return normalize(U.cam_forward.xyz + U.cam_right.xyz * sx + U.cam_up.xyz * sy);
}

fn background(d: vec3<f32>) -> vec3<f32> {
    let t = clamp(d.y * 3.0, 0.0, 1.0);
    var c = mix(U.fog_color.xyz, U.sky_color.xyz, t);
    if (U.sun_rect.z > 0.0) {
        let s = U.sun_dir.xyz;
        let cs = dot(d, s);
        if (cs > 0.0) {
            var t1 = vec3<f32>(1.0, 0.0, 0.0);
            if (abs(s.y) < 0.99) {
                t1 = normalize(cross(s, vec3<f32>(0.0, 1.0, 0.0)));
            }
            let t2 = cross(t1, s);
            let px = dot(d, t1) / cs / 0.3;
            let py = dot(d, t2) / cs / 0.3;
            if (abs(px) < 1.0 && abs(py) < 1.0) {
                let u = U.sun_rect.x + (px * 0.5 + 0.5) * U.sun_rect.z;
                let v = U.sun_rect.y + (py * 0.5 + 0.5) * U.sun_rect.w;
                let texel = unpack(atlas[u32(floor(v)) * U.atlas_size.x + u32(floor(u))]);
                c += texel.xyz * texel.w;
            }
        }
    }
    return min(c, vec3<f32>(1.0));
}

fn fog_factor(rel: vec3<f32>) -> f32 {
    let d = max(sqrt(rel.x * rel.x + rel.z * rel.z), abs(rel.y));
    let start = U.fog_color.w;
    let end = U.sun_dir.w;
    return clamp((d - start) / max(end - start, 1e-3), 0.0, 1.0);
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
    let tex = fetch(i.uv);
    if (tex.w < 0.5) {
        discard;
    }
    var rgb = tex.xyz * i.color.xyz;
    rgb = mix(rgb, U.fog_color.xyz, fog_factor(i.pos));
    return vec4<f32>(rgb, 1.0);
}

@fragment
fn fs_translucent(i: VsOut) -> @location(0) vec4<f32> {
    let tex = fetch(i.uv);
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
