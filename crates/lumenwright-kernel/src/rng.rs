//! PCG-hash based RNG. Identical bit-for-bit to the WGSL implementation.

#[inline(always)]
pub fn pcg_hash(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

pub struct Rng {
    pub state: u32,
}

#[allow(clippy::should_implement_trait)]
impl Rng {
    #[inline(always)]
    pub fn new(pixel: u32, sample: u32, seed: u32) -> Rng {
        Rng {
            state: pcg_hash(pixel ^ pcg_hash(sample.wrapping_add(pcg_hash(seed)))),
        }
    }

    /// Uniform float in [0, 1).
    #[inline(always)]
    pub fn next(&mut self) -> f32 {
        self.state = pcg_hash(self.state);
        (self.state >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}
