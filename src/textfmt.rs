//! Interface files and text formats (C-05): SkyUI MCM translations
//! (`Interface/Translations/<mod>_<langue>.txt`, UTF-16LE, « $Clé ⇥ texte »),
//! the game's `translate_<langue>.txt`, INI, flat JSON, CSV — any key /
//! value file a regular expression can describe. A file is opened as a
//! document (the lines the expression recognises become the grid's lines,
//! the others are kept invisible at their place) and written back in the
//! target language with its encoding, its line endings and its foreign
//! lines untouched: only the recognised values change.
//!
//! The formats live in `data/text_formats.json` of the configuration folder,
//! seeded from `assets/data/text_formats.json`; the shipped ones cannot be
//! edited (duplicate them), the user's are edited in Outils ▸ Données ▸
//! Formats texte… (R-C05-01).
//!
//! Besides the formats by line, the structured ones (D-08): JSON and XML by
//! paths, CSV by columns, with a description of their fields
//! ([`structured`]); the file is kept whole and only the translated values
//! change at their positions (D-88).

pub mod csv;
pub mod json;
pub mod structured;
pub mod xml;

pub use structured::{Fields, Resolve};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::games::Game;
use crate::tm::{TmEntry, TmMeta, TranslationMemory};

/// The field signature of every text line (R-C05-02).
pub const FIELD: &str = "TXT";

/// Whether a field signature is a text file's (C-05, D-08): `TXT`, or a
/// structured format's field name — anything but a record's four
/// capitals (`FULL`, `DESC`) and the scripts' `STR`, `VMAD`, `SCTX`.
pub fn is_text_field(field: &str) -> bool {
    field == FIELD
        || !(field.len() >= 3
            && field.len() <= 4
            && field
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
}

/// The key that matches a line between a file and its translated copy:
/// the identifier, with the field for a structured format.
pub fn entry_key(field: &str, id: &str) -> String {
    if field == FIELD {
        id.to_string()
    } else {
        format!("{field}\u{1f}{id}")
    }
}

/// One text format (R-C05-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFormat {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub builtin: bool,
    /// File-name patterns, `*` matching anything (`Interface/Translations/*_*.txt`).
    #[serde(default)]
    pub patterns: Vec<String>,
    /// auto | utf-8 | utf-16le | utf-16be | windows-1252.
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// keep | crlf | lf.
    #[serde(default = "default_newline")]
    pub newline: String,
    /// line | json | xml | csv (D-08, R-D08-01).
    #[serde(default = "default_kind")]
    pub kind: String,
    /// The line expression, with the named groups `key` and `value` (and
    /// `comment` at will); a structured format has none.
    #[serde(default)]
    pub line: String,
    /// The description of the fields of a structured format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Fields>,
    /// CSV: the separator (empty: found in the first line).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub separator: String,
    /// CSV: the first row names the columns.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub header: bool,
    /// none | json | c.
    #[serde(default = "default_escape")]
    pub escape: String,
    /// Recognised lines whose value is empty are not translatable lines.
    #[serde(default)]
    pub skip_empty: bool,
    /// suffix | none: whether `<base>_<langue>.<ext>` names the language.
    #[serde(default = "default_lang_in_name")]
    pub lang_in_name: String,
}

fn default_encoding() -> String {
    "auto".into()
}
fn default_newline() -> String {
    "keep".into()
}
fn default_escape() -> String {
    "none".into()
}
fn default_lang_in_name() -> String {
    "suffix".into()
}
fn default_kind() -> String {
    "line".into()
}
fn default_true() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

pub const ENCODINGS: [&str; 5] = ["auto", "utf-8", "utf-16le", "utf-16be", "windows-1252"];
pub const NEWLINES: [&str; 3] = ["keep", "crlf", "lf"];
pub const ESCAPES: [&str; 3] = ["none", "json", "c"];
pub const LANG_IN_NAME: [&str; 2] = ["suffix", "none"];

impl TextFormat {
    /// A blank user format.
    pub fn new_user(id: &str, name: &str) -> TextFormat {
        TextFormat {
            id: id.to_string(),
            name: name.to_string(),
            builtin: false,
            patterns: vec!["*.txt".into()],
            encoding: "auto".into(),
            newline: "keep".into(),
            kind: "line".into(),
            line: r"^(?P<key>[^=]+?)\s*=\s*(?P<value>.*)$".into(),
            fields: None,
            separator: String::new(),
            header: true,
            escape: "none".into(),
            skip_empty: false,
            lang_in_name: "suffix".into(),
        }
    }

    /// A blank structured format of a type (D-08), its pattern from the
    /// type.
    pub fn new_structured(id: &str, name: &str, kind: &str) -> TextFormat {
        let mut f = Self::new_user(id, name);
        f.kind = kind.to_string();
        f.line = String::new();
        f.patterns = vec![format!("*.{kind}")];
        f.fields = Some(Fields::blank(kind));
        f
    }

    /// A format by line (C-05).
    pub fn is_line(&self) -> bool {
        self.kind.is_empty() || self.kind == "line"
    }

    /// The translation goes in the file itself (D-08).
    pub fn in_place(&self) -> bool {
        !self.is_line() && self.fields.as_ref().is_some_and(Fields::in_place)
    }

    /// The type guessed from a file's extension.
    pub fn kind_of_path(path: &Path) -> &'static str {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref()
        {
            Some("json") => "json",
            Some("xml") => "xml",
            Some("csv" | "tsv") => "csv",
            _ => "line",
        }
    }

    /// Whether a pattern names more than an extension (`MCM/Config/*/config.json`,
    /// `translate_*.txt`): such a pattern decides alone (R-D08-06).
    pub fn specific_match(&self, path: &Path) -> bool {
        self.specific_score(path).is_some()
    }

    /// How precise the best pattern taking the file is (its length: a
    /// folder in the pattern makes it longer), `None` when only an
    /// extension takes it.
    pub fn specific_score(&self, path: &Path) -> Option<usize> {
        let full = path.to_string_lossy().replace('\\', "/").to_lowercase();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        self.patterns
            .iter()
            .filter_map(|p| {
                let p = p.replace('\\', "/").to_lowercase();
                let generic = p.starts_with("*.") && !p[2..].contains('*') && !p.contains('/');
                let hit = !generic
                    && if p.contains('/') {
                        glob_tail(&p, &full)
                    } else {
                        glob_match(&p, &name)
                    };
                hit.then(|| p.replace('*', "").len())
            })
            .max()
    }

    /// The grid's GRUP of this format: its id in capitals (R-C05-02).
    pub fn sig(&self) -> String {
        self.id.to_uppercase()
    }

    /// The compiled line expression, refused without the two groups
    /// (R-C05-01).
    pub fn compile(&self) -> std::result::Result<Regex, CompileError> {
        if !self.is_line() {
            return Err(CompileError::Structured);
        }
        let re = Regex::new(&self.line).map_err(|e| CompileError::Invalid(e.to_string()))?;
        let names: Vec<&str> = re.capture_names().flatten().collect();
        if !names.contains(&"key") || !names.contains(&"value") {
            return Err(CompileError::Groups);
        }
        Ok(re)
    }

    /// Whether a file name matches one of the patterns: a pattern with a
    /// `/` is matched against the tail of the path, one without against the
    /// file name; `*` matches anything; case aside.
    pub fn matches_name(&self, path: &Path) -> bool {
        let full = path.to_string_lossy().replace('\\', "/").to_lowercase();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        self.patterns.iter().any(|p| {
            let p = p.replace('\\', "/").to_lowercase();
            if p.contains('/') {
                glob_tail(&p, &full)
            } else {
                glob_match(&p, &name)
            }
        })
    }
}

