//! Packaging a translation for a mod manager (M-03): from the files a
//! translator exported (patch plugin, STRINGS, DSD, PAK…), build the folder
//! layout Mod Organizer 2 and Vortex expect for the game, with an optional
//! `fomod/info.xml`, and zip it.
//!
//! The layout follows the game family (manual § 19): Creation Engine games
//! install relative to `Data` (plugins at the root, `Strings/` for STRINGS,
//! `SKSE/Plugins/DynamicStringDistributor/<plugin>/` for DSD); Morrowind
//! relative to `Data Files`; Oblivion Remastered puts PAK / UCAS / UTOC under
//! `OblivionRemastered/Content/Paks/~mods/`.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ximod_games::Game;

/// What the user asks for.
#[derive(Debug, Clone)]
pub struct PackageSpec {
    pub game: Game,
    /// Mod name as published (« Riverwood Home »).
    pub mod_name: String,
    pub version: String,
    /// The translator (author of the package).
    pub author: String,
    /// Target language (ISO 639-3) — used in the folder name and info.xml.
    pub lang: String,
    pub website: String,
    /// Files to package (as exported by XIMOD Translator).
    pub files: Vec<PathBuf>,
    /// For DSD files: the plugin they apply to (« MonMod.esp »).
    pub dsd_plugin: String,
    pub out_dir: PathBuf,
    pub with_fomod_info: bool,
}

/// One planned copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub source: PathBuf,
    /// Path inside the package, with `/` separators.
    pub dest: String,
}

/// The plan: copies plus the notes a translator should read before publishing.
#[derive(Debug, Default)]
pub struct Plan {
    pub copies: Vec<Planned>,
    /// i18n keys of the notes that apply (`pack-note-…`).
    pub notes: Vec<&'static str>,
    /// Files whose place is unknown for the game (left out, reported).
    pub unknown: Vec<PathBuf>,
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("_")
        .to_string()
}

/// Where each file goes for the game.
pub fn plan(spec: &PackageSpec) -> Plan {
    let mut plan = Plan::default();
    let strings_games = matches!(
        spec.game,
        Game::Skyrim | Game::SkyrimSE | Game::Fallout4 | Game::Fallout76 | Game::Starfield
    );
    let mut note = |k: &'static str| {
        if !plan.notes.contains(&k) {
            plan.notes.push(k);
        }
    };
    let mut copies = Vec::new();
    let mut unknown = Vec::new();
    for f in &spec.files {
        let ext = ext_of(f);
        let name = name_of(f);
        let dest = match (spec.game, ext.as_str()) {
            // The manifest of the translation (D-06, R-D06-04).
            _ if name.eq_ignore_ascii_case(crate::manifest::FILE_NAME) => {
                note("pack-note-manifest");
                format!("fomod/{}", crate::manifest::FILE_NAME)
            }
            (Game::OblivionRD, "pak" | "ucas" | "utoc") => {
                note("pack-note-or-mods");
                format!("OblivionRemastered/Content/Paks/~mods/{name}")
            }
            (Game::OblivionRD, "esp" | "esm") => {
                note("pack-note-or-plugin");
                format!("OblivionRemastered/Content/Dev/ObvData/Data/{name}")
            }
            (_, "esp" | "esm" | "esl") => {
                note("pack-note-plugin-override");
                name.clone()
            }
            (_, "strings" | "dlstrings" | "ilstrings") if strings_games => {
                if spec.game == Game::Starfield {
                    note("pack-note-starfield-loose");
                }
                note("pack-note-strings");
                format!("Strings/{name}")
            }
            (Game::SkyrimSE, "json") => {
                note("pack-note-dsd");
                let plugin = if spec.dsd_plugin.trim().is_empty() {
                    "MonMod.esp".to_string()
                } else {
                    spec.dsd_plugin.trim().to_string()
                };
                format!("SKSE/Plugins/DynamicStringDistributor/{plugin}/{name}")
            }
            (_, "ba2" | "bsa") => {
                note("pack-note-archive");
                name.clone()
            }
            (Game::Morrowind, _) => {
                note("pack-note-morrowind");
                name.clone()
            }
            _ => {
                unknown.push(f.clone());
                continue;
            }
        };
        copies.push(Planned {
            source: f.clone(),
            dest,
        });
    }
    plan.copies = copies;
    plan.unknown = unknown;
    plan
}

/// Folder and archive names: « <Mod> - Traduction <LANG> ».
pub fn package_name(spec: &PackageSpec) -> String {
    let base = spec.mod_name.trim();
    let base = if base.is_empty() { "Traduction" } else { base };
    let safe: String = base
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let lang = spec.lang.trim().to_uppercase();
    let version = spec.version.trim();
    let mut n = if lang.is_empty() {
        safe
    } else {
        format!("{safe} - {lang}")
    };
    if !version.is_empty() {
        n.push_str(&format!(" - {version}"));
    }
    n
}

fn xml_escape(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}

/// The `fomod/info.xml` of the package.
pub fn info_xml(spec: &PackageSpec, description: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<fomod>\n  <Name>{}</Name>\n  <Author>{}</Author>\n  <Version>{}</Version>\n  <Website>{}</Website>\n  <Description>{}</Description>\n</fomod>\n",
        xml_escape(&package_name(spec)),
        xml_escape(spec.author.trim()),
        xml_escape(spec.version.trim()),
        xml_escape(spec.website.trim()),
        xml_escape(description),
    )
}

