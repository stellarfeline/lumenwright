//! Layered resource lookup over the vanilla client jar and resource packs.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Asset sub-paths we need, in any namespace. Everything else (sounds, lang,
/// shaders, GUI...) is skipped.
const WANTED: &[&str] = &[
    "blockstates/",
    "models/",
    "items/",
    "textures/",
    "font/",
    "lang/",
];
const SKIPPED: &[&str] = &[
    "textures/gui/",
    "textures/particle/",
    "textures/mob_effect/",
    "textures/trims/",
];

/// All resources loaded into memory, keyed by path inside the pack
/// (e.g. `assets/minecraft/textures/block/stone.png`).
#[derive(Default)]
pub struct ResourceStore {
    files: HashMap<String, Vec<u8>>,
    pub sources: Vec<PathBuf>,
}

fn wanted(name: &str) -> bool {
    if name == "version.json" {
        return true;
    }
    let Some(rest) = name.strip_prefix("assets/") else {
        return false;
    };
    let Some((_ns, rest)) = rest.split_once('/') else {
        return false;
    };
    WANTED.iter().any(|p| rest.starts_with(p)) && !SKIPPED.iter().any(|p| rest.starts_with(p))
}

impl ResourceStore {
    pub fn new() -> ResourceStore {
        ResourceStore::default()
    }

    /// Adds a zip/jar or a directory. Later layers override earlier ones.
    pub fn add_layer(&mut self, path: &Path) -> Result<()> {
        if path.is_dir() {
            self.add_dir(path, path)?;
        } else {
            let f =
                std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
            let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f))
                .with_context(|| format!("{} is not a zip/jar", path.display()))?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i)?;
                let name = entry.name()?.to_string();
                if entry.is_dir() || !wanted(&name) {
                    continue;
                }
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf)?;
                self.files.insert(name, buf);
            }
        }
        self.sources.push(path.to_path_buf());
        Ok(())
    }

    fn add_dir(&mut self, root: &Path, dir: &Path) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let p = e?.path();
            if p.is_dir() {
                self.add_dir(root, &p)?;
            } else if let Ok(rel) = p.strip_prefix(root) {
                let name = rel.to_string_lossy().replace('\\', "/");
                if wanted(&name) {
                    self.files.insert(name, std::fs::read(&p)?);
                }
            }
        }
        Ok(())
    }

    /// Inserts a file directly (tests, generated resources).
    pub fn insert(&mut self, path: &str, data: Vec<u8>) {
        self.files.insert(path.to_string(), data);
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|v| v.as_slice())
    }

    /// Paths of every loaded file ending with `suffix`, sorted.
    pub fn paths_ending_with(&self, suffix: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .files
            .keys()
            .filter(|k| k.ends_with(suffix))
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Resolves a resource location like `minecraft:block/stone` under `kind`
    /// (`models`, `textures`, `blockstates`) with the given extension.
    pub fn resolve(&self, kind: &str, loc: &str, ext: &str) -> Option<&[u8]> {
        let (ns, path) = split_loc(loc);
        self.get(&format!("assets/{ns}/{kind}/{path}.{ext}"))
    }

    pub fn version_name(&self) -> Option<String> {
        let v: serde_json::Value = serde_json::from_slice(self.get("version.json")?).ok()?;
        v.get("id").and_then(|s| s.as_str()).map(str::to_string)
    }
}

/// Splits `ns:path` (default namespace `minecraft`).
pub fn split_loc(loc: &str) -> (&str, &str) {
    match loc.split_once(':') {
        Some((ns, p)) => (ns, p),
        None => ("minecraft", loc),
    }
}

/// Normalizes a resource location to `ns:path`.
pub fn norm_loc(loc: &str) -> String {
    let (ns, p) = split_loc(loc);
    format!("{ns}:{p}")
}
