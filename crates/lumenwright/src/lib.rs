//! Lumenwright: offline headless renderer for Minecraft Java Edition worlds.
//!
//! Pipeline: [`world`] (read the save) → [`assets`] (vanilla client jar) →
//! [`scene`] (baked quads, quick-mode mesh, HQ voxel grid) → [`render`]
//! (CPU / Metal / Vulkan / DX12 / CUDA backends) → PNG. [`job`] ties it together.

pub mod assets;
pub mod fixture;
pub mod job;
pub mod render;
pub mod scene;
pub mod world;
