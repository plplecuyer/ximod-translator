//! Archive export — 7z (priority 3) and BA2 packaging (priority 12).
//!
//! V1 exports a distribution `.zip` (see [`crate::export`]). V2 adds:
//! - **7z export**, preferred by Nexus for download size, selectable alongside
//!   ZIP with a compression level;
//! - **BA2 packaging**, bundling loose files into Bethesda archives (Skyrim SE,
//!   Fallout 4, Starfield) — an advanced, heavier feature.
//!
//! This module presents one entry point ([`export_archive`]) that dispatches on
//! [`ArchiveFormat`]; ZIP delegates to the existing V1 code so behavior is
//! unchanged, and 7z is added without touching the ZIP path.
//!
//! Status (Lot A): ZIP delegates to V1; **7z export is implemented** (LZMA2 via
//! the `sevenz-rust` crate). BA2 remains a TODO.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::export;
use crate::model::Ximod;

/// Output archive format for a distribution package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchiveFormat {
    /// ZIP (V1 behavior, always available).
    #[default]
    Zip,
    /// 7-Zip (LZMA2). Smaller downloads; planned for V2.
    SevenZip,
}

impl ArchiveFormat {
    /// File extension (without the dot). Used by the UI/CLI format pickers.
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::SevenZip => "7z",
        }
    }
}

/// Compression effort, mapped per-format to concrete levels at build time.
/// Only `Normal` is wired to the UI today; the other levels await the 7z
/// level-selection control (Lot C/D).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionLevel {
    /// No compression (fastest, store only).
    Store,
    /// Fast, lower ratio.
    Fast,
    /// Balanced (default).
    #[default]
    Normal,
    /// Maximum ratio (slowest).
    Max,
}

/// Build a distribution archive of the FOMOD under `root_dir` into `out_path`,
/// in the requested `format`. Returns the number of files written.
///
/// ZIP delegates to [`export::build_distribution_archive`] (unchanged V1 path).
/// 7z is a TODO.
pub fn export_archive(
    ximod: &Ximod,
    root_dir: &Path,
    out_path: &Path,
    format: ArchiveFormat,
    level: CompressionLevel,
) -> Result<usize> {
    crate::xml::save_ximod(ximod, root_dir)
        .with_context(|| "writing the FOMOD XML before packaging")?;
    package_directory(root_dir, out_path, format, level, &mut |_, _, _| true)
}

/// Package an already-saved mod folder into an archive, reporting progress.
///
/// Unlike [`export_archive`] this does not touch the FOMOD XML, so it can run
/// on a worker thread while the UI stays responsive: the GUI saves first, then
/// hands the folder over.
pub fn package_directory(
    root_dir: &Path,
    out_path: &Path,
    format: ArchiveFormat,
    level: CompressionLevel,
    progress: export::Progress<'_>,
) -> Result<usize> {
    match format {
        ArchiveFormat::Zip => export::zip_directory_with(root_dir, out_path, progress),
        ArchiveFormat::SevenZip => sevenzip_directory(root_dir, out_path, level, progress),
    }
}

/// Package the whole directory into a `.7z` archive (LZMA2). Mirrors
/// [`export::zip_directory_with`] but with 7-Zip output. Returns the number of
/// files written.
fn sevenzip_directory(
    root_dir: &Path,
    out_path: &Path,
    _level: CompressionLevel,
    progress: export::Progress<'_>,
) -> Result<usize> {
    export_with_overrides(
        root_dir,
        out_path,
        ArchiveFormat::SevenZip,
        &HashMap::new(),
        None,
        progress,
    )
}

/// Where the content of an archive entry comes from.
enum EntrySource<'a> {
    /// A file on disk (streamed).
    Disk(PathBuf),
    /// Bytes held in memory (an override).
    Bytes(&'a [u8]),
}

/// One file to write into an archive.
struct PlannedEntry<'a> {
    /// Archive-relative path, with `/` separators.
    name: String,
    source: EntrySource<'a>,
}

/// Archive-relative form of a path: `/` separators, no leading `./` or `/`.
fn archive_name(path: &str) -> String {
    let name = path.replace('\\', "/");
    let name = name.trim_start_matches("./").trim_start_matches('/');
    name.to_string()
}

