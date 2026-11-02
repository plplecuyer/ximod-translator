//! Translating an existing FOMOD: file-level operations shared by the
//! graphical windows of Architect and Translator and by Architect's
//! `translate` command group: [`read_sources`], [`extract_translation`],
//! [`update_translation`], [`apply_translation`] (sibling or in place) and
//! [`package_translation`] (zip / 7z).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::archive::ArchiveFormat;
use crate::translate::{
    SourceFingerprint, TFile, TUnit, TranslationDoc, UpdateReport, merge_update,
    sanitize_file_stem, stale_keys, validate_doc, write_atomic,
};
use crate::xml::patch::{
    Encoding, FomodFiles, PatchOpts, apply_units, decode_xml_bytes, encode, extract_units,
    fingerprint, locate_fomod_files,
};

/// One source file loaded for translation.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    /// Decoded text (no byte order mark).
    pub xml: String,
    pub encoding: Encoding,
    /// Raw bytes as found on disk.
    pub bytes: Vec<u8>,
}

/// The two XML files of a FOMOD, as found on disk.
#[derive(Debug, Clone, Default)]
pub struct SourceFiles {
    /// The `fomod` directory.
    pub dir: PathBuf,
    pub info: Option<SourceFile>,
    pub config: Option<SourceFile>,
}

impl SourceFiles {
    /// Fingerprints of the files, for the sidecar.
    pub fn fingerprint(&self) -> SourceFingerprint {
        let of = |f: &Option<SourceFile>| f.as_ref().map(|f| fingerprint(&f.bytes, f.encoding));
        SourceFingerprint {
            info: of(&self.info),
            config: of(&self.config),
        }
    }

    /// Translatable units of the files.
    pub fn units(&self) -> Vec<TUnit> {
        extract_units(
            self.info.as_ref().map(|f| f.xml.as_str()),
            self.config.as_ref().map(|f| f.xml.as_str()),
        )
    }
}

fn read_source(path: &Path) -> Result<SourceFile> {
    let bytes =
        std::fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let (xml, encoding) = decode_xml_bytes(&bytes);
    Ok(SourceFile {
        path: path.to_path_buf(),
        xml,
        encoding,
        bytes,
    })
}

/// Read `info.xml` and `ModuleConfig.xml` of the FOMOD in `root`. Fails when
/// neither exists.
pub fn read_sources(root: &Path) -> Result<SourceFiles> {
    let FomodFiles { dir, info, config } = locate_fomod_files(root);
    if info.is_none() && config.is_none() {
        bail!(
            "no FOMOD found in {} (expected fomod/info.xml or fomod/ModuleConfig.xml)",
            root.display()
        );
    }
    Ok(SourceFiles {
        dir,
        info: info.as_deref().map(read_source).transpose()?,
        config: config.as_deref().map(read_source).transpose()?,
    })
}

/// Build a new translation document for the FOMOD in `root`.
pub fn extract_translation(
    root: &Path,
    source_lang: &str,
    target_lang: &str,
) -> Result<TranslationDoc> {
    let sources = read_sources(root)?;
    let mut doc = TranslationDoc::new(source_lang, target_lang);
    doc.units = sources.units();
    doc.source = sources.fingerprint();

    let find = |key: &str| {
        doc.units
            .iter()
            .find(|u| u.key == key)
            .map(|u| u.source.clone())
    };
    doc.mod_name = find("info/Name")
        .or_else(|| find("config/moduleName"))
        .unwrap_or_default();
    if let Some(info) = &sources.info {
        // Best effort: the version is informative only.
        let mut model = crate::model::Ximod::default();
        if crate::xml::parse_info_xml(&info.xml, &mut model).is_ok() {
            doc.mod_version = model.version;
        }
    }
    Ok(doc)
}

/// Re-extract the FOMOD in `root` and merge an existing translation into it.
pub fn update_translation(
    root: &Path,
    doc: &TranslationDoc,
) -> Result<(TranslationDoc, UpdateReport)> {
    let sources = read_sources(root)?;
    let (mut merged, report) = merge_update(doc, sources.units());
    merged.source = sources.fingerprint();
    Ok((merged, report))
}

/// Where [`apply_translation`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// `<root>/fomod_<lang>/`, leaving the original files untouched.
    Sibling,
    /// Over the original files, after a timestamped `.bak` copy.
    InPlace,
}

/// Options of [`apply_translation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOptions {
    pub mode: ApplyMode,
    pub force_explicit_order: bool,
    /// Proceed despite blocking issues or a changed source. Units whose
    /// source changed are skipped, never applied to another string.
    pub force: bool,
}

/// What [`apply_translation`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Files written.
    pub written: Vec<PathBuf>,
    /// Backups created (in-place mode).
    pub backups: Vec<PathBuf>,
    /// Number of translations handed to the patcher.
    pub applied: usize,
    /// Units skipped because the FOMOD changed since the last update.
    pub stale: usize,
}

