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

use anyhow::{Result, bail};
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
    /// The first backend in [`auto_order`] this machine can open.
    Auto,
    /// The CPU backend: software rasterizer and the Rust kernel on rayon.
    Cpu,
    /// The best hardware adapter wgpu finds on any of Metal, Vulkan, DX12 and GL.
    Gpu,
    /// Metal through wgpu (macOS).
    Metal,
    /// Vulkan through wgpu (Linux, Windows).
    Vulkan,
    /// DX12 through wgpu (Windows).
    Dx12,
    /// OpenGL / OpenGL ES through wgpu (Linux, Windows; EGL or WGL).
    Gl,
    /// CUDA, with the Rust kernel compiled to PTX (cargo feature `cuda`). Quick mode
    /// falls back to the CPU rasterizer.
    Cuda,
}

/// The backends [`BackendKind::Auto`] tries for `mode`, in order; the CPU, last, always opens.
///
/// HQ: CUDA first, because it runs the CPU's own kernel compiled to PTX, so its frame is
/// the reference kernel's; then the best wgpu adapter (Metal, Vulkan, DX12, then GL;
/// [`BackendKind::Gpu`]), which runs the WGSL port of the kernel; then the CPU. Quick: the
/// best wgpu adapter, then the CPU; CUDA has no rasterizer, so it is passed over.
pub fn auto_order(mode: Mode) -> &'static [BackendKind] {
    match mode {
        Mode::Hq => &[BackendKind::Cuda, BackendKind::Gpu, BackendKind::Cpu],
        Mode::Quick => &[BackendKind::Gpu, BackendKind::Cpu],
    }
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

/// A backend and, for [`BackendKind::Auto`], why the backends before it were passed over.
pub struct OpenedBackend {
    /// The opened backend.
    pub backend: Box<dyn Backend>,
    /// Set when `Auto` passed over a backend in [`auto_order`]: each one and why, in order
    /// (`cuda: ...; gpu: ...`).
    pub fallback: Option<String>,
}

fn open_one(kind: BackendKind) -> Result<Box<dyn Backend>> {
    match kind {
        BackendKind::Auto => bail!("Auto is not a backend"),
        BackendKind::Cpu => Ok(Box::<cpu::CpuBackend>::default()),
        BackendKind::Cuda => {
            #[cfg(feature = "cuda")]
            {
                Ok(Box::new(cuda::CudaBackend::new()?))
            }
            #[cfg(not(feature = "cuda"))]
            {
                bail!("this build has no CUDA backend (cargo feature `cuda`)")
            }
        }
        BackendKind::Gpu
        | BackendKind::Metal
        | BackendKind::Vulkan
        | BackendKind::Dx12
        | BackendKind::Gl => {
            #[cfg(feature = "gpu")]
            {
                Ok(Box::new(gpu::GpuBackend::new(kind)?))
            }
            #[cfg(not(feature = "gpu"))]
            {
                bail!("this build has no wgpu backends (cargo feature `gpu`)")
            }
        }
    }
}

/// The lower-case name `--backend` takes for `kind`.
pub fn backend_name(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Auto => "auto",
        BackendKind::Cpu => "cpu",
        BackendKind::Gpu => "gpu",
        BackendKind::Metal => "metal",
        BackendKind::Vulkan => "vulkan",
        BackendKind::Dx12 => "dx12",
        BackendKind::Gl => "gl",
        BackendKind::Cuda => "cuda",
    }
}

/// Opens a backend to draw `mode`. A named backend that is unavailable is an error;
/// [`BackendKind::Auto`] takes the first of [`auto_order`] that opens, saying why it passed
/// over each one before it in [`OpenedBackend::fallback`].
pub fn open_backend(kind: BackendKind, mode: Mode) -> Result<OpenedBackend> {
    if kind != BackendKind::Auto {
        return Ok(OpenedBackend {
            backend: open_one(kind)?,
            fallback: None,
        });
    }
    let mut passed = Vec::new();
    if mode == Mode::Quick {
        passed.push("cuda: no rasterizer for quick mode".to_string());
    }
    for &k in auto_order(mode) {
        match open_one(k) {
            Ok(backend) => {
                return Ok(OpenedBackend {
                    backend,
                    fallback: (!passed.is_empty()).then(|| passed.join("; ")),
                });
            }
            Err(e) => passed.push(format!("{}: {e:#}", backend_name(k))),
        }
    }
    bail!("no backend opened: {}", passed.join("; "))
}

/// [`open_backend`], logging what `Auto` passed over as a warning.
pub fn create_backend(kind: BackendKind, mode: Mode) -> Result<Box<dyn Backend>> {
    let opened = open_backend(kind, mode)?;
    if let Some(why) = &opened.fallback {
        log::warn!(
            "auto passed over {why}; drawing on {}",
            opened.backend.name()
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_ends_on_the_cpu_and_tries_cuda_only_for_hq() {
        for mode in [Mode::Hq, Mode::Quick] {
            assert_eq!(auto_order(mode).last(), Some(&BackendKind::Cpu));
            assert!(!auto_order(mode).contains(&BackendKind::Auto));
        }
        assert_eq!(auto_order(Mode::Hq)[0], BackendKind::Cuda);
        assert!(!auto_order(Mode::Quick).contains(&BackendKind::Cuda));
    }

    #[test]
    fn auto_names_every_backend_it_passed_over() {
        for mode in [Mode::Hq, Mode::Quick] {
            let opened = open_backend(BackendKind::Auto, mode).unwrap();
            let name = opened.backend.name();
            let passed = opened.fallback.unwrap_or_default();
            // Each backend before the one that opened is named with its reason.
            for &k in auto_order(mode) {
                if open_one(k).is_ok() {
                    break;
                }
                assert!(
                    passed.contains(&format!("{}: ", backend_name(k))),
                    "{mode:?}: {name} opened, yet `{passed}` does not name {k:?}"
                );
            }
            if mode == Mode::Quick {
                assert!(passed.starts_with("cuda: no rasterizer"), "{passed}");
            }
        }
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn the_packaged_ptx_carries_the_kernel_entry() {
        assert!(cuda::PTX.contains(".visible .entry pt_render("));
        assert!(cuda::PTX.contains(".target sm_"));
    }
}
