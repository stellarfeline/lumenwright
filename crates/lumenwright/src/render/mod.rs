//! Rendering backends and the inputs they share.
//!
//! Every backend implements [`Backend`]. Quick mode consumes a [`Mesh`] (rasterized),
//! HQ mode consumes a [`HqScene`] (path traced). Shading formulas are kept
//! identical between the Rust and WGSL implementations so backends agree.

pub mod cpu;
#[cfg(feature = "cuda")]
pub mod cuda;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod post;
pub mod sky;
pub mod texture;

use anyhow::Result;
#[cfg(any(not(feature = "cuda"), not(feature = "gpu")))]
use anyhow::bail;
use glam::{Mat4, Vec3};

use crate::scene::mesh::Mesh;
use crate::scene::voxel::HqScene;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// How a frame is drawn.
pub enum Mode {
    /// Vanilla-client look: rasterized, no ray tracing.
    Quick,
    /// Chunky-like path tracing.
    Hq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Which backend draws a frame.
pub enum BackendKind {
    /// Best available: Metal/Vulkan/DX12 GPU, then CPU.
    Auto,
    /// The CPU backend: software rasterizer and the Rust kernel on rayon.
    Cpu,
    /// Any native GPU API wgpu can find.
    Gpu,
    /// Metal through wgpu (macOS).
    Metal,
    /// Vulkan through wgpu (Linux, Windows).
    Vulkan,
    /// DX12 through wgpu (Windows).
    Dx12,
    /// CUDA, with the Rust kernel compiled to PTX (cargo feature `cuda`). Quick mode
    /// falls back to the CPU rasterizer.
    Cuda,
}

/// Inputs for quick mode, already scaled for supersampling.
pub struct QuickFrame<'a> {
    /// The triangle mesh to draw, with camera-relative positions.
    pub mesh: &'a Mesh,
    /// Texture atlas texels, packed RGBA8 with red in the low byte.
    pub atlas_pixels: &'a [u32],
    /// Atlas width and height in texels.
    pub atlas_size: [u32; 2],
    /// The atlas's mip levels above 0, concatenated (see [`crate::assets::atlas::Atlas::mips`]).
    pub atlas_mips: &'a [u32],
    /// Number of mip levels above 0.
    pub mip_levels: u32,
    /// Atlas rows `0..mip_rows` hold the mipmapped (block) textures.
    pub mip_rows: u32,
    /// The binned star table ([`sky::star_table`]).
    pub stars: &'a [u32],
    /// Output width in pixels (after supersampling).
    pub width: u32,
    /// Output height in pixels (after supersampling).
    pub height: u32,
    /// Camera-relative view-projection.
    pub view_proj: Mat4,
    /// Background, fog and sun.
    pub sky: sky::QuickSky,
}

/// Inputs for HQ mode.
pub struct HqFrame<'a> {
    /// The voxel scene to trace.
    pub scene: &'a HqScene,
    /// Kernel parameters: camera, frame size, sky, sun and sampling settings.
    pub params: lumenwright_kernel::Params,
    /// Samples per pixel.
    pub spp: u32,
    /// The sky cache for `params` ([`crate::render::sky::sky_cache`]).
    pub sky: Vec<f32>,
}

/// A rendering backend. Open one with [`open_backend`] or [`create_backend`].
pub trait Backend {
    /// A human-readable name, such as `cpu (10 threads)` or the GPU adapter's name.
    fn name(&self) -> String;
    /// Returns RGBA8 (sRGB) pixels, row major, top row first.
    fn render_quick(&mut self, frame: &QuickFrame) -> Result<Vec<u32>>;
    /// Returns the mean linear radiance per pixel (rgb).
    fn render_hq(
        &mut self,
        frame: &HqFrame,
        progress: &(dyn Fn(u32, u32) + Sync),
    ) -> Result<Vec<[f32; 3]>>;
}

/// A backend and, for [`BackendKind::Auto`], why it is not a GPU.
pub struct OpenedBackend {
    /// The opened backend.
    pub backend: Box<dyn Backend>,
    /// Set when `Auto` found no usable GPU and took the CPU: the reason.
    pub fallback: Option<String>,
}

/// Opens a backend, falling back from GPU to CPU for `Auto` and saying so in
/// [`OpenedBackend::fallback`]. A named backend that is unavailable is an error.
pub fn open_backend(kind: BackendKind) -> Result<OpenedBackend> {
    let named = |backend: Box<dyn Backend>| OpenedBackend {
        backend,
        fallback: None,
    };
    match kind {
        BackendKind::Cpu => Ok(named(Box::<cpu::CpuBackend>::default())),
        BackendKind::Cuda => {
            #[cfg(feature = "cuda")]
            {
                Ok(named(Box::new(cuda::CudaBackend::new()?)))
            }
            #[cfg(not(feature = "cuda"))]
            {
                bail!(
                    "this build has no CUDA support; rebuild with `--features cuda` (after scripts/build-ptx.sh), \
                     or use `--backend vulkan` on NVIDIA GPUs"
                )
            }
        }
        BackendKind::Auto => {
            #[cfg(feature = "gpu")]
            let fallback = match gpu::GpuBackend::new(BackendKind::Gpu) {
                Ok(b) => return Ok(named(Box::new(b))),
                Err(e) => format!("no usable GPU ({e:#})"),
            };
            #[cfg(not(feature = "gpu"))]
            let fallback = "built without the `gpu` feature".to_string();
            Ok(OpenedBackend {
                backend: Box::<cpu::CpuBackend>::default(),
                fallback: Some(fallback),
            })
        }
        other => {
            #[cfg(feature = "gpu")]
            {
                Ok(named(Box::new(gpu::GpuBackend::new(other)?)))
            }
            #[cfg(not(feature = "gpu"))]
            {
                bail!("backend {other:?} needs the `gpu` feature")
            }
        }
    }
}

/// [`open_backend`], logging a fallback as a warning.
pub fn create_backend(kind: BackendKind) -> Result<Box<dyn Backend>> {
    let opened = open_backend(kind)?;
    if let Some(why) = &opened.fallback {
        log::warn!("{why}; falling back to the CPU backend");
    }
    Ok(opened.backend)
}

/// Camera basis passed to background shading.
#[derive(Clone, Copy, Debug)]
pub struct CameraRays {
    /// Unit view direction.
    pub forward: Vec3,
    /// Unit vector to the right of the view.
    pub right: Vec3,
    /// Unit vector up in the image plane.
    pub up: Vec3,
    /// Tangent of half the vertical field of view.
    pub tan_half_fov: f32,
    /// Width divided by height.
    pub aspect: f32,
}
