//! The manifest of a translation (D-06, R-D06-04, decision D-93): what was
//! read (the source plugin with its size and CRC-32 — the version of the
//! mod the translation was made for, B-12), the mod's dictionary, the
//! stack, the statuses exported, what was produced and the checks. It goes
//! in the package (`fomod/manifest.json`, M-03) and to the continuous
//! integration: two manifests compared say what changed.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file format of the manifest.
pub const FORMAT: u32 = 1;
/// Its name in an output folder and in the package.
pub const FILE_NAME: &str = "manifest.json";

/// A file read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub path: String,
    pub size: u64,
    pub crc32: String,
}

/// The mod's dictionary and its statuses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dictionary {
    pub path: String,
    pub lines: usize,
    pub validated: usize,
    pub pending: usize,
    pub ignored: usize,
}

/// One level of the stack and the lines it filled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    pub role: String,
    pub file: String,
    pub filled: usize,
}

/// A file produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub crc32: String,
    /// The lines of the files the application reads (plugin, STRINGS,
    /// DSD, dictionary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub app: String,
    pub date: String,
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub inputs: Vec<Input>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionary: Option<Dictionary>,
    #[serde(default)]
    pub stack: Vec<Level>,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<Output>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Checks>,
}

impl Manifest {
    /// An empty manifest dated now.
    pub fn new(game: &str, from: &str, to: &str) -> Manifest {
        Manifest {
            format: FORMAT,
            app: format!("{} {}", crate::APP_NAME, crate::APP_VERSION),
            date: crate::history::iso(crate::backup::now_secs()),
            game: game.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            inputs: Vec::new(),
            dictionary: None,
            stack: Vec::new(),
            statuses: Vec::new(),
            outputs: Vec::new(),
            checks: None,
        }
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_default();
        s.push('\n');
        s
    }

    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if m.format != FORMAT {
            return Err(crate::l10n!("err-manifest-format", format = m.format));
        }
        Ok(m)
    }

    /// The CRC-32 of the first input (the source plugin).
    pub fn source_crc(&self) -> Option<&str> {
        self.inputs.first().map(|i| i.crc32.as_str())
    }

    /// Lines of all the outputs.
    pub fn output_lines(&self) -> usize {
        self.outputs.iter().filter_map(|o| o.lines).sum()
    }
}

/// `8` uppercase hexadecimal digits.
pub fn crc_text(crc: u32) -> String {
    format!("{crc:08X}")
}

/// Size and CRC-32 of a file.
pub fn size_crc(path: &Path) -> std::io::Result<(u64, u32)> {
    let mut f = std::fs::File::open(path)?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        h.update(&buf[..n]);
    }
    Ok((size, h.finalize()))
}

/// `path` relative to `base` when under it, with `/`.
pub fn rel(path: &Path, base: Option<&Path>) -> String {
    let p = base.and_then(|b| path.strip_prefix(b).ok()).unwrap_or(path);
    p.to_string_lossy().replace('\\', "/")
}

pub fn input(path: &Path, base: Option<&Path>) -> std::io::Result<Input> {
    let (size, crc) = size_crc(path)?;
    Ok(Input {
        path: rel(path, base),
        size,
        crc32: crc_text(crc),
    })
}

/// The kind of an output by its name.
pub fn kind_of(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if crate::games::registry().is_plugin_extension(&ext) {
        return "plugin";
    }
    match ext.as_str() {
        "strings" => "strings",
        "dlstrings" => "dlstrings",
        "ilstrings" => "ilstrings",
        "json" => "dsd",
        "ximdb" => "dictionary",
        "zip" | "7z" => "package",
        "bsa" | "ba2" => "archive",
        "pak" | "ucas" | "utoc" => "pak",
        _ => "other",
    }
}

/// The lines of a file the application reads.
pub fn lines_of(path: &Path, kind: &str) -> Option<usize> {
    match kind {
        "plugin" => crate::plugin::read_file(path)
            .ok()
            .map(|tm| tm.entries.iter().filter(|e| !e.unused).count()),
        "strings" | "dlstrings" | "ilstrings" | "dsd" => {
            crate::tm::load(path).ok().map(|tm| tm.entries.len())
        }
        "dictionary" => crate::basedb::ximdb_to_tm(path)
            .ok()
            .map(|tm| tm.entries.len()),
        _ => None,
    }
}

pub fn output(path: &Path, base: Option<&Path>) -> std::io::Result<Output> {
    let (size, crc) = size_crc(path)?;
    let kind = kind_of(path);
    Ok(Output {
        path: rel(path, base),
        kind: kind.to_string(),
        size,
        crc32: crc_text(crc),
        lines: lines_of(path, kind),
    })
}

/// The files of an output folder, in the order of their paths (the
/// manifest itself left out).
pub fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.file_name().and_then(|n| n.to_str()) != Some(FILE_NAME) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The manifest of an output folder: its files described.
pub fn of_dir(dir: &Path, game: &str, from: &str, to: &str) -> Manifest {
    let mut m = Manifest::new(game, from, to);
    for p in files_in(dir) {
        if let Ok(o) = output(&p, Some(dir)) {
            m.outputs.push(o);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_described_and_read_back() {
        let dir = std::env::temp_dir().join(format!("ximod-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("sub").join("b.zip"), b"PK").unwrap();
        std::fs::write(dir.join(FILE_NAME), b"{}").unwrap();
        let m = of_dir(&dir, "SkyrimSE", "eng", "fra");
        assert_eq!(m.outputs.len(), 2);
        assert_eq!(m.outputs[0].path, "a.txt");
        assert_eq!(m.outputs[0].crc32, crc_text(crc32fast::hash(b"hello")));
        assert_eq!(m.outputs[1].kind, "package");
        let back = Manifest::parse(&m.to_json()).unwrap();
        assert_eq!(back, m);
        let mut bad = m.clone();
        bad.format = 2;
        assert!(Manifest::parse(&serde_json::to_string(&bad).unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
