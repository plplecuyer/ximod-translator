//! Keyword resolution across the load order (minimal).
//!
//! A weapon's function (pistol, rifle, sword…) is encoded as `KWDA` keyword
//! references, and the keyword records (`KYWD`, e.g. `WeaponTypePistol`) live in
//! the **base game master** (`Starfield.esm`, `Fallout4.esm`, `Skyrim.esm`…),
//! not in the mod. This parses that base master once — memory-mapped, decoding
//! only `KYWD` records — to map a keyword FormID (as referenced from the loaded
//! plugin) to its EditorID.
//!
//! Scope: resolves keywords defined by the **base master** (the common case for
//! vanilla weapon-type keywords). Keywords defined by a mod's own master chain or
//! by ESL/medium masters are left unresolved (category falls back to "non classé").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use super::games::Game;
use super::reader::{decompress_record, parse_subrecords};
use super::record::FLAG_COMPRESSED;

const HEADER_LEN_MODERN: usize = 24;
const HEADER_LEN_OBLIVION: usize = 20;

fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn detect_header_len(data: &[u8]) -> usize {
    if data.len() >= 24 && &data[20..24] == b"HEDR" {
        HEADER_LEN_OBLIVION
    } else {
        HEADER_LEN_MODERN
    }
}

/// The base game master file name for a game (lowercased for matching).
fn base_master_name(game: Game) -> Option<&'static str> {
    Some(match game {
        Game::Skyrim | Game::SkyrimSE => "skyrim.esm",
        Game::Fallout3 => "fallout3.esm",
        Game::FalloutNV => "falloutnv.esm",
        Game::Fallout4 => "fallout4.esm",
        Game::Fallout76 => "seventysix.esm",
        Game::Starfield => "starfield.esm",
        Game::Oblivion => "oblivion.esm",
        _ => return None,
    })
}

/// Keyword FormID → EditorID, keyed in the **loaded plugin's** FormID space. It
/// merges keywords defined by the loaded plugin itself (e.g. when the loaded file
/// *is* Starfield.esm) with those from its base master (for mods).
#[derive(Default)]
pub struct KeywordMap {
    by_formid: HashMap<u32, String>,
    /// Name of the base master that was parsed, if any (None when the loaded
    /// plugin is itself a base master, or the master was not found).
    pub base_master: Option<String>,
}

impl KeywordMap {
    /// Resolve a keyword FormID (as stored in the loaded plugin) to its EditorID.
    pub fn resolve(&self, formid: u32) -> Option<&str> {
        self.by_formid.get(&formid).map(String::as_str)
    }
    pub fn is_empty(&self) -> bool {
        self.by_formid.is_empty()
    }
    pub fn len(&self) -> usize {
        self.by_formid.len()
    }
    /// Add a keyword defined by the loaded plugin itself (FormID already in the
    /// plugin's space). Plugin-own definitions take priority over the master's.
    pub fn insert(&mut self, formid: u32, edid: String) {
        self.by_formid.insert(formid, edid);
    }
}

/// Case-insensitively find a file named `name` inside `dir`.
fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let target = name.to_lowercase();
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        let fname = p.file_name()?.to_string_lossy().to_lowercase();
        (fname == target).then_some(p)
    })
}

/// Build the keyword map for a loaded plugin by parsing its base master (found in
/// `data_dir`). Returns an empty map when the base master is unknown or absent.
pub fn load(data_dir: &Path, masters: &[String], game: Game) -> KeywordMap {
    let mut km = KeywordMap::default();
    let Some(base) = base_master_name(game) else {
        return km;
    };
    let Some(idx) = masters.iter().position(|m| m.to_lowercase() == base) else {
        return km;
    };
    let Some(path) = find_ci(data_dir, &masters[idx]) else {
        return km;
    };
    let Ok(file) = std::fs::File::open(&path) else {
        return km;
    };
    // SAFETY: read-only mapping, valid for the duration of the walk.
    let Ok(mmap) = (unsafe { Mmap::map(&file) }) else {
        return km;
    };
    if mmap.len() < 16 || &mmap[0..4] != b"TES4" {
        return km;
    }
    let header_len = detect_header_len(&mmap);
    // Keywords defined by the base master are referenced from the loaded plugin
    // with the master's load-order index in the high byte of the FormID.
    walk(
        &mmap[..],
        0,
        mmap.len(),
        header_len,
        0,
        idx as u8,
        &mut km.by_formid,
    );
    km.base_master = Some(masters[idx].clone());
    km
}

