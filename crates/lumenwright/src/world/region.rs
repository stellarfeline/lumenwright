//! Anvil region file (`r.<x>.<z>.mca`) reader.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// An in-memory region file.
pub struct Region {
    path: PathBuf,
    data: Vec<u8>,
}

impl Region {
    /// Opens a region file. Returns `Ok(None)` when it does not exist or is empty.
    pub fn open(path: &Path) -> Result<Option<Region>> {
        match std::fs::read(path) {
            Ok(data) if data.len() >= 8192 => Ok(Some(Region {
                path: path.to_path_buf(),
                data,
            })),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn from_bytes(path: PathBuf, data: Vec<u8>) -> Region {
        Region { path, data }
    }

    /// Decompressed NBT bytes of the chunk at region-local coordinates (0..32).
    pub fn chunk_nbt(&self, lx: usize, lz: usize) -> Result<Option<Vec<u8>>> {
        let idx = 4 * ((lx & 31) + (lz & 31) * 32);
        let loc = u32::from_be_bytes(self.data[idx..idx + 4].try_into().unwrap());
        let offset = (loc >> 8) as usize * 4096;
        let sectors = (loc & 0xFF) as usize;
        if offset == 0 || sectors == 0 {
            return Ok(None);
        }
        if offset + 5 > self.data.len() {
            bail!(
                "{}: chunk ({lx},{lz}) points past end of file",
                self.path.display()
            );
        }
        let len = u32::from_be_bytes(self.data[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = self.data[offset + 4];
        if len == 0 {
            return Ok(None);
        }
        let payload: Vec<u8> = if kind & 0x80 != 0 {
            // Oversized chunk stored in an external c.<x>.<z>.mcc file.
            let (rx, rz) =
                parse_region_name(&self.path).context("external chunk in unnamed region")?;
            let cx = rx * 32 + lx as i32;
            let cz = rz * 32 + lz as i32;
            let ext = self.path.with_file_name(format!("c.{cx}.{cz}.mcc"));
            std::fs::read(&ext).with_context(|| format!("reading {}", ext.display()))?
        } else {
            let end = offset + 4 + len;
            if end > self.data.len() {
                bail!("{}: chunk ({lx},{lz}) truncated", self.path.display());
            }
            self.data[offset + 5..end].to_vec()
        };
        decompress(kind & 0x7F, &payload).map(Some)
    }
}

pub fn decompress(kind: u8, payload: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match kind {
        1 => {
            flate2::read::GzDecoder::new(payload).read_to_end(&mut out)?;
        }
        2 => {
            flate2::read::ZlibDecoder::new(payload).read_to_end(&mut out)?;
        }
        3 => out.extend_from_slice(payload),
        4 => out = decompress_lz4_block_stream(payload)?,
        k => bail!("unsupported chunk compression type {k}"),
    }
    Ok(out)
}

/// Decodes lz4-java's `LZ4BlockOutputStream` framing (used by Minecraft 1.20.5+
/// when `region-file-compression=lz4`).
fn decompress_lz4_block_stream(mut data: &[u8]) -> Result<Vec<u8>> {
    const MAGIC: &[u8] = b"LZ4Block";
    let mut out = Vec::new();
    while data.len() >= 21 {
        if &data[..8] != MAGIC {
            bail!("bad LZ4Block magic");
        }
        let token = data[8];
        let clen = u32::from_le_bytes(data[9..13].try_into().unwrap()) as usize;
        let dlen = u32::from_le_bytes(data[13..17].try_into().unwrap()) as usize;
        data = &data[21..];
        if clen == 0 && dlen == 0 {
            break;
        }
        if data.len() < clen {
            bail!("truncated LZ4 block");
        }
        let block = &data[..clen];
        match token & 0xF0 {
            0x10 => out.extend_from_slice(block),
            0x20 => out.extend_from_slice(&lz4_flex::block::decompress(block, dlen)?),
            m => bail!("unknown LZ4Block method {m:#x}"),
        }
        data = &data[clen..];
    }
    Ok(out)
}

/// Parses `r.<x>.<z>.mca` into region coordinates.
pub fn parse_region_name(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let mut parts = name.strip_prefix("r.")?.strip_suffix(".mca")?.split('.');
    let x = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    Some((x, z))
}

/// Writes a region file containing the given chunks (zlib). Used by tests and
/// fixture generation.
pub fn write_region(chunks: &[(usize, usize, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    let mut header = vec![0u8; 8192];
    let mut body: Vec<u8> = Vec::new();
    for (lx, lz, nbt) in chunks {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(nbt).unwrap();
        let comp = enc.finish().unwrap();
        let mut rec = Vec::new();
        rec.extend_from_slice(&((comp.len() + 1) as u32).to_be_bytes());
        rec.push(2);
        rec.extend_from_slice(&comp);
        while rec.len() % 4096 != 0 {
            rec.push(0);
        }
        let offset_sectors = 2 + body.len() / 4096;
        let sectors = rec.len() / 4096;
        let idx = 4 * (lx + lz * 32);
        let loc = ((offset_sectors as u32) << 8) | sectors as u32;
        header[idx..idx + 4].copy_from_slice(&loc.to_be_bytes());
        body.extend_from_slice(&rec);
    }
    header.extend_from_slice(&body);
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_roundtrip() {
        let file = write_region(&[(3, 7, b"hello".to_vec()), (31, 31, vec![9; 10_000])]);
        let r = Region::from_bytes(PathBuf::from("r.0.0.mca"), file);
        assert_eq!(r.chunk_nbt(3, 7).unwrap().unwrap(), b"hello");
        assert_eq!(r.chunk_nbt(31, 31).unwrap().unwrap(), vec![9; 10_000]);
        assert!(r.chunk_nbt(0, 0).unwrap().is_none());
    }

    #[test]
    fn region_name() {
        assert_eq!(
            parse_region_name(Path::new("/a/r.-1.12.mca")),
            Some((-1, 12))
        );
        assert_eq!(parse_region_name(Path::new("/a/level.dat")), None);
    }

    #[test]
    fn lz4_block_stream() {
        let raw = b"minecraft minecraft minecraft minecraft".to_vec();
        let comp = lz4_flex::block::compress(&raw);
        let mut s = Vec::new();
        s.extend_from_slice(b"LZ4Block");
        s.push(0x20);
        s.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        s.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        s.extend_from_slice(&0u32.to_le_bytes());
        s.extend_from_slice(&comp);
        s.extend_from_slice(b"LZ4Block");
        s.push(0x10);
        s.extend_from_slice(&[0; 12]);
        assert_eq!(decompress(4, &s).unwrap(), raw);
    }
}
