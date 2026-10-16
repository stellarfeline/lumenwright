//! Lumenwright: offline headless renderer for Minecraft Java Edition worlds.
//!
//! # Embedding
//!
//! [`render`] draws one frame from a [`Request`] and returns a [`Rendered`]:
//! the RGBA8 pixels, the HQ radiance, the backend that drew it (and why, when
//! `auto` fell back to the CPU), the per-stage timings and the per-kind entity
//! and block-entity counts. The request carries every input: the world (a
//! directory or an opened [`world::World`]) and the inclusive [`ChunkRect`] to
//! load, the client jar and resource packs as paths, the camera, the mode,
//! the backend, the size, the mode options, the entity policy and the sky,
//! either derived from a time of day ([`Sky::Vanilla`]) or stated by the
//! caller ([`Sky::Stated`]).
//!
//! `render` resolves no jar, performs no network access, writes no file and
//! prints nothing; it emits [`log`] records only. A request it refuses before
//! drawing carries a [`RequestError`] (`err.downcast_ref::<RequestError>()`).
//! On the CPU backend a request gives byte-identical pixels for any rayon
//! thread count.
//!
//! An embedding crate depends on Lumenwright with `default-features = false,
//! features = ["gpu"]`: the `download` feature (the CLI's client-jar
//! downloader and its TLS stack) stays off, and no C build script is reached.
//! [`glam`] is re-exported for the camera and sky vectors.
//!
//! # Pipeline
//!
//! [`world`] (read the save) → [`assets`] (vanilla client jar) →
//! [`scene`] (baked quads, quick-mode mesh, HQ voxel grid) → [`mod@render`]
//! (CPU / Metal / Vulkan / DX12 / CUDA backends) → pixels. [`job`] is the
//! CLI's `render` subcommand over [`render`]: it resolves the jar, derives the
//! camera and chunk square from the flags, and writes the PNG.

pub use glam;

pub mod assets;
pub mod fixture;
pub mod job;
pub mod render;
pub mod request;
pub mod scene;
pub mod world;

pub use request::{
    ChunkRect, EntityPolicy, HqOptions, QuickOptions, QuickSkyColours, Rendered, Request,
    RequestError, Sky, StatedSky, UnknownEntities, WorldSource, render,
};