/// Lower-cased language tag usable in a folder name.
fn lang_slug(lang: &str) -> String {
    let slug: String = lang
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if slug.is_empty() {
        "und".to_string()
    } else {
        slug
    }
}

/// Folder receiving the translated files in sibling mode: `<root>/fomod_<lang>`.
pub fn sibling_dir(root: &Path, target_lang: &str) -> PathBuf {
    root.join(format!("fomod_{}", lang_slug(target_lang)))
}

/// Write the translation `doc` into the FOMOD in `root`.
///
/// Refuses (unless `force`) when the document has blocking issues or when the
/// FOMOD no longer matches the sources the translation was made from — run an
/// update first in that case. The files are patched losslessly and written in
/// their original encoding.
pub fn apply_translation(
    root: &Path,
    doc: &TranslationDoc,
    opts: &ApplyOptions,
) -> Result<ApplyOutcome> {
    let sources = read_sources(root)?;

    let blocking = validate_doc(doc)
        .iter()
        .filter(|(_, i)| i.is_blocking())
        .count();
    if blocking > 0 && !opts.force {
        bail!("{blocking} blocking issue(s) in the translation; fix them or use --force");
    }

    let stale = stale_keys(doc, &sources.units());
    if !stale.is_empty() && !opts.force {
        bail!(
            "the FOMOD changed since this translation was made ({} string(s) no longer match); \
             run 'translate update' first, or use --force to skip them",
            stale.len()
        );
    }
    let mut targets: HashMap<String, String> = doc.export_targets();
    targets.retain(|key, _| !stale.contains(key));

    let patch_opts = PatchOpts {
        force_explicit_order: opts.force_explicit_order,
    };
    let mut jobs: Vec<(&SourceFile, String)> = Vec::new();
    for (file, kind) in [
        (&sources.info, TFile::Info),
        (&sources.config, TFile::Config),
    ] {
        if let Some(f) = file {
            let patched = apply_units(&f.xml, kind, &targets, &patch_opts)
                .with_context(|| format!("patching {}", f.path.display()))?;
            jobs.push((f, patched));
        }
    }

    let mut outcome = ApplyOutcome {
        applied: targets.len(),
        stale: stale.len(),
        ..Default::default()
    };
    match opts.mode {
        ApplyMode::Sibling => {
            let dir = sibling_dir(root, &doc.target_lang);
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            for (f, patched) in &jobs {
                let name = f.path.file_name().context("source file has no name")?;
                let out = dir.join(name);
                write_atomic(&out, &encode(patched, f.encoding))?;
                outcome.written.push(out);
            }
        }
        ApplyMode::InPlace => {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            for (f, patched) in &jobs {
                // An unchanged file is left alone: no backup, no rewrite.
                if *patched == f.xml {
                    continue;
                }
                let name = f
                    .path
                    .file_name()
                    .context("source file has no name")?
                    .to_string_lossy();
                let backup = f.path.with_file_name(format!("{name}.{stamp}.bak"));
                std::fs::write(&backup, &f.bytes)
                    .with_context(|| format!("writing the backup {}", backup.display()))?;
                outcome.backups.push(backup);
                write_atomic(&f.path, &encode(patched, f.encoding))?;
                outcome.written.push(f.path.clone());
            }
        }
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Packaging a translation
// ---------------------------------------------------------------------------

/// What a translation package contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackageKind {
    /// Only the translated `fomod` XML files and a README: a patch to install
    /// over the original mod.
    #[default]
    PatchOnly,
    /// The whole mod, with the translated XML files in place of the originals.
    Full,
}

/// Options of [`package_translation`].
#[derive(Debug, Clone)]
pub struct PackageOptions {
    pub kind: PackageKind,
    pub format: ArchiveFormat,
    /// Archive to write.
    pub out: PathBuf,
    pub force_explicit_order: bool,
    /// README text, already localised by the caller and stored as it is
    /// (UTF-8). `None`: the English default ([`default_readme`]).
    pub readme: Option<String>,
}

/// What [`package_translation`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageOutcome {
    /// The archive written.
    pub path: PathBuf,
    /// Number of files in the archive.
    pub files: usize,
    /// Number of translations handed to the patcher.
    pub applied: usize,
    /// Units skipped because the FOMOD changed since the last update.
    pub stale: usize,
}

/// Default README of a patch-only translation package, in English. `{name}`
/// stands for the mod name and `{langname}` for the target language;
/// [`default_readme`] fills them in.
pub const DEFAULT_README_EN: &str = "\
{name} - {langname} translation of the installer

This archive contains the translated installer files of {name}:
fomod/info.xml and fomod/ModuleConfig.xml. It does not contain the mod itself.