/// Build the package folder and its zip; returns `(folder, zip)`.
pub fn build(spec: &PackageSpec, description: &str) -> Result<(PathBuf, PathBuf)> {
    let p = plan(spec);
    if p.copies.is_empty() {
        bail!(crate::l10n!("err-pack-empty"));
    }
    let name = package_name(spec);
    let folder = spec.out_dir.join(&name);
    if folder.exists() {
        std::fs::remove_dir_all(&folder)
            .with_context(|| crate::l10n!("err-pack-clean", path = folder.display()))?;
    }
    std::fs::create_dir_all(&folder)?;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for c in &p.copies {
        let data = std::fs::read(&c.source)
            .with_context(|| crate::l10n!("err-pack-read", path = c.source.display()))?;
        let dest = folder.join(c.dest.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &data)?;
        entries.push((c.dest.clone(), data));
    }
    if spec.with_fomod_info {
        let xml = info_xml(spec, description);
        std::fs::create_dir_all(folder.join("fomod"))?;
        std::fs::write(folder.join("fomod").join("info.xml"), xml.as_bytes())?;
        entries.push(("fomod/info.xml".into(), xml.into_bytes()));
    }
    let zip_path = spec.out_dir.join(format!("{name}.zip"));
    let file = std::fs::File::create(&zip_path)
        .with_context(|| crate::l10n!("err-pack-create", path = zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (dest, data) in &entries {
        zip.start_file(dest, opts)?;
        zip.write_all(data)?;
    }
    zip.finish()?;
    Ok((folder, zip_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(game: Game, files: &[&str]) -> PackageSpec {
        PackageSpec {
            game,
            mod_name: "Riverwood Home".into(),
            version: "1.2".into(),
            author: "Patrice".into(),
            lang: "fra".into(),
            website: String::new(),
            files: files.iter().map(PathBuf::from).collect(),
            dsd_plugin: "RiverwoodHome.esp".into(),
            out_dir: std::env::temp_dir(),
            with_fomod_info: true,
        }
    }

    #[test]
    fn layouts_follow_the_game_family() {
        let p = plan(&spec(
            Game::SkyrimSE,
            &[
                "/x/RiverwoodHome_FR.esp",
                "/x/RiverwoodHome_french.strings",
                "/x/RiverwoodHome_fra.json",
                "/x/readme.txt",
                "/x/manifest.json",
            ],
        ));
        let d: Vec<&str> = p.copies.iter().map(|c| c.dest.as_str()).collect();
        assert_eq!(
            d,
            vec![
                "RiverwoodHome_FR.esp",
                "Strings/RiverwoodHome_french.strings",
                "SKSE/Plugins/DynamicStringDistributor/RiverwoodHome.esp/RiverwoodHome_fra.json",
                "fomod/manifest.json"
            ]
        );
        assert!(p.notes.contains(&"pack-note-manifest"));
        assert_eq!(p.unknown.len(), 1);
        assert!(p.notes.contains(&"pack-note-dsd") && p.notes.contains(&"pack-note-strings"));
        let p = plan(&spec(
            Game::OblivionRD,
            &["/x/Trad_FR_P.pak", "/x/Trad_FR_P.ucas", "/x/Mod.esp"],
        ));
        let d: Vec<&str> = p.copies.iter().map(|c| c.dest.as_str()).collect();
        assert_eq!(d[0], "OblivionRemastered/Content/Paks/~mods/Trad_FR_P.pak");
        assert_eq!(d[2], "OblivionRemastered/Content/Dev/ObvData/Data/Mod.esp");
        let p = plan(&spec(
            Game::Oblivion,
            &["/x/Mod_FR.esp", "/x/Mod_french.strings"],
        ));
        assert_eq!(p.copies.len(), 1, "no STRINGS for Oblivion");
        assert_eq!(p.unknown.len(), 1);
        let p = plan(&spec(Game::Starfield, &["/x/Mod_fr.strings"]));
        assert!(p.notes.contains(&"pack-note-starfield-loose"));
        assert_eq!(
            package_name(&spec(Game::SkyrimSE, &[])),
            "Riverwood Home - FRA - 1.2"
        );
    }

    #[test]
    fn builds_folder_and_zip() {
        let dir = std::env::temp_dir().join(format!("ximod-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Mod_FR.esp"), b"TES4").unwrap();
        std::fs::write(dir.join("Mod_french.strings"), b"STR").unwrap();
        let mut s = spec(Game::SkyrimSE, &[]);
        s.files = vec![dir.join("Mod_FR.esp"), dir.join("Mod_french.strings")];
        s.out_dir = dir.clone();
        let (folder, zip_path) = build(&s, "Traduction française").unwrap();
        assert!(folder.join("Mod_FR.esp").is_file());
        assert!(folder.join("Strings").join("Mod_french.strings").is_file());
        let info = std::fs::read_to_string(folder.join("fomod").join("info.xml")).unwrap();
        assert!(
            info.contains("<Name>Riverwood Home - FRA - 1.2</Name>")
                && info.contains("<Author>Patrice</Author>")
        );
        let z = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.name_for_index(i).unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["Mod_FR.esp", "Strings/Mod_french.strings", "fomod/info.xml"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
