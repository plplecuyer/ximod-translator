//! Bethesda archive reading & extraction: **BSA** (Morrowind TES3, plus Oblivion
//! / Fallout 3-NV / Skyrim / Skyrim SE) and **BA2** (Fallout 4 / Fallout 76 /
//! Starfield), including **DX10 texture archives**.
//!
//! For the TES4-era BSA and the BA2 we wrap [`ba2`] — the pure-Rust archive
//! library by the author of the canonical C++ `bsa`/`ba2` tools, released under
//! the very permissive 0BSD license — so we never hand-roll the many format
//! variants (BSA 103/104/105, BA2 GNRL, BA2 DX10, zlib / LZ4 / LZ4F). The
//! **Morrowind (TES3)** BSA is a much older, simpler layout the `ba2` crate does
//! not read, so we parse it directly here (see [`Archive::parse_tes3_bsa`]).
//!
//! Every file is listed with its size, compression flag and, for DX10 archives,
//! its texture dimensions. Extraction returns the file already **decompressed**;
//! for a DX10 texture the `.dds` header is reconstructed by the library, so the
//! extracted bytes are a valid stand-alone `.dds`.
//!
//! The public shapes (`Archive`, `ArchiveEntry`) mirror ESM Explorer's so the two
//! tools share one mental model and fold together cleanly inside XIMOD Translator.

use std::path::{Path, PathBuf};

use ba2::{
    Reader,
    fo4::{self, FileHeader},
    tes4,
};
use memmap2::Mmap;

/// How an [`Archive`]'s raw bytes are held: a memory-mapped file (the normal
/// path — huge archives open instantly and stay paged on demand instead of being
/// read into RAM), or an owned buffer (in-memory / test path).
enum Backing {
    Mmap(Mmap),
    #[cfg(test)]
    Owned(Vec<u8>),
}

impl Backing {
    fn as_slice(&self) -> &[u8] {
        match self {
            Backing::Mmap(m) => &m[..],
            #[cfg(test)]
            Backing::Owned(v) => &v[..],
        }
    }
}

/// Which on-disk container an [`Archive`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// BSA — Oblivion / Fallout 3-NV / Skyrim / Skyrim SE (via `ba2`, TES4 layout).
    Bsa,
    /// BSA — **Morrowind** (TES3 layout, version `0x100`), parsed natively here.
    Tes3Bsa,
    /// BA2 — Fallout 4 / Fallout 76 / Starfield.
    Ba2,
}

/// Texture metadata for a DX10 entry (BA2 texture archives).
#[derive(Debug, Clone, Copy)]
pub struct TexInfo {
    pub width: u16,
    pub height: u16,
    pub mips: u8,
    /// DXGI format code (as stored in the BA2 DX10 header).
    pub format: u8,
}

/// One file inside an archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// Display path, always `/`-separated (e.g. `textures/actors/foo.dds`).
    pub path: String,
    /// Decompressed size in bytes (best effort).
    pub size: u64,
    /// Whether the data is stored compressed inside the archive.
    pub compressed: bool,
    /// Present for BA2 DX10 texture entries: the texture's dimensions.
    pub texture: Option<TexInfo>,
    /// Flat position in the archive's deterministic iteration order, used to
    /// re-find the file on extraction without depending on its name (some BA2s
    /// ship no name table).
    order: usize,
}

impl ArchiveEntry {
    /// The file name (last path component).
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The lowercased extension (without the dot), or "" if none.
    pub fn ext(&self) -> String {
        match self.file_name().rsplit_once('.') {
            Some((_, e)) => e.to_ascii_lowercase(),
            None => String::new(),
        }
    }

    /// A short human label of the entry's kind for the table's "Type" column.
    pub fn kind_label(&self) -> String {
        match &self.texture {
            Some(t) => format!("DDS {}×{}", t.width, t.height),
            None => {
                let e = self.ext();
                if e.is_empty() {
                    "fichier".to_string()
                } else {
                    e
                }
            }
        }
    }

    /// Extra detail shown in the row tooltip (texture mip count / DXGI format).
    pub fn detail(&self) -> Option<String> {
        self.texture.as_ref().map(|t| {
            format!(
                "{}×{} · {} mips · DXGI {}",
                t.width, t.height, t.mips, t.format
            )
        })
    }