/// Why an expression is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    Invalid(String),
    Groups,
    /// A structured format has no line expression.
    Structured,
}

/// `pattern` with `*` wildcards against the whole of `text`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| rec(&p[1..], &t[i..])),
            Some(c) => t.first() == Some(c) && rec(&p[1..], &t[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    rec(&p, &t)
}

/// `pattern` against the end of a path (`Interface/Translations/*_*.txt`
/// matches `…/Data/Interface/Translations/MonMod_english.txt`).
fn glob_tail(pattern: &str, full: &str) -> bool {
    glob_match(&format!("*/{pattern}"), full) || glob_match(pattern, full)
}

// ------------------------------------------------------------ the formats

/// The formats known to the application.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Formats {
    /// 2 since D-08 (the field `kind`); a file of format 1 (C-05) is read
    /// and written again as 2.
    #[serde(default = "format_one")]
    pub format: u32,
    pub formats: Vec<TextFormat>,
}

fn format_one() -> u32 {
    1
}

/// The format of `text_formats.json` written (R-D08-01).
pub const FILE_FORMAT: u32 = 2;

/// The shipped formats (`assets/data/text_formats.json`).
const BUILTIN_JSON: &str = include_str!("../assets/data/text_formats.json");

impl Formats {
    /// The shipped formats alone.
    pub fn builtin() -> Formats {
        let mut f: Formats = serde_json::from_str(BUILTIN_JSON).unwrap_or_default();
        for x in &mut f.formats {
            x.builtin = true;
        }
        f
    }

    /// `<config>/data/text_formats.json`.
    pub fn user_path() -> PathBuf {
        crate::config::paths::get()
            .config_dir
            .join("data")
            .join("text_formats.json")
    }

    /// The shipped formats (always the current ones) followed by the user's
    /// own, read from the configuration folder.
    pub fn load() -> Formats {
        Self::load_from(&Self::user_path())
    }

    pub fn load_from(path: &Path) -> Formats {
        let mut all = Self::builtin();
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(user) = serde_json::from_str::<Formats>(&text)
        {
            for mut f in user.formats {
                if f.builtin || all.formats.iter().any(|b| b.id == f.id) {
                    continue;
                }
                f.builtin = false;
                all.formats.push(f);
            }
        }
        all
    }

    /// Write the user's formats (the shipped ones are never written).
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::user_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        let user = Formats {
            format: FILE_FORMAT,
            formats: self
                .formats
                .iter()
                .filter(|f| !f.builtin)
                .cloned()
                .collect(),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(&user)?;
        crate::config::paths::write_atomic(path, &json)?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&TextFormat> {
        self.formats.iter().find(|f| f.id == id)
    }

    /// A free id derived from a name (`mon_format`, `mon_format2`…).
    pub fn free_id(&self, base: &str) -> String {
        let mut id: String = base
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        if id.is_empty() {
            id = "format".into();
        }
        let mut cand = id.clone();
        let mut n = 2;
        while self.formats.iter().any(|f| f.id == cand) {
            cand = format!("{id}{n}");
            n += 1;
        }
        cand
    }

    /// The formats a file may be in (R-C05-02): each with whether its name
    /// patterns match and how many of the first hundred lines it recognises.
    pub fn candidates(&self, path: &Path, data: &[u8]) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = self
            .formats
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let (recognized, total) = count_recognized(f, data, 100);
                Candidate {
                    index: i,
                    by_name: f.matches_name(path),
                    recognized,
                    total,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.by_name
                .cmp(&a.by_name)
                .then(b.recognized.cmp(&a.recognized))
                .then(a.index.cmp(&b.index))
        });
        out
    }

    /// The formats whose name patterns take a file, without reading it
    /// (enough for [`pick_for`](Self::pick_for)).
    pub fn name_candidates(&self, path: &Path) -> Vec<Candidate> {
        self.formats
            .iter()
            .enumerate()
            .map(|(i, f)| Candidate {
                index: i,
                by_name: f.matches_name(path),
                recognized: 0,
                total: 0,
            })
            .collect()
    }

    /// The format to open a file with, when the name patterns designate
    /// one; `None` means « ask » (R-C05-02, R-D08-06): a pattern that names
    /// more than an extension decides alone when it is the only one; else
    /// a single format by line whose pattern matches, for a file that is
    /// neither JSON nor XML (those, untaken by a precise pattern, go to the
    /// import dialog).
    pub fn pick_for(&self, path: &Path, cands: &[Candidate]) -> Option<usize> {
        // The most precise pattern decides, when it is alone at its level.
        let scored: Vec<(usize, usize)> = cands
            .iter()
            .filter(|c| c.by_name)
            .filter_map(|c| {
                self.formats[c.index]
                    .specific_score(path)
                    .map(|s| (s, c.index))
            })
            .collect();
        if let Some(best) = scored.iter().map(|x| x.0).max() {
            let top: Vec<usize> = scored.iter().filter(|x| x.0 == best).map(|x| x.1).collect();
            return (top.len() == 1).then(|| top[0]);
        }
        let structured_ext = matches!(TextFormat::kind_of_path(path), "json" | "xml");
        let by_name: Vec<&Candidate> = cands.iter().filter(|c| c.by_name).collect();
        if by_name.len() == 1 && !structured_ext && self.formats[by_name[0].index].is_line() {
            return Some(by_name[0].index);
        }
        None
    }

    /// [`pick_for`](Self::pick_for) without a path (the tests of C-05).
    pub fn pick(&self, cands: &[Candidate]) -> Option<usize> {
        let by_name: Vec<&Candidate> = cands.iter().filter(|c| c.by_name).collect();
        if by_name.len() == 1 {
            return Some(by_name[0].index);
        }
        None
    }
}

