//! The interface translation editor (E-10), ported from XIMOD Architect
//! (`ui/translation.rs`).
//!
//! The user picks a country (flag), the language to translate (ISO 639-3)
//! and a language shown as reference, then fills a three-column table:
//! key (read-only) | label in the displayed language (read-only) |
//! translation. Saving writes `main.ftl` with the structure of the
//! reference file (English) and the metadata header; the window driver
//! lives in `app_translation.rs`.
//!
//! Protected tokens (Fluent placeables `{ $num }`, message references
//! `{ menu-file }`, macros `$MODNAME$`) are never typed by the translator:
//! - at the start or end of the text they are hidden and re-attached on save
//!   (`{ $n } lines` shows `lines`);
//! - in the middle they are shown as compact markers `[[1]]`, `[[2]]`… that
//!   the translator moves within the sentence; a lost or duplicated marker
//!   is shown in red and the key falls back to the label on save.

use crate::ui::theme::ComboField as _;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::i18n::I18n;

/// First line of the metadata block of a translated file.
pub const META_HEADER: &str = "# XIMOD Translator - translation metadata";
/// The same block written by XIMOD Architect (recognised, never written).
const META_HEADER_ARCHITECT: &str = "# XIMOD Architect - translation metadata";
/// Metadata keys (Fluent comments, so the file stays valid). `country` and
/// `endonym` are only read: a language serves several countries.
const META_KEYS: [&str; 6] = [
    "country", "language", "font", "endonym", "langname", "author",
];
/// Recipient of the translations sent from the editor.
pub const PROJECT_MAIL: &str = "plplecuyer@gmail.com";
/// Keys never translated (brand names). The reference file may add others
/// with a `# @untranslatable = key, key` line.
const NON_TRANSLATABLE: &[&str] = &["app-title"];

// ============================================================================
// Fluent file structure
// ============================================================================

/// One item of a `.ftl` file, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FtlItem {
    /// A message: `key = value` and its indented continuation lines.
    Msg {
        key: String,
        value: String,
        cont: Vec<String>,
    },
    /// Anything else (comment, blank line), kept verbatim.
    Line(String),
}

/// The `key` of a message line (`key = …` or `key =`), if it is one.
fn message_key(line: &str) -> Option<(&str, &str)> {
    let first = line.chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let eq = line.find('=')?;
    let key = line[..eq].trim_end();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let rest = &line[eq + 1..];
    Some((key, rest.strip_prefix(' ').unwrap_or(rest)))
}

/// Split a `.ftl` file into messages and other lines.
pub fn parse_items(content: &str) -> Vec<FtlItem> {
    let mut out: Vec<FtlItem> = Vec::new();
    for line in content.lines() {
        if line.starts_with(' ')
            && !line.trim().is_empty()
            && let Some(FtlItem::Msg { cont, .. }) = out.last_mut()
        {
            cont.push(line.to_string());
            continue;
        }
        match message_key(line) {
            Some((key, value)) => out.push(FtlItem::Msg {
                key: key.to_string(),
                value: value.to_string(),
                cont: Vec::new(),
            }),
            None => out.push(FtlItem::Line(line.to_string())),
        }
    }
    out
}

/// A message as written: its value and continuation lines.
pub type MsgBody = (String, Vec<String>);

/// The messages of a file by key.
pub fn messages(content: &str) -> HashMap<String, MsgBody> {
    parse_items(content)
        .into_iter()
        .filter_map(|i| match i {
            FtlItem::Msg { key, value, cont } => Some((key, (value, cont))),
            FtlItem::Line(_) => None,
        })
        .collect()
}

/// Whether a message is shown read-only: several lines or a selector.
fn is_complex(body: &MsgBody) -> bool {
    !body.1.is_empty() || body.0.contains("->")
}

/// The keys never translated: `app-title` and the reference file's
/// `# @untranslatable = …` list.
pub fn untranslatable(reference: &str) -> Vec<String> {
    let mut keys: Vec<String> = NON_TRANSLATABLE.iter().map(|s| s.to_string()).collect();
    for line in reference.lines() {
        if let Some(rest) = line.trim().strip_prefix("# @untranslatable") {
            let list = rest.trim_start().trim_start_matches('=');
            keys.extend(
                list.split(',')
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty()),
            );
        }
    }
    keys
}

/// Whether a line belongs to the metadata block (header or `# @key …`).
pub fn is_metadata_line(line: &str) -> bool {
    let t = line.trim();
    if t == META_HEADER || t == META_HEADER_ARCHITECT {
        return true;
    }
    let Some(rest) = t.strip_prefix("# @") else {
        return false;
    };
    let key = rest.split('=').next().unwrap_or("").trim();
    META_KEYS.contains(&key) || key == "untranslatable"
}

/// The metadata block at the top of a file.
pub fn parse_metadata(content: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("# @") {
            if let Some((k, v)) = rest.split_once('=') {
                let k = k.trim();
                if META_KEYS.contains(&k) {
                    out.insert(k.to_string(), v.trim().to_string());
                }
            }
        } else if !t.is_empty() && !t.starts_with('#') {
            break;
        }
    }
    out
}

// ============================================================================
// Protected tokens
// ============================================================================

/// Byte ranges of the protected tokens of `s`: `{ … }` and `$IDENT$`.
fn find_tokens(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = bytes[i];
        if c == b'{' {
            if let Some(rel) = s[i..].find('}') {
                let mut start = i;
                let mut end = i + rel + 1;
                // « [[{ $n }]] » (a literal marker around a variable) is one
                // token, so it never mixes with the editor's markers.
                if s[..i].ends_with("[[") && s[end..].starts_with("]]") {
                    start -= 2;
                    end += 2;
                }
                tokens.push((start, end));
                i = end;
                continue;
            }
        } else if c == b'$' {
            let rest = &s[i + 1..];
            let rb = rest.as_bytes();
            let mut j = 0;
            while j < rest.len() && (rb[j].is_ascii_alphanumeric() || rb[j] == b'_') {
                j += 1;
            }
            if j > 0 && j < rest.len() && rb[j] == b'$' {
                let end = i + j + 2;
                tokens.push((i, end));
                i = end;
                continue;
            }
        }
        i += s[i..].chars().next().map_or(1, char::len_utf8);
    }
    tokens
}

/// A separator that may sit between a token and the human text.
fn is_sep(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '-' | '\u{2013}' | '\u{2014}' | ':' | '\u{00B7}' | '|' | '/'
        )
}

/// How a label is shown and rebuilt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Analysis {
    display: String,
    prefix: String,
    suffix: String,
    marker_mode: bool,
    tokens: Vec<String>,
}