/// List the entries of an archive of `root`, shared by the ZIP and 7z
/// writers: the distribution files of `root` (or, when `only` is given, just
/// the named ones), each replaced by its override when there is one, plus the
/// overrides that have no file on disk. Sorted by archive path.
fn plan_entries<'a>(
    root: &Path,
    out: &Path,
    overrides: &'a HashMap<String, Vec<u8>>,
    only: Option<&[String]>,
) -> Result<Vec<PlannedEntry<'a>>> {
    // BTreeMap: sorted and free of duplicates, whatever the callers pass.
    let mut planned: BTreeMap<String, EntrySource<'a>> = BTreeMap::new();
    match only {
        Some(paths) => {
            for path in paths {
                let name = archive_name(path);
                if name.is_empty() || name.split('/').any(|part| part == "..") {
                    anyhow::bail!("'{path}' is not a path inside the archive");
                }
                let abs = root.join(&name);
                planned.insert(name, EntrySource::Disk(abs));
            }
        }
        None => {
            for rel in export::collect_distribution_files(root, out)? {
                let name = archive_name(&rel.to_string_lossy());
                planned.insert(name, EntrySource::Disk(root.join(&rel)));
            }
        }
    }
    for (path, bytes) in overrides {
        let name = archive_name(path);
        if name.is_empty() {
            anyhow::bail!("an override has an empty archive path");
        }
        planned.insert(name, EntrySource::Bytes(bytes));
    }
    // A file named in `only` must exist, unless an override supplies it.
    for (name, source) in &planned {
        if let EntrySource::Disk(abs) = source
            && !abs.is_file()
        {
            anyhow::bail!("{name}: {} is not a file", abs.display());
        }
    }
    Ok(planned
        .into_iter()
        .map(|(name, source)| PlannedEntry { name, source })
        .collect())
}

/// Open the content of an entry for reading.
fn open_entry<'a>(entry: &PlannedEntry<'a>) -> Result<Box<dyn Read + 'a>> {
    Ok(match &entry.source {
        EntrySource::Disk(abs) => Box::new(std::io::BufReader::new(
            std::fs::File::open(abs).with_context(|| format!("reading {}", abs.display()))?,
        )),
        EntrySource::Bytes(bytes) => Box::new(Cursor::new(*bytes)),
    })
}

