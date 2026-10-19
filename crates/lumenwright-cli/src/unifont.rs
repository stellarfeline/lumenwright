//! Locating, or downloading, the Unifont the client draws CJK and other
//! non-bitmap characters with.
//!
//! The client jar's `font/include/unifont.json` lists no providers; the real one and
//! the hex zips it names are in the launcher's asset index, which Mojang serves at
//! `resources.download.minecraft.net` by SHA-1. Lumenwright resolves them the way it
//! resolves the jar: an explicit directory, then its cache, then a SHA-1 and size
//! checked download unless `--offline`. The library takes the directory as a path
//! ([`lumenwright::Request::unifont`]); everything here belongs to the command.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use lumenwright::assets::unifont as hex;

use crate::jar;

const OBJECTS_URL: &str = "https://resources.download.minecraft.net";

/// What the lock file in a cached unifont directory records: the SHA-1 and size of
/// every file the download wrote.
const LOCK: &str = "unifont.lock";

pub struct UnifontRequest<'a> {
    /// A directory given on the command line.
    pub explicit: Option<&'a Path>,
    /// The Minecraft version whose asset index holds the files (the jar's), or
    /// `None` for the latest release.
    pub version: Option<&'a str>,
    /// Fall back to a download from Mojang.
    pub allow_download: bool,
}

/// Finds the unifont directory following the documented resolution order.
pub fn resolve_unifont(req: &UnifontRequest) -> Result<PathBuf> {
    resolve_in(&jar::cache_dir(), req, &download_into)
}

/// The resolution, with the cache directory and the downloader stated, so that a test
/// resolves without touching the user's cache or the network.
pub(crate) fn resolve_in(
    cache: &Path,
    req: &UnifontRequest,
    download: &dyn Fn(&Path, &str) -> Result<PathBuf>,
) -> Result<PathBuf> {
    if let Some(dir) = req.explicit {
        hex::read_dir(dir).with_context(|| format!("--unifont {}", dir.display()))?;
        return Ok(dir.to_path_buf());
    }
    let version = req.version.unwrap_or("latest");
    let dir = cache.join("unifont").join(version);
    let cached = verify_cached(&dir);
    match cached {
        Ok(()) => {
            log::info!("using unifont {}", dir.display());
            return Ok(dir);
        }
        Err(e) if !req.allow_download => {
            let why = if dir.exists() {
                format!("the cached copy is refused: {e:#}")
            } else {
                format!("none is cached at {}", dir.display())
            };
            bail!(
                "no usable unifont for Minecraft {version}: {why}; --offline forbids the download. \
                 Pass --unifont <dir> (a directory holding minecraft/font/include/unifont.json and the hex zip it names)"
            );
        }
        Err(e) => log::info!("fetching unifont for {version} ({e:#})"),
    }
    download(cache, version)
}

/// A cached directory is usable when its lock lists files that are all present with
/// the recorded SHA-1, and the directory reads as a unifont.
fn verify_cached(dir: &Path) -> Result<()> {
    let lock_path = dir.join(LOCK);
    let lock: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&lock_path).with_context(|| format!("reading {}", lock_path.display()))?,
    )
    .with_context(|| format!("{} is not JSON", lock_path.display()))?;
    let files = lock["files"]
        .as_object()
        .with_context(|| format!("{} lists no files", lock_path.display()))?;
    for (key, want) in files {
        let path = dir.join(key);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let got = sha1_smol::Sha1::from(&bytes).digest().to_string();
        if want.as_str() != Some(&got) {
            bail!(
                "{} has SHA-1 {got}, the lock records {}",
                path.display(),
                want.as_str().unwrap_or("?")
            );
        }
    }
    hex::read_dir(dir)?;
    Ok(())
}

fn object(hash: &str, size: u64, what: &str) -> Result<Vec<u8>> {
    let url = format!("{OBJECTS_URL}/{}/{hash}", &hash[..2]);
    log::info!("downloading {what} ({} KB)", size >> 10);
    let bytes = jar::http_get(&url)?;
    let got = sha1_smol::Sha1::from(&bytes).digest().to_string();
    if got != hash || bytes.len() as u64 != size {
        bail!(
            "{what}: expected SHA-1 {hash} and {size} bytes, got {got} and {}",
            bytes.len()
        );
    }
    Ok(bytes)
}

/// Downloads the unifont files of `version` ("latest" = latest release) from Mojang
/// into the cache, verifying the asset index and every file by SHA-1 and size.
pub fn download_unifont(version: &str) -> Result<PathBuf> {
    download_into(&jar::cache_dir(), version)
}

