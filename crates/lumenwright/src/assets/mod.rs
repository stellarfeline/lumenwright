//! Vanilla assets: client jar resolution, block models, textures, tints.

pub mod atlas;
pub mod biome;
pub mod blockinfo;
pub mod blockstate;
pub mod jar;
pub mod model;
pub mod source;

use std::path::{Path, PathBuf};

use anyhow::Result;

pub use source::ResourceStore;

/// Loads the client jar plus resource packs (later packs win).
pub fn load_resources(jar: &Path, packs: &[PathBuf]) -> Result<ResourceStore> {
    let t = std::time::Instant::now();
    let mut store = ResourceStore::new();
    store.add_layer(jar)?;
    for p in packs {
        store.add_layer(p)?;
    }
    log::info!(
        "loaded {} resources from {} source(s) (Minecraft {}) in {:.2?}",
        store.len(),
        store.sources.len(),
        store.version_name().unwrap_or_else(|| "unknown".into()),
        t.elapsed()
    );
    Ok(store)
}
