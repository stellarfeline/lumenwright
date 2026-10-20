//! Unidirectional path tracer ported from Chunky's (GPL-3.0, `PathTracer.java` at
//! chunky-dev/chunky 156e2bb): diffuse surfaces lit by the sun (next-event estimation)
//! and by emitters (Chunky's emitter sampling over its emitter grid), the Preetham sky,
//! Fresnel water and stochastic translucency.
//!
//! An emitter is a surface whose block's material has an emittance (Chunky's
//! `BlockPalette`): it emits its texel colour squared times its emittance times the
//! emitter intensity `misc_f.x` (Chunky's quadratic emitter mapping: a texel half as
//! bright emits a quarter of the light).

use crate::math::{V3, abs, acos, clamp, exp, gamma_to_linear, max, onb, sin_cos_turns, sqrt, v3};
use crate::rng::Rng;
use crate::scene::{
    ALPHA_TRANSLUCENT, EMITTER_GRID_HEADER, EMITTER_SAMPLING_ALL, EMITTER_SAMPLING_NONE,
    EMITTER_SAMPLING_ONE, EMITTER_SAMPLING_ONE_BLOCK, FLAG_ALPHA_MASK, FLAG_WATER, GpuEmitter,
    GpuQuad, SceneView, unpack_rgba,
};
use crate::trace::{is_see_through, see_through_hit, trace};

const PI: f32 = core::f32::consts::PI;
/// Upper bound on surface interactions per path (diffuse + specular + pass-through).
const MAX_EVENTS: u32 = 48;

#[inline(always)]
fn linear_rgb(c: u32) -> V3 {
    let c = unpack_rgba(c);
    v3(
        gamma_to_linear(c[0]),
        gamma_to_linear(c[1]),
        gamma_to_linear(c[2]),
    )
}

/// The radiance of the sky in the unit direction `d`: Chunky's Preetham sky (GPL-3.0,
/// `PreethamSky.calcIncidentLight` at chunky-dev/chunky 156e2bb; A. J. Preetham, P.
/// Shirley and B. Smits, "A Practical Analytic Model for Daylight", SIGGRAPH 1999).
/// The Perez distribution gives x, y and Y at the direction's zenith angle θ (its height
/// plus the horizon offset, no lower than the horizon) and angle γ from the sun; xyY
/// goes to XYZ and to RGB by the matrix Chunky uses (Bruce Lindbloom's), scaled by
/// 0.045 and by the sky light. Below the horizon the sky holds its horizon value.
/// Chunky evaluates the same function through a 128 × 128 cache interpolated in HSL
/// (`SkyCache`); the kernel evaluates it directly.
pub fn sky_radiance(scene: &SceneView, d: V3) -> V3 {
    let p = scene.params;
    let sun = V3::from4(p.sky_sun);
    let cos_theta = max(d.y + p.sky_zenith[3], 0.0);
    let cos_gamma = clamp(d.dot(sun), -1.0, 1.0);
    let gamma = acos(cos_gamma);
    let cos2_gamma = cos_gamma * cos_gamma;
    let perez = |k: usize| -> f32 {
        // B is negative, so at the horizon (cos θ = 0) the first factor is 1, as Java's
        // exp(B / 0) = exp(-∞) = 0 gives it.
        let a = if cos_theta > 0.0 {
            p.perez_a[k] * exp(p.perez_b[k] / cos_theta)
        } else {
            0.0
        };
        (1.0 + a) * (1.0 + p.perez_c[k] * exp(p.perez_d[k] * gamma) + p.perez_e[k] * cos2_gamma)
    };
    let x = p.sky_zenith[0] * perez(0);
    let y = p.sky_zenith[1] * perez(1);
    let lum = p.sky_zenith[2] * perez(2);
    if y <= 5e-8 {
        return V3::ZERO;
    }
    let f = lum / y;
    let (cx, cy, cz) = (x * f, lum, (1.0 - x - y) * f);
    v3(
        2.370_674_3 * cx - 0.900_040_5 * cy - 0.470_633_8 * cz,
        -0.513_885 * cx + 1.425_303_6 * cy + 0.088_581_4 * cz,
        0.005_298_2 * cx - 0.014_694_9 * cy + 1.009_396_8 * cz,
    ) * (0.045 * p.sky_sun[3])
}