fn download_into(cache: &Path, version: &str) -> Result<PathBuf> {
    let (id, meta) = jar::version_meta(version)?;
    let index = &meta["assetIndex"];
    let index_url = index["url"]
        .as_str()
        .context("version without assetIndex")?;
    let index_sha1 = index["sha1"].as_str().context("assetIndex without sha1")?;
    log::info!(
        "fetching asset index {}",
        index["id"].as_str().unwrap_or("?")
    );
    let index_bytes = jar::http_get(index_url)?;
    let got = sha1_smol::Sha1::from(&index_bytes).digest().to_string();
    if got != index_sha1 {
        bail!("SHA-1 mismatch for the {id} asset index: expected {index_sha1}, got {got}");
    }
    let objects = serde_json::from_slice::<serde_json::Value>(&index_bytes)?;
    let entry = |key: &str| -> Result<(String, u64)> {
        let o = &objects["objects"][key];
        Ok((
            o["hash"]
                .as_str()
                .with_context(|| format!("{key} is not in the {id} asset index"))?
                .to_string(),
            o["size"].as_u64().unwrap_or(0),
        ))
    };
    // Files by their asset-index key; the include file names the hex zips to fetch.
    let (hash, size) = entry(hex::INCLUDE)?;
    let include = object(&hash, size, hex::INCLUDE)?;
    let mut files = vec![(hex::INCLUDE.to_string(), hash, include.clone())];
    for loc in hex::hex_files(&include).map_err(|e| anyhow::anyhow!("{}: {e}", hex::INCLUDE))? {
        let key = loc.replacen(':', "/", 1);
        let (hash, size) = entry(&key)?;
        let bytes = object(&hash, size, &key)?;
        files.push((key, hash, bytes));
    }
    let dest = cache.join("unifont").join(&id);
    let part = cache.join("unifont").join(format!("{id}.part"));
    let _ = std::fs::remove_dir_all(&part);
    let mut lock = serde_json::Map::new();
    for (key, hash, bytes) in &files {
        let path = part.join(key);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes)?;
        lock.insert(key.clone(), serde_json::Value::String(hash.clone()));
    }
    std::fs::write(
        part.join(LOCK),
        serde_json::to_vec_pretty(&serde_json::json!({ "files": lock }))?,
    )?;
    hex::read_dir(&part).context("the downloaded unifont does not read")?;
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&part, &dest)?;
    log::info!("saved {}", dest.display());
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "lumenwright-unifont-unit-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A cached directory with one glyph and the lock `download_into` writes.
    fn cached(cache: &Path, version: &str) -> PathBuf {
        let dir = cache.join("unifont").join(version);
        let font = dir.join("minecraft/font");
        std::fs::create_dir_all(font.join("include")).unwrap();
        let json = br#"{"providers":[{"type":"unihex","hex_file":"minecraft:font/unifont.zip"}]}"#;
        std::fs::write(font.join("include/unifont.json"), json).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(font.join("unifont.zip")).unwrap());
        zip.start_file("a.hex", zip::write::SimpleFileOptions::default())
            .unwrap();
        writeln!(zip, "4E00:{}", "F".repeat(64)).unwrap();
        zip.finish().unwrap();
        let sha = |p: &str| {
            sha1_smol::Sha1::from(&std::fs::read(dir.join(p)).unwrap())
                .digest()
                .to_string()
        };
        let lock = serde_json::json!({"files": {
            "minecraft/font/include/unifont.json": sha("minecraft/font/include/unifont.json"),
            "minecraft/font/unifont.zip": sha("minecraft/font/unifont.zip"),
        }});
        std::fs::write(dir.join(LOCK), lock.to_string()).unwrap();
        dir
    }

    fn no_download(_: &Path, _: &str) -> Result<PathBuf> {
        panic!("the resolution went to the network");
    }

    fn request<'a>(allow_download: bool) -> UnifontRequest<'a> {
        UnifontRequest {
            explicit: None,
            version: Some("1.21.11"),
            allow_download,
        }
    }

    #[test]
    fn a_cached_unifont_is_used_without_the_network() {
        let cache = tmp("hit");
        let dir = cached(&cache, "1.21.11");
        // Even with downloads allowed, a verified cache hit never downloads.
        for allow in [false, true] {
            let got = resolve_in(&cache, &request(allow), &no_download).unwrap();
            assert_eq!(got, dir);
        }
    }

    #[test]
    fn offline_with_nothing_cached_is_refused_without_the_network() {
        let cache = tmp("empty");
        let e = resolve_in(&cache, &request(false), &no_download).unwrap_err();
        let msg = format!("{e:#}");
        assert!(
            msg.contains("no usable unifont for Minecraft 1.21.11"),
            "{msg}"
        );
        assert!(
            msg.contains("--offline") && msg.contains("--unifont"),
            "{msg}"
        );
    }

    #[test]
    fn a_cached_file_that_no_longer_matches_its_hash_is_refused_by_name() {
        let cache = tmp("mismatch");
        let dir = cached(&cache, "1.21.11");
        let zip = dir.join("minecraft/font/unifont.zip");
        let mut bytes = std::fs::read(&zip).unwrap();
        bytes.push(0);
        std::fs::write(&zip, bytes).unwrap();
        let e = resolve_in(&cache, &request(false), &no_download).unwrap_err();
        let msg = format!("{e:#}");
        assert!(msg.contains(&zip.display().to_string()), "{msg}");
        assert!(msg.contains("SHA-1"), "{msg}");
        // With downloads allowed, the downloader is what runs.
        let dl =
            |_: &Path, v: &str| -> Result<PathBuf> { Ok(PathBuf::from(format!("fetched-{v}"))) };
        assert_eq!(
            resolve_in(&cache, &request(true), &dl).unwrap(),
            PathBuf::from("fetched-1.21.11")
        );
    }

    #[test]
    fn an_explicit_directory_is_checked_and_beats_the_cache() {
        let cache = tmp("explicit-cache");
        cached(&cache, "1.21.11");
        let given = tmp("explicit-given");
        let r = UnifontRequest {
            explicit: Some(&given),
            ..request(false)
        };
        let msg = format!("{:#}", resolve_in(&cache, &r, &no_download).unwrap_err());
        assert!(
            msg.contains("--unifont") && msg.contains("unifont.json"),
            "{msg}"
        );
        let good = cached(&tmp("explicit-good"), "x");
        let r = UnifontRequest {
            explicit: Some(&good),
            ..request(false)
        };
        assert_eq!(resolve_in(&cache, &r, &no_download).unwrap(), good);
    }
}