/// Header-only walk decoding just `KYWD` records' EditorID. `base_index` is the
/// load-order index of the parsed file as seen from the loaded plugin; each
/// keyword is stored under `(base_index << 24) | (formid & 0x00FF_FFFF)` so it
/// resolves against FormIDs as they appear in the loaded plugin.
#[allow(clippy::too_many_arguments)]
fn walk(
    data: &[u8],
    base: usize,
    len: usize,
    header_len: usize,
    depth: u32,
    base_index: u8,
    out: &mut HashMap<u32, String>,
) {
    if depth > 32 {
        return;
    }
    let end = base + len;
    let mut p = base;
    while p + header_len <= end {
        let is_grup = &data[p..p + 4] == b"GRUP";
        let size = read_u32(data, p + 4) as usize;
        if is_grup {
            if size < header_len || p + size > end {
                break;
            }
            walk(
                data,
                p + header_len,
                size - header_len,
                header_len,
                depth + 1,
                base_index,
                out,
            );
            p += size;
        } else {
            let data_start = p + header_len;
            let data_end = data_start + size;
            if data_end > data.len() {
                break;
            }
            if &data[p..p + 4] == b"KYWD" {
                let flags = read_u32(data, p + 8);
                let formid = read_u32(data, p + 12);
                let raw = &data[data_start..data_end];
                let subs = if flags & FLAG_COMPRESSED != 0 {
                    parse_subrecords(&decompress_record(raw).unwrap_or_default())
                } else {
                    parse_subrecords(raw)
                };
                if let Some(edid) = subs.iter().find(|s| s.sig == "EDID") {
                    let key = ((base_index as u32) << 24) | (formid & 0x00FF_FFFF);
                    out.insert(key, edid.as_zstring());
                }
            }
            p += header_len + size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subrec(sig: &str, data: &[u8]) -> Vec<u8> {
        let mut v = sig.as_bytes().to_vec();
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(data);
        v
    }
    fn record(sig: &str, formid: u32, subs: &[u8]) -> Vec<u8> {
        let mut v = sig.as_bytes().to_vec();
        v.extend_from_slice(&(subs.len() as u32).to_le_bytes()); // dataSize
        v.extend_from_slice(&0u32.to_le_bytes()); // flags
        v.extend_from_slice(&formid.to_le_bytes());
        v.extend_from_slice(&[0u8; 8]); // timestamp / vc / version / unknown → 24-byte header
        v.extend_from_slice(subs);
        v
    }
    fn grup(label: &str, body: &[u8]) -> Vec<u8> {
        let total = (24 + body.len()) as u32;
        let mut v = b"GRUP".to_vec();
        v.extend_from_slice(&total.to_le_bytes());
        v.extend_from_slice(label.as_bytes());
        v.extend_from_slice(&0i32.to_le_bytes()); // top group
        v.extend_from_slice(&[0u8; 8]);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn resolves_keyword_from_base_master() {
        // Minimal TES4 master: header (HEDR at offset 24) + a KYWD in a top GRUP.
        let hedr = subrec("HEDR", &[0, 0, 0x70, 0x41, 1, 0, 0, 0, 0, 0, 0, 0]); // f32 + counts
        let tes4 = record("TES4", 0, &hedr);
        let mut edid = b"WeaponTypePistol".to_vec();
        edid.push(0);
        let kywd = record("KYWD", 0x0000_1234, &subrec("EDID", &edid));
        let mut bytes = tes4;
        bytes.extend_from_slice(&grup("KYWD", &kywd));

        // Write it as "Skyrim.esm" in a temp dir.
        let dir = std::env::temp_dir().join(format!("esmx_kw_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Skyrim.esm");
        std::fs::write(&path, &bytes).unwrap();

        let km = load(&dir, &["Skyrim.esm".to_string()], Game::SkyrimSE);
        assert_eq!(km.base_master.as_deref(), Some("Skyrim.esm"));
        assert_eq!(km.len(), 1);
        // A weapon in the plugin references the keyword by the master's index (0).
        assert_eq!(km.resolve(0x0000_1234), Some("WeaponTypePistol"));
        // A FormID from a different master index does not resolve.
        assert_eq!(km.resolve(0x0100_1234), None);

        std::fs::remove_dir_all(&dir).ok();
    }
}
