//! Flat scene layout shared by every backend.
//!
//! Grid buffer (`u32` words), offsets in [`Params::off`]:
//! * `top[nb]`: one entry per 16³ brick. `EMPTY`, `UNIFORM | model`, or a brick index.
//! * `bricks[n * 2048]`: 4096 u16 model-instance ids per brick, two per word
//!   (low half = even voxel). Voxel index inside a brick is `(y * 16 + z) * 16 + x`.
//! * `ov_top[nb]`: `EMPTY` or an overlay brick index (entity geometry).
//! * `ov_cells[n * 8192]`: per voxel `(offset, count)` into `ov_quads`.
//! * `ov_quads[..]`: indices into the quad array (world-space entity quads).

use bytemuck::{Pod, Zeroable};

use crate::math::{V3, v3};

pub const EMPTY: u32 = 0xFFFF_FFFF;
pub const UNIFORM: u32 = 0x8000_0000;
pub const BRICK_WORDS: u32 = 2048;
pub const OV_CELL_WORDS: u32 = 8192;

/// Alpha handling for a quad (bits 0-1 of the flags).
pub const ALPHA_OPAQUE: u32 = 0;
pub const ALPHA_CUTOUT: u32 = 1;
pub const ALPHA_TRANSLUCENT: u32 = 2;
pub const FLAG_ALPHA_MASK: u32 = 3;
/// Water surface: dielectric with Fresnel reflection/refraction.
pub const FLAG_WATER: u32 = 1 << 2;
/// Vanilla `shade` flag (directional face shading in quick mode).
pub const FLAG_SHADE: u32 = 1 << 3;

/// A textured parallelogram `p0 + s*eu + t*ev`, `s,t ∈ [0,1]`, 80 bytes.
///
/// Texture coordinates are in atlas pixels: `uv = uv0 + s*uvs + t*uvt`.
/// Block-model quads are in block-local coordinates (0..1), entity quads in
/// grid-relative world coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuQuad {
    /// xyz = p0, w = flags (bit cast)
    pub p0: [f32; 4],
    /// xyz = edge along s, w = tint rgba8 (bit cast, sRGB)
    pub eu: [f32; 4],
    /// xyz = edge along t, w = emission strength (block light / 15)
    pub ev: [f32; 4],
    /// uv0.xy, uvs.xy
    pub uv: [f32; 4],
    /// uvt.xy, unused, unused
    pub uvt: [f32; 4],
}

impl GpuQuad {
    #[inline(always)]
    pub fn flags(&self) -> u32 {
        self.p0[3].to_bits()
    }
    #[inline(always)]
    pub fn tint(&self) -> u32 {
        self.eu[3].to_bits()
    }
    #[inline(always)]
    pub fn emission(&self) -> f32 {
        self.ev[3]
    }
    #[inline(always)]
    pub fn origin(&self) -> V3 {
        V3::from4(self.p0)
    }
    #[inline(always)]
    pub fn edge_u(&self) -> V3 {
        V3::from4(self.eu)
    }
    #[inline(always)]
    pub fn edge_v(&self) -> V3 {
        V3::from4(self.ev)
    }
    /// Outward normal (vanilla winding: `ev × eu`), not normalized.
    #[inline(always)]
    pub fn normal(&self) -> V3 {
        self.edge_v().cross(self.edge_u())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuModel {
    pub quad_offset: u32,
    pub quad_count: u32,
    pub flags: u32,
    pub _pad: u32,
}

/// Per-frame parameters (uniform buffer), 16 × vec4.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Params {
    /// Camera position relative to grid origin; w = tan(fov_y / 2).
    pub cam_pos: [f32; 4],
    /// w = aspect ratio (width / height).
    pub cam_fwd: [f32; 4],
    pub cam_right: [f32; 4],
    pub cam_up: [f32; 4],
    /// Unit vector towards the sun; w = cos(angular radius).
    pub sun_dir: [f32; 4],
    /// Linear sun irradiance colour; w = angular radius (rad).
    pub sun_color: [f32; 4],
    /// Linear sky colour at the zenith; w = sky intensity.
    pub sky_zenith: [f32; 4],
    /// Linear sky colour at the horizon; w = sun-glow strength.
    pub sky_horizon: [f32; 4],
    /// Water absorption coefficients per block (Beer-Lambert); w = water IOR.
    pub water_color: [f32; 4],
    /// x = emitter intensity, y = ray epsilon, z = sun disk radiance scale, w = unused.
    pub misc_f: [f32; 4],
    /// Grid size in bricks (xyz); w = brick count.
    pub grid_dims: [u32; 4],
    /// Word offsets of top, bricks, ov_top, ov_cells.
    pub off: [u32; 4],
    /// x = ov_quads offset, y = atlas width, z = atlas height, w = quad count.
    pub off2: [u32; 4],
    /// x = width, y = height, z = sample index, w = max diffuse bounces.
    pub frame: [u32; 4],
    /// x = seed, y = samples per dispatch, z = model count, w = unused.
    pub misc_u: [u32; 4],
    pub _reserved: [u32; 4],
}

/// Borrowed view of all scene buffers.
#[derive(Clone, Copy)]
pub struct SceneView<'a> {
    pub params: &'a Params,
    pub grid: &'a [u32],
    pub models: &'a [GpuModel],
    pub quads: &'a [GpuQuad],
    pub atlas: &'a [u32],
}

impl SceneView<'_> {
    #[inline(always)]
    pub fn grid_size(&self) -> V3 {
        let d = self.params.grid_dims;
        v3((d[0] * 16) as f32, (d[1] * 16) as f32, (d[2] * 16) as f32)
    }

    /// Model id stored at voxel `(x, y, z)` (grid-local, in range).
    #[inline(always)]
    pub fn voxel(&self, x: i32, y: i32, z: i32) -> u32 {
        let d = self.params.grid_dims;
        let b = (((y >> 4) as u32 * d[2] + (z >> 4) as u32) * d[0] + (x >> 4) as u32) as usize;
        let e = self.grid[self.params.off[0] as usize + b];
        if e == EMPTY {
            return 0;
        }
        if e & UNIFORM != 0 {
            return e & !UNIFORM;
        }
        let local = (((y & 15) * 16 + (z & 15)) * 16 + (x & 15)) as u32;
        let w = self.grid[(self.params.off[1] + e * BRICK_WORDS + (local >> 1)) as usize];
        (w >> ((local & 1) * 16)) & 0xFFFF
    }

    /// RGBA8 texel (sRGB, straight alpha) at atlas pixel (u, v).
    #[inline(always)]
    pub fn texel(&self, u: f32, v: f32) -> u32 {
        let w = self.params.off2[1] as i32;
        let h = self.params.off2[2] as i32;
        let x = (crate::math::floor(u) as i32).clamp(0, w - 1);
        let y = (crate::math::floor(v) as i32).clamp(0, h - 1);
        self.atlas[(y * w + x) as usize]
    }
}

#[inline(always)]
pub fn unpack_rgba(c: u32) -> [f32; 4] {
    [
        (c & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 24) & 0xFF) as f32 / 255.0,
    ]
}