/// The sun drawn in the sky: Chunky's `Sun.intersect` (GPL-3.0, chunky-dev/chunky
/// 156e2bb). The client's sun texture covers a square around the sun whose half-width is
/// four times the sun's radius, laid along the axes `su` and `sv` by the angles
/// `π/2 − acos(d·su)` and `π/2 − acos(d·sv)`, sampled at the nearest texel (as
/// `Texture.getColor`), decoded to linear and scaled by the texture's brightness; the
/// caller adds it to the sky, as `Sky.addSunColor` does.
#[inline(always)]
pub fn sun_texture(scene: &SceneView, d: V3) -> V3 {
    let p = scene.params;
    let sprite = p.sun_sprite;
    if sprite[2] <= 0.0 || d.dot(V3::from4(p.sun_dir)) < 0.5 {
        return V3::ZERO;
    }
    let width = p.sun_sv[3];
    let width2 = width * 2.0;
    let a = 0.5 * PI - acos(d.dot(V3::from4(p.sun_su))) + width;
    let b = 0.5 * PI - acos(d.dot(V3::from4(p.sun_sv))) + width;
    if a < 0.0 || a >= width2 || b < 0.0 || b >= width2 {
        return V3::ZERO;
    }
    // `Texture.getColor(u, v)`: texel ((int) (u w − ε), (int) ((1 − v) h − ε)).
    let x = clamp(
        crate::math::floor(a / width2 * sprite[2] - 5e-5),
        0.0,
        sprite[2] - 1.0,
    );
    let y = clamp(
        crate::math::floor((1.0 - b / width2) * sprite[3] - 5e-5),
        0.0,
        sprite[3] - 1.0,
    );
    linear_rgb(scene.texel(sprite[0] + x + 0.5, sprite[1] + y + 0.5)) * p.sun_su[3]
}

#[inline(always)]
fn cosine_hemisphere(n: V3, rng: &mut Rng) -> V3 {
    let r1 = rng.next();
    let r2 = rng.next();
    let r = sqrt(r1);
    let (sin_phi, cos_phi) = sin_cos_turns(r2);
    let (t, b) = onb(n);
    (t * (r * cos_phi) + b * (r * sin_phi) + n * sqrt(max(1.0 - r1, 0.0))).normalize()
}

/// Uniform direction inside the sun's cone.
#[inline(always)]
fn sample_sun(scene: &SceneView, rng: &mut Rng) -> V3 {
    let p = scene.params;
    let sun = V3::from4(p.sun_dir);
    let cos_max = p.sun_dir[3];
    let r1 = rng.next();
    let r2 = rng.next();
    let cos_t = 1.0 - r1 * (1.0 - cos_max);
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let (sin_phi, cos_phi) = sin_cos_turns(r2);
    let (t, b) = onb(sun);
    (t * (sin_t * cos_phi) + b * (sin_t * sin_phi) + sun * cos_t).normalize()
}

/// Origin of a ray in grid space for the given quad hit. Model quads live in
/// block-local space but `trace` already returns grid-space `t`, so the hit
/// point is simply `o + d*t`.
#[inline(always)]
fn shading_normal(q: &GpuQuad, d: V3) -> (V3, bool) {
    let n = q.normal().normalize();
    if n.dot(d) < 0.0 {
        (n, true)
    } else {
        (-n, false)
    }
}

/// Transmittance from `o` along `d` up to distance `tmax` (0 when blocked).
#[cfg_attr(target_arch = "nvptx64", inline(always))]
fn shadow(scene: &SceneView, mut o: V3, d: V3, mut tmax: f32) -> V3 {
    let eps = scene.params.misc_f[1];
    let mut tr = V3::ONE;
    for _ in 0..8 {
        match trace(scene, o, d, eps, tmax) {
            None => return tr,
            Some(h) => {
                let q = &scene.quads[h.quad as usize];
                let flags = q.flags();
                if flags & FLAG_WATER != 0 {
                    // Leaving the water volume: absorb along the submerged segment.
                    if q.normal().dot(d) > 0.0 {
                        tr = tr.mul(V3::from4(scene.params.water_color).transmittance(h.t));
                    }
                } else if flags & FLAG_ALPHA_MASK == ALPHA_TRANSLUCENT {
                    let a = (h.texel >> 24) as f32 / 255.0;
                    let col = linear_rgb(h.texel).mul(linear_rgb(q.tint()));
                    tr = tr.mul(V3::ONE.lerp(col, a));
                } else {
                    return V3::ZERO;
                }
                o = o + d * (h.t + eps);
                tmax -= h.t + eps;
            }
        }
    }
    tr
}