fn analyze(value: &str) -> Analysis {
    let toks = find_tokens(value);
    if toks.is_empty() {
        return Analysis {
            display: value.to_string(),
            ..Default::default()
        };
    }
    let tokens: Vec<String> = toks.iter().map(|&(s, e)| value[s..e].to_string()).collect();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0;
    for &(s, e) in &toks {
        if s > cursor {
            runs.push((cursor, s));
        }
        cursor = e;
    }
    if cursor < value.len() {
        runs.push((cursor, value.len()));
    }
    let human: Vec<(usize, usize)> = runs
        .into_iter()
        .filter(|&(s, e)| !value[s..e].trim().is_empty())
        .collect();
    if human.len() > 1 {
        return Analysis {
            display: marker_form(value, &tokens),
            marker_mode: true,
            tokens,
            ..Default::default()
        };
    }
    let Some(&(hs, he)) = human.first() else {
        // Only tokens: nothing to translate.
        return Analysis {
            prefix: value.to_string(),
            tokens,
            ..Default::default()
        };
    };
    let run = &value[hs..he];
    let before = toks.iter().any(|&(_, e)| e <= hs);
    let after = toks.iter().any(|&(s, _)| s >= he);
    let start_trim = if before {
        run.trim_start_matches(is_sep)
    } else {
        run.trim_start()
    };
    let start = hs + run.len() - start_trim.len();
    let inner = &value[start..he];
    let end_trim = if after {
        inner.trim_end_matches(is_sep)
    } else {
        inner.trim_end()
    };
    let end = start + end_trim.len();
    Analysis {
        display: value[start..end].to_string(),
        prefix: value[..start].to_string(),
        suffix: value[end..].to_string(),
        marker_mode: false,
        tokens,
    }
}

/// `value` with each token replaced by its marker `[[k]]`, `k` being its
/// rank in `tokens` (a repeated token takes the next unused rank); a token
/// not in `tokens` stays as it is.
fn marker_form(value: &str, tokens: &[String]) -> String {
    let mut used = vec![false; tokens.len()];
    let mut out = String::new();
    let mut cursor = 0;
    for (s, e) in find_tokens(value) {
        out.push_str(&value[cursor..s]);
        let tok = &value[s..e];
        match (0..tokens.len()).find(|&k| !used[k] && tokens[k] == tok) {
            Some(k) => {
                used[k] = true;
                out.push_str(&format!("[[{}]]", k + 1));
            }
            None => out.push_str(tok),
        }
        cursor = e;
    }
    out.push_str(&value[cursor..]);
    out
}