/// Write `entries` into a new archive at `out`, streaming each file. Returns
/// the number of files written. A failed or cancelled archive is removed.
fn write_archive(
    out: &Path,
    format: ArchiveFormat,
    entries: &[PlannedEntry<'_>],
    progress: export::Progress<'_>,
) -> Result<usize> {
    let total = entries.len();
    // Report progress before each entry; `false` cancels.
    let mut step = |i: usize, entry: &PlannedEntry<'_>| -> Result<()> {
        if progress(i, total, Path::new(&entry.name)) {
            Ok(())
        } else {
            Err(export::Cancelled.into())
        }
    };

    let result = (|| -> Result<()> {
        match format {
            ArchiveFormat::Zip => {
                let file = std::fs::File::create(out)
                    .with_context(|| format!("creating {}", out.display()))?;
                let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
                let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .large_file(true);
                for (i, entry) in entries.iter().enumerate() {
                    step(i, entry)?;
                    zip.start_file(entry.name.as_str(), opts)?;
                    std::io::copy(&mut open_entry(entry)?, &mut zip)
                        .with_context(|| format!("adding {} to the archive", entry.name))?;
                }
                zip.finish()?.into_inner().map_err(|e| e.into_error())?;
            }
            ArchiveFormat::SevenZip => {
                let mut sz = sevenz_rust::SevenZWriter::create(out)
                    .with_context(|| format!("creating {}", out.display()))?;
                // Default content method is LZMA2. TODO(v2): map a compression
                // level to an LZMA2 preset via set_content_methods.
                for (i, entry) in entries.iter().enumerate() {
                    step(i, entry)?;
                    let header = match &entry.source {
                        EntrySource::Disk(abs) => {
                            sevenz_rust::SevenZArchiveEntry::from_path(abs, entry.name.clone())
                        }
                        EntrySource::Bytes(_) => {
                            let mut header = sevenz_rust::SevenZArchiveEntry::new();
                            header.name = entry.name.clone();
                            header.has_stream = true;
                            if let Ok(now) = std::time::SystemTime::now().try_into() {
                                header.last_modified_date = now;
                                header.has_last_modified_date = true;
                            }
                            header
                        }
                    };
                    sz.push_archive_entry(header, Some(open_entry(entry)?))
                        .with_context(|| format!("adding {} to the 7z archive", entry.name))?;
                }
                sz.finish().with_context(|| "finalizing the 7z archive")?;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            progress(total, total, Path::new(""));
            Ok(total)
        }
        Err(e) => {
            let _ = std::fs::remove_file(out);
            Err(e)
        }
    }
}

/// Package `root` into `out` like [`package_directory`], but with some files
/// replaced and, optionally, limited to a list of files.
///
/// * `overrides` maps an archive-relative path (`/` separators, matched
///   exactly, case included) to the bytes to store instead of the file on
///   disk. An override is always included, even when no such file exists on
///   disk and even when `only` does not name it.
/// * `only`, when given, replaces the enumeration of `root`: the archive holds
///   exactly those archive-relative paths (read from disk unless overridden)
///   plus the overrides. Naming a file that neither exists nor is overridden
///   is an error.
///
/// Returns the number of files written. `root` itself is never modified.
pub fn export_with_overrides(
    root: &Path,
    out: &Path,
    format: ArchiveFormat,
    overrides: &HashMap<String, Vec<u8>>,
    only: Option<&[String]>,
    progress: export::Progress<'_>,
) -> Result<usize> {
    let entries = plan_entries(root, out, overrides, only)?;
    write_archive(out, format, &entries, progress)
}

/// Standard CRC-32 (IEEE 802.3) of `bytes` — the hash BA2 uses for name/dir.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_correct() {
        assert_eq!(ArchiveFormat::Zip.extension(), "zip");
        assert_eq!(ArchiveFormat::SevenZip.extension(), "7z");
    }

    #[test]
    fn sevenzip_export_writes_a_readable_archive() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("ximod_7z_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("textures")).unwrap();
        fs::write(dir.join("MyMod.esp"), b"plugin").unwrap();
        fs::write(dir.join("textures/a.dds"), b"texture-bytes").unwrap();

        let mut x = Ximod::new("Test7z");
        x.required_files
            .push(crate::model::InstallFile::new_file("MyMod.esp"));

        let out = dir.join("Test7z.7z");
        let n = export_archive(
            &x,
            &dir,
            &out,
            ArchiveFormat::SevenZip,
            CompressionLevel::Normal,
        )
        .expect("7z export should succeed");
        // At least the two mod files + the two FOMOD XML files written by save_ximod.
        assert!(n >= 3, "expected several files, got {n}");
        assert!(out.is_file());
        // The archive must decompress back to the same files.
        let back = dir.join("unpacked");
        sevenz_rust::decompress_file(&out, &back).expect("7z should be readable");
        assert!(back.join("MyMod.esp").is_file());
        assert!(back.join("fomod/ModuleConfig.xml").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    /// Names of the files of a ZIP archive, in archive order.
    fn zip_names(path: &Path) -> Vec<String> {
        let zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        (0..zip.len())
            .map(|i| zip.name_for_index(i).unwrap().to_string())
            .collect()
    }

    fn zip_read(path: &Path, name: &str) -> Vec<u8> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut out = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    /// Relative paths (with `/`) of the files under `dir`, sorted.
    fn files_under(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = walkdir::WalkDir::new(dir)
            .into_iter()
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                e.path()
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        out.sort();
        out
    }

    fn overrides_root(tag: &str) -> PathBuf {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("ximod_ovr_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("root/fomod")).unwrap();
        fs::create_dir_all(dir.join("root/textures")).unwrap();
        fs::write(dir.join("root/fomod/info.xml"), b"<original/>").unwrap();
        fs::write(dir.join("root/fomod/ModuleConfig.xml"), b"<config/>").unwrap();
        fs::write(dir.join("root/textures/a.dds"), b"texture-bytes").unwrap();
        fs::write(dir.join("root/Plugin.esp"), b"plugin").unwrap();
        fs::write(dir.join("root/Thumbs.db"), b"junk").unwrap();
        dir
    }

    #[test]
    fn export_with_overrides_zip() {
        let dir = overrides_root("zip");
        let root = dir.join("root");
        let mut overrides = HashMap::new();
        overrides.insert("fomod/info.xml".to_string(), b"<translated/>".to_vec());
        overrides.insert("README_FR.txt".to_string(), b"lisez-moi".to_vec());

        // Limited to two files from disk (one of them overridden) + the
        // override that only exists in memory.
        let only = vec!["fomod/info.xml".to_string(), "Plugin.esp".to_string()];
        let out = dir.join("only.zip");
        let mut seen = Vec::new();
        let n = export_with_overrides(
            &root,
            &out,
            ArchiveFormat::Zip,
            &overrides,
            Some(&only),
            &mut |d, t, p| {
                seen.push((d, t, p.to_string_lossy().to_string()));
                true
            },
        )
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(
            zip_names(&out),
            vec!["Plugin.esp", "README_FR.txt", "fomod/info.xml"]
        );
        assert_eq!(zip_read(&out, "fomod/info.xml"), b"<translated/>");
        assert_eq!(zip_read(&out, "Plugin.esp"), b"plugin");
        assert_eq!(zip_read(&out, "README_FR.txt"), b"lisez-moi");
        assert_eq!(seen.first(), Some(&(0, 3, "Plugin.esp".to_string())));
        assert_eq!(seen.last(), Some(&(3, 3, String::new())));

        // Whole folder: everything but junk, with the override applied.
        let out = dir.join("full.zip");
        let n = export_with_overrides(
            &root,
            &out,
            ArchiveFormat::Zip,
            &overrides,
            None,
            &mut |_, _, _| true,
        )
        .unwrap();
        assert_eq!(n, 5);
        assert_eq!(
            zip_names(&out),
            vec![
                "Plugin.esp",
                "README_FR.txt",
                "fomod/ModuleConfig.xml",
                "fomod/info.xml",
                "textures/a.dds"
            ]
        );
        assert_eq!(zip_read(&out, "fomod/info.xml"), b"<translated/>");
        assert_eq!(zip_read(&out, "fomod/ModuleConfig.xml"), b"<config/>");

        // The folder itself is untouched.
        assert_eq!(
            std::fs::read(root.join("fomod/info.xml")).unwrap(),
            b"<original/>"
        );
        assert!(!root.join("README_FR.txt").exists());

        // A missing file that nothing overrides is an error; cancelling
        // removes the partial archive.
        let out = dir.join("bad.zip");
        let missing = vec!["nope.esp".to_string()];
        assert!(
            export_with_overrides(
                &root,
                &out,
                ArchiveFormat::Zip,
                &overrides,
                Some(&missing),
                &mut |_, _, _| true
            )
            .is_err()
        );
        let escape = vec!["../outside.txt".to_string()];
        assert!(
            export_with_overrides(
                &root,
                &out,
                ArchiveFormat::Zip,
                &overrides,
                Some(&escape),
                &mut |_, _, _| true
            )
            .is_err()
        );
        let err = export_with_overrides(
            &root,
            &out,
            ArchiveFormat::Zip,
            &overrides,
            None,
            &mut |d, _, _| d == 0,
        )
        .unwrap_err();
        assert!(err.downcast_ref::<export::Cancelled>().is_some(), "{err}");
        assert!(!out.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_with_overrides_sevenzip() {
        let dir = overrides_root("7z");
        let root = dir.join("root");
        let mut overrides = HashMap::new();
        overrides.insert("fomod/info.xml".to_string(), b"<translated/>".to_vec());
        overrides.insert("README_FR.txt".to_string(), b"lisez-moi".to_vec());

        let only = vec!["fomod/info.xml".to_string(), "textures/a.dds".to_string()];
        let out = dir.join("only.7z");
        let n = export_with_overrides(
            &root,
            &out,
            ArchiveFormat::SevenZip,
            &overrides,
            Some(&only),
            &mut |_, _, _| true,
        )
        .unwrap();
        assert_eq!(n, 3);
        let back = dir.join("unpacked");
        sevenz_rust::decompress_file(&out, &back).expect("7z should be readable");
        assert_eq!(
            files_under(&back),
            vec!["README_FR.txt", "fomod/info.xml", "textures/a.dds"]
        );
        assert_eq!(
            std::fs::read(back.join("fomod/info.xml")).unwrap(),
            b"<translated/>"
        );
        assert_eq!(
            std::fs::read(back.join("textures/a.dds")).unwrap(),
            b"texture-bytes"
        );
        assert_eq!(
            std::fs::read(back.join("README_FR.txt")).unwrap(),
            b"lisez-moi"
        );

        let out = dir.join("full.7z");
        let n = export_with_overrides(
            &root,
            &out,
            ArchiveFormat::SevenZip,
            &overrides,
            None,
            &mut |_, _, _| true,
        )
        .unwrap();
        assert_eq!(n, 5);
        let back = dir.join("unpacked_full");
        sevenz_rust::decompress_file(&out, &back).expect("7z should be readable");
        assert_eq!(
            files_under(&back),
            vec![
                "Plugin.esp",
                "README_FR.txt",
                "fomod/ModuleConfig.xml",
                "fomod/info.xml",
                "textures/a.dds"
            ]
        );
        assert_eq!(
            std::fs::read(back.join("fomod/info.xml")).unwrap(),
            b"<translated/>"
        );
        assert_eq!(
            std::fs::read(root.join("fomod/info.xml")).unwrap(),
            b"<original/>"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crc32_matches_reference_vector() {
        // Standard CRC-32/IEEE check value for "123456789".
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
}