/// A uniform index in `0..n` (`n > 0`) from one random number.
#[inline(always)]
fn pick(n: u32, u: f32) -> u32 {
    let i = (u * n as f32) as u32;
    if i < n { i } else { n - 1 }
}

/// The emitters near grid-space point `p`: `(start, count)` into the emitter grid's
/// index list (Chunky's `Grid.sampleEmitterPosition` / `getEmitterPositions`).
#[inline(always)]
pub fn emitter_cell(scene: &SceneView, p: V3) -> (u32, u32) {
    let g = scene.emitter_grid;
    let cell = g[0] as f32;
    let size = [g[1] as i32, g[2] as i32, g[3] as i32];
    let c = [
        crate::math::floor(p.x / cell) as i32 - g[4] as i32,
        crate::math::floor(p.y / cell) as i32 - g[5] as i32,
        crate::math::floor(p.z / cell) as i32 - g[6] as i32,
    ];
    if c[0] < 0 || c[1] < 0 || c[2] < 0 || c[0] >= size[0] || c[1] >= size[1] || c[2] >= size[2] {
        return (0, 0);
    }
    let i = ((c[1] * size[2] + c[2]) * size[0] + c[0]) as u32;
    let at = (EMITTER_GRID_HEADER + i * 2) as usize;
    (g[at], g[at + 1])
}

/// One face of one emitter seen from `origin` (normal `n`): Chunky's
/// `PathTracer.sampleEmitterFace`. A uniform point on the face; when it lies in front
/// of the surface and the first thing a ray towards it meets is at its distance, the
/// face gives the colour met there times `|cos θ| / max(d², 1)` times the face's area,
/// its emittance, the emitter intensity and `scaler`.
#[inline(always)]
fn sample_emitter_face(
    scene: &SceneView,
    origin: V3,
    n: V3,
    e: &GpuEmitter,
    face: u32,
    scaler: f32,
    rng: &mut Rng,
) -> V3 {
    let p = scene.params;
    let q = &scene.quads[(e.first_quad() + face) as usize];
    let s = rng.next();
    let t = rng.next();
    let at = e.offset() + q.origin() + q.edge_u() * s + q.edge_v() * t;
    let to = at - origin;
    if to.dot(n) <= 0.0 {
        return V3::ZERO;
    }
    let dist = to.length();
    let l = to / dist;
    let eps = p.misc_f[1];
    let Some(h) = trace(scene, origin, l, eps, dist + 2.0 * eps) else {
        return V3::ZERO;
    };
    if abs(h.t - dist) >= 2.0 * eps {
        return V3::ZERO;
    }
    let nq = q.normal();
    let area = nq.length();
    let cos_l = abs(nq.dot(l)) / area;
    let hq = &scene.quads[h.quad as usize];
    let c = linear_rgb(h.texel).mul(linear_rgb(hq.tint()));
    c * (cos_l / max(dist * dist, 1.0) * area * q.emittance() * p.misc_f[0] * scaler)
}

/// Every face of emitter `e`, each weighted `1 / faces` (Chunky's `sampleEmitter` for
/// `ONE_BLOCK` and `ALL`), or one face picked uniformly (`ONE`).
#[inline(always)]
fn sample_emitter(scene: &SceneView, origin: V3, n: V3, e: &GpuEmitter, rng: &mut Rng) -> V3 {
    let faces = e.face_count();
    if faces == 0 {
        return V3::ZERO;
    }
    if scene.params.emitter[0] == EMITTER_SAMPLING_ONE {
        let f = pick(faces, rng.next());
        return sample_emitter_face(scene, origin, n, e, f, 1.0, rng);
    }
    let mut sum = V3::ZERO;
    for f in 0..faces {
        sum = sum + sample_emitter_face(scene, origin, n, e, f, 1.0 / faces as f32, rng);
    }
    sum
}

