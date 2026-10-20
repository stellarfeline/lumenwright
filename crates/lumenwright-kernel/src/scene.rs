//! Flat scene layout shared by every backend.
//!
//! Grid buffer (`u32` words), offsets in [`Params::off`]:
//! * `top[nb]`: one entry per 16³ brick. `EMPTY`, `UNIFORM | model`, or a brick index.
//! * `bricks[n * 2048]`: 4096 u16 model-instance ids per brick, two per word
//!   (low half = even voxel). Voxel index inside a brick is `(y * 16 + z) * 16 + x`.
//! * `ov_top[nb]`: `EMPTY` or an overlay brick index (entity geometry).
//! * `ov_cells[n * 8192]`: per voxel `(offset, count)` into `ov_quads`.
//! * `ov_quads[..]`: indices into the quad array (world-space entity quads).
//! * `lt_top[nb]`: the game's stored light per brick (offset [`Params::light_grid`]`.x`):
//!   `UNIFORM | level byte` when every voxel of the brick has the same levels, else a
//!   light-brick index.
//! * `lt_bricks[n * 1024]` (offset [`Params::light_grid`]`.y`): one level byte per voxel,
//!   four per word (lowest byte = voxel index ≡ 0 mod 4), the byte being
//!   `sky << 4 | block`. Voxel index as in `bricks`.

use bytemuck::{Pod, Zeroable};

use crate::math::{V3, v3};

/// A top-level or overlay entry for a brick that holds nothing.
pub const EMPTY: u32 = 0xFFFF_FFFF;
/// Set on a top-level entry whose whole brick is one model; the low bits are the model id.
pub const UNIFORM: u32 = 0x8000_0000;
/// Words per brick in the `bricks` region: 4096 u16 model ids, two per word.
pub const BRICK_WORDS: u32 = 2048;
/// Words per light brick in the `lt_bricks` region: 4096 level bytes, four per word.
pub const LIGHT_BRICK_WORDS: u32 = 1024;
/// Words per overlay brick in the `ov_cells` region: an `(offset, count)` pair per voxel.
pub const OV_CELL_WORDS: u32 = 8192;

/// Alpha handling for a quad (bits 0-1 of the flags).
pub const ALPHA_OPAQUE: u32 = 0;
/// Alpha mode: texels with alpha below 128 are holes.
pub const ALPHA_CUTOUT: u32 = 1;
/// Alpha mode: texels are blended by their alpha (stochastically in HQ mode).
pub const ALPHA_TRANSLUCENT: u32 = 2;
/// Mask for the alpha mode bits of a quad's flags.
pub const FLAG_ALPHA_MASK: u32 = 3;
/// Water surface: dielectric with Fresnel reflection/refraction.
pub const FLAG_WATER: u32 = 1 << 2;
/// Vanilla `shade` flag (directional face shading in quick mode).
pub const FLAG_SHADE: u32 = 1 << 3;
/// Seen from the front only (text displays): a ray meeting the back passes through.
pub const FLAG_FRONT_ONLY: u32 = 1 << 4;

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
    /// uvt.xy, z = emitter table offset + 1 (bit cast; 0 = not sampled as a light),
    /// w = light radiance scale
    pub uvt: [f32; 4],
}

impl GpuQuad {
    /// The flag bits: alpha mode (`FLAG_ALPHA_MASK`), [`FLAG_WATER`], [`FLAG_SHADE`].
    #[inline(always)]
    pub fn flags(&self) -> u32 {
        self.p0[3].to_bits()
    }
    /// The tint colour, RGBA8 sRGB packed little-endian (red in the low byte).
    #[inline(always)]
    pub fn tint(&self) -> u32 {
        self.eu[3].to_bits()
    }
    /// Visible emission: the radiance a camera or specular ray sees on the quad is
    /// its texel colour times this times [`Params::misc_f`]`.x`.
    #[inline(always)]
    pub fn emission(&self) -> f32 {
        self.ev[3]
    }
    /// The quad's entry in [`SceneView::emitters`], when light sampling can pick it.
    #[inline(always)]
    pub fn emitter(&self) -> Option<u32> {
        let e = self.uvt[2].to_bits();
        if e == 0 { None } else { Some(e - 1) }
    }
    /// Emitted radiance scale: the radiance a diffuse surface receives from the quad
    /// is its texel colour times this times [`Params::misc_f`]`.w`; 0 casts no light.
    #[inline(always)]
    pub fn light_scale(&self) -> f32 {
        self.uvt[3]
    }
    /// The corner `p0`.
    #[inline(always)]
    pub fn origin(&self) -> V3 {
        V3::from4(self.p0)
    }
    /// The edge along `s`.
    #[inline(always)]
    pub fn edge_u(&self) -> V3 {
        V3::from4(self.eu)
    }
    /// The edge along `t`.
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

/// A block model: a run of consecutive quads in [`SceneView::quads`],
/// in block-local coordinates. Model id 0 is air and is never traced.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuModel {
    /// Index of the model's first quad.
    pub quad_offset: u32,
    /// Number of quads in the model.
    pub quad_count: u32,
    /// Reserved, written as 0.
    pub flags: u32,
    /// Padding to 16 bytes.
    pub _pad: u32,
}

