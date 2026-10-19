//! The path-tracing kernel of [Lumenwright](https://crates.io/crates/lumenwright),
//! an offline renderer for Minecraft Java Edition worlds.
//!
//! This crate is the part of Lumenwright's HQ mode that runs per pixel: it traces
//! one path sample through a scene laid out in flat `#[repr(C)]` buffers and
//! returns its radiance. The same source runs in two places: on the host, where
//! the `lumenwright` CPU backend calls [`render_sample`] on every core, and on an
//! NVIDIA GPU, where it is compiled to PTX (`nvptx64-nvidia-cuda`) for the CUDA
//! backend. The WGSL kernel of `lumenwright`'s Metal, Vulkan and DX12 backends
//! mirrors it line for line with the same buffer layout and the same random
//! number generator, so every backend draws the same random numbers for each
//! pixel and sample.
//!
//! It does not read worlds, bake models or write images: `lumenwright` builds the
//! buffers ([`scene`] describes their layout) and does everything else. Most
//! programs want `lumenwright` itself.
//!
//! # Features
//!
//! * `libm` (default): pure-Rust float math, for `no_std` targets.
//! * `std`: the standard library's float math, for host builds. When both are
//!   enabled, `std` is used.
//!
//! Without `std` the crate is `#![no_std]` and allocates nothing.

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![warn(missing_docs)]

// The WGSL kernel in crates/lumenwright/src/render/gpu/pathtrace.wgsl mirrors this crate's
// buffer layout and algorithms exactly: a change to a struct or algorithm here is made there too.

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
