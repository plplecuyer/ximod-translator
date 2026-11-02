//! Bethesda archive navigation (CDC §3): reading **BSA** (Oblivion / Skyrim /
//! Skyrim SE) and **BA2** (Fallout 4 / Starfield) archives.
//!
//! The reference (`TESVT_bsa.pas`) hand-rolls both formats and links `zlz4/` for
//! zlib + LZ4. To stay correct across every format variant (BSA 103/104/105,
//! BA2 GNRL v1/v2/v8, zlib/LZ4/LZ4F/zstd) without reimplementing it all, we wrap
//! [`ba2`] — the pure-Rust archive library by the author of the canonical C++
//! `bsa`/`ba2` tools, released under the very permissive 0BSD license. We expose
//! a single flat, format-agnostic view: list the entries, then extract any one
//! of them **already decompressed**, which is all a translator needs (pull the
//! plugins and STRINGS out of an archive).
//!
//! Textures (BA2 `DX10`) are intentionally out of scope — they carry no
//! translatable text — so only `GNRL` general files are listed for BA2.

use std::path::{Path, PathBuf};

use ba2::{Reader, fo4, tes4};

/// Which on-disk format an [`Archive`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// BSA — Oblivion / Fallout 3-NV / Skyrim / Skyrim SE.
    Bsa,
    /// BSA — **Morrowind** (TES3 layout, version `0x100`), parsed natively.
    Tes3Bsa,
    /// BA2 — Fallout 4 / Starfield (GNRL general files).
    Ba2,
}

/// The translatable file extensions a translator cares about inside an archive.
const TRANSLATABLE_EXTS: &[&str] = &["esp", "esm", "esl", "strings", "dlstrings", "ilstrings"];

/// Whether a path points at a plugin or a STRINGS file (case-insensitive).
pub fn is_translatable(path: &str) -> bool {
    match path.rsplit('.').next() {
        Some(ext) => {
            let ext = ext.to_ascii_lowercase();
            TRANSLATABLE_EXTS.contains(&ext.as_str())
        }
        None => false,
    }
}

/// One file inside an archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// Display path, always `/`-separated (e.g. `strings/mymod_english.strings`).
    pub path: String,
    /// Decompressed size in bytes (best effort).
    pub size: u64,
    /// Whether the data is stored compressed.
    pub compressed: bool,
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

    /// Whether this entry is a plugin or STRINGS file.
    pub fn is_translatable(&self) -> bool {
        is_translatable(&self.path)
    }
}

/// A loaded archive: its bytes are kept in memory and re-parsed on demand for
/// extraction (directory parsing is cheap), which avoids self-referential
/// storage of the borrowed [`ba2`] archive view.
pub struct Archive {
    path: PathBuf,
    bytes: Vec<u8>,
    format: Format,
    kind: String,
    entries: Vec<ArchiveEntry>,
}