How to install: install the original mod first, then install this archive over
it and let it overwrite the two files (or let your mod manager merge it with
the original mod). The installer of the mod is then shown in {langname}.

Produced with XIMOD Architect.
";

/// Default README of a full translation package (the whole mod with its
/// installer translated), in English. Same placeholders as
/// [`DEFAULT_README_EN`].
pub const DEFAULT_README_FULL_EN: &str = "\
{name} - {langname} translation of the installer

This archive contains {name} with its installer files (fomod/info.xml and
fomod/ModuleConfig.xml) translated. The other files are those of the original
mod.

How to install: install this archive with your mod manager instead of the
original mod. The installer of the mod is then shown in {langname}.

Produced with XIMOD Architect.
";

/// Default template of a package name.
pub const DEFAULT_NAME_TEMPLATE: &str = "{name}_{LANG}";

/// ISO 639-1 code of a language tag, when the built-in table knows it
/// (`fra` → `fr`). `crate::lang::iso639_3_to_1` answers `en` for anything it
/// does not know, hence the check.
fn iso1_of(lang: &str) -> Option<String> {
    let lang = lang_slug(lang);
    let iso1 = crate::lang::iso639_3_to_1(&lang);
    (iso1 != "en" || lang == "eng" || lang == "en").then_some(iso1)
}

/// Upper-case language code used in file names: the two-letter code when the
/// language has one (`FR`), else the tag itself (`FRA`).
fn lang_code_upper(target_lang: &str) -> String {
    iso1_of(target_lang)
        .unwrap_or_else(|| lang_slug(target_lang))
        .to_uppercase()
}

/// English [`DEFAULT_README_EN`] filled in for `doc`.
pub fn default_readme(doc: &TranslationDoc) -> String {
    default_readme_for(doc, PackageKind::PatchOnly)
}

/// English default README of a package of the given kind, filled in for `doc`.
pub fn default_readme_for(doc: &TranslationDoc, kind: PackageKind) -> String {
    let name = if doc.mod_name.trim().is_empty() {
        "this mod"
    } else {
        doc.mod_name.trim()
    };
    let lang = crate::lang::locale_display_name(&lang_slug(&doc.target_lang));
    let lang = if iso1_of(&doc.target_lang).is_some() && !lang.is_empty() {
        lang.to_string()
    } else {
        lang_code_upper(&doc.target_lang)
    };
    let template = match kind {
        PackageKind::PatchOnly => DEFAULT_README_EN,
        PackageKind::Full => DEFAULT_README_FULL_EN,
    };
    template
        .replace("{name}", name)
        .replace("{langname}", &lang)
}

/// Name of the README stored in a translation package: `README_<LANG>.txt`.
pub fn readme_file_name(doc: &TranslationDoc) -> String {
    format!("README_{}.txt", lang_code_upper(&doc.target_lang))
}