/// Emitter light reaching a diffuse surface at `pos` (offset to `origin`, normal `n`),
/// before its albedo: Chunky's emitter sampling in `doDiffuseReflection`, over the
/// emitters the grid cell of `pos` lists. `ONE` and `ONE_BLOCK` take one of them
/// uniformly; `ALL` averages over all of them. The result is scaled by π, as Chunky's.
// On CUDA the whole path tracer is one function: see `pt_render` in crates/lumenwright-ptx.
#[cfg_attr(target_arch = "nvptx64", inline(always))]
pub(crate) fn sample_emitters(scene: &SceneView, pos: V3, origin: V3, n: V3, rng: &mut Rng) -> V3 {
    let (start, count) = emitter_cell(scene, pos);
    if count == 0 {
        return V3::ZERO;
    }
    let list = EMITTER_GRID_HEADER as usize
        + scene.emitter_grid[1] as usize
            * scene.emitter_grid[2] as usize
            * scene.emitter_grid[3] as usize
            * 2
        + start as usize;
    match scene.params.emitter[0] {
        EMITTER_SAMPLING_ONE | EMITTER_SAMPLING_ONE_BLOCK => {
            let i = scene.emitter_grid[list + pick(count, rng.next()) as usize];
            let e = scene.emitters[i as usize];
            sample_emitter(scene, origin, n, &e, rng) * PI
        }
        EMITTER_SAMPLING_ALL => {
            let mut sum = V3::ZERO;
            for k in 0..count {
                let i = scene.emitter_grid[list + k as usize];
                let e = scene.emitters[i as usize];
                sum = sum + sample_emitter(scene, origin, n, &e, rng);
            }
            sum * (PI / count as f32)
        }
        _ => V3::ZERO,
    }
}