impl Archive {
    /// Open an archive from disk.
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("lecture de l'archive : {e}"))?;
        Self::from_bytes(bytes, path.to_path_buf())
    }

    /// Parse an archive already in memory (also the test entry point).
    pub fn from_bytes(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        match ba2::guess_format(&mut &bytes[..]) {
            Some(ba2::FileFormat::TES4) => Self::parse_bsa(bytes, path),
            Some(ba2::FileFormat::FO4) => Self::parse_ba2(bytes, path),
            Some(ba2::FileFormat::TES3) => Self::parse_tes3_bsa(bytes, path),
            None => Err("format d'archive inconnu (ni BSA ni BA2)".into()),
        }
    }

    fn parse_bsa(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        let (archive, meta) = tes4::Archive::read(ba2::Borrowed(&bytes))
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
                    order,
                });
                order += 1;
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let kind = format!("BSA v{}", meta.version() as u32);
        Ok(Self {
            path,
            bytes,
            format: Format::Bsa,
            kind,
            entries,
        })
    }

    fn parse_ba2(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        let (archive, _meta) = fo4::Archive::read(ba2::Borrowed(&bytes))
            .map_err(|e| format!("BA2 illisible : {e}"))?;
        let mut entries = Vec::new();
        for (order, (key, file)) in archive.iter().enumerate() {
            let name = key.name().to_string();
            let size: u64 = file
                .iter()
                .map(|c| c.decompressed_len().unwrap_or_else(|| c.len()) as u64)
                .sum();
            let compressed = file.iter().any(|c| c.is_compressed());
            // A BA2 without a name table yields empty names; keep the entry
            // usable with a synthetic label (extraction is by order, not name).
            let path = if name.is_empty() {
                format!("(fichier {order})")
            } else {
                normalize(&name)
            };
            entries.push(ArchiveEntry {
                path,
                size,
                compressed,
                order,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            path,
            bytes,
            format: Format::Ba2,
            kind: "BA2 GNRL".into(),
            entries,
        })
    }

    /// Parse a **Morrowind (TES3)** BSA (version `0x100`, uncompressed). Layout:
    /// header (version, hash_offset, file_count), then file records
    /// `{u32 size, u32 data_offset}`, name-offset table, name table, hash table
    /// (at `12 + hash_offset`), then file data at `12 + hash_offset + 8*count`.
    fn parse_tes3_bsa(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        let b = &bytes[..];
        let trunc = || "BSA Morrowind tronqué".to_string();
        let version = rd_u32(b, 0).ok_or_else(trunc)?;
        if version != 0x0000_0100 {
            return Err(format!(
                "BSA Morrowind : version inattendue 0x{version:08x}"
            ));
        }
        let hash_off = rd_u32(b, 4).ok_or_else(trunc)? as usize;
        let count = rd_u32(b, 8).ok_or_else(trunc)? as usize;
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
        if name_tbl > hash_tbl || hash_tbl > b.len() || data_start > b.len() {
            return Err(bad());
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let size = rd_u32(b, recs + i * 8).ok_or_else(trunc)? as u64;
            let data_off = rd_u32(b, recs + i * 8 + 4).ok_or_else(trunc)? as usize;
            let name_off = rd_u32(b, name_off_tbl + i * 4).ok_or_else(trunc)? as usize;
            let name = read_cstr(b, name_tbl + name_off, hash_tbl)
                .ok_or("BSA Morrowind : nom de fichier illisible")?;
            let end = data_start
                .checked_add(data_off)
                .and_then(|s| s.checked_add(size as usize));
            if end.map(|e| e > b.len()).unwrap_or(true) {
                return Err("BSA Morrowind : données de fichier hors limites".into());
            }
            entries.push(ArchiveEntry {
                path: normalize(&name),
                size,
                compressed: false,
                order: i,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            path,
            bytes,
            format: Format::Tes3Bsa,
            kind: "BSA Morrowind".into(),
            entries,
        })
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

    /// A human-readable format label (`"BSA v105"`, `"BA2 GNRL"`).
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The archive format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The listed entries (sorted by path).
    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    /// Extract entry `index`, returning its **decompressed** bytes.
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

    /// Extract an uncompressed file slice from a Morrowind (TES3) BSA.
    fn extract_tes3(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let b = &self.bytes[..];
        let trunc = || "BSA Morrowind tronqué".to_string();
        let hash_off = rd_u32(b, 4).ok_or_else(trunc)? as usize;
        let count = rd_u32(b, 8).ok_or_else(trunc)? as usize;
        let i = entry.order;
        if i >= count {
            return Err("index d'entrée invalide".into());
        }
        let data_start = 12usize
            .checked_add(hash_off)
            .and_then(|h| h.checked_add(count.checked_mul(8)?))
            .ok_or("BSA Morrowind : structure incohérente")?;
        let size = rd_u32(b, 12 + i * 8).ok_or_else(trunc)? as usize;
        let data_off = rd_u32(b, 12 + i * 8 + 4).ok_or_else(trunc)? as usize;
        let start = data_start.checked_add(data_off).ok_or_else(trunc)?;
        let end = start.checked_add(size).ok_or_else(trunc)?;
        b.get(start..end)
            .map(|s| s.to_vec())
            .ok_or_else(|| "BSA Morrowind : données de fichier hors limites".to_string())
    }

    fn extract_bsa(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let (archive, meta) = tes4::Archive::read(ba2::Borrowed(&self.bytes))
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
        let (archive, meta) = fo4::Archive::read(ba2::Borrowed(&self.bytes))
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

/// Join a directory and a file name with a backslash (as stored), tolerating an
/// empty directory.
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

/// Read a NUL-terminated name at `start` (ending before `limit`), decoded as
/// Windows-1252/Latin-1 (Morrowind's encoding). `None` if unterminated.
fn read_cstr(bytes: &[u8], start: usize, limit: usize) -> Option<String> {
    let region = bytes.get(start..limit.min(bytes.len()))?;
    let n = region.iter().position(|&b| b == 0)?;
    Some(region[..n].iter().map(|&b| b as char).collect())
}
