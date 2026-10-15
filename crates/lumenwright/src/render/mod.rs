//! Rendering backends and the inputs they share.
//!
//! Every backend implements [`Backend`]. Quick mode consumes a [`Mesh`] (rasterized),
//! HQ mode consumes a [`voxel::HqScene`] (path traced). Shading formulas are kept
//! identical between the Rust and WGSL implementations so backends agree.

pub mod cpu;
#[cfg(feature = "cuda")]
pub mod cuda;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod post;
pub mod sky;

use anyhow::Result;
#[cfg(any(not(feature = "cuda"), not(feature = "gpu")))]
use anyhow::bail;
use glam::{Mat4, Vec3};

use crate::scene::mesh::Mesh;
use crate::scene::voxel::HqScene;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Vanilla-client look: rasterized, no ray tracing.
    Quick,
    /// Chunky-like path tracing.
    Hq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum BackendKind {
    /// Best available: Metal/Vulkan/DX12 GPU, then CPU.
    Auto,
    Cpu,
    /// Any native GPU API wgpu can find.
    Gpu,
    Metal,
    Vulkan,
    Dx12,
    Cuda,
}

/// Inputs for quick mode, already scaled for supersampling.
pub struct QuickFrame<'a> {
    pub mesh: &'a Mesh,
    pub atlas_pixels: &'a [u32],
    pub atlas_size: [u32; 2],
    pub width: u32,
    pub height: u32,
    /// Camera-relative view-projection.
    pub view_proj: Mat4,
    pub sky: sky::QuickSky,
}

/// Inputs for HQ mode.
pub struct HqFrame<'a> {
    pub scene: &'a HqScene,
    pub params: lumenwright_kernel::Params,
    pub spp: u32,
}

pub trait Backend {
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

/// Creates a backend, falling back from GPU to CPU for `Auto`.
pub fn create_backend(kind: BackendKind) -> Result<Box<dyn Backend>> {
    match kind {
        BackendKind::Cpu => Ok(Box::new(cpu::CpuBackend::new())),
        BackendKind::Cuda => {
            #[cfg(feature = "cuda")]
            {
                Ok(Box::new(cuda::CudaBackend::new()?))
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
            match gpu::GpuBackend::new(BackendKind::Gpu) {
                Ok(b) => return Ok(Box::new(b)),
                Err(e) => log::warn!("no usable GPU ({e:#}); falling back to the CPU backend"),
            }
            Ok(Box::new(cpu::CpuBackend::new()))
        }
        other => {
            #[cfg(feature = "gpu")]
            {
                Ok(Box::new(gpu::GpuBackend::new(other)?))
            }
            #[cfg(not(feature = "gpu"))]
            {
                bail!("backend {other:?} needs the `gpu` feature")
            }
        }
    }
}

/// Camera basis passed to background shading.
#[derive(Clone, Copy, Debug)]
pub struct CameraRays {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub tan_half_fov: f32,
    pub aspect: f32,
}
