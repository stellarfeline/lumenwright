//! Unidirectional path tracer in the spirit of Chunky: diffuse surfaces lit by a
//! sun and by emissive quads (both with next-event estimation; emitters combined
//! with BSDF sampling by multiple importance sampling) and an analytic sky,
//! Fresnel water and stochastic translucency.
//!
//! An emissive quad has two radiances. What a camera ray, or a chain of specular
//! events from the camera, sees on it is its texel colour times
//! [`GpuQuad::emission`] times `misc_f.x`; what lights a diffuse surface is its
//! texel colour squared (Chunky's quadratic emitter mapping: a texel half as
//! bright casts a quarter of the light, so a torch's flame lights the room and
//! its stick barely does) times [`GpuQuad::light_scale`] times `misc_f.w`.

use crate::math::{V3, abs, clamp, max, onb, sin_cos_turns, sqrt, srgb_to_linear, v3};
use crate::rng::Rng;
use crate::scene::{
    ALPHA_CUTOUT, ALPHA_TRANSLUCENT, FLAG_ALPHA_MASK, FLAG_FRONT_ONLY, FLAG_WATER, GpuQuad,
    SceneView, unpack_rgba,
};
use crate::trace::{is_see_through, quad_texel, see_through_hit, trace};

const PI: f32 = core::f32::consts::PI;
/// Upper bound on surface interactions per path (diffuse + specular + pass-through).
const MAX_EVENTS: u32 = 48;

#[inline(always)]
fn linear_rgb(c: u32) -> V3 {
    let c = unpack_rgba(c);
    v3(
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
    )
}

/// Analytic sky: zenith/horizon gradient plus a glow around the sun.
pub fn sky_radiance(scene: &SceneView, d: V3) -> V3 {
    let p = scene.params;
    let zenith = V3::from4(p.sky_zenith);
    let horizon = V3::from4(p.sky_horizon);
    let intensity = p.sky_zenith[3];
    let up = max(d.y, 0.0);
    let base = if d.y >= 0.0 {
        horizon.lerp(zenith, sqrt(up))
    } else {
        // Below the horizon: fade towards a darker ground-bounce colour.
        horizon * (1.0 - 0.5 * clamp(-d.y * 4.0, 0.0, 1.0))
    };
    let sun = V3::from4(p.sun_dir);
    let cs = max(d.dot(sun), 0.0);
    let glow = cs * cs;
    let glow = glow * glow * glow * glow; // ^8
    let sun_col = V3::from4(p.sun_color);
    (base + sun_col * (glow * p.sky_horizon[3])) * intensity
}