/// The markers `[[n]]` of a text, in order.
/// The markers `[[n]]` of a text: `(start, end, n)` byte ranges, in order.
/// A bracket next to a marker (« [[[1]]] ») is text, not part of it.
fn marker_spans(text: &str) -> Vec<(usize, usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 < b.len() + 1 {
        if b[i] == b'[' && b.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 2 && b.get(j) == Some(&b']') && b.get(j + 1) == Some(&b']') {
                let n = text[i + 2..j].parse().unwrap_or(0);
                out.push((i, j + 2, n));
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The markers `[[n]]` of a text, in order.
fn markers(text: &str) -> Vec<usize> {
    marker_spans(text).into_iter().map(|(_, _, n)| n).collect()
}

/// The first marker that is lost, repeated or unknown in `input` (markers
/// `1..=count` must each appear once).
pub fn marker_problem(input: &str, count: usize) -> Option<usize> {
    let found = markers(input);
    for &n in &found {
        if n == 0 || n > count {
            return Some(n);
        }
    }
    (1..=count).find(|k| found.iter().filter(|&&n| n == *k).count() != 1)
}

// ============================================================================
// Rows
// ============================================================================

/// One row of the table.
#[derive(Debug, Clone)]
pub struct TrEntry {
    pub key: String,
    /// The label (displayed language), written when the translation is empty
    /// or broken.
    pub label_value: String,
    /// The label as shown (tokens hidden or as markers).
    pub display: String,
    /// The translation typed.
    pub input: String,
    prefix: String,
    suffix: String,
    marker_mode: bool,
    tokens: Vec<String>,
    /// A selector or a multi-line message: shown as is, read-only.
    pub complex: Option<String>,
}

impl TrEntry {
    /// A row from the label and the existing translation, if any.
    pub fn new(key: &str, label: &MsgBody, target: Option<&MsgBody>) -> Self {
        if is_complex(label) || target.is_some_and(is_complex) {
            let shown = target.unwrap_or(label);
            let mut raw = shown.0.clone();
            for c in &shown.1 {
                raw.push_str(" \u{23CE} ");
                raw.push_str(c.trim());
            }
            return Self {
                key: key.to_string(),
                label_value: label.0.clone(),
                display: label.0.clone(),
                input: String::new(),
                prefix: String::new(),
                suffix: String::new(),
                marker_mode: false,
                tokens: Vec::new(),
                complex: Some(raw),
            };
        }
        let mut a = analyze(&label.0);
        let mut input = String::new();
        if let Some((t, _)) = target {
            if a.tokens.is_empty() {
                input = t.clone();
            } else if a.marker_mode {
                input = marker_form(t, &a.tokens);
            } else {
                let ta = analyze(t);
                let toks = |x: &str| -> Vec<String> {
                    find_tokens(x)
                        .iter()
                        .map(|&(s, e)| x[s..e].to_string())
                        .collect()
                };
                if !ta.marker_mode
                    && toks(&ta.prefix) == toks(&a.prefix)
                    && toks(&ta.suffix) == toks(&a.suffix)
                {
                    // Same tokens on the same sides: the translation keeps
                    // its own separators (« ab in: » vs « dans : »).
                    a.prefix = ta.prefix;
                    a.suffix = ta.suffix;
                    input = ta.display;
                } else {
                    // The translation places the tokens elsewhere: markers on
                    // both sides.
                    a.display = marker_form(&label.0, &a.tokens);
                    a.marker_mode = true;
                    a.prefix.clear();
                    a.suffix.clear();
                    input = marker_form(t, &a.tokens);
                }
            }
        }
        Self {
            key: key.to_string(),
            label_value: label.0.clone(),
            display: a.display,
            input,
            prefix: a.prefix,
            suffix: a.suffix,
            marker_mode: a.marker_mode,
            tokens: a.tokens,
            complex: None,
        }
    }

    /// Whether the row has a translation.
    pub fn translated(&self) -> bool {
        self.complex.is_some() || !self.input.trim().is_empty()
    }

    /// The marker problem of the row, if any.
    pub fn problem(&self) -> Option<usize> {
        if !self.marker_mode || self.input.trim().is_empty() {
            return None;
        }
        marker_problem(&self.input, self.tokens.len())
    }

    /// The value written for this key: the translation with its tokens
    /// re-attached, or the label when it is empty or has a marker problem.
    pub fn value(&self) -> String {
        let input = self.input.trim();
        if input.is_empty() || self.problem().is_some() {
            return self.label_value.clone();
        }
        if self.tokens.is_empty() {
            return input.to_string();
        }
        if self.marker_mode {
            let mut out = String::new();
            let mut cursor = 0;
            for (start, end, n) in marker_spans(input) {
                out.push_str(&input[cursor..start]);
                match self.tokens.get(n.wrapping_sub(1)) {
                    Some(tok) if n >= 1 => out.push_str(tok),
                    _ => out.push_str(&input[start..end]),
                }
                cursor = end;
            }
            out.push_str(&input[cursor..]);
            out
        } else {
            format!("{}{}{}", self.prefix, input, self.suffix)
        }
    }

    /// Number of markers to keep.
    pub fn marker_count(&self) -> usize {
        if self.marker_mode {
            self.tokens.len()
        } else {
            0
        }
    }
}

/// The metadata written at the top of a saved file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub language: String,
    pub langname: String,
    pub font: String,
    pub author: String,
}

/// Build the rows: every key of the reference file but the untranslatable
/// ones, labelled in the displayed language (the reference when it lacks
/// the key), pre-filled from the target file.
pub fn build_entries(reference: &str, displayed: &str, target: &str) -> Vec<TrEntry> {
    let hidden = untranslatable(reference);
    let shown = messages(displayed);
    let existing = messages(target);
    parse_items(reference)
        .into_iter()
        .filter_map(|i| match i {
            FtlItem::Msg { key, value, cont } if !hidden.contains(&key) => {
                let reference_body = (value, cont);
                let label = shown.get(&key).unwrap_or(&reference_body);
                let e = TrEntry::new(&key, label, existing.get(&key));
                // Nothing to translate (only tokens): kept as it is.
                (e.complex.is_some() || !e.display.trim().is_empty()).then_some(e)
            }
            _ => None,
        })
        .collect()
}

/// The text of the saved file: the metadata block, then the reference file
/// line by line (order, comments, blank lines), each message replaced by
/// its translation. Keys outside the table (untranslatable, read-only)
/// keep the target's text, else the reference's.
pub fn render_file(reference: &str, target: &str, entries: &[TrEntry], meta: &Meta) -> String {
    let values: HashMap<&str, String> = entries
        .iter()
        .filter(|e| e.complex.is_none())
        .map(|e| (e.key.as_str(), e.value()))
        .collect();
    let existing = messages(target);
    let mut out = String::new();
    out.push_str(META_HEADER);
    out.push('\n');
    out.push_str(&format!("# @language = {}\n", meta.language));
    out.push_str(&format!("# @font = {}\n", meta.font));
    out.push_str(&format!("# @langname = {}\n", meta.langname));
    out.push_str(&format!("# @author = {}\n", meta.author));
    out.push('\n');
    // The target's own comments when it follows the reference message for
    // message (a French file keeps its French comments).
    let body_items = |t: &str| -> Vec<FtlItem> {
        parse_items(t)
            .into_iter()
            .filter(|i| !matches!(i, FtlItem::Line(l) if is_metadata_line(l)))
            .skip_while(|i| matches!(i, FtlItem::Line(l) if l.trim().is_empty()))
            .collect()
    };
    let (ref_items, tgt_items) = (body_items(reference), body_items(target));
    let shape = |items: &[FtlItem]| -> Vec<Option<String>> {
        items
            .iter()
            .map(|i| match i {
                FtlItem::Msg { key, .. } => Some(key.clone()),
                FtlItem::Line(l) => (!l.trim().is_empty()).then(|| "#".to_string()),
            })
            .collect()
    };
    let own_comments = shape(&ref_items) == shape(&tgt_items);
    let mut started = false;
    for (n, item) in ref_items.into_iter().enumerate() {
        match item {
            FtlItem::Line(line) => {
                let line = match (&tgt_items[..], own_comments) {
                    (t, true) => match &t[n] {
                        FtlItem::Line(own) => own.clone(),
                        FtlItem::Msg { .. } => line,
                    },
                    _ => line,
                };
                if is_metadata_line(&line) {
                    continue;
                }
                if !started && line.trim().is_empty() {
                    continue;
                }
                started = true;
                out.push_str(&line);
                out.push('\n');
            }
            FtlItem::Msg { key, value, cont } => {
                started = true;
                let (v, c) = match values.get(key.as_str()) {
                    Some(v) => (v.clone(), Vec::new()),
                    None => existing
                        .get(&key)
                        .cloned()
                        .unwrap_or((value.clone(), cont.clone())),
                };
                if v.is_empty() {
                    out.push_str(&format!("{key} =\n"));
                } else {
                    out.push_str(&format!("{key} = {v}\n"));
                }
                for line in c {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }
    out
}

// ============================================================================
// Side files, package, mail
// ============================================================================

/// The country endonym recorded in `<dir>/countryEndonyms.tsv`
/// (`<ISO 3166-1 alpha-3>\t<ISO 639-3>\t<endonym>`).
pub fn read_tsv_country_endonym(lang_dir: &Path, country: &str) -> Option<String> {
    if country.is_empty() {
        return None;
    }
    let text = std::fs::read_to_string(lang_dir.join("countryEndonyms.tsv")).ok()?;
    text.lines().find_map(|line| {
        let mut it = line.split('\t');
        let c = it.next()?;
        let _lang = it.next()?;
        let endo = it.next()?.trim();
        (c.eq_ignore_ascii_case(country) && !endo.is_empty()).then(|| endo.to_string())
    })
}

/// Insert or update the row of `country` in `<dir>/countryEndonyms.tsv`.
pub fn upsert_tsv(
    lang_dir: &Path,
    country: &str,
    lang: &str,
    endonym: &str,
) -> std::io::Result<()> {
    let path = lang_dir.join("countryEndonyms.tsv");
    let mut rows: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(&path) {
        for line in text.lines() {
            let mut it = line.split('\t');
            if let (Some(a3), Some(_), Some(endo)) = (it.next(), it.next(), it.next()) {
                rows.insert(a3.trim().to_string(), endo.to_string());
            }
        }
    }
    rows.insert(
        country.to_string(),
        endonym.replace(['\t', '\r', '\n'], " "),
    );
    let out: String = rows
        .iter()
        .map(|(a3, e)| format!("{a3}\t{lang}\t{e}\n"))
        .collect();
    std::fs::write(path, out)
}

/// `ximod-translator_<code>.zip` with `locales/<code>/main.ftl` and, when
/// present, `countryEndonyms.tsv` — never the ISO data files.
pub fn build_zip(lang_dir: &Path, code: &str, out: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for name in ["main.ftl", "countryEndonyms.tsv"] {
        let p = lang_dir.join(name);
        if p.is_file() {
            zip.start_file(format!("locales/{code}/{name}"), opts)?;
            zip.write_all(&std::fs::read(&p)?)?;
        }
    }
    zip.finish()?;
    Ok(())
}

/// Where a language is saved (D-35): the user's copy when it exists, else
/// `assets/locales` when it is writable, else `<config>/locales`.
pub fn choose_save_dir(
    assets: &Path,
    user: Option<&Path>,
    code: &str,
    writable: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(u) = user
        && (u.join(code).join("main.ftl").is_file() || !writable(assets))
    {
        return u.join(code);
    }
    assets.join(code)
}

/// Whether files can be created in `dir`.
pub fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(".ximod-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Percent-encoding for a `mailto:` URL.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The pre-filled mail of a translation.
pub fn mailto(langname: &str, code: &str, author: &str, zip: &Path) -> String {
    let subject = format!("XIMOD Translator — traduction {langname} ({code})");
    let body = format!(
        "Language: {langname} ({code})\nAuthor: {author}\n\nPlease attach: {}",
        zip.display()
    );
    format!(
        "mailto:{PROJECT_MAIL}?subject={}&body={}",
        urlencode(&subject),
        urlencode(&body)
    )
}

// ============================================================================
// Editor state and window body
// ============================================================================

/// What the body asks the driver to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrAction {
    PickCountry,
    SetTarget(String),
    SetDisplayed(String),
    BrowseFont,
    GoogleFonts,
    Save,
    Reload,
    Send,
    Close,
}

/// An action waiting for « discard the unsaved changes? ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Reload,
    Close,
    Target(String),
}

/// The editor state (one window).
#[derive(Debug, Default)]
pub struct TrEditor {
    pub country: String,
    pub target: String,
    pub displayed: String,
    pub endonym: String,
    pub endonym_loaded: String,
    pub endonym_authoritative: bool,
    pub lang_endonym: String,
    pub lang_endonym_loaded: String,
    pub author: String,
    pub font: String,
    pub font_loaded: String,
    pub entries: Vec<TrEntry>,
    /// Visible rows (indices into `entries`), rebuilt when the search or the
    /// filter changes — not while typing, so a row being translated stays.
    pub view: Vec<usize>,
    pub search: String,
    pub only_untranslated: bool,
    view_key: Option<(String, bool, usize)>,
    /// Cursor, an index into `view`.
    pub cursor: usize,
    scroll_to: Option<Option<egui::Align>>,
    offset: f32,
    viewport_h: f32,
    pub message: String,
    pub dirty: bool,
    pub confirm: Option<Pending>,
    edit_origin: Option<(usize, String)>,
    trans_focused: bool,
    focus_search: bool,
    focus_row: Option<usize>,
    /// Where the next save writes.
    pub save_path: PathBuf,
}

/// What the body needs from the application.
pub struct Feed<'a> {
    pub i18n: &'a I18n,
    pub flag: Option<PathBuf>,
    /// `Pays : <name> (<A3>)`.
    pub country_label: String,
    /// Languages offered for translation: `(code, label)`.
    pub targets: Vec<(String, String)>,
    /// Languages that can be displayed: `(code, label)`.
    pub displayed: Vec<(String, String)>,
    pub can_translate: bool,
}

const RED: egui::Color32 = egui::Color32::from_rgb(210, 60, 60);

/// A left-aligned table cell of a fixed size.
fn cell(ui: &mut egui::Ui, w: f32, h: f32, widget: impl egui::Widget) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(w, h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(w, h));
            ui.add(widget)
        },
    )
    .inner
}

