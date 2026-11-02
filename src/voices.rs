//! The voice map (C-13): which responses of a plugin's replies have a voice
//! file, found without extracting anything — among the loose files under
//! `<Data>/Sound/Voice/<plugin>/` and in the archives of the plugin's folder
//! whose name starts with the plugin's (`MonMod.bsa`, `MonMod - Voices.bsa`,
//! `MonMod - Voices_en.ba2`…; the game's archives are not read, decision
//! D-27). A file is recognised by its suffix `_<FormID>_<n>.<ext>`: the low
//! 24 bits of the FormID match the reply (the load index in the name is the
//! author's), `n` the response number.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dialogue::VoiceSpec;
use crate::plugin::context::Family;

/// Where a voice file was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Where {
    /// A loose file, with its full path.
    Loose(PathBuf),
    /// Inside an archive (its file name) at an internal path.
    Archive(String),
}

/// One voice file of a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceFile {
    /// The file name.
    pub name: String,
    /// The internal path (archive) or the path under Data (loose).
    pub path: String,
    pub location: Where,
    /// The voice type (Skyrim, Fallout 4) or `race/sex` (Oblivion), from
    /// the parent folders.
    pub voice_type: String,
    pub ext: String,
}

impl VoiceFile {
    /// `.lip` alone: the lips, not the voice (R-C13-02).
    pub fn is_lip(&self) -> bool {
        self.ext.eq_ignore_ascii_case("lip")
    }
}

/// What a response has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// At least one voice file (several voice types are listed).
    Voiced(Vec<VoiceFile>),
    /// Only `.lip` files.
    LipOnly(Vec<VoiceFile>),
    Missing,
    /// Starfield: the files are named by hash (so many `.wem` found).
    Unresolved(usize),
}

impl Status {
    pub fn is_voiced(&self) -> bool {
        matches!(self, Status::Voiced(_))
    }
}

/// The map of a document.
#[derive(Debug, Clone, Default)]
pub struct VoiceMap {
    /// The plugin's file name (`MonMod.esp`).
    pub plugin: String,
    /// The sources read: the loose folder and the archives, by name.
    pub sources: Vec<String>,
    /// Files by (low 24 bits of the FormID, response number).
    by_key: HashMap<(u32, u8), Vec<VoiceFile>>,
    /// Files found (recognised or not).
    pub files: usize,
    /// Starfield (naming by hash): nothing is matched.
    pub unresolved: bool,
}

impl VoiceMap {
    /// Build the map of a plugin (R-C13-01).
    pub fn build(plugin: &Path, family: Family, spec: &VoiceSpec) -> VoiceMap {
        let name = plugin
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = plugin
            .file_stem()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mut map = VoiceMap {
            plugin: name.clone(),
            unresolved: spec.naming == "hash" || family == Family::Starfield,
            ..Default::default()
        };
        let exts: Vec<String> = spec.extensions.iter().map(|e| e.to_lowercase()).collect();
        let Some(dir) = plugin.parent() else {
            return map;
        };
        // Loose files: <Data>/Sound/Voice/<plugin>/…
        let root = spec_root(dir, spec);
        let loose = find_dir_ci(&root, &name);
        if let Some(loose) = loose {
            let mut found = Vec::new();
            walk(&loose, &mut found, 0);
            if !found.is_empty() {
                map.sources.push(format!("Sound/Voice/{name}"));
            }
            for p in found {
                let rel = p
                    .strip_prefix(&loose)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                map.take(&rel, Where::Loose(p.clone()), &exts, spec);
            }
        }
        // Archives of the plugin's folder named after it.
        let mut archives: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .filter(|p| {
                        let n = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        (n.ends_with(".bsa") || n.ends_with(".ba2")) && n.starts_with(&stem)
                    })
                    .collect()
            })
            .unwrap_or_default();
        archives.sort();
        for a in archives {
            let aname = a
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let Ok(paths) = crate::archive::Archive::list_paths(&a) else {
                continue;
            };
            let prefix = format!("{}/{}/", spec.root.to_lowercase(), name.to_lowercase());
            let mut any = false;
            for p in paths {
                let low = p.to_lowercase();
                if !low.starts_with(&prefix) {
                    continue;
                }
                any = true;
                let rel = &p[prefix.len()..];
                map.take(rel, Where::Archive(aname.clone()), &exts, spec);
            }
            if any {
                map.sources.push(aname);
            }
        }
        map
    }

    /// Record a file found at `rel` (under the plugin's voice folder).
    fn take(&mut self, rel: &str, location: Where, exts: &[String], spec: &VoiceSpec) {
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        let Some((stem, ext)) = name.rsplit_once('.') else {
            return;
        };
        let ext = ext.to_lowercase();
        if !exts.contains(&ext) {
            return;
        }
        self.files += 1;
        if self.unresolved {
            return;
        }
        let Some((formid, number)) = parse_suffix(stem) else {
            return;
        };
        let folders: Vec<&str> = rel.split('/').collect();
        let voice_type = match spec.folders.as_str() {
            "race-sex" if folders.len() >= 3 => {
                format!(
                    "{}/{}",
                    folders[folders.len() - 3],
                    folders[folders.len() - 2]
                )
            }
            _ if folders.len() >= 2 => folders[folders.len() - 2].to_string(),
            _ => String::new(),
        };
        self.by_key
            .entry((formid & 0x00FF_FFFF, number))
            .or_default()
            .push(VoiceFile {
                name,
                path: rel.to_string(),
                location,
                voice_type,
                ext,
            });
    }

    /// Whether anything was found at all (R-C13-04).
    pub fn has_sources(&self) -> bool {
        !self.sources.is_empty()
    }

    /// The status of a response (R-C13-02).
    pub fn status(&self, info: u32, number: u8) -> Status {
        if !self.has_sources() {
            return Status::Missing;
        }
        if self.unresolved {
            return Status::Unresolved(self.files);
        }
        match self.by_key.get(&(info & 0x00FF_FFFF, number)) {
            Some(files) => {
                let voiced: Vec<VoiceFile> =
                    files.iter().filter(|f| !f.is_lip()).cloned().collect();
                if voiced.is_empty() {
                    Status::LipOnly(files.clone())
                } else {
                    Status::Voiced(voiced)
                }
            }
            None => Status::Missing,
        }
    }

    /// Whether a response is voiced; `None` when nothing can be said (no
    /// source, or unresolved naming).
    pub fn voiced(&self, info: u32, number: u8) -> Option<bool> {
        if !self.has_sources() || self.unresolved {
            return None;
        }
        Some(self.status(info, number).is_voiced())
    }

    /// Recognised (FormID, number) pairs.
    pub fn matched(&self) -> usize {
        self.by_key.len()
    }
}