#[inline(always)]
fn sun_disk(scene: &SceneView, d: V3) -> V3 {
    let p = scene.params;
    let sun = V3::from4(p.sun_dir);
    if d.dot(sun) >= p.sun_dir[3] {
        V3::from4(p.sun_color) * p.misc_f[2]
    } else {
        V3::ZERO
    }
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

/// The index of the light whose cumulative pick probability first exceeds `u`.
#[inline(always)]
fn pick_light(scene: &SceneView, u: f32) -> u32 {
    let (mut lo, mut hi) = (0u32, scene.params.misc_u[3] - 1);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if scene.lights[mid as usize].cdf[0] > u {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// Emitted light reaching a surface `dist` blocks away is scaled by `min(dist², 1)`:
/// its falloff is `1 / max(dist², 1)` rather than `1 / dist²` (Chunky's emitter
/// sampling does the same), so a surface touching a lamp is lit as one a block away
/// rather than without bound. Light sampling and BSDF hits apply it alike.
#[inline(always)]
fn near_field(dist: f32) -> f32 {
    let d2 = dist * dist;
    if d2 < 1.0 { d2 } else { 1.0 }
}

/// Power heuristic (β = 2) weight of a strategy with density `a` against one with `b`.
#[inline(always)]
fn power_heuristic(a: f32, b: f32) -> f32 {
    let (a2, b2) = (a * a, b * b);
    if a2 + b2 > 0.0 { a2 / (a2 + b2) } else { 0.0 }
}

/// The probability light sampling gives texel cell `k` of the emitter-table entry at `e`.
#[inline(always)]
fn texel_share(scene: &SceneView, e: usize, k: usize) -> f32 {
    let hi = f32::from_bits(scene.emitters[e + 4 + k]);
    let lo = if k > 0 {
        f32::from_bits(scene.emitters[e + 3 + k])
    } else {
        0.0
    };
    hi - lo
}

/// Solid-angle density with which light sampling picks the point `(s, t)` of quad
/// `q`, at distance `dist` along `d` (unit) from the shading point.
#[inline(always)]
fn light_pdf(scene: &SceneView, q: &GpuQuad, d: V3, dist: f32, s: f32, t: f32) -> f32 {
    let Some(e) = q.emitter() else {
        return 0.0;
    };
    let e = e as usize;
    let nq = q.normal();
    let area = nq.length();
    let cos_l = abs(nq.dot(d)) / area;
    if cos_l < 1e-6 {
        return 0.0;
    }
    let (ns, nt) = (scene.emitters[e + 1], scene.emitters[e + 2]);
    let i = ((s * ns as f32) as u32).min(ns - 1);
    let j = ((t * nt as f32) as u32).min(nt - 1);
    let share = texel_share(scene, e, (j * ns + i) as usize);
    let pick = f32::from_bits(scene.emitters[e]);
    pick * share * (ns * nt) as f32 / area * dist * dist / cos_l
}

/// Next-event estimation towards one emissive quad, weighted by MIS against
/// cosine-weighted BSDF sampling. Returns the radiance reflected towards the path
/// by a diffuse surface of unit albedo at `origin` with normal `n`.
fn sample_emitter(scene: &SceneView, origin: V3, n: V3, rng: &mut Rng) -> V3 {
    let p = scene.params;
    let u = rng.next();
    let v = rng.next();
    let ju = rng.next();
    let jv = rng.next();
    let light = scene.lights[pick_light(scene, u) as usize];
    let q = &scene.quads[light.quad() as usize];
    let Some(e) = q.emitter() else {
        return V3::ZERO;
    };
    let e = e as usize;
    // A texel cell by its share of the quad's power, then a point inside it.
    let (ns, nt) = (scene.emitters[e + 1], scene.emitters[e + 2]);
    let (mut lo, mut hi) = (0u32, ns * nt - 1);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if f32::from_bits(scene.emitters[e + 4 + mid as usize]) > v {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let s = ((lo % ns) as f32 + ju) / ns as f32;
    let t = ((lo / ns) as f32 + jv) / nt as f32;
    let at = light.offset() + q.origin() + q.edge_u() * s + q.edge_v() * t;
    let to = at - origin;
    let dist = to.length();
    if dist < 1e-6 {
        return V3::ZERO;
    }
    let l = to / dist;
    let cos_s = n.dot(l);
    if cos_s <= 0.0 {
        return V3::ZERO;
    }
    // A one-sided quad casts light from its front only.
    if q.flags() & FLAG_FRONT_ONLY != 0 && q.normal().dot(l) >= 0.0 {
        return V3::ZERO;
    }
    let pdf_l = light_pdf(scene, q, l, dist, s, t);
    if pdf_l <= 0.0 {
        return V3::ZERO;
    }
    let texel = quad_texel(scene, q, s, t);
    let alpha = (texel >> 24) as f32 / 255.0;
    let mode = q.flags() & FLAG_ALPHA_MASK;
    let cover = if mode == ALPHA_CUTOUT {
        if texel >> 24 < 128 {
            return V3::ZERO;
        }
        1.0
    } else if mode == ALPHA_TRANSLUCENT {
        alpha
    } else {
        1.0
    };
    let tr = shadow(scene, origin, l, dist * (1.0 - 1e-4) - p.misc_f[1]);
    let c = linear_rgb(texel).mul(linear_rgb(q.tint()));
    let le = c.mul(c) * (q.light_scale() * p.misc_f[3] * cover * near_field(dist));
    let pdf_b = cos_s / PI;
    le.mul(tr) * (cos_s / PI * power_heuristic(pdf_l, pdf_b) / pdf_l)
}

/// Radiance estimate for one camera sample at pixel (px, py).
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
    // Since the last diffuse bounce: where it was, its BSDF density for the
    // direction taken, and whether the path has crossed water since.
    let mut last_diffuse = V3::ZERO;
    let mut pdf_bsdf = 0.0f32;
    let mut via_water = false;
    let lights = p.misc_u[3];
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
                    // Only purely specular camera paths see the sun disk; diffuse
                    // paths already account for it through next-event estimation.
                    if specular && bounces == 0 {
                        radiance = radiance + beta.mul(sun_disk(scene, d));
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
            via_water = true;
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

        if bounces == 0 {
            // Seen from the camera, directly or through specular events.
            let emission = q.emission();
            if emission > 0.0 {
                radiance = radiance + beta.mul(albedo) * (emission * p.misc_f[0]);
            }
        } else if q.light_scale() > 0.0 && !via_water {
            // Found by BSDF sampling; light sampling also finds it unless the path
            // crossed water, whose light reaches diffuse surfaces by light sampling alone.
            let dist = (pos - last_diffuse).length();
            let w = if q.emitter().is_some() && lights > 0 {
                power_heuristic(pdf_bsdf, light_pdf(scene, q, d, dist, h.s, h.tt))
            } else {
                1.0
            };
            radiance = radiance
                + beta.mul(albedo.mul(albedo))
                    * (q.light_scale() * p.misc_f[3] * w * near_field(dist));
        }

        if bounces >= max_bounces {
            break;
        }
        bounces += 1;

        // Next-event estimation towards the sun.
        let origin = pos + n * eps;
        let sun = V3::from4(p.sun_dir);
        if n.dot(sun) > 0.0 && p.sun_dir[1] > -0.1 {
            let l = sample_sun(scene, &mut rng);
            let cos_l = n.dot(l);
            if cos_l > 0.0 {
                let tr = shadow(scene, origin, l, f32::INFINITY);
                radiance = radiance + beta.mul(albedo).mul(V3::from4(p.sun_color)).mul(tr) * cos_l;
            }
        }
        // Next-event estimation towards an emissive quad.
        if lights > 0 {
            radiance = radiance
                + beta
                    .mul(albedo)
                    .mul(sample_emitter(scene, origin, n, &mut rng));
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
        last_diffuse = origin;
        pdf_bsdf = n.dot(d) / PI;
        via_water = false;
    }
    radiance
}