impl TrEditor {
    /// Rebuild the visible rows if the search or the filter changed.
    pub fn refresh_view(&mut self, force: bool) {
        let key = (
            self.search.clone(),
            self.only_untranslated,
            self.entries.len(),
        );
        if !force && self.view_key.as_ref() == Some(&key) {
            return;
        }
        let needle = self.search.to_lowercase();
        let current = self.view.get(self.cursor).copied();
        self.view = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !self.only_untranslated || !e.translated())
            .filter(|(_, e)| {
                needle.is_empty()
                    || e.key.to_lowercase().contains(&needle)
                    || e.display.to_lowercase().contains(&needle)
                    || e.input.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        self.cursor = current
            .and_then(|c| self.view.iter().position(|&i| i == c))
            .unwrap_or(0);
        self.view_key = Some(key);
    }

    /// `(translated, total)` over the editable rows.
    pub fn progress(&self) -> (usize, usize) {
        let total = self.entries.iter().filter(|e| e.complex.is_none()).count();
        let done = self
            .entries
            .iter()
            .filter(|e| e.complex.is_none() && e.translated())
            .count();
        (done, total)
    }

    /// The translations as full values (kept across a change of the
    /// displayed language).
    pub fn values(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .filter(|e| e.complex.is_none() && !e.input.trim().is_empty() && e.problem().is_none())
            .map(|e| (e.key.clone(), e.value()))
            .collect()
    }

    /// Keyboard: rows, Enter, Tab / Shift+Tab, Escape, Ctrl+S, Ctrl+F.
    fn keys(&mut self, ctx: &egui::Context, page: usize, actions: &mut Vec<TrAction>) {
        if self.confirm.is_some() {
            return;
        }
        let n = self.view.len();
        let editing = ctx.wants_keyboard_input();
        ctx.input_mut(|i| {
            use egui::{Align, Key, Modifiers};
            if i.consume_key(Modifiers::COMMAND, Key::S) {
                actions.push(TrAction::Save);
            }
            if i.consume_key(Modifiers::COMMAND, Key::F) {
                self.focus_search = true;
            }
            if n == 0 {
                return;
            }
            if self.cursor >= n {
                self.cursor = n - 1;
            }
            // Escape in a translation: back to the text before the edit.
            if self.trans_focused
                && i.key_pressed(Key::Escape)
                && let Some((idx, text)) = self.edit_origin.take()
                && let Some(e) = self.entries.get_mut(idx)
            {
                e.input = text;
            }
            if self.trans_focused || !editing {
                if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                    self.cursor = self.cursor.saturating_sub(1);
                    self.focus_row = Some(self.cursor);
                    self.scroll_to = Some(None);
                }
                if i.consume_key(Modifiers::NONE, Key::Tab) {
                    self.cursor = (self.cursor + 1).min(n - 1);
                    self.focus_row = Some(self.cursor);
                    self.scroll_to = Some(None);
                }
            }
            if editing {
                return;
            }
            let page = page.max(1);
            let before = self.cursor;
            let mut align = None;
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                self.cursor = (self.cursor + 1).min(n - 1);
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                self.cursor = self.cursor.saturating_sub(1);
            }
            if i.consume_key(Modifiers::NONE, Key::PageDown) {
                self.cursor = (self.cursor + page).min(n - 1);
                align = Some(Align::TOP);
            }
            if i.consume_key(Modifiers::NONE, Key::PageUp) {
                self.cursor = self.cursor.saturating_sub(page);
                align = Some(Align::TOP);
            }
            if i.consume_key(Modifiers::NONE, Key::Home) {
                self.cursor = 0;
                align = Some(Align::TOP);
            }
            if i.consume_key(Modifiers::NONE, Key::End) {
                self.cursor = n - 1;
                align = Some(Align::BOTTOM);
            }
            if self.cursor != before || align.is_some() {
                self.scroll_to = Some(align);
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                self.focus_row = Some(self.cursor);
            }
        });
    }

    /// The scroll offset that shows row `row` (`None` = minimal move).
    fn offset_for(&self, row: usize, row_h: f32, align: Option<egui::Align>) -> f32 {
        let top = row as f32 * row_h;
        match align {
            Some(egui::Align::TOP) => top,
            Some(egui::Align::BOTTOM) => (top + row_h - self.viewport_h).max(0.0),
            _ => {
                if top < self.offset {
                    top
                } else if top + row_h > self.offset + self.viewport_h {
                    top + row_h - self.viewport_h
                } else {
                    self.offset
                }
            }
        }
    }
}