    /// Build a bare entry for tests (only path/size matter to comparison).
    #[cfg(test)]
    pub fn for_test(path: &str, size: u64) -> Self {
        ArchiveEntry {
            path: path.to_string(),
            size,
            compressed: false,
            texture: None,
            order: 0,
        }
    }
}

/// A loaded archive. The raw bytes are **memory-mapped** (see [`Backing`]) and
/// re-parsed on demand for extraction (directory parsing is cheap), which avoids
/// self-referential storage of the borrowed [`ba2`] archive view and lets even
/// multi-gigabyte archives (e.g. `Starfield*.ba2`) open instantly.
pub struct Archive {
    path: PathBuf,
    data: Backing,
    format: Format,
    kind: String,
    entries: Vec<ArchiveEntry>,
}

impl Archive {
    /// Open an archive from disk, memory-mapping it (no full read into RAM).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file =
            std::fs::File::open(path).map_err(|e| format!("ouverture de l'archive : {e}"))?;
        // SAFETY: read-only mapping kept alive for the Archive's lifetime; the
        // file is only read, never mutated through the map.
        let mmap = unsafe { Mmap::map(&file) }.map_err(|e| format!("mmap de l'archive : {e}"))?;
        Self::from_backing(Backing::Mmap(mmap), path.to_path_buf())
    }

    /// Parse an archive already in memory (test entry point).
    #[cfg(test)]
    pub fn from_bytes(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        Self::from_backing(Backing::Owned(bytes), path)
    }

    fn from_backing(data: Backing, path: PathBuf) -> Result<Self, String> {
        let bytes = data.as_slice();
        let (format, kind, entries) = match ba2::guess_format(&mut &bytes[..]) {
            Some(ba2::FileFormat::TES4) => Self::parse_bsa(bytes)?,
            Some(ba2::FileFormat::FO4) => Self::parse_ba2(bytes)?,
            Some(ba2::FileFormat::TES3) => Self::parse_tes3_bsa(bytes)?,
            None => return Err("format d'archive inconnu (ni BSA ni BA2)".into()),
        };
        Ok(Self {
            path,
            data,
            format,
            kind,
            entries,
        })
    }

    fn parse_bsa(bytes: &[u8]) -> Result<(Format, String, Vec<ArchiveEntry>), String> {
        let (archive, meta) = tes4::Archive::read(ba2::Borrowed(bytes))
            .map_err(|e| format!("BSA illisible : {e}"))?;
        let mut entries = Vec::new();
        let mut order = 0usize;
        for (dir_key, dir) in archive.iter() {
            let dname = dir_key.name().to_string();
            // ba2 stores a root-level directory as ".".
            let ddisplay = if dname == "." { "" } else { dname.as_str() };
            for (file_key, file) in dir.iter() {
                let fname = file_key.name().to_string();
                let size = file.decompressed_len().unwrap_or_else(|| file.len()) as u64;
                let display = normalize(&join(ddisplay, &fname));
                entries.push(ArchiveEntry {
                    path: display,
                    size,
                    compressed: file.is_compressed(),
                    texture: None,
                    order,
                });
                order += 1;
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let kind = format!("BSA v{}", meta.version() as u32);
        Ok((Format::Bsa, kind, entries))
    }

    /// Parse a **Morrowind (TES3)** BSA. Layout (all little-endian, no
    /// compression):
    ///
    /// ```text
    /// 0x00  u32 version = 0x00000100
    /// 0x04  u32 hash_offset   (from byte 12: size of records + name offsets + names)
    /// 0x08  u32 file_count
    /// 0x0C  file records      8 * count : { u32 size, u32 data_offset }
    ///       name offsets      4 * count : u32 offset into the name table
    ///       name table        count null-terminated names (backslash paths)
    ///       hash table        8 * count : at 12 + hash_offset
    ///       file data         at 12 + hash_offset + 8*count ; data_offset is
    ///                         relative to this
    /// ```
    fn parse_tes3_bsa(bytes: &[u8]) -> Result<(Format, String, Vec<ArchiveEntry>), String> {
        let trunc = || "BSA Morrowind tronqué".to_string();
        let version = rd_u32(bytes, 0).ok_or_else(trunc)?;
        if version != 0x0000_0100 {
            return Err(format!(
                "BSA Morrowind : version inattendue 0x{version:08x}"
            ));
        }
        let hash_off = rd_u32(bytes, 4).ok_or_else(trunc)? as usize;
        let count = rd_u32(bytes, 8).ok_or_else(trunc)? as usize;

        let bad = || "BSA Morrowind : structure incohérente".to_string();
        let recs = 12usize;
        let name_off_tbl = recs
            .checked_add(count.checked_mul(8).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        let name_tbl = name_off_tbl
            .checked_add(count.checked_mul(4).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        let hash_tbl = 12usize.checked_add(hash_off).ok_or_else(bad)?;
        let data_start = hash_tbl
            .checked_add(count.checked_mul(8).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        if name_tbl > hash_tbl || hash_tbl > bytes.len() || data_start > bytes.len() {
            return Err(bad());
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let size = rd_u32(bytes, recs + i * 8).ok_or_else(trunc)? as u64;
            let data_off = rd_u32(bytes, recs + i * 8 + 4).ok_or_else(trunc)? as usize;
            let name_off = rd_u32(bytes, name_off_tbl + i * 4).ok_or_else(trunc)? as usize;
            let name = read_cstr(bytes, name_tbl + name_off, hash_tbl)
                .ok_or("BSA Morrowind : nom de fichier illisible")?;
            // Validate the file's byte range lies inside the archive.
            let end = data_start
                .checked_add(data_off)
                .and_then(|s| s.checked_add(size as usize));
            if end.map(|e| e > bytes.len()).unwrap_or(true) {
                return Err("BSA Morrowind : données de fichier hors limites".into());
            }
            entries.push(ArchiveEntry {
                path: normalize(&name),
                size,
                compressed: false,
                texture: None,
                order: i,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok((Format::Tes3Bsa, "BSA Morrowind".to_string(), entries))
    }

    fn parse_ba2(bytes: &[u8]) -> Result<(Format, String, Vec<ArchiveEntry>), String> {
        let (archive, _meta) =
            fo4::Archive::read(ba2::Borrowed(bytes)).map_err(|e| format!("BA2 illisible : {e}"))?;
        let mut entries = Vec::new();
        let mut has_dx10 = false;
        for (order, (key, file)) in archive.iter().enumerate() {
            let name = key.name().to_string();
            let size: u64 = file
                .iter()
                .map(|c| c.decompressed_len().unwrap_or_else(|| c.len()) as u64)
                .sum();
            let compressed = file.iter().any(|c| c.is_compressed());
            let texture = match &file.header {
                FileHeader::DX10(d) => {
                    has_dx10 = true;
                    Some(TexInfo {
                        width: d.width,
                        height: d.height,
                        mips: d.mip_count,
                        format: d.format,
                    })
                }
                _ => None,
            };
            // A BA2 without a name table yields empty names; keep the entry usable
            // with a synthetic label (extraction is by order, not by name).
            let path = if name.is_empty() {
                format!("(fichier {order})")
            } else {
                normalize(&name)
            };
            entries.push(ArchiveEntry {
                path,
                size,
                compressed,
                texture,
                order,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let kind = if has_dx10 {
            "BA2 DX10".to_string()
        } else {
            "BA2 GNRL".to_string()
        };
        Ok((Format::Ba2, kind, entries))
    }

    /// The archive file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The archive file name.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string()
    }

    /// A human-readable format label (`"BSA v105"`, `"BA2 GNRL"`, `"BA2 DX10"`).
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The listed entries (sorted by path).
    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Total decompressed size of all entries.
    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }

    /// Extract entry `index`, returning its **decompressed** bytes (a valid
    /// stand-alone `.dds` for DX10 textures).
    pub fn extract(&self, index: usize) -> Result<Vec<u8>, String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| "index d'entrée invalide".to_string())?;
        match self.format {
            Format::Bsa => self.extract_bsa(entry),
            Format::Tes3Bsa => self.extract_tes3(entry),
            Format::Ba2 => self.extract_ba2(entry),
        }
    }

    /// Extract a file from a Morrowind (TES3) BSA: a plain, uncompressed slice of
    /// the mapped bytes located from the file's record (see [`Self::parse_tes3_bsa`]).
    fn extract_tes3(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let bytes = self.data.as_slice();
        let trunc = || "BSA Morrowind tronqué".to_string();
        let hash_off = rd_u32(bytes, 4).ok_or_else(trunc)? as usize;
        let count = rd_u32(bytes, 8).ok_or_else(trunc)? as usize;
        let i = entry.order;
        if i >= count {
            return Err("index d'entrée invalide".into());
        }
        let data_start = 12usize
            .checked_add(hash_off)
            .and_then(|h| h.checked_add(count.checked_mul(8)?))
            .ok_or("BSA Morrowind : structure incohérente")?;
        let size = rd_u32(bytes, 12 + i * 8).ok_or_else(trunc)? as usize;
        let data_off = rd_u32(bytes, 12 + i * 8 + 4).ok_or_else(trunc)? as usize;
        let start = data_start.checked_add(data_off).ok_or_else(trunc)?;
        let end = start.checked_add(size).ok_or_else(trunc)?;
        bytes
            .get(start..end)
            .map(|s| s.to_vec())
            .ok_or_else(|| "BSA Morrowind : données de fichier hors limites".to_string())
    }

    fn extract_bsa(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let (archive, meta) = tes4::Archive::read(ba2::Borrowed(self.data.as_slice()))
            .map_err(|e| format!("BSA illisible : {e}"))?;
        let opts: tes4::FileCompressionOptions = (&meta).into();
        let mut i = 0usize;
        for (_dir_key, dir) in archive.iter() {
            for (_file_key, file) in dir.iter() {
                if i == entry.order {
                    let mut out = Vec::new();
                    file.write(&mut out, &opts)
                        .map_err(|e| format!("extraction impossible : {e}"))?;
                    return Ok(out);
                }
                i += 1;
            }
        }
        Err("fichier introuvable dans l'archive".into())
    }

    fn extract_ba2(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let (archive, meta) = fo4::Archive::read(ba2::Borrowed(self.data.as_slice()))
            .map_err(|e| format!("BA2 illisible : {e}"))?;
        let opts: fo4::FileWriteOptions = (&meta).into();
        let (_key, file) = archive
            .iter()
            .nth(entry.order)
            .ok_or_else(|| "fichier introuvable dans l'archive".to_string())?;
        let mut out = Vec::new();
        file.write(&mut out, &opts)
            .map_err(|e| format!("extraction impossible : {e}"))?;
        Ok(out)
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}\\{name}")
    }
}

/// Normalize a stored archive path to forward slashes for display.
fn normalize(s: &str) -> String {
    s.replace('\\', "/")
}

/// Read a little-endian `u32` at `off`, or `None` if it would run past the end.
fn rd_u32(bytes: &[u8], off: usize) -> Option<u32> {
    bytes
        .get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read a NUL-terminated name starting at `start` (must end before `limit`),
/// decoding bytes as Windows-1252/Latin-1 (Morrowind's encoding for its mostly
/// ASCII paths). Returns `None` if unterminated or out of range.
fn read_cstr(bytes: &[u8], start: usize, limit: usize) -> Option<String> {
    let region = bytes.get(start..limit.min(bytes.len()))?;
    let n = region.iter().position(|&b| b == 0)?;
    Some(region[..n].iter().map(|&b| b as char).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree;
    use ba2::fo4::{Archive as Fo4Archive, ArchiveKey, ArchiveOptions, Chunk, File};
    use ba2::prelude::*;

    /// Build a small in-memory BA2 (GNRL) with three files across two folders.
    fn synthetic_ba2() -> Vec<u8> {
        let files = [
            (b"a.txt".to_vec(), b"AAA".to_vec()),
            (b"sub\\b.txt".to_vec(), b"BBBB".to_vec()),
            (b"sub\\deep\\c.txt".to_vec(), b"CCCCC".to_vec()),
        ];
        let archive: Fo4Archive = files
            .iter()
            .map(|(name, data)| {
                let chunk = Chunk::from_decompressed(data.as_slice());
                let file: File = [chunk].into_iter().collect();
                let key: ArchiveKey = name.as_slice().into();
                (key, file)
            })
            .collect();
        // `strings(true)` writes the file-name table, as real BA2 tools do.
        let options = ArchiveOptions::builder().strings(true).build();
        let mut out = Vec::new();
        archive.write(&mut out, &options).expect("write ba2");
        out
    }

    #[test]
    fn parse_extract_and_tree() {
        let bytes = synthetic_ba2();
        let archive = Archive::from_bytes(bytes, PathBuf::from("test.ba2")).expect("parse");
        assert!(archive.kind().starts_with("BA2"));
        assert_eq!(archive.entries().len(), 3);

        // Paths are normalized to forward slashes and sorted.
        let paths: Vec<&str> = archive.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "sub/b.txt", "sub/deep/c.txt"]);

        // Extraction returns the original decompressed bytes.
        let idx = archive
            .entries()
            .iter()
            .position(|e| e.path == "sub/b.txt")
            .unwrap();
        assert_eq!(archive.extract(idx).unwrap(), b"BBBB");

        // The folder tree is reconstructed from the flat paths.
        let root = tree::build(archive.entries());
        assert_eq!(root.subtree_files, 3);
        assert_eq!(root.files.len(), 1); // a.txt at the root
        let sub = tree::find(&root, "sub").expect("sub folder");
        assert_eq!(sub.files.len(), 1); // sub/b.txt
        assert_eq!(sub.subtree_files, 2); // b.txt + deep/c.txt
        let deep = tree::find(&root, "sub/deep").expect("deep folder");
        assert_eq!(deep.files.len(), 1);
    }

    /// Build a small in-memory Morrowind (TES3) BSA with two files.
    fn synthetic_tes3_bsa() -> Vec<u8> {
        let files: [(&str, &[u8]); 2] = [("a.txt", b"AAA"), ("sub\\b.txt", b"BBBB")];
        let count = files.len();
        let mut name_tbl = Vec::new();
        let mut name_offs = Vec::new();
        for (name, _) in &files {
            name_offs.push(name_tbl.len() as u32);
            name_tbl.extend_from_slice(name.as_bytes());
            name_tbl.push(0);
        }
        let mut data = Vec::new();
        let mut recs = Vec::new();
        for (_, b) in &files {
            let off = data.len() as u32;
            recs.push((b.len() as u32, off));
            data.extend_from_slice(b);
        }
        let hash_off = 8 * count + 4 * count + name_tbl.len();
        let mut out = Vec::new();
        out.extend_from_slice(&0x100u32.to_le_bytes());
        out.extend_from_slice(&(hash_off as u32).to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());
        for (sz, off) in &recs {
            out.extend_from_slice(&sz.to_le_bytes());
            out.extend_from_slice(&off.to_le_bytes());
        }
        for no in &name_offs {
            out.extend_from_slice(&no.to_le_bytes());
        }
        out.extend_from_slice(&name_tbl);
        out.extend_from_slice(&vec![0u8; 8 * count]); // hash table (unused by us)
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn parse_extract_tes3_bsa() {
        let bytes = synthetic_tes3_bsa();
        let archive = Archive::from_bytes(bytes, PathBuf::from("Morrowind.bsa")).expect("parse");
        assert_eq!(archive.kind(), "BSA Morrowind");
        assert_eq!(archive.entries().len(), 2);
        let paths: Vec<&str> = archive.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "sub/b.txt"]); // normalized + sorted
        // Extraction returns the raw (uncompressed) bytes for either file.
        let idx = archive
            .entries()
            .iter()
            .position(|e| e.path == "sub/b.txt")
            .unwrap();
        assert_eq!(archive.extract(idx).unwrap(), b"BBBB");
        let idx0 = archive
            .entries()
            .iter()
            .position(|e| e.path == "a.txt")
            .unwrap();
        assert_eq!(archive.extract(idx0).unwrap(), b"AAA");
    }

    #[test]
    fn rejects_unknown_format() {
        match Archive::from_bytes(vec![0u8; 32], PathBuf::from("x.bin")) {
            Err(e) => assert!(e.contains("inconnu")),
            Ok(_) => panic!("should reject a non-archive"),
        }
    }
}