/// Radiance estimate for one camera sample at pixel (px, py).
// On CUDA the whole path tracer is one function: see `pt_render` in crates/lumenwright-ptx.
#[cfg_attr(target_arch = "nvptx64", inline(always))]
pub fn render_sample(scene: &SceneView, px: u32, py: u32, sample: u32) -> V3 {
    let p = scene.params;
    let width = p.frame[0];
    let height = p.frame[1];
    let max_bounces = p.frame[3];
    let eps = p.misc_f[1];
    let mut rng = Rng::new(py * width + px, sample, p.misc_u[0]);

    // Pinhole camera with pixel jitter.
    let tan_half = p.cam_pos[3];
    let aspect = p.cam_fwd[3];
    let jx = rng.next();
    let jy = rng.next();
    let sx = (2.0 * (px as f32 + jx) / width as f32 - 1.0) * aspect * tan_half;
    let sy = (1.0 - 2.0 * (py as f32 + jy) / height as f32) * tan_half;
    let mut d =
        (V3::from4(p.cam_fwd) + V3::from4(p.cam_right) * sx + V3::from4(p.cam_up) * sy).normalize();
    let mut o = V3::from4(p.cam_pos);

    let mut radiance = V3::ZERO;
    let mut beta = V3::ONE;
    let mut bounces = 0u32;
    let mut specular = true; // camera rays see the sun disk
    let mut in_water = false;
    let strategy = if p.misc_u[3] > 0 {
        p.emitter[0]
    } else {
        EMITTER_SAMPLING_NONE
    };
    let mut tmin = 0.0f32;
    let sigma = V3::from4(p.water_color);

    for _ in 0..MAX_EVENTS {
        let mut hit = trace(scene, o, d, tmin, f32::INFINITY);
        // Until the first diffuse bounce, a see-through quad behind the surface the
        // ray met is drawn instead of it (a translucent one with its own alpha).
        let mut forced = false;
        if bounces == 0
            && p.see_through[1] > 0
            && let Some(h) = hit
            && !is_see_through(scene, h.quad)
            && let Some(s) = see_through_hit(scene, o, d, h.t)
        {
            let sq = &scene.quads[s.quad as usize];
            if sq.flags() & FLAG_ALPHA_MASK == ALPHA_TRANSLUCENT {
                if rng.next() < (s.texel >> 24) as f32 / 255.0 {
                    hit = Some(s);
                    forced = true;
                }
            } else {
                hit = Some(s);
            }
        }
        let h = match hit {
            None => {
                if !in_water {
                    radiance = radiance + beta.mul(sky_radiance(scene, d));
                    // Only purely specular camera paths see the sun drawn (Chunky's
                    // direct and specular sky hits); diffuse paths already account for
                    // the sun through next-event estimation (Chunky's `FAST`).
                    if specular && bounces == 0 {
                        radiance = radiance + beta.mul(sun_texture(scene, d));
                    }
                }
                break;
            }
            Some(h) => h,
        };
        if in_water {
            beta = beta.mul(sigma.transmittance(h.t));
        }
        let q = &scene.quads[h.quad as usize];
        let flags = q.flags();
        let pos = o + d * h.t;
        let alpha = (h.texel >> 24) as f32 / 255.0;
        let albedo = linear_rgb(h.texel).mul(linear_rgb(q.tint()));
        let (n, front) = shading_normal(q, d);

        if flags & FLAG_WATER != 0 {
            // Dielectric interface. Water quads face out of the water volume.
            let ior = p.water_color[3];
            let eta = if front { 1.0 / ior } else { ior };
            let cos_i = -n.dot(d);
            let sin2_t = eta * eta * max(1.0 - cos_i * cos_i, 0.0);
            let r0 = (1.0 - ior) / (1.0 + ior);
            let r0 = r0 * r0;
            let m = 1.0 - cos_i;
            let fresnel = r0 + (1.0 - r0) * m * m * m * m * m;
            if sin2_t >= 1.0 || rng.next() < fresnel {
                d = (d + n * (2.0 * cos_i)).normalize();
                o = pos + n * eps;
            } else {
                let cos_t = sqrt(1.0 - sin2_t);
                d = (d * eta + n * (eta * cos_i - cos_t)).normalize();
                o = pos - n * eps;
                in_water = front;
            }
            specular = true;
            tmin = 0.0;
            continue;
        }

        if !forced && flags & FLAG_ALPHA_MASK == ALPHA_TRANSLUCENT && rng.next() >= alpha {
            // Pass through (stained glass, ice...), filtered by the surface colour.
            beta = beta.mul(V3::ONE.lerp(albedo, alpha));
            o = pos + d * eps;
            tmin = 0.0;
            continue;
        }

        // Chunky's path depth: the surface after the last bounce adds nothing.
        if bounces >= max_bounces {
            break;
        }
        // Chunky's `doDiffuseReflection`: an emitting surface adds its emitted light
        // (unless emitters are sampled, the path has bounced before and
        // `preventNormalEmitterWithSampling` is set) and is then not lit by emitter
        // sampling; any other surface is.
        let emittance = q.emittance() * p.misc_f[0];
        let emits = emittance > 0.0
            && (p.emitter[1] == 0 || strategy == EMITTER_SAMPLING_NONE || bounces == 0);
        if emits {
            radiance = radiance + beta.mul(albedo.mul(albedo)) * emittance;
        }
        bounces += 1;

        // Next-event estimation towards the sun: a uniform direction in its cone lights
        // the surface when it faces it (Chunky's `frontLight` in `doDiffuseReflection`)
        // and, Lumenwright's own, when it is not below the horizon, where the ground
        // beyond the world hides the sun. Some direction in the cone passes both tests
        // only when the sun's centre is less than its radius below the surface's plane
        // and below the horizon, so no sample is drawn otherwise.
        let origin = pos + n * eps;
        let sun = V3::from4(p.sun_dir);
        let sin_r = sqrt(max(1.0 - p.sun_dir[3] * p.sun_dir[3], 0.0));
        if n.dot(sun) > -sin_r && p.sun_dir[1] > -sin_r {
            let l = sample_sun(scene, &mut rng);
            let cos_l = n.dot(l);
            if cos_l > 0.0 && l.y >= 0.0 {
                let tr = shadow(scene, origin, l, f32::INFINITY);
                radiance = radiance + beta.mul(albedo).mul(V3::from4(p.sun_color)).mul(tr) * cos_l;
            }
        }
        if !emits && strategy != EMITTER_SAMPLING_NONE {
            radiance = radiance
                + beta
                    .mul(albedo)
                    .mul(sample_emitters(scene, pos, origin, n, &mut rng));
        }

        beta = beta.mul(albedo);
        if bounces >= 3 {
            let q_rr = clamp(beta.max_elem(), 0.05, 0.95);
            if rng.next() > q_rr {
                break;
            }
            beta = beta / q_rr;
        }
        d = cosine_hemisphere(n, &mut rng);
        o = origin;
        tmin = 0.0;
        specular = false;
    }
    radiance
}