/// A format considered for a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub index: usize,
    pub by_name: bool,
    pub recognized: usize,
    pub total: usize,
}

/// Recognised lines among the first `limit` of a file, in a format (a
/// structured format: its lines among the text values of the file).
pub fn count_recognized(fmt: &TextFormat, data: &[u8], limit: usize) -> (usize, usize) {
    if !fmt.is_line() {
        let (text, _) = decode(data, &fmt.encoding);
        return match structured::parse(&text, fmt) {
            Ok(s) => (s.items.len(), s.stats.read),
            Err(_) => (0, 0),
        };
    }
    let Ok(re) = fmt.compile() else {
        return (0, 0);
    };
    let (text, _) = decode(data, &fmt.encoding);
    let mut n = 0;
    let mut total = 0;
    for line in text.lines().take(limit) {
        total += 1;
        if re.is_match(line.trim_end_matches('\r')) {
            n += 1;
        }
    }
    (n, total)
}

/// « Essai sur un fichier » (R-C05-01): recognised lines, total, and the
/// first three pairs.
pub type Trial = (usize, usize, Vec<(String, String)>);

pub fn try_file(fmt: &TextFormat, path: &Path) -> Result<Trial> {
    let re = match fmt.compile() {
        Ok(re) => re,
        Err(CompileError::Invalid(e)) => bail!("{e}"),
        Err(CompileError::Groups) => bail!(crate::l10n!("formats-regex-groups")),
        Err(CompileError::Structured) => bail!(crate::l10n!("err-no-text-format")),
    };
    let data = std::fs::read(path).with_context(|| path.display().to_string())?;
    let (text, _) = decode(&data, &fmt.encoding);
    let mut n = 0;
    let mut total = 0;
    let mut first = Vec::new();
    for line in text.lines() {
        total += 1;
        if let Some(c) = re.captures(line.trim_end_matches('\r')) {
            n += 1;
            if first.len() < 3 {
                let key = c.name("key").map(|m| m.as_str()).unwrap_or("").to_string();
                let value = c
                    .name("value")
                    .map(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                first.push((key, unescape(&value, &fmt.escape)));
            }
        }
    }
    Ok((n, total, first))
}

/// « Essai sur un fichier » of a structured format (R-D08-07): the reading
/// and the first line, or why it fails.
pub fn try_structured(
    fmt: &TextFormat,
    path: &Path,
) -> std::result::Result<structured::Structured, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (text, _) = decode(&data, &fmt.encoding);
    if let Some(c) = structured::missing_columns(fmt, &text) {
        return Err(structured::Problem::Column(c).message());
    }
    structured::parse(&text, fmt).map_err(|p| p.message())
}

// ------------------------------------------------------------ encodings

/// The encoding a file turned out to be in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Used {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Win1252,
    /// Another page of code (D-09), by its WHATWG name.
    Page(String),
}

impl Used {
    /// The badge text (« UTF-16 », « UTF-8 », « ANSI », « 1251 »).
    pub fn label(&self) -> &'static str {
        match self {
            Used::Utf8 | Used::Utf8Bom => "UTF-8",
            Used::Utf16Le | Used::Utf16Be => "UTF-16",
            Used::Win1252 => "ANSI",
            Used::Page(id) => crate::codepage::Page::from_id(id)
                .map(|p| p.short())
                .unwrap_or("?"),
        }
    }
}

/// Decode a file's bytes for a format's `encoding`: `auto` reads the byte
/// order mark, then tries UTF-8, then Windows-1252.
pub fn decode(data: &[u8], encoding: &str) -> (String, Used) {
    let enc = encoding.trim().to_lowercase();
    // The mark always wins, whatever the setting says.
    if data.starts_with(&[0xFF, 0xFE]) {
        return (utf16(&data[2..], true), Used::Utf16Le);
    }
    if data.starts_with(&[0xFE, 0xFF]) {
        return (utf16(&data[2..], false), Used::Utf16Be);
    }
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (
            String::from_utf8_lossy(&data[3..]).into_owned(),
            Used::Utf8Bom,
        );
    }
    match enc.as_str() {
        "utf-16le" => (utf16(data, true), Used::Utf16Le),
        "utf-16be" => (utf16(data, false), Used::Utf16Be),
        "windows-1252" => (win1252(data), Used::Win1252),
        "utf-8" => (String::from_utf8_lossy(data).into_owned(), Used::Utf8),
        other => {
            // A page named by the format (D-09), else the page of the
            // reading in progress (the document's languages), else the
            // historical fallback.
            let named =
                crate::codepage::Page::from_id(other).filter(|p| *p != crate::codepage::Page::Utf8);
            let codec = crate::tm::encoding::input_codec();
            if let Some(p) = named {
                return (p.decode(data), page_used(p));
            }
            if let Some(c) = codec
                && c.strict
            {
                return (c.page.decode(data), page_used(c.page));
            }
            match std::str::from_utf8(data) {
                Ok(s) => (s.to_string(), Used::Utf8),
                Err(_) => match codec {
                    Some(c) => (c.page.decode(data), page_used(c.page)),
                    None => (win1252(data), Used::Win1252),
                },
            }
        }
    }
}

fn page_used(p: crate::codepage::Page) -> Used {
    match p {
        crate::codepage::Page::Win1252 => Used::Win1252,
        crate::codepage::Page::Utf8 => Used::Utf8,
        p => Used::Page(p.id().to_string()),
    }
}

fn utf16(data: &[u8], le: bool) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

fn win1252(data: &[u8]) -> String {
    crate::tm::encoding::decode_win1252(data)
}

/// Encode a text the way its file was (the mark included when it had one).
pub fn encode(text: &str, used: &Used) -> Vec<u8> {
    match used {
        Used::Utf8 => text.as_bytes().to_vec(),
        Used::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(text.as_bytes());
            v
        }
        Used::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        }
        Used::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_be_bytes());
            }
            v
        }
        // A page of code: the one of the writing in progress when the
        // document forces one (D-09), else the page the file was read in.
        Used::Win1252 => crate::tm::encoding::output_override()
            .unwrap_or(crate::codepage::Page::Win1252)
            .encode(text),
        Used::Page(id) => {
            let page = crate::codepage::Page::from_id(id).unwrap_or(crate::codepage::Page::Win1252);
            crate::tm::encoding::output_override()
                .unwrap_or(page)
                .encode(text)
        }
    }
}

