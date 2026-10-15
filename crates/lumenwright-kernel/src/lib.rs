//! Path-tracing kernel shared by the CPU backend (host build with `std`) and the
//! CUDA backend (compiled to PTX for `nvptx64-nvidia-cuda` without `std`).
//!
//! Everything here operates on flat `#[repr(C)]` buffers whose layout is mirrored
//! exactly by the WGSL kernel in `crates/lumenwright/src/render/gpu/pathtrace.wgsl`.
//! Any change to a struct or algorithm here must be applied there too.

#![cfg_attr(not(any(feature = "std", test)), no_std)]

pub mod integrator;
// With both `std` and `libm` enabled (feature unification), std math wins.
#[cfg(feature = "libm")]
use libm as _;

pub mod math;
pub mod rng;
pub mod scene;
pub mod trace;

pub use integrator::render_sample;
pub use scene::{GpuModel, GpuQuad, Params, SceneView};

#[cfg(test)]
mod tests;
