//! An offline, headless renderer for Minecraft Java Edition worlds.
//!
//! Give it a world save, the official client jar and a camera, and [`render()`]
//! returns the frame as RGBA8 pixels. It has two looks:
//!
//! * **quick** ([`render::Mode::Quick`]): rasterized to look like the vanilla Java
//!   client, with smooth lighting, ambient occlusion, biome tints and fog.
//! * **hq** ([`render::Mode::Hq`]): a path tracer in the style of Chunky, with sun
//!   and sky light, soft shadows, emissive blocks and refractive water.
//!
//! # What it draws
//!
//! Blocks from the vanilla blockstates and block models in the client jar, with
//! resource packs layered on top; water and lava; entities, from cuboid models
//! shipped with the crate (a box the size of the hitbox for an entity with no
//! model); and the block entities vanilla draws in Java code: signs with their
//! text, chests, banners, bells, lectern and enchanting-table books, decorated
//! pots, skulls and heads. Worlds from 1.16 onward are read (Anvil regions,
//! gzip, zlib, LZ4 or uncompressed), in the Overworld, the Nether or the End.
//!
//! It does not draw items (dropped items, item frame contents, item displays),
//! beds, shulker boxes or the conduit, clouds or weather, or entity layers that
//! need a second texture (sheep wool, armour). It does not denoise.
//!
//! # Backends
//!
//! | backend | quick | hq | cargo feature |
//! |---|---|---|---|
//! | CPU, all cores | software rasterizer | [`lumenwright_kernel`] on rayon | always |
//! | Metal, Vulkan, DX12 (wgpu) | raster pipeline | WGSL compute port of the kernel | `gpu` (default) |
//! | CUDA | falls back to the CPU rasterizer | the kernel compiled to PTX | `cuda` |
//!
//! [`render::BackendKind::Auto`] takes a GPU when one is available and the CPU
//! otherwise, and says why in [`Rendered::backend_fallback`]. On the CPU backend a
//! request gives byte-identical pixels for any rayon thread count.
//!
//! The `cuda` feature embeds a PTX file that is built from the source repository
//! (`scripts/build-ptx.sh`), so it builds only from a checkout of the repository.
//! With the default features no C code is compiled.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//!
//! use lumenwright::glam::DVec3;
//! use lumenwright::render::{BackendKind, Mode};
//! use lumenwright::scene::camera::Camera;
//! use lumenwright::world::Dimension;
//! use lumenwright::{
//!     ChunkRect, EntityPolicy, HqOptions, QuickOptions, Request, Sky, WorldSource,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let camera = Camera { pos: DVec3::new(18.5, 73.6, 22.5), yaw: 150.0, pitch: 18.0, fov_y: 70.0 };
//! let frame = lumenwright::render(&Request {
//!     world: WorldSource::Dir { path: Path::new("saves/MyWorld"), dimension: Dimension::Overworld },
//!     chunks: ChunkRect::around(camera.pos, 8),
//!     jar: Path::new("minecraft-1.21.11-client.jar"),
//!     unifont: None,
//!     resource_packs: &[],
//!     camera,
//!     mode: Mode::Quick,
//!     backend: BackendKind::Auto,
//!     width: 1280,
//!     height: 720,
//!     entities: EntityPolicy::default(),
//!     quick: QuickOptions::default(),
//!     hq: HqOptions::default(),
//!     sky: Sky::default(),
//! })?;
//! assert_eq!(frame.rgba.len(), 1280 * 720 * 4); // RGBA8, top row first
//! # Ok(())
//! # }
//! ```
//!
//! # The request
//!
//! The [`Request`] carries every input: the world (a directory or an opened
//! [`world::World`]) and the inclusive [`ChunkRect`] to load, the client jar and
//! the Unifont directory and resource packs as paths, the camera, the mode, the
//! backend, the size, the mode options, the entity policy and the sky, either derived from a time of day
//! ([`Sky::Vanilla`]) or stated by the caller ([`Sky::Stated`]). The [`Rendered`]
//! frame carries the pixels, the HQ radiance, the backend that drew it, the
//! per-stage timings and per-kind entity and block-entity counts.
//!
//! `render` resolves no jar, performs no network access, writes no file and
//! prints nothing; it emits [`log`] records only. Mojang's assets are not
//! redistributable, so the caller supplies the jar, and the Unifont directory when text
//! outside the bitmap fonts (CJK among it) is to be drawn ([`assets::unifont`]). A
//! request it refuses before drawing carries a [`RequestError`] (`err.downcast_ref::<RequestError>()`).
//!
//! The `lumenwright` command (crate `lumenwright-cli`) is a caller of this API:
//! it adds the jar and Unifont lookup and download, the flags and the PNG writer.
//! [`glam`] is re-exported for the camera and sky vectors.
//!
//! # Pipeline
//!
//! [`world`] (read the save) → [`assets`] (vanilla client jar) →
//! [`scene`] (baked quads, quick-mode mesh, HQ voxel grid) → [`mod@render`]
//! (CPU / Metal / Vulkan / DX12 / CUDA backends) → pixels.

#![warn(missing_docs)]

pub use glam;

pub mod assets;
pub mod render;
pub mod request;
pub mod scene;
pub mod world;

pub use request::{
    ChunkRect, EmitterSampling, EntityPolicy, HqOptions, HqSkyModel, QuickOptions, QuickSkyColours,
    Rendered, Request, RequestError, Sky, SkyLook, StatedSky, StatedTick, UnknownEntities,
    WorldSource, render,
};