/// Unescape a value for a format (`json` / `c`: `\"`, `\\`, `\n`, `\t`, `\r`).
pub fn unescape(value: &str, escape: &str) -> String {
    if escape == "none" {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut it = value.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The reverse of [`unescape`].
pub fn escape_value(value: &str, escape: &str) -> String {
    if escape == "none" {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 4);
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            o => out.push(o),
        }
    }
    out
}

// ------------------------------------------------------------ the document

/// One line of the file: its text, and the recognised pair when the
/// expression matched (with the byte span of the value in `raw`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub raw: String,
    pub pair: Option<Pair>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub key: String,
    /// The value as the translator sees it (unescaped).
    pub value: String,
    /// Where the escaped value sits in `raw`.
    pub span: (usize, usize),
}

/// A text file read in a format: everything needed to write it back.
#[derive(Debug, Clone)]
pub struct TextDoc {
    pub format_id: String,
    pub encoding: Used,
    pub crlf: bool,
    pub trailing_newline: bool,
    pub lines: Vec<Line>,
    /// The line of each grid entry, in order (R-C05-02).
    pub pairs: Vec<usize>,
    /// A structured format's reading (D-08): the text, the values and
    /// their positions.
    pub structured: Option<structured::Structured>,
    /// The size and date of the file read, to refuse writing over a file
    /// changed since (R-D08-04).
    pub origin: Option<(u64, Option<std::time::SystemTime>)>,
}

/// The size and date of a file.
pub fn stamp(path: &Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()))
}

impl TextDoc {
    /// Read a file in a format.
    pub fn read(path: &Path, fmt: &TextFormat) -> Result<TextDoc> {
        let data = std::fs::read(path).with_context(|| path.display().to_string())?;
        let mut doc = Self::parse(&data, fmt)?;
        doc.origin = stamp(path);
        Ok(doc)
    }

    /// The file changed since it was read (R-D08-04).
    pub fn changed_since(&self, path: &Path) -> bool {
        match &self.origin {
            Some(o) => stamp(path).as_ref() != Some(o),
            None => false,
        }
    }

    /// Parse a file's bytes in a format.
    pub fn parse(data: &[u8], fmt: &TextFormat) -> Result<TextDoc> {
        if !fmt.is_line() {
            let (text, encoding) = decode(data, &fmt.encoding);
            let s = structured::parse(&text, fmt).map_err(|p| anyhow::anyhow!(p.message()))?;
            return Ok(TextDoc {
                format_id: fmt.id.clone(),
                encoding,
                crlf: text.contains("\r\n"),
                trailing_newline: text.ends_with('\n'),
                lines: Vec::new(),
                pairs: (0..s.items.len()).collect(),
                structured: Some(s),
                origin: None,
            });
        }
        let re = match fmt.compile() {
            Ok(re) => re,
            Err(CompileError::Invalid(e)) => bail!(crate::l10n!("formats-regex-invalid", err = e)),
            Err(CompileError::Groups) => {
                bail!(crate::l10n!("formats-regex-groups"))
            }
            Err(CompileError::Structured) => bail!(crate::l10n!("err-no-text-format")),
        };
        let (text, encoding) = decode(data, &fmt.encoding);
        let crlf = text.contains("\r\n");
        let trailing_newline = text.ends_with('\n');
        let mut lines = Vec::new();
        let mut pairs = Vec::new();
        let body = text.strip_suffix('\n').unwrap_or(&text);
        for raw in body.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw).to_string();
            let pair = re.captures(&raw).and_then(|c| {
                let key = c.name("key")?.as_str().to_string();
                let v = c.name("value")?;
                let value = unescape(v.as_str(), &fmt.escape);
                if fmt.skip_empty && value.is_empty() {
                    return None;
                }
                Some(Pair {
                    key,
                    value,
                    span: (v.start(), v.end()),
                })
            });
            if pair.is_some() {
                pairs.push(lines.len());
            }
            lines.push(Line { raw, pair });
        }
        Ok(TextDoc {
            format_id: fmt.id.clone(),
            encoding,
            crlf,
            trailing_newline,
            lines,
            pairs,
            structured: None,
            origin: None,
        })
    }

    /// The number of recognised lines.
    pub fn keys(&self) -> usize {
        self.pairs.len()
    }

    /// The pairs as a lookup (`key` → values, in order; a structured
    /// format's key is [`entry_key`]).
    pub fn values_by_key(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut m: std::collections::HashMap<String, Vec<String>> = Default::default();
        if let Some(s) = &self.structured {
            for it in &s.items {
                m.entry(entry_key(&it.field, &it.id))
                    .or_default()
                    .push(it.value.clone());
            }
            return m;
        }
        for &i in &self.pairs {
            if let Some(p) = &self.lines[i].pair {
                m.entry(p.key.clone()).or_default().push(p.value.clone());
            }
        }
        m
    }

    /// The document's lines as a translation memory (R-C05-02): GRUP = the
    /// format's id in capitals, Champ = TXT, EDID = the key, the index
    /// counting a repeated key.
    pub fn to_tm(&self, fmt: &TextFormat, file_name: &str, game: &str) -> TranslationMemory {
        let sig = fmt.sig();
        let mut counts: std::collections::HashMap<String, i32> = Default::default();
        let mut entries = Vec::with_capacity(self.pairs.len());
        // A structured format (R-D08-03): GRUP = the record, Champ = the
        // field, EDID = the identifier, the index the rank among the lines
        // of the same field and identifier; a translation the file gives
        // is Validé, provenance « fichier cible ».
        if let Some(s) = &self.structured {
            for it in &s.items {
                let idx = counts.entry(entry_key(&it.field, &it.id)).or_insert(0);
                *idx += 1;
                let mut e = TmEntry {
                    source: it.value.clone(),
                    edid: it.id.clone(),
                    rec_sig: if it.record.is_empty() {
                        sig.clone()
                    } else {
                        it.record.clone()
                    },
                    field_sig: if it.field.is_empty() {
                        FIELD.to_string()
                    } else {
                        it.field.clone()
                    },
                    index: *idx,
                    idstexte: -1,
                    icon: -1,
                    ..Default::default()
                };
                if let Some(t) = &it.existing {
                    e.trans = t.clone();
                    e.prov = crate::provenance::TARGET.to_string();
                    crate::status::set(&mut e, crate::status::Status::Validated);
                }
                entries.push(e);
            }
        }
        for &i in &self.pairs {
            if self.structured.is_some() {
                break;
            }
            let Some(p) = &self.lines[i].pair else {
                continue;
            };
            let idx = counts.entry(p.key.clone()).or_insert(0);
            *idx += 1;
            entries.push(TmEntry {
                source: p.value.clone(),
                edid: p.key.clone(),
                rec_sig: sig.clone(),
                field_sig: FIELD.to_string(),
                index: *idx,
                idstexte: -1,
                icon: -1,
                ..Default::default()
            });
        }
        TranslationMemory {
            meta: TmMeta {
                game: game.to_string(),
                source_format: "text".to_string(),
                plugin_name: file_name.to_string(),
                text_format: fmt.id.clone(),
                ..Default::default()
            },
            entries,
            history: Vec::new(),
        }
    }

    /// Write the document in the target language (R-C05-04): each
    /// recognised line gets the entry's translation in place of its value
    /// — the source when the line is untranslated and `copy_untranslated`
    /// is set, else the line is left out —, the other lines are copied as
    /// they are, in the file's encoding and line endings. `entries` are the
    /// grid's lines in order (an excluded line has an empty translation).
    pub fn write(&self, fmt: &TextFormat, entries: &[TmEntry], copy_untranslated: bool) -> Vec<u8> {
        // A structured file (R-D08-04): the text whole, the translations at
        // their positions; an untranslated value stays as it is.
        if let Some(s) = &self.structured {
            let out = s.write(|k| {
                entries
                    .get(k)
                    .map(|e| e.trans.as_str())
                    .filter(|t| !t.is_empty())
            });
            return encode(&out, &self.encoding);
        }
        let nl = match fmt.newline.as_str() {
            "crlf" => "\r\n",
            "lf" => "\n",
            _ => {
                if self.crlf {
                    "\r\n"
                } else {
                    "\n"
                }
            }
        };
        let mut out = String::new();
        let mut k = 0usize;
        let mut first = true;
        for line in &self.lines {
            let text = match &line.pair {
                Some(p) => {
                    let e = entries.get(k);
                    k += 1;
                    let value = match e {
                        Some(e) if !e.trans.is_empty() => Some(e.trans.as_str()),
                        _ if copy_untranslated => Some(p.value.as_str()),
                        _ => None,
                    };
                    let Some(value) = value else {
                        continue;
                    };
                    let escaped = escape_value(value, &fmt.escape);
                    let mut raw = line.raw.clone();
                    raw.replace_range(p.span.0..p.span.1, &escaped);
                    raw
                }
                None => line.raw.clone(),
            };
            if !first {
                out.push_str(nl);
            }
            first = false;
            out.push_str(&text);
        }
        if self.trailing_newline {
            out.push_str(nl);
        }
        encode(&out, &self.encoding)
    }
}

