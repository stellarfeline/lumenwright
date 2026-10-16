//! Locating, or downloading, the vanilla client jar, the same way Chunky does.
//!
//! Mojang's assets are not redistributable. Chunky never ships them: it reads
//! the textures and models out of the official client jar, which it downloads
//! from Mojang's version manifest into its own cache. Lumenwright does the
//! same, and also reuses jars that a launcher or Chunky already downloaded.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

pub fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("LUMENWRIGHT_CACHE") {
        return PathBuf::from(p);
    }
    let home = home_dir();
    if cfg!(target_os = "macos") {
        home.join("Library/Caches/lumenwright")
    } else if cfg!(windows) {
        std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("lumenwright")
    } else {
        std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".cache"))
            .join("lumenwright")
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Candidate locations where a launcher (or Chunky) may already hold the jar.
fn local_candidates(version: &str) -> Vec<PathBuf> {
    let home = home_dir();
    let mut dirs = vec![
        home.join("Library/Application Support/minecraft"),
        home.join(".minecraft"),
    ];
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(appdata).join(".minecraft"));
    }
    let mut out: Vec<PathBuf> = dirs
        .iter()
        .map(|d| {
            d.join("versions")
                .join(version)
                .join(format!("{version}.jar"))
        })
        .collect();
    for prism in [
        home.join("Library/Application Support/PrismLauncher"),
        home.join(".local/share/PrismLauncher"),
    ] {
        out.push(prism.join(format!(
            "libraries/com/mojang/minecraft/{version}/minecraft-{version}-client.jar"
        )));
    }
    out.push(
        cache_dir()
            .join("versions")
            .join(version)
            .join("client.jar"),
    );
    out
}

pub fn chunky_jar() -> PathBuf {
    home_dir().join(".chunky/resources/minecraft.jar")
}

/// Reads the `id` from a jar's `version.json`.
pub fn jar_version(path: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let mut e = z.by_name("version.json").ok()?;
    let mut s = String::new();
    e.read_to_string(&mut s).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("id")?.as_str().map(str::to_string)
}

pub struct JarRequest<'a> {
    pub explicit: Option<&'a Path>,
    /// Version wanted (from the CLI or the world's level.dat).
    pub version: Option<&'a str>,
    /// Fall back to a download from Mojang.
    pub allow_download: bool,
}

/// Finds a client jar following the documented resolution order.
pub fn resolve_jar(req: &JarRequest) -> Result<PathBuf> {
    if let Some(p) = req.explicit {
        if !p.exists() {
            bail!("--minecraft-jar {} does not exist", p.display());
        }
        return Ok(p.to_path_buf());
    }
    if let Some(v) = req.version {
        for c in local_candidates(v) {
            if c.exists() {
                log::info!("using client jar {}", c.display());
                return Ok(c);
            }
        }
        let chunky = chunky_jar();
        if chunky.exists() && jar_version(&chunky).as_deref() == Some(v) {
            log::info!("using Chunky's client jar {} ({v})", chunky.display());
            return Ok(chunky);
        }
        if req.allow_download {
            match download_client_jar(v) {
                Ok(p) => return Ok(p),
                Err(e) => log::warn!("could not download {v} client jar: {e:#}"),
            }
        }
    }
    // Version unknown or unavailable: any jar we can find.
    let chunky = chunky_jar();
    if chunky.exists() {
        log::warn!(
            "using Chunky's client jar ({}) which may not match the world version",
            jar_version(&chunky).unwrap_or_else(|| "unknown".into())
        );
        return Ok(chunky);
    }
    if let Ok(rd) = std::fs::read_dir(cache_dir().join("versions"))
        && let Some(p) = rd
            .flatten()
            .map(|e| e.path().join("client.jar"))
            .find(|p| p.exists())
    {
        return Ok(p);
    }
    if req.allow_download {
        return download_client_jar("latest");
    }
    bail!("no Minecraft client jar found; pass --minecraft-jar or allow downloading")
}

fn http_get(url: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp
        .body_mut()
        .with_config()
        .limit(512 << 20)
        .read_to_vec()?)
}

/// Downloads the client jar for `version` ("latest" = latest release) from
/// Mojang into the cache, verifying its SHA-1. Returns the cached path.
pub fn download_client_jar(version: &str) -> Result<PathBuf> {
    log::info!("fetching Minecraft version manifest");
    let manifest: serde_json::Value = serde_json::from_slice(&http_get(MANIFEST_URL)?)?;
    let id = if version == "latest" {
        manifest["latest"]["release"]
            .as_str()
            .context("manifest without latest release")?
            .to_string()
    } else {
        version.to_string()
    };
    let dest = cache_dir().join("versions").join(&id).join("client.jar");
    if dest.exists() {
        return Ok(dest);
    }
    let entry = manifest["versions"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["id"].as_str() == Some(&id)))
        .with_context(|| format!("version {id} not in Mojang's manifest"))?;
    let meta: serde_json::Value =
        serde_json::from_slice(&http_get(entry["url"].as_str().context("version url")?)?)?;
    let client = &meta["downloads"]["client"];
    let url = client["url"].as_str().context("no client download")?;
    let sha1 = client["sha1"].as_str().unwrap_or("");
    log::info!(
        "downloading Minecraft {id} client jar ({} MB)",
        client["size"].as_u64().unwrap_or(0) >> 20
    );
    let bytes = http_get(url)?;
    let got = sha1_smol::Sha1::from(&bytes).digest().to_string();
    if !sha1.is_empty() && got != sha1 {
        bail!("SHA-1 mismatch for {id} client jar: expected {sha1}, got {got}");
    }
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let tmp = dest.with_extension("jar.part");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &dest)?;
    log::info!("saved {}", dest.display());
    Ok(dest)
}