/// `…_0101A3F2_1` → (FormID, number).
pub fn parse_suffix(stem: &str) -> Option<(u32, u8)> {
    let (rest, num) = stem.rsplit_once('_')?;
    let number: u8 = num.parse().ok()?;
    let (_, hex) = rest.rsplit_once('_')?;
    if hex.len() != 8 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let formid = u32::from_str_radix(hex, 16).ok()?;
    Some((formid, number))
}

fn spec_root(data: &Path, spec: &VoiceSpec) -> PathBuf {
    let mut p = data.to_path_buf();
    for part in spec.root.split(['/', '\\']).filter(|s| !s.is_empty()) {
        p = find_dir_ci(&p, part).unwrap_or_else(|| p.join(part));
    }
    p
}

/// A child directory by name, case aside (Linux installs).
fn find_dir_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let direct = parent.join(name);
    if direct.is_dir() {
        return Some(direct);
    }
    let rd = std::fs::read_dir(parent).ok()?;
    rd.flatten().map(|e| e.path()).find(|p| {
        p.is_dir()
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, depth + 1);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// Open a loose file's folder in the system's file manager.
pub fn open_folder(file: &Path) {
    let Some(dir) = file.parent() else {
        return;
    };
    let s = dir.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&s).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&s).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&s).spawn();
    if let Err(e) = r {
        tracing::warn!("open folder: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> VoiceSpec {
        crate::dialogue::DialogueSpec::load(Family::Skyrim).voice
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ximod_voices_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn suffix_parses_formid_and_number() {
        assert_eq!(
            parse_suffix("MQ101_MQ101RalofTopic01_0101A3F2_1"),
            Some((0x0101A3F2, 1))
        );
        assert_eq!(
            parse_suffix("dialoguegen_dialoguegeneric_000A2C8E_12"),
            Some((0x000A2C8E, 12))
        );
        assert_eq!(parse_suffix("readme"), None);
        assert_eq!(parse_suffix("x_0101A3F_1"), None);
        assert_eq!(parse_suffix("x_0101A3F2_a"), None);
    }

    /// T-C13-01 / T-C13-02 / T-C13-04: loose files and an archive, a
    /// different load index, lips alone, a missing response, no source.
    #[test]
    fn maps_loose_and_archived_files() {
        let data = temp("skyrim");
        let plugin = data.join("MonMod.esp");
        std::fs::write(&plugin, b"TES4").unwrap();
        let male = data
            .join("Sound")
            .join("Voice")
            .join("MonMod.esp")
            .join("MaleNord");
        let female = data
            .join("Sound")
            .join("Voice")
            .join("MonMod.esp")
            .join("FemaleNord");
        std::fs::create_dir_all(&male).unwrap();
        std::fs::create_dir_all(&female).unwrap();
        std::fs::write(male.join("MQ101_T01_01000301_1.fuz"), b"x").unwrap();
        std::fs::write(female.join("MQ101_T01_01000301_1.fuz"), b"x").unwrap();
        // A different load index in the name: still matched (24 low bits).
        std::fs::write(male.join("MQ101_T01_05000302_1.fuz"), b"x").unwrap();
        // Lips only.
        std::fs::write(male.join("MQ101_T01_01000303_1.lip"), b"x").unwrap();
        std::fs::write(male.join("notes.txt"), b"x").unwrap();
        // An archive with the second response of 302.
        {
            use ba2::prelude::*;
            use ba2::tes4::{
                Archive as Tes4Archive, ArchiveKey, ArchiveOptions, Directory, DirectoryKey,
                File as Tes4File, Version,
            };
            let f = Tes4File::from_decompressed(b"fuz");
            let d: Directory = [(
                DirectoryKey::from(b"MQ101_T01_01000302_2.fuz".as_slice()),
                f,
            )]
            .into_iter()
            .collect();
            let a: Tes4Archive = [(
                ArchiveKey::from(b"sound\\voice\\monmod.esp\\malenord".as_slice()),
                d,
            )]
            .into_iter()
            .collect();
            let opts = ArchiveOptions::builder().version(Version::SSE).build();
            let mut bytes = Vec::new();
            a.write(&mut bytes, &opts).unwrap();
            std::fs::write(data.join("MonMod - Voices.bsa"), bytes).unwrap();
            // An archive of another mod is not read.
            std::fs::write(data.join("Other.bsa"), b"nope").unwrap();
        }
        let map = VoiceMap::build(&plugin, Family::Skyrim, &spec());
        assert_eq!(
            map.sources,
            vec![
                "Sound/Voice/MonMod.esp".to_string(),
                "MonMod - Voices.bsa".to_string()
            ]
        );
        assert_eq!(map.files, 5, "the .txt is not a voice file");
        match map.status(0x01000301, 1) {
            Status::Voiced(files) => {
                let mut types: Vec<&str> = files.iter().map(|f| f.voice_type.as_str()).collect();
                types.sort();
                assert_eq!(types, vec!["FemaleNord", "MaleNord"]);
                assert!(matches!(files[0].location, Where::Loose(_)));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            map.status(0x01000302, 1).is_voiced(),
            "load index 05 matched"
        );
        match map.status(0x01000302, 2) {
            Status::Voiced(files) => {
                assert_eq!(
                    files[0].location,
                    Where::Archive("MonMod - Voices.bsa".into())
                );
                assert_eq!(files[0].voice_type, "malenord");
                assert_eq!(
                    files[0].path, "malenord/mq101_t01_01000302_2.fuz",
                    "BSA names are lowercase"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(map.status(0x01000303, 1), Status::LipOnly(_)));
        assert_eq!(map.status(0x01000304, 1), Status::Missing);
        assert_eq!(map.voiced(0x01000304, 1), Some(false));
        assert_eq!(map.voiced(0x01000301, 1), Some(true));
        // No source at all: nothing is missing.
        let empty = temp("empty");
        let p2 = empty.join("Lonely.esp");
        std::fs::write(&p2, b"TES4").unwrap();
        std::fs::create_dir_all(empty.join("Sound").join("Voice").join("Lonely.esp")).unwrap();
        let m2 = VoiceMap::build(&p2, Family::Skyrim, &spec());
        assert!(!m2.has_sources());
        assert_eq!(m2.voiced(0x01000301, 1), None);
        assert_eq!(m2.status(0x01000301, 1), Status::Missing);
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// T-C13-03: Oblivion race / sex folders, Starfield unresolved.
    #[test]
    fn oblivion_folders_and_starfield_hashes() {
        let data = temp("obl");
        let plugin = data.join("MonMod.esp");
        std::fs::write(&plugin, b"TES4").unwrap();
        let d = data
            .join("Sound")
            .join("Voice")
            .join("MonMod.esp")
            .join("Imperial")
            .join("M");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("T_0001A3F2_1.mp3"), b"x").unwrap();
        let ospec = crate::dialogue::DialogueSpec::load(Family::Oblivion).voice;
        let map = VoiceMap::build(&plugin, Family::Oblivion, &ospec);
        match map.status(0x0301A3F2, 1) {
            Status::Voiced(f) => assert_eq!(f[0].voice_type, "Imperial/M"),
            other => panic!("{other:?}"),
        }
        let sf = temp("sf");
        let p = sf.join("MonMod.esm");
        std::fs::write(&p, b"TES4").unwrap();
        let d = sf.join("Sound").join("Voice").join("MonMod.esm").join("x");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("1234567890.wem"), b"x").unwrap();
        std::fs::write(d.join("1234567891.wem"), b"x").unwrap();
        let sspec = crate::dialogue::DialogueSpec::load(Family::Starfield).voice;
        let m = VoiceMap::build(&p, Family::Starfield, &sspec);
        assert_eq!(m.status(0x01000301, 1), Status::Unresolved(2));
        assert_eq!(m.voiced(0x01000301, 1), None);
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&sf);
    }
}