/// A translated copy of a text document's file, read in the same format
/// and paired with the document's lines by [`entry_key`] and rank (A-17
/// on a text document, D-08): the lines of the document with the copy's
/// values as their translation, for « Charger dans la relecture ».
pub fn pair_copy(
    fmt: &TextFormat,
    entries: &[TmEntry],
    copy: &Path,
) -> std::result::Result<TranslationMemory, String> {
    let doc = TextDoc::read(copy, fmt).map_err(|e| e.to_string())?;
    let values = doc.values_by_key();
    let mut out = Vec::new();
    for e in entries {
        let key = entry_key(&e.field_sig, &e.edid);
        let k = (e.index.max(1) - 1) as usize;
        // A value left as the source is no translation.
        if let Some(v) = values.get(&key).and_then(|l| l.get(k))
            && !v.is_empty()
            && *v != e.source
        {
            let mut c = e.clone();
            c.trans = v.clone();
            out.push(c);
        }
    }
    if out.is_empty() {
        return Err(crate::l10n!("err-no-text-format"));
    }
    Ok(TranslationMemory {
        meta: TmMeta {
            source_format: "text".to_string(),
            text_format: fmt.id.clone(),
            ..Default::default()
        },
        entries: out,
        history: Vec::new(),
    })
}

// ------------------------------------------------------------ languages

/// The language token of a file name (`MonMod_english` → `("MonMod",
/// "english")`), when the token is a known language name or code
/// (R-C05-03).
pub fn lang_of_stem(stem: &str) -> Option<(String, String)> {
    let us = stem.rfind('_')?;
    let token = &stem[us + 1..];
    let base = &stem[..us];
    if base.is_empty() || token.is_empty() {
        return None;
    }
    iso3_of_token(token).map(|_| (base.to_string(), token.to_string()))
}

/// ISO 639-3 of a file-name token: Bethesda names (english), ISO codes
/// (en, eng) and the Fallout 4 `ptbr`.
pub fn iso3_of_token(token: &str) -> Option<String> {
    let t = token.trim().to_lowercase();
    if t == "ptbr" {
        return Some("por".into());
    }
    crate::lang::iso3_from_token(&t).map(str::to_string)
}

/// The token a game's interface files use for a language (R-C05-03): the
/// lowercase English name for Skyrim and before (`french`), the ISO 639-1
/// code for Fallout 4 / 76 and Starfield (`fr`, `ptbr`).
pub fn mcm_token(game: Game, code: &str) -> String {
    let c = code.trim().to_lowercase();
    match game {
        Game::Fallout4 | Game::Fallout76 | Game::Starfield => {
            if c == "por" || c == "pt" {
                "ptbr".to_string()
            } else {
                crate::lang::strings_suffix(ximod_games::LangStyle::Iso2, &c).to_lowercase()
            }
        }
        _ => crate::lang::strings_suffix(ximod_games::LangStyle::Name, &c).to_lowercase(),
    }
}

/// The sibling files `<base>_<langue>.<ext>` of a file, with their language
/// (ISO 639-3), the file itself included.
pub fn siblings(path: &Path) -> Vec<(String, PathBuf)> {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let Some((base, _)) = lang_of_stem(stem) else {
        return Vec::new();
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let same_ext = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(&ext));
            if !same_ext {
                continue;
            }
            let Some(s) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some((b, tok)) = lang_of_stem(s)
                && b.eq_ignore_ascii_case(&base)
                && let Some(iso) = iso3_of_token(&tok)
            {
                out.push((iso, p));
            }
        }
    }
    out.sort();
    out
}