/// File name of a translation package, from a template.
///
/// Tokens: `{name}` (mod name, `fomod` when empty), `{version}` (mod version),
/// `{LANG}` (upper-case two-letter code of the target language when it has
/// one, else the upper-case three-letter tag), `{lang}` (the same in lower
/// case), `{lang3}` (the target language tag of the document) and
/// `{langname}` (`lang_display`, the name of the language as the caller wants
/// it shown). Unknown tokens are left as they are. The result is made safe
/// for a file name and gets `.ext` appended (nothing when `ext` is empty). An
/// empty template means [`DEFAULT_NAME_TEMPLATE`].
pub fn package_name(doc: &TranslationDoc, template: &str, lang_display: &str, ext: &str) -> String {
    let template = if template.trim().is_empty() {
        DEFAULT_NAME_TEMPLATE
    } else {
        template.trim()
    };

    let mut name = sanitize_file_stem(&doc.mod_name);
    if name.is_empty() {
        name = "fomod".to_string();
    }
    let version = sanitize_file_stem(&doc.mod_version);
    let lang3 = lang_slug(&doc.target_lang);
    let lang_lower = iso1_of(&doc.target_lang).unwrap_or_else(|| lang3.clone());
    let lang_upper = lang_lower.to_uppercase();
    let lang_name = sanitize_file_stem(lang_display);

    let mut values: HashMap<&str, &str> = HashMap::new();
    values.insert("name", &name);
    values.insert("version", &version);
    values.insert("LANG", &lang_upper);
    values.insert("lang", &lang_lower);
    values.insert("lang3", &lang3);
    values.insert("langname", &lang_name);

    // Single pass, so that a value containing braces is never expanded again.
    let mut stem = String::with_capacity(template.len() + name.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        stem.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').map(|close| (&after[..close], close)) {
            Some((token, close)) if values.contains_key(token) => {
                stem.push_str(values[token]);
                rest = &after[close + 1..];
            }
            _ => {
                stem.push('{');
                rest = after;
            }
        }
    }
    stem.push_str(rest);

    let mut stem = sanitize_file_stem(&stem);
    if stem.is_empty() {
        stem = "fomod".to_string();
    }
    let ext = ext.trim().trim_start_matches('.');
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Default location of a translation package: next to the mod folder (in the
/// current directory when `root` has no parent), named after `template`.
pub fn default_package_path(
    root: &Path,
    doc: &TranslationDoc,
    template: &str,
    format: ArchiveFormat,
) -> PathBuf {
    // `canonicalize` so that a root given as "." still has a parent.
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let dir = root
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join(package_name(
        doc,
        template,
        &lang_slug(&doc.target_lang),
        format.extension(),
    ))
}

/// Path of `file` inside an archive of `root`: its real relative path (case
/// preserved), with `/` separators.
fn archive_rel_path(root: &Path, files_dir: &Path, file: &Path) -> Result<String> {
    let rel = match file.strip_prefix(root) {
        Ok(rel) => rel.to_path_buf(),
        // Cannot happen with `locate_fomod_files`; rebuild `<fomod>/<file>`.
        Err(_) => {
            let dir = files_dir
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("fomod"));
            dir.join(file.file_name().context("source file has no name")?)
        }
    };
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Build an archive holding the translation `doc` of the FOMOD in `root`,
/// without modifying anything in `root`.
///
/// * [`PackageKind::PatchOnly`]: the translated `info.xml` and/or
///   `ModuleConfig.xml`, at their real relative path (`fomod/info.xml`,
///   `Fomod/Info.xml`…), plus `README_<LANG>.txt`.
/// * [`PackageKind::Full`]: every distribution file of the mod, with the two
///   XML files replaced by their translation, plus the README.
///
/// The XML files are patched losslessly and stored in their original
/// encoding. Blocking issues are refused. Units whose source changed since
/// the translation was made are skipped (never written over another string)
/// and counted in [`PackageOutcome::stale`].
pub fn package_translation(
    root: &Path,
    doc: &TranslationDoc,
    opts: &PackageOptions,
    progress: crate::export::Progress<'_>,
) -> Result<PackageOutcome> {
    package_translation_excluding(root, doc, opts, &[], progress)
}

/// [`package_translation`], leaving the files `exclude` out of a full
/// package (archives built earlier inside `root`, for instance).
pub(crate) fn package_translation_excluding(
    root: &Path,
    doc: &TranslationDoc,
    opts: &PackageOptions,
    exclude: &[PathBuf],
    progress: crate::export::Progress<'_>,
) -> Result<PackageOutcome> {
    let sources = read_sources(root)?;

    let blocking = validate_doc(doc)
        .iter()
        .filter(|(_, i)| i.is_blocking())
        .count();
    if blocking > 0 {
        bail!("{blocking} blocking issue(s) in the translation; fix them before packaging");
    }

    let stale = stale_keys(doc, &sources.units());
    let mut targets: HashMap<String, String> = doc.export_targets();
    targets.retain(|key, _| !stale.contains(key));

    let patch_opts = PatchOpts {
        force_explicit_order: opts.force_explicit_order,
    };
    let mut overrides: HashMap<String, Vec<u8>> = HashMap::new();
    for (file, kind) in [
        (&sources.info, TFile::Info),
        (&sources.config, TFile::Config),
    ] {
        if let Some(f) = file {
            let patched = apply_units(&f.xml, kind, &targets, &patch_opts)
                .with_context(|| format!("patching {}", f.path.display()))?;
            let rel = archive_rel_path(root, &sources.dir, &f.path)?;
            overrides.insert(rel, encode(&patched, f.encoding));
        }
    }
    let readme = opts
        .readme
        .clone()
        .unwrap_or_else(|| default_readme_for(doc, opts.kind));
    overrides.insert(readme_file_name(doc), readme.into_bytes());

    if let Some(parent) = opts.out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let only: Option<Vec<String>> = match opts.kind {
        PackageKind::PatchOnly => Some(overrides.keys().cloned().collect()),
        PackageKind::Full if exclude.is_empty() => None,
        PackageKind::Full => {
            let skip: Vec<PathBuf> = exclude
                .iter()
                .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
                .collect();
            let files = crate::export::collect_distribution_files(root, &opts.out)?;
            Some(
                files
                    .into_iter()
                    .filter(|rel| {
                        let abs = root.join(rel);
                        !skip.contains(&abs.canonicalize().unwrap_or(abs))
                    })
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                    .collect(),
            )
        }
    };
    let files = crate::archive::export_with_overrides(
        root,
        &opts.out,
        opts.format,
        &overrides,
        only.as_deref(),
        progress,
    )?;

    Ok(PackageOutcome {
        path: opts.out.clone(),
        files,
        applied: targets.len(),
        stale: stale.len(),
    })
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------