/// The window body; returns the actions for the driver.
pub fn body(ui: &mut egui::Ui, ed: &mut TrEditor, f: &Feed) -> Vec<TrAction> {
    let i18n = f.i18n;
    let mut actions = Vec::new();
    ed.refresh_view(false);
    let preview = egui::FontFamily::Name(crate::fonts::PREVIEW_FAMILY.into());
    let body_size = ui.style().text_styles[&egui::TextStyle::Body].size;
    let preview_font = egui::FontId::new(body_size, preview.clone());
    let lang_name = |code: &str| {
        f.displayed
            .iter()
            .chain(f.targets.iter())
            .find(|(c, _)| c == code)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| i18n.display_name(code))
    };

    // ---- Identity: flag, languages, endonyms, author, font --------------
    egui::TopBottomPanel::top("tr_top").show_inside(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            let size = egui::vec2(108.0, 72.0);
            let resp = match &f.flag {
                Some(p) => ui.add_sized(
                    size,
                    egui::ImageButton::new(
                        egui::Image::from_uri(format!("file://{}", p.display()))
                            .fit_to_exact_size(size),
                    ),
                ),
                None => ui.add_sized(
                    size,
                    egui::Button::new(egui::RichText::new("\u{1F3F3}").size(26.0)),
                ),
            };
            if resp.on_hover_text(i18n.t("tr-pick-country")).clicked() {
                actions.push(TrAction::PickCountry);
            }
            ui.add_space(8.0);
            egui::Grid::new("tr_ident")
                .num_columns(4)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label(i18n.t("tr-country"));
                    ui.label(egui::RichText::new(&f.country_label).strong());
                    ui.label(i18n.t("tr-endonym-country"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut ed.endonym)
                                .desired_width(220.0)
                                .font(preview_font.clone()),
                        )
                        .changed()
                    {
                        ed.dirty = true;
                    }
                    ui.end_row();

                    ui.label(i18n.t("tr-language"));
                    let current = if ed.target.is_empty() {
                        "—".to_string()
                    } else {
                        lang_name(&ed.target)
                    };
                    egui::ComboBox::from_id_salt("tr_target")
                        .selected_text(current)
                        .width(220.0)
                        .height(300.0)
                        .show_field(ui, |ui| {
                            for (code, label) in &f.targets {
                                if ui.selectable_label(ed.target == *code, label).clicked()
                                    && ed.target != *code
                                {
                                    actions.push(TrAction::SetTarget(code.clone()));
                                }
                            }
                        });
                    ui.label(i18n.t("tr-endonym-lang"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut ed.lang_endonym)
                                .desired_width(220.0)
                                .font(preview_font.clone()),
                        )
                        .changed()
                    {
                        ed.dirty = true;
                    }
                    ui.end_row();

                    ui.label(i18n.t("tr-displayed"));
                    egui::ComboBox::from_id_salt("tr_displayed")
                        .selected_text(lang_name(&ed.displayed))
                        .width(220.0)
                        .height(300.0)
                        .show_field(ui, |ui| {
                            for (code, label) in &f.displayed {
                                if ui.selectable_label(ed.displayed == *code, label).clicked()
                                    && ed.displayed != *code
                                {
                                    actions.push(TrAction::SetDisplayed(code.clone()));
                                }
                            }
                        });
                    ui.label(i18n.t("tr-author"));
                    if ui
                        .add(egui::TextEdit::singleline(&mut ed.author).desired_width(220.0))
                        .changed()
                    {
                        ed.dirty = true;
                    }
                    ui.end_row();

                    ui.label(i18n.t("tr-font"));
                    let shown = if ed.font.is_empty() {
                        i18n.t("tr-no-font")
                    } else {
                        ed.font.clone()
                    };
                    ui.add(
                        egui::Label::new(egui::RichText::new(shown).monospace().small()).truncate(),
                    );
                    ui.label("");
                    ui.horizontal(|ui| {
                        if ui.button(i18n.t("tr-font-browse")).clicked() {
                            actions.push(TrAction::BrowseFont);
                        }
                        if ui.button(i18n.t("tr-font-google")).clicked() {
                            actions.push(TrAction::GoogleFonts);
                        }
                    });
                    ui.end_row();
                });
        });
        if !f.can_translate && !ed.target.is_empty() {
            ui.colored_label(RED, i18n.t("tr-lang-not-iso"));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut ed.search)
                    .hint_text(i18n.t("tr-search"))
                    .desired_width(300.0),
            );
            if ed.focus_search {
                search.request_focus();
                ed.focus_search = false;
            }
            let (done, total) = ed.progress();
            let mut only = ed.only_untranslated;
            let label = i18n.t_args(
                "tr-only-untranslated",
                &[
                    ("n", &(total - done).to_string()),
                    ("total", &total.to_string()),
                ],
            );
            if ui.checkbox(&mut only, label).changed() {
                ed.only_untranslated = only;
                ed.refresh_view(true);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                ui.label(
                    egui::RichText::new(i18n.t_args(
                        "tr-progress",
                        &[
                            ("n", &done.to_string()),
                            ("total", &total.to_string()),
                            ("pct", &pct.to_string()),
                        ],
                    ))
                    .weak(),
                );
            });
        });
        ui.add_space(2.0);
    });

    // ---- Buttons, preview, hints ------------------------------------------
    egui::TopBottomPanel::bottom("tr_bottom").show_inside(ui, |ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(i18n.t("tr-token-hint")).small().weak());
        ui.label(egui::RichText::new(i18n.t("tr-keys-hint")).small().weak());
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(i18n.t("tr-preview")).strong());
            if let Some(e) = ed.view.get(ed.cursor).and_then(|&i| ed.entries.get(i)) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{} — {}", e.key, e.display)).weak(),
                    )
                    .wrap(),
                );
                let text = match &e.complex {
                    Some(raw) => raw.clone(),
                    None if e.input.is_empty() => String::new(),
                    None => e.input.clone(),
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .font(egui::FontId::new(body_size + 4.0, preview.clone())),
                    )
                    .wrap(),
                );
                if let Some(n) = e.problem() {
                    ui.colored_label(RED, i18n.t_args("tr-token-error", &[("n", &n.to_string())]));
                }
            }
            ui.label(
                egui::RichText::new(i18n.t_arg(
                    "tr-saved-where",
                    "path",
                    &ed.save_path.display().to_string(),
                ))
                .small()
                .weak(),
            );
        });
        if !ed.message.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&ed.message).color(egui::Color32::from_rgb(200, 120, 40)),
                )
                .wrap(),
            );
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(f.can_translate, |ui| {
                if ui
                    .button(format!(
                        "{}  {}",
                        i18n.t("tr-save"),
                        crate::ui::app::key_text(
                            ui.ctx(),
                            i18n,
                            egui::Modifiers::COMMAND,
                            egui::Key::S
                        )
                    ))
                    .clicked()
                {
                    actions.push(TrAction::Save);
                }
            });
            if ui.button(i18n.t("tr-reload")).clicked() {
                if ed.dirty {
                    ed.confirm = Some(Pending::Reload);
                } else {
                    actions.push(TrAction::Reload);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(i18n.t("btn-close")).clicked() {
                    if ed.dirty {
                        ed.confirm = Some(Pending::Close);
                    } else {
                        actions.push(TrAction::Close);
                    }
                }
                ui.add_enabled_ui(f.can_translate, |ui| {
                    if ui
                        .button(i18n.t("tr-send"))
                        .on_hover_text(i18n.t("tr-send-tip"))
                        .clicked()
                    {
                        actions.push(TrAction::Send);
                    }
                });
            });
        });
        ui.add_space(4.0);
    });

    // ---- The table ----------------------------------------------------------
    egui::CentralPanel::default().show_inside(ui, |ui| {
        let row_h = ui.spacing().interact_size.y + 2.0;
        let spacing = ui.spacing().item_spacing;
        let page = ((ed.viewport_h / (row_h + spacing.y)).floor() as usize).max(1);
        ed.keys(ui.ctx(), page, &mut actions);
        let key_w = 220.0_f32;
        let col_w = ((ui.available_width() - key_w - spacing.x * 2.0 - 16.0) / 2.0).max(140.0);
        let lang_label = lang_name(&ed.displayed);
        let trans_label = if ed.target.is_empty() {
            String::new()
        } else {
            lang_name(&ed.target)
        };
        ui.horizontal(|ui| {
            cell(
                ui,
                key_w,
                row_h,
                egui::Label::new(egui::RichText::new(i18n.t("tr-col-key")).strong()),
            );
            cell(
                ui,
                col_w,
                row_h,
                egui::Label::new(
                    egui::RichText::new(i18n.t_arg("tr-col-label", "lang", &lang_label)).strong(),
                ),
            );
            cell(
                ui,
                col_w,
                row_h,
                egui::Label::new(
                    egui::RichText::new(i18n.t_arg("tr-col-trans", "lang", &trans_label)).strong(),
                ),
            );
        });
        ui.separator();
        let n = ed.view.len();
        let mut area = egui::ScrollArea::vertical()
            .id_salt("tr_rows")
            .auto_shrink([false, false]);
        if let Some(align) = ed.scroll_to.take() {
            area = area.vertical_scroll_offset(ed.offset_for(ed.cursor, row_h + spacing.y, align));
        }
        let sel_bg = ui.visuals().selection.bg_fill;
        let sel_fg = ui.visuals().selection.stroke.color;
        let mut trans_focused = false;
        let out = area.show_rows(ui, row_h, n, |ui, range| {
            for vi in range {
                let idx = ed.view[vi];
                let selected = vi == ed.cursor;
                ui.horizontal(|ui| {
                    let e = &mut ed.entries[idx];
                    let key = cell(ui, key_w, row_h, {
                        let mut t = egui::RichText::new(&e.key).monospace();
                        if selected {
                            t = t.background_color(sel_bg).color(sel_fg);
                        }
                        egui::Label::new(t).truncate().sense(egui::Sense::click())
                    })
                    .on_hover_text(&e.key);
                    if key.clicked() {
                        ed.cursor = vi;
                    }
                    cell(ui, col_w, row_h, egui::Label::new(&e.display).truncate())
                        .on_hover_text(&e.label_value);
                    match &e.complex {
                        Some(raw) => {
                            cell(
                                ui,
                                col_w,
                                row_h,
                                egui::Label::new(egui::RichText::new(raw).weak().italics())
                                    .truncate(),
                            )
                            .on_hover_text(i18n.t("tr-selector-readonly"));
                        }
                        None => {
                            let problem = e.problem();
                            let mut edit = egui::TextEdit::singleline(&mut e.input)
                                .id(egui::Id::new(("tr_input", idx)))
                                .desired_width(col_w)
                                .font(preview_font.clone());
                            if problem.is_some() {
                                edit = edit.text_color(RED);
                            }
                            let resp = ui.add_sized([col_w, row_h], edit);
                            let resp = match problem {
                                Some(k) => resp.on_hover_text(
                                    i18n.t_args("tr-token-error", &[("n", &k.to_string())]),
                                ),
                                None => resp,
                            };
                            if resp.gained_focus() {
                                ed.edit_origin = Some((idx, e.input.clone()));
                            }
                            if resp.changed() {
                                ed.dirty = true;
                            }
                            if resp.has_focus() {
                                trans_focused = true;
                                ed.cursor = vi;
                            }
                            if ed.focus_row == Some(vi) {
                                resp.request_focus();
                                ed.focus_row = None;
                                ed.edit_origin = Some((idx, e.input.clone()));
                            }
                        }
                    }
                });
            }
        });
        // A row asked for focus but drawn off screen: bring it in next frame.
        if let Some(r) = ed.focus_row {
            if r < n {
                ed.scroll_to = Some(None);
            } else {
                ed.focus_row = None;
            }
        }
        ed.trans_focused = trans_focused;
        ed.offset = out.state.offset.y;
        ed.viewport_h = out.inner_rect.height();
    });

    // ---- « Discard the unsaved changes? » -----------------------------------
    if let Some(pending) = ed.confirm.clone() {
        let mut answer = None;
        egui::Window::new(i18n.t("tr-title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(i18n.t("tr-reload-confirm"));
                ui.horizontal(|ui| {
                    if ui.button(i18n.t("btn-yes")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        answer = Some(true);
                    }
                    if ui.button(i18n.t("btn-no")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => {
                ed.confirm = None;
                actions.push(match pending {
                    Pending::Reload => TrAction::Reload,
                    Pending::Close => TrAction::Close,
                    Pending::Target(c) => TrAction::SetTarget(c),
                });
                ed.dirty = false;
            }
            Some(false) => ed.confirm = None,
            None => {}
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: &str) -> MsgBody {
        (v.to_string(), Vec::new())
    }

    #[test]
    fn tokens_hidden_at_the_edges_and_reattached() {
        let e = TrEntry::new("k", &body("{ $n } lines exported"), None);
        assert_eq!(e.display, "lines exported");
        let mut e2 = e.clone();
        e2.input = "Zeilen exportiert".into();
        assert_eq!(e2.value(), "{ $n } Zeilen exportiert");
        let e = TrEntry::new("k", &body("$MODNAME$ – Mod name"), None);
        assert_eq!(e.display, "Mod name");
        let mut e = e;
        e.input = "Nom du mod".into();
        assert_eq!(e.value(), "$MODNAME$ – Nom du mod");
        // An existing translation with the token at the same edge.
        let e = TrEntry::new("k", &body("{ $n } lines"), Some(&body("{ $n } Zeilen")));
        assert_eq!(e.input, "Zeilen");
        // The translation's own separators are kept.
        let e = TrEntry::new(
            "k",
            &body("Déposez-les dans : { $dir }"),
            Some(&body("Legen Sie sie ab in: { $dir }")),
        );
        assert_eq!(e.input, "Legen Sie sie ab in");
        assert_eq!(e.value(), "Legen Sie sie ab in: { $dir }");
    }

    /// T-E10-02: markers moved, then one lost.
    #[test]
    fn markers_move_and_a_lost_one_falls_back() {
        let src = "{ $lines } replies · { $done } translated · { $unvoiced } unvoiced · { $npcs } NPCs · filter: { $filter }";
        let mut e = TrEntry::new("dialog-status", &body(src), None);
        assert_eq!(
            e.display,
            "[[1]] replies · [[2]] translated · [[3]] unvoiced · [[4]] NPCs · filter: [[5]]"
        );
        e.input =
            "[[3]] ohne Stimme · [[1]] Antworten · [[2]] übersetzt · [[4]] NPCs · Filter: [[5]]"
                .into();
        assert_eq!(e.problem(), None);
        assert_eq!(
            e.value(),
            "{ $unvoiced } ohne Stimme · { $lines } Antworten · { $done } übersetzt · { $npcs } NPCs · Filter: { $filter }"
        );
        e.input =
            "[[3]] ohne Stimme · [[1]] Antworten · übersetzt · [[4]] NPCs · Filter: [[5]]".into();
        assert_eq!(e.problem(), Some(2));
        assert_eq!(e.value(), src);
        e.input = "[[1]] [[1]] [[2]] [[3]] [[4]] [[5]]".into();
        assert_eq!(e.problem(), Some(1));
        e.input = "[[1]] [[2]] [[3]] [[4]] [[5]] [[6]]".into();
        assert_eq!(e.problem(), Some(6));
        // Pre-filled from a translation whose tokens are in another order.
        let t =
            "{ $done } übersetzt, { $lines } Antworten · { $unvoiced } · { $npcs } · { $filter }";
        let e = TrEntry::new("dialog-status", &body(src), Some(&body(t)));
        assert_eq!(
            e.input,
            "[[2]] übersetzt, [[1]] Antworten · [[3]] · [[4]] · [[5]]"
        );
        assert_eq!(e.value(), t);
        // A bracket right before a marker stays text.
        let e = TrEntry::new(
            "k",
            &body("Bases compiled [{ $lang }]"),
            Some(&body("Bases compilées [{ $lang }]")),
        );
        assert_eq!(e.value(), "Bases compilées [{ $lang }]");
    }

    #[test]
    fn empty_and_complex_rows() {
        let e = TrEntry::new("k", &body("Open…"), None);
        assert!(!e.translated());
        assert_eq!(e.value(), "Open…");
        let sel = (
            "{ $n ->".to_string(),
            vec![
                "    [one] one line".to_string(),
                "   *[other] { $n } lines".to_string(),
                "}".to_string(),
            ],
        );
        // "}" alone is not indented: the parser keeps it apart, but a selector
        // on the first line is enough to make the row read-only.
        let e = TrEntry::new("k", &sel, None);
        assert!(e.complex.is_some());
        assert!(e.translated());
    }

    /// T-E10-03: structure, comments and header kept; empty keys take the
    /// label; untranslatable and read-only keys keep the target's text.
    #[test]
    fn saved_file_keeps_the_reference_structure() {
        let reference = "# XIMOD Translator - translation metadata\n# @language = eng\n# @langname = English\n# @author = XIMOD Team\n\napp-title = XIMOD Translator\n\n# --- Menus\nmenu-file = File\nmenu-open = Open…\nstatus = { $n } lines\nundo-done =\n    { $n ->\n        [one] one\n       *[other] many\n    }\n";
        let displayed = "menu-file = Fichier\nmenu-open = Ouvrir…\nstatus = { $n } lignes\n";
        let target = "# @language = deu\napp-title = XIMOD Übersetzer\nmenu-file = Datei\nundo-done =\n    { $n ->\n        [one] eins\n       *[other] viele\n    }\n";
        let mut entries = build_entries(reference, displayed, target);
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["menu-file", "menu-open", "status", "undo-done"]);
        assert_eq!(entries[0].input, "Datei");
        assert_eq!(entries[1].display, "Ouvrir…");
        entries[2].input = "Zeilen".into();
        let meta = Meta {
            language: "deu".into(),
            langname: "Deutsch".into(),
            font: "Noto_Sans/static/NotoSans-Regular.ttf".into(),
            author: "Patrice".into(),
        };
        let out = render_file(reference, target, &entries, &meta);
        let expected = "# XIMOD Translator - translation metadata\n# @language = deu\n# @font = Noto_Sans/static/NotoSans-Regular.ttf\n# @langname = Deutsch\n# @author = Patrice\n\napp-title = XIMOD Übersetzer\n\n# --- Menus\nmenu-file = Datei\nmenu-open = Ouvrir…\nstatus = { $n } Zeilen\nundo-done =\n    { $n ->\n        [one] eins\n       *[other] viele\n    }\n";
        assert_eq!(out, expected);
        // Saved twice: identical (no blank lines piling up).
        let again = render_file(
            reference,
            &out,
            &build_entries(reference, displayed, &out),
            &meta,
        );
        assert_eq!(again, expected);
        assert_eq!(
            parse_metadata(&out).get("author").map(String::as_str),
            Some("Patrice")
        );
        // The result is valid Fluent.
        assert!(fluent_bundle::FluentResource::try_new(out).is_ok());
    }

    #[test]
    fn untranslatable_list_and_metadata_lines() {
        let r = "# @untranslatable = app-version, foo\napp-title = X\n";
        assert_eq!(untranslatable(r), ["app-title", "app-version", "foo"]);
        assert!(is_metadata_line("# @author ="));
        assert!(is_metadata_line("# XIMOD Architect - translation metadata"));
        assert!(!is_metadata_line("# --- Menus"));
    }

    /// T-E10-04: the package holds the language folder only.
    #[test]
    fn tsv_zip_and_mail() {
        let dir = std::env::temp_dir().join(format!("ximod_tr_{}", std::process::id()));
        let lang = dir.join("deu");
        std::fs::create_dir_all(&lang).unwrap();
        std::fs::write(lang.join("main.ftl"), "menu-file = Datei\n").unwrap();
        upsert_tsv(&lang, "DEU", "deu", "Deutschland").unwrap();
        upsert_tsv(&lang, "CHE", "deu", "Schweiz\t").unwrap();
        upsert_tsv(&lang, "DEU", "deu", "Bundesrepublik Deutschland").unwrap();
        assert_eq!(
            std::fs::read_to_string(lang.join("countryEndonyms.tsv")).unwrap(),
            "CHE\tdeu\tSchweiz \nDEU\tdeu\tBundesrepublik Deutschland\n"
        );
        assert_eq!(read_tsv_country_endonym(&lang, "FRA").as_deref(), None);
        assert_eq!(
            read_tsv_country_endonym(&lang, "DEU").as_deref(),
            Some("Bundesrepublik Deutschland")
        );
        let zip_path = dir.join("ximod-translator_deu.zip");
        build_zip(&lang, "deu", &zip_path).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .collect();
        assert_eq!(
            names,
            ["locales/deu/main.ftl", "locales/deu/countryEndonyms.tsv"]
        );
        let m = mailto("Deutsch", "deu", "Patrice", &zip_path);
        assert!(m.starts_with("mailto:plplecuyer@gmail.com?subject=XIMOD%20Translator"));
        assert!(m.contains("Deutsch%20%28deu%29"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_filter_and_search() {
        let mut ed = TrEditor {
            entries: build_entries("a = Open\nb = Close\nc = Save\n", "", "a = Öffnen\n"),
            ..Default::default()
        };
        ed.refresh_view(true);
        assert_eq!(ed.view, [0, 1, 2]);
        ed.only_untranslated = true;
        ed.refresh_view(true);
        assert_eq!(ed.view, [1, 2]);
        // Typing in a row does not hide it until the filter is refreshed.
        ed.entries[1].input = "Schließen".into();
        ed.refresh_view(false);
        assert_eq!(ed.view, [1, 2]);
        assert_eq!(ed.progress(), (2, 3));
        ed.only_untranslated = false;
        ed.search = "SAV".into();
        ed.refresh_view(false);
        assert_eq!(ed.view, [2]);
        assert_eq!(ed.values().len(), 2);
    }

    /// Opening a shipped language and saving it unchanged rewrites the same
    /// file, whatever the displayed language.
    #[test]
    fn unchanged_save_is_identity_for_every_locale() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales");
        let reference = std::fs::read_to_string(dir.join("eng/main.ftl")).unwrap();
        let mut bad = Vec::new();
        for code in ["deu", "fra", "jpn", "pol", "rus", "ara", "zho", "eng"] {
            let target = std::fs::read_to_string(dir.join(code).join("main.ftl")).unwrap();
            let h = parse_metadata(&target);
            let meta = Meta {
                language: code.into(),
                langname: h.get("langname").cloned().unwrap_or_default(),
                font: h.get("font").cloned().unwrap_or_default(),
                author: h.get("author").cloned().unwrap_or_default(),
            };
            for shown in ["eng", "fra", "deu"] {
                let displayed = std::fs::read_to_string(dir.join(shown).join("main.ftl")).unwrap();
                let out = render_file(
                    &reference,
                    &target,
                    &build_entries(&reference, &displayed, &target),
                    &meta,
                );
                // Bodies (after the metadata block) compared line by line.
                let body = |t: &str| -> Vec<String> {
                    t.lines()
                        .skip_while(|l| l.starts_with('#'))
                        .map(String::from)
                        .collect()
                };
                let (tb, ob) = (body(&target), body(&out));
                for (a, b) in tb.iter().zip(ob.iter()) {
                    // Comments come from the reference; Fluent trims the
                    // blanks at the start of a value.
                    let norm = |l: &str| l.split_whitespace().collect::<Vec<_>>().join(" ");
                    if norm(a) != norm(b) {
                        bad.push(format!("{code}/{shown}: {a}  =>  {b}"));
                    }
                }
                assert_eq!(tb.len(), ob.len(), "{code}/{shown}");
            }
        }
        assert!(
            bad.is_empty(),
            "{} differences, e.g.\n{}",
            bad.len(),
            bad[..bad.len().min(12)].join("\n")
        );
    }

    /// T-E10-04 (D-35): assets/ read-only → the configuration folder, and a
    /// user copy, once written, stays the one saved.
    #[test]
    fn save_dir_follows_d35() {
        let base = std::env::temp_dir().join(format!("ximod_d35_{}", std::process::id()));
        let (assets, user) = (base.join("assets"), base.join("config/locales"));
        std::fs::create_dir_all(&assets).unwrap();
        let w = |_: &Path| true;
        let ro = |_: &Path| false;
        assert_eq!(
            choose_save_dir(&assets, Some(&user), "deu", w),
            assets.join("deu")
        );
        assert_eq!(
            choose_save_dir(&assets, Some(&user), "deu", ro),
            user.join("deu")
        );
        std::fs::create_dir_all(user.join("deu")).unwrap();
        std::fs::write(user.join("deu/main.ftl"), "a = b\n").unwrap();
        assert_eq!(
            choose_save_dir(&assets, Some(&user), "deu", w),
            user.join("deu")
        );
        assert!(dir_writable(&assets));
        assert!(!dir_writable(&base.join("missing")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