/// One instance of a light-casting quad, 32 bytes: the quad at `quad`, moved by
/// `offset` into grid space (a block model's voxel corner; zero for a quad
/// already in grid space). Lights are drawn by binary search on `cdf`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuLight {
    /// xyz = offset into grid space, w = quad index (bit cast)
    pub pos: [f32; 4],
    /// x = cumulative pick probability up to and including this light; y, z, w unused
    pub cdf: [f32; 4],
}

impl GpuLight {
    /// The index of the light's quad in [`SceneView::quads`].
    #[inline(always)]
    pub fn quad(&self) -> u32 {
        self.pos[3].to_bits()
    }
    /// The offset that moves the quad into grid space.
    #[inline(always)]
    pub fn offset(&self) -> V3 {
        V3::from4(self.pos)
    }
}

/// Per-frame parameters (uniform buffer), 18 × vec4.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Params {
    /// Camera position relative to grid origin; w = tan(fov_y / 2).
    pub cam_pos: [f32; 4],
    /// w = aspect ratio (width / height).
    pub cam_fwd: [f32; 4],
    /// Camera right unit vector; w unused.
    pub cam_right: [f32; 4],
    /// Camera up unit vector; w unused.
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
    /// x = visible emitter radiance scale, y = ray epsilon, z = sun disk radiance scale,
    /// w = emitted light scale (see [`GpuQuad::emission`] and [`GpuQuad::light_scale`]).
    pub misc_f: [f32; 4],
    /// Grid size in bricks (xyz); w = brick count.
    pub grid_dims: [u32; 4],
    /// Word offsets of top, bricks, ov_top, ov_cells.
    pub off: [u32; 4],
    /// x = ov_quads offset, y = atlas width, z = atlas height, w = quad count.
    pub off2: [u32; 4],
    /// x = width, y = height, z = sample index, w = max diffuse bounces.
    pub frame: [u32; 4],
    /// x = seed, y = samples per dispatch, z = model count, w = light count.
    pub misc_u: [u32; 4],
    /// x = index of the first see-through quad (text displays drawn over everything),
    /// y = their count; they are also in the overlay cells. z, w = unused.
    pub see_through: [u32; 4],
    /// x = word offset of `lt_top`, y = word offset of `lt_bricks`, z = 1 when the
    /// light emissive blocks cast is limited by the game's light levels (the world
    /// stores them), 0 when it is not; w unused.
    pub light_grid: [u32; 4],
    /// x = the lightmap's daylight factor (vanilla `skyDarken`) for the frame;
    /// y, z, w unused.
    pub lightmap: [f32; 4],
}

/// Borrowed view of all scene buffers.
#[derive(Clone, Copy)]
pub struct SceneView<'a> {
    /// Per-frame parameters.
    pub params: &'a Params,
    /// The grid buffer, laid out as the module documentation describes.
    pub grid: &'a [u32],
    /// Block models, indexed by the model ids in the grid.
    pub models: &'a [GpuModel],
    /// Every quad: block-model quads first, then world-space entity quads.
    pub quads: &'a [GpuQuad],
    /// The texture atlas, RGBA8 sRGB, row-major, `off2.y` wide and `off2.z` high.
    pub atlas: &'a [u32],
    /// The light list, `misc_u.w` entries (a buffer is never empty: it holds one
    /// unused entry when there are no lights).
    pub lights: &'a [GpuLight],
    /// The emitter table: per light-casting quad, at the offset its
    /// [`GpuQuad::emitter`] gives, the words `[pick, ns, nt, 0]` and then `ns × nt`
    /// cumulative texel probabilities (f32 bits, row by row along `s`). `pick` (f32
    /// bits) is the probability that light sampling picks one given instance of the
    /// quad; a texel is picked by its share of the quad's emitted power.
    pub emitters: &'a [u32],
}

impl SceneView<'_> {
    /// The grid's extent in voxels (blocks).
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

    /// The game's stored `(sky, block)` light levels at voxel `(x, y, z)`; outside
    /// the grid, full sky light and no block light (vanilla's light above the world).
    #[inline(always)]
    pub fn light_levels(&self, x: i32, y: i32, z: i32) -> (u32, u32) {
        let d = self.params.grid_dims;
        if x < 0
            || y < 0
            || z < 0
            || x >= (d[0] * 16) as i32
            || y >= (d[1] * 16) as i32
            || z >= (d[2] * 16) as i32
        {
            return (15, 0);
        }
        let b = (((y >> 4) as u32 * d[2] + (z >> 4) as u32) * d[0] + (x >> 4) as u32) as usize;
        let e = self.grid[self.params.light_grid[0] as usize + b];
        let byte = if e & UNIFORM != 0 {
            e & 0xFF
        } else {
            let local = (((y & 15) * 16 + (z & 15)) * 16 + (x & 15)) as u32;
            let w = self.grid
                [(self.params.light_grid[1] + e * LIGHT_BRICK_WORDS + (local >> 2)) as usize];
            (w >> ((local & 3) * 8)) & 0xFF
        };
        (byte >> 4, byte & 15)
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

/// Unpacks RGBA8 (red in the low byte) into four `[0, 1]` floats, without colour conversion.
#[inline(always)]
pub fn unpack_rgba(c: u32) -> [f32; 4] {
    [
        (c & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 24) & 0xFF) as f32 / 255.0,
    ]
}
