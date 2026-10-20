//! The game's lightmap: how a block's stored sky and block light levels become
//! the brightness its faces are drawn with, and the share of block light that
//! brightness lets show.
//!
//! The client draws a face at `clamp(block colour + sky colour)` per channel (its
//! `lightmap` shader; `lumenwright::scene::mesh::lightmap` is the quick mode's
//! transcription and calls the functions here). The two terms add and then clip,
//! so block light beside full daylight adds nothing: the sky term alone already
//! reaches 1. [`block_share`] is that clipping as a fraction of the block term,
//! which HQ applies to the light emissive blocks cast (see `integrator`).

use crate::math::{V3, clamp, max, min, v3};

/// The client's brightness curve for a light level `0..=15` (fractional levels
/// allowed): `f / (4 - 3f)` with `f = level / 15`.
#[inline(always)]
pub fn curve(level: f32) -> f32 {
    let f = clamp(level / 15.0, 0.0, 1.0);
    f / (4.0 - 3.0 * f)
}

/// The block-light term of the lightmap for a block light level: the curve scaled
/// by 1.5 and warmed (green and blue fall off faster than red).
#[inline(always)]
pub fn block_rgb(block: f32) -> V3 {
    let b = curve(block) * 1.5;
    v3(
        b,
        b * ((b * 0.6 + 0.4) * 0.6 + 0.4),
        b * (b * b * 0.6 + 0.4),
    )
}

/// The sky-light term of the lightmap for a sky light level under the daylight
/// factor `sky_factor` (vanilla `skyDarken`: 1 at noon, about 0.2 at midnight).
#[inline(always)]
pub fn sky_rgb(sky: f32, sky_factor: f32) -> V3 {
    let col = v3(sky_factor, sky_factor, 1.0).lerp(V3::ONE, 0.35);
    col * (curve(sky) * sky_factor)
}

/// The share of the block-light term the lightmap shows at a block with these
/// levels, per channel: `min(block, max(1 - sky, 0)) / block`, the part of the
/// block term left after the sum is clipped at 1. It is 1 where block light is
/// all that lights a place, 0 where sky light alone reaches full brightness, and
/// 0 where the game's block light is 0 (the game shows no block light there).
#[inline(always)]
pub fn block_share(sky: f32, block: f32, sky_factor: f32) -> V3 {
    let b = block_rgb(block);
    let s = sky_rgb(sky, sky_factor);
    let share = |b: f32, s: f32| {
        if b <= 0.0 {
            0.0
        } else {
            min(max(1.0 - s, 0.0), b) / b
        }
    };
    v3(share(b.x, s.x), share(b.y, s.y), share(b.z, s.z))
}
