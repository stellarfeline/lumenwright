//! Vanilla assets read from a client jar and resource packs: block models,
//! item model definitions, textures, tints, the font and the language files.

pub mod atlas;
pub mod biome;
pub mod blockinfo;
pub mod blockstate;
pub mod environment;
pub mod font;
pub mod items;
pub mod lang;
pub mod mipmap;
pub mod model;
pub mod source;
pub mod unifont;

use std::path::{Path, PathBuf};

use anyhow::Result;

pub use source::ResourceStore;

/// Loads the client jar, then the unifont directory if there is one (the files the
/// client lays over the jar from the launcher's asset index, see [`unifont`]), then
/// the resource packs (later packs win).
pub fn load_resources(
    jar: &Path,
    unifont: Option<&Path>,
    packs: &[PathBuf],
) -> Result<ResourceStore> {
    let t = std::time::Instant::now();
    let mut store = ResourceStore::new();
    store.add_layer(jar)?;
    if let Some(dir) = unifont {
        for (name, bytes) in unifont::read_dir(dir)? {
            store.insert(&name, bytes);
        }
        store.sources.push(dir.to_path_buf());
    }
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
