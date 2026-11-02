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
        let bytes = std::fs::read(path).map_err(|e| crate::l10n!("err-archive-read", error = e))?;
        Self::from_bytes(bytes, path.to_path_buf())
    }

    /// The paths of an archive's entries, without loading the archive in
    /// memory (the file is mapped, only its directory is read): what the
    /// voice map (C-13) needs from a « MonMod - Voices.bsa ».
    pub fn list_paths(path: &Path) -> Result<Vec<String>, String> {
        let file =
            std::fs::File::open(path).map_err(|e| crate::l10n!("err-archive-read", error = e))?;
        // SAFETY: the file is only read; a concurrent modification would at
        // worst yield a parse error.
        let mmap = unsafe { memmap2::Mmap::map(&file) }
            .map_err(|e| crate::l10n!("err-archive-read", error = e))?;
        let bytes: &[u8] = &mmap;
        match ba2::guess_format(&mut &bytes[..]) {
            Some(ba2::FileFormat::TES4) => {
                let (archive, _) = tes4::Archive::read(ba2::Borrowed(bytes))
                    .map_err(|e| crate::l10n!("err-bsa-unreadable", error = e))?;
                let mut out = Vec::new();
                for (dir_key, dir) in archive.iter() {
                    let dname = dir_key.name().to_string();
                    let ddisplay = if dname == "." { "" } else { dname.as_str() };
                    for (file_key, _) in dir.iter() {
                        out.push(normalize(&join(ddisplay, &file_key.name().to_string())));
                    }
                }
                Ok(out)
            }
            Some(ba2::FileFormat::FO4) => {
                let (archive, _) = fo4::Archive::read(ba2::Borrowed(bytes))
                    .map_err(|e| crate::l10n!("err-ba2-unreadable", error = e))?;
                Ok(archive
                    .iter()
                    .map(|(key, _)| normalize(&key.name().to_string()))
                    .filter(|p| !p.is_empty())
                    .collect())
            }
            Some(ba2::FileFormat::TES3) => Err(crate::l10n!("err-archive-tes3")),
            None => Err(crate::l10n!("err-archive-unknown")),
        }
    }

    /// Parse an archive already in memory (also the test entry point).
    pub fn from_bytes(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        match ba2::guess_format(&mut &bytes[..]) {
            Some(ba2::FileFormat::TES4) => Self::parse_bsa(bytes, path),
            Some(ba2::FileFormat::FO4) => Self::parse_ba2(bytes, path),
            Some(ba2::FileFormat::TES3) => Err(crate::l10n!("err-archive-tes3")),
            None => Err(crate::l10n!("err-archive-unknown")),
        }
    }

    fn parse_bsa(bytes: Vec<u8>, path: PathBuf) -> Result<Self, String> {
        let (archive, meta) = tes4::Archive::read(ba2::Borrowed(&bytes))
            .map_err(|e| crate::l10n!("err-bsa-unreadable", error = e))?;
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
            .map_err(|e| crate::l10n!("err-ba2-unreadable", error = e))?;
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
                format!("#{order}")
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
            .ok_or_else(|| crate::l10n!("err-archive-index"))?;
        match self.format {
            Format::Bsa => self.extract_bsa(entry),
            Format::Ba2 => self.extract_ba2(entry),
        }
    }

    fn extract_bsa(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let (archive, meta) = tes4::Archive::read(ba2::Borrowed(&self.bytes))
            .map_err(|e| crate::l10n!("err-bsa-unreadable", error = e))?;
        let opts: tes4::FileCompressionOptions = (&meta).into();
        let mut i = 0usize;
        for (_dir_key, dir) in archive.iter() {
            for (_file_key, file) in dir.iter() {
                if i == entry.order {
                    let mut out = Vec::new();
                    file.write(&mut out, &opts)
                        .map_err(|e| crate::l10n!("err-archive-extract", error = e))?;
                    return Ok(out);
                }
                i += 1;
            }
        }
        Err(crate::l10n!("err-archive-missing"))
    }

    fn extract_ba2(&self, entry: &ArchiveEntry) -> Result<Vec<u8>, String> {
        let (archive, meta) = fo4::Archive::read(ba2::Borrowed(&self.bytes))
            .map_err(|e| crate::l10n!("err-ba2-unreadable", error = e))?;
        let opts: fo4::FileWriteOptions = (&meta).into();
        let (_key, file) = archive
            .iter()
            .nth(entry.order)
            .ok_or_else(|| crate::l10n!("err-archive-missing"))?;
        let mut out = Vec::new();
        file.write(&mut out, &opts)
            .map_err(|e| crate::l10n!("err-archive-extract", error = e))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ba2::prelude::*;
    use ba2::{
        fo4::{
            Archive as Fo4Archive, ArchiveKey as Fo4AKey, ArchiveOptions as Fo4Opts, Chunk,
            File as Fo4File,
        },
        tes4::{
            Archive as Tes4Archive, ArchiveKey as Tes4AKey, ArchiveOptions as Tes4Opts, Directory,
            DirectoryKey, File as Tes4File, Version,
        },
    };

    /// Build an in-memory BSA (SSE) with two files, one in a subdirectory.
    fn make_bsa() -> Vec<u8> {
        let f1 = Tes4File::from_decompressed(b"plugin bytes here");
        let f2 = Tes4File::from_decompressed(b"hello world string");
        let d_root: Directory = [(DirectoryKey::from(b"mymod.esp".as_slice()), f1)]
            .into_iter()
            .collect();
        let d_str: Directory = [(DirectoryKey::from(b"mymod_english.strings".as_slice()), f2)]
            .into_iter()
            .collect();
        let archive: Tes4Archive = [
            (Tes4AKey::from(b"".as_slice()), d_root),
            (Tes4AKey::from(b"strings".as_slice()), d_str),
        ]
        .into_iter()
        .collect();
        let opts = Tes4Opts::builder().version(Version::SSE).build();
        let mut bytes = Vec::new();
        archive.write(&mut bytes, &opts).expect("write bsa");
        bytes
    }

    /// Build an in-memory BA2 (GNRL) with one STRINGS file.
    fn make_ba2() -> Vec<u8> {
        let chunk = Chunk::from_decompressed(b"ba2 strings payload");
        let file: Fo4File = [chunk].into_iter().collect();
        let archive: Fo4Archive = [(
            Fo4AKey::from(b"strings\\mymod_french.strings".as_slice()),
            file,
        )]
        .into_iter()
        .collect();
        // Real Bethesda BA2s embed the file-name (string) table; enable it so
        // names round-trip (the default omits it).
        let opts = Fo4Opts::builder().strings(true).build();
        let mut bytes = Vec::new();
        archive.write(&mut bytes, &opts).expect("write ba2");
        bytes
    }

    #[test]
    fn opens_and_lists_bsa() {
        let a = Archive::from_bytes(make_bsa(), "Test.bsa".into()).unwrap();
        assert_eq!(a.format(), Format::Bsa);
        assert!(a.kind().starts_with("BSA v"));
        let paths: Vec<&str> = a.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["mymod.esp", "strings/mymod_english.strings"]);
    }

    /// C-13: the directory alone, from a mapped file.
    #[test]
    fn lists_paths_without_loading() {
        let dir = std::env::temp_dir().join(format!("ximod_list_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bsa = dir.join("t.bsa");
        std::fs::write(&bsa, make_bsa()).unwrap();
        let mut paths = Archive::list_paths(&bsa).unwrap();
        paths.sort();
        assert_eq!(paths, ["mymod.esp", "strings/mymod_english.strings"]);
        let ba2 = dir.join("t.ba2");
        std::fs::write(&ba2, make_ba2()).unwrap();
        assert_eq!(
            Archive::list_paths(&ba2).unwrap(),
            ["strings/mymod_french.strings"]
        );
        std::fs::write(dir.join("x.bsa"), b"nope").unwrap();
        assert!(Archive::list_paths(&dir.join("x.bsa")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_bsa_bytes() {
        let a = Archive::from_bytes(make_bsa(), "Test.bsa".into()).unwrap();
        let esp = a
            .entries()
            .iter()
            .position(|e| e.path == "mymod.esp")
            .unwrap();
        assert_eq!(a.extract(esp).unwrap(), b"plugin bytes here");
        let s = a
            .entries()
            .iter()
            .position(|e| e.file_name() == "mymod_english.strings")
            .unwrap();
        assert_eq!(a.extract(s).unwrap(), b"hello world string");
    }

    #[test]
    fn opens_and_extracts_ba2() {
        let a = Archive::from_bytes(make_ba2(), "Test.ba2".into()).unwrap();
        assert_eq!(a.format(), Format::Ba2);
        assert_eq!(a.entries().len(), 1);
        let e = &a.entries()[0];
        assert_eq!(e.path, "strings/mymod_french.strings");
        assert!(e.is_translatable());
        assert_eq!(a.extract(0).unwrap(), b"ba2 strings payload");
    }

    #[test]
    fn translatable_detection() {
        assert!(is_translatable("Foo/bar.esp"));
        assert!(is_translatable("x.DLSTRINGS"));
        assert!(!is_translatable("textures/foo.dds"));
        assert!(!is_translatable("meshes/foo.nif"));
    }

    #[test]
    fn rejects_non_archive() {
        let err = Archive::from_bytes(b"not an archive at all".to_vec(), "x.bin".into());
        assert!(err.is_err());
    }
}
