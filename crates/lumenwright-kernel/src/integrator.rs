//! Unidirectional path tracer in the spirit of Chunky: diffuse surfaces lit by a
//! sun (next-event estimation) and an analytic sky, emissive blocks, Fresnel
//! water and stochastic translucency.

use crate::math::{V3, clamp, cos, max, onb, sin, sqrt, srgb_to_linear, v3};
use crate::rng::Rng;
use crate::scene::{
    ALPHA_TRANSLUCENT, FLAG_ALPHA_MASK, FLAG_WATER, GpuQuad, SceneView, unpack_rgba,
};
use crate::trace::{is_see_through, see_through_hit, trace};

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
    let phi = 2.0 * PI * r2;
    let (t, b) = onb(n);
    (t * (r * cos(phi)) + b * (r * sin(phi)) + n * sqrt(max(1.0 - r1, 0.0))).normalize()
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
    let phi = 2.0 * PI * r2;
    let (t, b) = onb(sun);
    (t * (sin_t * cos(phi)) + b * (sin_t * sin(phi)) + sun * cos_t).normalize()
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

/// Transmittance from `o` towards the sun direction `d` (0 when blocked).
fn shadow(scene: &SceneView, mut o: V3, d: V3) -> V3 {
    let eps = scene.params.misc_f[1];
    let mut tr = V3::ONE;
    for _ in 0..8 {
        match trace(scene, o, d, eps, f32::INFINITY) {
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
            }
        }
    }
    tr
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

        let emission = q.emission();
        if emission > 0.0 {
            radiance = radiance + beta.mul(albedo) * (emission * p.misc_f[0]);
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
                let tr = shadow(scene, origin, l);
                radiance = radiance + beta.mul(albedo).mul(V3::from4(p.sun_color)).mul(tr) * cos_l;
            }
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