/// The target file of a source file (`MonMod_english.txt` → `MonMod_french.txt`
/// for Skyrim, `MonMod_en.txt` → `MonMod_fr.txt` for Fallout 4); `None`
/// when the name carries no language.
pub fn target_path(path: &Path, game: Game, target: &str) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let (base, _) = lang_of_stem(stem)?;
    let ext = path.extension()?.to_str()?;
    let token = mcm_token(game, target);
    Some(path.with_file_name(format!("{base}_{token}.{ext}")))
}

/// The interface files of a plugin (R-C05-05): `Interface/Translations/
/// <stem>_<langue source>.txt` under the plugin's folder — the source
/// language's file, or every language's when it has none.
pub fn interface_files_of(plugin: &Path, source: &str, game: Game) -> Vec<PathBuf> {
    let all = interface_files_all(plugin, source, game);
    let wanted = mcm_token(game, source);
    let own: Vec<PathBuf> = all
        .iter()
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(lang_of_stem)
                .is_some_and(|(_, tok)| tok.eq_ignore_ascii_case(&wanted))
        })
        .cloned()
        .collect();
    if own.is_empty() { all } else { own }
}

/// Every interface file of a plugin, the source language's first.
pub fn interface_files_all(plugin: &Path, source: &str, game: Game) -> Vec<PathBuf> {
    let Some(dir) = plugin.parent() else {
        return Vec::new();
    };
    let Some(stem) = plugin.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let folder = dir.join("Interface").join("Translations");
    let Ok(rd) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let wanted = mcm_token(game, source);
    let mut found: Vec<(bool, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let Some(s) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let is_txt = p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        if !is_txt {
            continue;
        }
        if let Some((base, tok)) = lang_of_stem(s)
            && base.eq_ignore_ascii_case(stem)
        {
            found.push((tok.eq_ignore_ascii_case(&wanted), p));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// The placeholders of text values that count as tags (R-C05-06), from
/// `assets/data/fields/Text.json`.
pub fn text_tags() -> &'static [String] {
    static TAGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    TAGS.get_or_init(|| {
        const JSON: &str = include_str!("../assets/data/fields/Text.json");
        #[derive(Deserialize)]
        struct F {
            #[serde(default)]
            tags: Vec<String>,
        }
        serde_json::from_str::<F>(JSON)
            .map(|f| f.tags)
            .unwrap_or_else(|_| vec!["{*}".into(), "%*".into()])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcm() -> TextFormat {
        Formats::builtin().get("mcm").unwrap().clone()
    }

    fn utf16le(s: &str) -> Vec<u8> {
        encode(s, &Used::Utf16Le)
    }

    #[test]
    fn builtin_formats_compile_and_match_names() {
        let f = Formats::builtin();
        assert!(f.formats.len() >= 4);
        for x in &f.formats {
            assert!(x.builtin);
            if x.is_line() {
                assert!(x.compile().is_ok(), "{}", x.id);
            } else {
                assert_eq!(structured::validate(x), Ok(()), "{}", x.id);
            }
        }
        assert_eq!(f.formats.len(), 8);
        let m = f.get("mcm").unwrap();
        assert!(m.matches_name(Path::new(
            "C:\\Games\\Data\\Interface\\Translations\\MonMod_english.txt"
        )));
        assert!(m.matches_name(Path::new("/data/Interface/translations/MonMod_FRENCH.txt")));
        assert!(m.matches_name(Path::new("/x/translate_english.txt")));
        assert!(!m.matches_name(Path::new("/x/readme.txt")));
        assert!(
            f.get("ini")
                .unwrap()
                .matches_name(Path::new("/x/MonMod.ini"))
        );
        assert!(glob_match("*_*.txt", "a_b.txt"));
        assert!(!glob_match("*_*.txt", "ab.txt"));
    }

    /// T-C05-01: an MCM file with comments and empty lines, read and
    /// written back byte for byte except the translated values.
    #[test]
    fn mcm_round_trip_keeps_foreign_lines_and_encoding() {
        let src = "; MonMod\r\n$Hello\tHello\r\n\r\n$Bye\tGoodbye {0}\r\n; end\r\n$Hello\tHello again\r\n";
        let data = utf16le(src);
        let fmt = mcm();
        let doc = TextDoc::parse(&data, &fmt).unwrap();
        assert_eq!(doc.encoding, Used::Utf16Le);
        assert!(doc.crlf);
        assert_eq!(doc.lines.len(), 6);
        assert_eq!(doc.keys(), 3);
        let tm = doc.to_tm(&fmt, "MonMod_english.txt", "SkyrimSE");
        assert_eq!(tm.meta.source_format, "text");
        assert_eq!(tm.entries.len(), 3);
        assert_eq!(
            (
                tm.entries[0].rec_sig.as_str(),
                tm.entries[0].field_sig.as_str()
            ),
            ("MCM", "TXT")
        );
        assert_eq!(
            (tm.entries[0].edid.as_str(), tm.entries[0].index),
            ("$Hello", 1)
        );
        assert_eq!(
            (tm.entries[2].edid.as_str(), tm.entries[2].index),
            ("$Hello", 2)
        );
        assert_eq!(tm.entries[1].source, "Goodbye {0}");
        let mut entries = tm.entries.clone();
        entries[0].trans = "Bonjour".into();
        entries[1].trans = "Au revoir {0}".into();
        let out = doc.write(&fmt, &entries, true);
        let expect = utf16le(
            "; MonMod\r\n$Hello\tBonjour\r\n\r\n$Bye\tAu revoir {0}\r\n; end\r\n$Hello\tHello again\r\n",
        );
        assert_eq!(out, expect);
        // Without the copy, the untranslated line is left out.
        let out = doc.write(&fmt, &entries, false);
        let expect = utf16le("; MonMod\r\n$Hello\tBonjour\r\n\r\n$Bye\tAu revoir {0}\r\n; end\r\n");
        assert_eq!(out, expect);
    }

    /// T-C05-02: a flat JSON file, quotes escaped at writing.
    #[test]
    fn flat_json_escapes_quotes() {
        let fmt = Formats::builtin().get("json").unwrap().clone();
        let src =
            "{\n  \"title\": \"Hello \\\"world\\\"\",\n  \"count\": 3,\n  \"bye\": \"Bye\"\n}\n";
        let doc = TextDoc::parse(src.as_bytes(), &fmt).unwrap();
        assert_eq!(doc.keys(), 2);
        assert_eq!(doc.lines[1].pair.as_ref().unwrap().value, "Hello \"world\"");
        let tm = doc.to_tm(&fmt, "strings.json", "");
        let mut entries = tm.entries.clone();
        entries[0].trans = "Salut « le » \"monde\"".into();
        let out = String::from_utf8(doc.write(&fmt, &entries, true)).unwrap();
        assert_eq!(
            out,
            "{\n  \"title\": \"Salut « le » \\\"monde\\\"\",\n  \"count\": 3,\n  \"bye\": \"Bye\"\n}\n"
        );
    }

    /// T-C05-03: a regex without the groups is refused; the trial counts.
    #[test]
    fn compile_refuses_missing_groups_and_trial_counts() {
        let mut f = TextFormat::new_user("skse", "Mon format SKSE");
        f.line = r"^(?P<key>\w+)=(.*)$".into();
        assert_eq!(f.compile().unwrap_err(), CompileError::Groups);
        f.line = "(".into();
        assert!(matches!(f.compile().unwrap_err(), CompileError::Invalid(_)));
        f.line = r"^(?P<key>\w+)=(?P<value>.*)$".into();
        assert!(f.compile().is_ok());
        let dir = std::env::temp_dir().join(format!("ximod_textfmt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.ini");
        let mut s = String::from("# header\n");
        for i in 0..12 {
            s.push_str(&format!("k{i}=v{i}\n"));
        }
        s.push_str("[section]\n");
        std::fs::write(&p, s).unwrap();
        let (n, total, first) = try_file(&f, &p).unwrap();
        assert_eq!((n, total), (12, 14));
        assert_eq!(first[0], ("k0".to_string(), "v0".to_string()));
        assert_eq!(first.len(), 3);
        let data = std::fs::read(&p).unwrap();
        let formats = Formats::builtin();
        let cands = formats.candidates(&p, &data);
        assert!(cands[0].by_name);
        assert_eq!(
            formats.get(&formats.formats[cands[0].index].id).unwrap().id,
            "ini"
        );
        assert_eq!(formats.pick(&cands), Some(cands[0].index));
        // User formats round trip through the file.
        let mut all = formats.clone();
        all.formats.push(f.clone());
        let fp = dir.join("text_formats.json");
        all.save_to(&fp).unwrap();
        let back = Formats::load_from(&fp);
        assert_eq!(back.get("skse"), Some(&f));
        assert_eq!(back.free_id("Mon format SKSE"), "mon_format_skse");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-C05-05: language tokens per game, siblings and target names.
    #[test]
    fn language_tokens_follow_the_game() {
        assert_eq!(
            lang_of_stem("MonMod_english"),
            Some(("MonMod".into(), "english".into()))
        );
        assert_eq!(
            lang_of_stem("MonMod_en"),
            Some(("MonMod".into(), "en".into()))
        );
        assert_eq!(
            lang_of_stem("MonMod_ptbr"),
            Some(("MonMod".into(), "ptbr".into()))
        );
        assert_eq!(lang_of_stem("MonMod_v2"), None);
        assert_eq!(
            lang_of_stem("translate_french"),
            Some(("translate".into(), "french".into()))
        );
        assert_eq!(mcm_token(Game::SkyrimSE, "fra"), "french");
        assert_eq!(mcm_token(Game::Fallout4, "fra"), "fr");
        assert_eq!(mcm_token(Game::Fallout4, "por"), "ptbr");
        assert_eq!(mcm_token(Game::Starfield, "deu"), "de");
        assert_eq!(
            target_path(Path::new("/x/MonMod_english.txt"), Game::SkyrimSE, "fra"),
            Some(PathBuf::from("/x/MonMod_french.txt"))
        );
        assert_eq!(
            target_path(Path::new("/x/MonMod_en.txt"), Game::Fallout4, "fra"),
            Some(PathBuf::from("/x/MonMod_fr.txt"))
        );
        assert_eq!(
            target_path(Path::new("/x/MonMod.ini"), Game::SkyrimSE, "fra"),
            None
        );
        let dir = std::env::temp_dir().join(format!("ximod_textfmt_sib_{}", std::process::id()));
        let tr = dir.join("Data").join("Interface").join("Translations");
        std::fs::create_dir_all(&tr).unwrap();
        std::fs::write(tr.join("MonMod_english.txt"), b"").unwrap();
        std::fs::write(tr.join("MonMod_german.txt"), b"").unwrap();
        std::fs::write(tr.join("Other_english.txt"), b"").unwrap();
        let sib = siblings(&tr.join("MonMod_english.txt"));
        let langs: Vec<&str> = sib.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(langs, vec!["deu", "eng"]);
        let plugin = dir.join("Data").join("MonMod.esp");
        let found = interface_files_all(&plugin, "eng", Game::SkyrimSE);
        assert_eq!(found.len(), 2);
        assert!(found[0].ends_with("MonMod_english.txt"));
        let own = interface_files_of(&plugin, "eng", Game::SkyrimSE);
        assert_eq!(own.len(), 1);
        let any = interface_files_of(&plugin, "spa", Game::SkyrimSE);
        assert_eq!(any.len(), 2, "no Spanish file: every language is offered");
        assert!(
            interface_files_of(&dir.join("Data").join("None.esp"), "eng", Game::SkyrimSE)
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-C05-01 / T-C05-04 (data): the shipped fixture — an MCM file next
    /// to its plugin — is recognised by name, read whole, and written back
    /// byte for byte but for the translated values.
    #[test]
    fn mcm_fixture_round_trip() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcm/Data");
        let plugin = root.join("MonMod.esp");
        let found = interface_files_of(&plugin, "eng", Game::Skyrim);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("MonMod_english.txt"), "{found:?}");
        let src = &found[0];
        let data = std::fs::read(src).unwrap();
        let formats = Formats::builtin();
        let cands = formats.candidates(src, &data);
        let idx = formats.pick(&cands).expect("the MCM pattern decides");
        let fmt = &formats.formats[idx];
        assert_eq!(fmt.id, "mcm");
        let doc = TextDoc::parse(&data, fmt).unwrap();
        assert_eq!((doc.keys(), doc.lines.len()), (20, 25));
        assert_eq!(doc.encoding, Used::Utf16Le);
        let tm = doc.to_tm(fmt, "MonMod_english.txt", "Skyrim");
        let yes: Vec<i32> = tm
            .entries
            .iter()
            .filter(|e| e.edid == "$MonMod_Yes")
            .map(|e| e.index)
            .collect();
        assert_eq!(yes, vec![1, 2]);
        // Unchanged lines write back identically.
        let out = doc.write(fmt, &tm.entries, true);
        assert_eq!(out, data);
        // The target file: 17 keys taken, one the source lacks.
        let target = target_path(src, Game::Skyrim, "fra").unwrap();
        assert!(target.ends_with("MonMod_french.txt"));
        let tdoc = TextDoc::read(&target, fmt).unwrap();
        let values = tdoc.values_by_key();
        assert_eq!(values["$MonMod_Yes"], vec!["Oui", "Oui !"]);
        assert!(values.contains_key("$MonMod_OldKey"));
        let known = tm
            .entries
            .iter()
            .filter(|e| values.contains_key(&e.edid))
            .count();
        assert_eq!(known, 17);
    }

    /// T-D08-07: a user file of format 1 (C-05) is read and written again
    /// as format 2 without loss.
    #[test]
    fn a_first_version_file_is_promoted() {
        let dir = std::env::temp_dir().join(format!("ximod_textfmt_v1_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("text_formats.json");
        let v1 = r#"{"formats":[{"id":"mine","name":"Mine","patterns":["*.cfg"],"line":"^(?P<key>\\w+)=(?P<value>.*)$"}]}"#;
        std::fs::write(&p, v1).unwrap();
        let all = Formats::load_from(&p);
        let mine = all.get("mine").unwrap().clone();
        assert!(mine.is_line());
        assert_eq!(mine.kind, "line");
        assert!(mine.compile().is_ok());
        all.save_to(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"format\": 2"), "{text}");
        let back = Formats::load_from(&p);
        assert_eq!(back.get("mine"), Some(&mine));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R-D08-06: a precise pattern decides; a JSON that only `*.json`
    /// takes goes to the import dialog; an INI opens as before.
    #[test]
    fn precise_patterns_decide() {
        let f = Formats::builtin();
        let pick = |path: &str, data: &[u8]| {
            let p = Path::new(path);
            let c = f.candidates(p, data);
            f.pick_for(p, &c).map(|i| f.formats[i].id.clone())
        };
        assert_eq!(
            pick("/m/MCM/Config/SS2/config.json", b"{\"content\": []}").as_deref(),
            Some("mcm-helper")
        );
        assert_eq!(pick("/m/data.json", b"{\"a\": \"b\"}"), None);
        assert_eq!(pick("/m/MonMod.ini", b"a=b\n").as_deref(), Some("ini"));
        assert_eq!(
            pick("/m/x/translate_english.txt", b"").as_deref(),
            Some("mcm")
        );
        assert_eq!(pick("/m/strings.xml", b"<a/>"), None);
        // A user format named after the file, and MCM Helper's folder:
        // the more precise wins.
        let mut f = f.clone();
        let mut own = TextFormat::new_structured("own", "Own", "json");
        own.patterns = vec!["config.json".into()];
        f.formats.push(own);
        let p = Path::new("/m/MCM/Config/SS2/config.json");
        let c = f.candidates(p, b"{}");
        assert_eq!(f.formats[f.pick_for(p, &c).unwrap()].id, "mcm-helper");
        let p = Path::new("/m/other/config.json");
        let c = f.candidates(p, b"{}");
        assert_eq!(f.formats[f.pick_for(p, &c).unwrap()].id, "own");
        assert!(is_text_field("TXT") && is_text_field("help") && is_text_field("pageDisplayName"));
        assert!(!is_text_field("FULL") && !is_text_field("STR") && !is_text_field("VMAD"));
    }

    /// T-D08-02: a JSON SKSE file (array of « id / text » objects) with a
    /// BOM and CRLF, two lines translated: only the two values change.
    #[test]
    fn a_skse_json_keeps_its_bom_and_line_endings() {
        let f = Formats::builtin();
        let fmt = f.get("skse-strings").unwrap();
        let src = "[\r\n  {\"id\": \"greet\", \"text\": \"Hello\", \"n\": 1.50},\r\n  {\"id\": \"bye\", \"text\": \"Bye\"},\r\n  {\"id\": \"x\", \"text\": \"Keep\"}\r\n]\r\n";
        let data = encode(src, &Used::Utf8Bom);
        let doc = TextDoc::parse(&data, fmt).unwrap();
        assert_eq!(doc.keys(), 3);
        assert_eq!(doc.encoding, Used::Utf8Bom);
        let tm = doc.to_tm(fmt, "strings.json", "SkyrimSE");
        assert_eq!(tm.entries[0].edid, "greet");
        assert_eq!(tm.entries[0].field_sig, "text");
        assert_eq!(tm.entries[0].rec_sig, "SKSE-STRINGS");
        let mut e = tm.entries.clone();
        e[0].trans = "Bonjour".into();
        e[1].trans = "Au revoir".into();
        let out = doc.write(fmt, &e, true);
        let expect = encode(
            &src.replace("\"Hello\"", "\"Bonjour\"")
                .replace("\"Bye\"", "\"Au revoir\""),
            &Used::Utf8Bom,
        );
        assert_eq!(out, expect);
        // Nothing translated: the file itself.
        assert_eq!(doc.write(fmt, &tm.entries, false), data);
        // Its translated copy pairs by field and identifier.
        let copy = TextDoc::parse(&expect, fmt).unwrap();
        assert_eq!(
            copy.values_by_key()[&entry_key("text", "greet")],
            vec!["Bonjour"]
        );
    }

    #[test]
    fn decode_follows_the_mark_then_the_setting() {
        assert_eq!(decode(&utf16le("é"), "utf-8").1, Used::Utf16Le);
        assert_eq!(
            decode("é".as_bytes(), "auto"),
            ("é".to_string(), Used::Utf8)
        );
        assert_eq!(decode(&[0xE9], "auto"), ("é".to_string(), Used::Win1252));
        assert_eq!(
            decode(&[0xEF, 0xBB, 0xBF, b'a'], "auto"),
            ("a".to_string(), Used::Utf8Bom)
        );
        assert_eq!(encode("a", &Used::Utf8Bom), vec![0xEF, 0xBB, 0xBF, b'a']);
        assert_eq!(unescape("a\\nb\\\"c", "json"), "a\nb\"c");
        assert_eq!(escape_value("a\nb\"c", "c"), "a\\nb\\\"c");
        assert_eq!(unescape("a\\n", "none"), "a\\n");
        assert!(text_tags().iter().any(|t| t == "{*}"));
    }
}
