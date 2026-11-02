//! The structured text formats (D-08): JSON by paths, XML by paths, CSV by
//! columns, and the description of their fields (R-D08-02) — where the
//! text is, its identifier, its record, its field, what to leave out,
//! where the translation goes. Each value recognised becomes a line with
//! the position of its value in the text; the writing puts the
//! translations at these positions and nothing else changes (D-88).

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{FIELD, TextFormat, csv, json, xml};

/// The types of a format (R-D08-01).
pub const KINDS: [&str; 4] = ["line", "json", "xml", "csv"];

/// The resolutions of a field (R-D08-02).
pub const BYS: [&str; 8] = [
    "column",
    "property",
    "path",
    "fixed",
    "auto",
    "property-name",
    "regex",
    "copy",
];

/// How a field is found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolve {
    #[serde(default)]
    pub by: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// What to do when `property` finds nothing (`auto`).
    #[serde(default, rename = "else", skip_serializing_if = "String::is_empty")]
    pub otherwise: String,
    /// For the texts: an expression whose group `text` is the part to
    /// translate.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub regex: String,
}

impl Resolve {
    pub fn new(by: &str, value: &str) -> Resolve {
        Resolve {
            by: by.into(),
            value: value.into(),
            ..Default::default()
        }
    }
}

/// The description of the fields (R-D08-01 / 02).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    /// The texts (for a CSV: the source column).
    #[serde(alias = "source")]
    pub text: Resolve,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Resolve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Resolve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<Resolve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Resolve>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<Resolve>,
}

/// The fields in the order of the editor, with their key.
pub const FIELD_NAMES: [&str; 6] = ["text", "id", "record", "field", "exclude", "translation"];

impl Fields {
    /// A blank description for a type.
    pub fn blank(kind: &str) -> Fields {
        match kind {
            "csv" => Fields {
                text: Resolve::new("column", "2"),
                id: Some(Resolve::new("column", "1")),
                translation: Some(Resolve::new("column", "3")),
                ..Default::default()
            },
            "xml" => Fields {
                text: Resolve::new("path", "//*/text()"),
                id: Some(Resolve {
                    by: "property".into(),
                    value: "id".into(),
                    otherwise: "auto".into(),
                    ..Default::default()
                }),
                field: Some(Resolve::new("property-name", "")),
                translation: Some(Resolve::new("copy", "")),
                ..Default::default()
            },
            _ => Fields {
                text: Resolve::new("path", "$..text"),
                id: Some(Resolve {
                    by: "property".into(),
                    value: "id".into(),
                    otherwise: "auto".into(),
                    ..Default::default()
                }),
                field: Some(Resolve::new("property-name", "")),
                translation: Some(Resolve::new("copy", "")),
                ..Default::default()
            },
        }
    }

    pub fn get(&self, name: &str) -> Option<&Resolve> {
        match name {
            "text" => Some(&self.text),
            "id" => self.id.as_ref(),
            "record" => self.record.as_ref(),
            "field" => self.field.as_ref(),
            "exclude" => self.exclude.as_ref(),
            "translation" => self.translation.as_ref(),
            _ => None,
        }
    }

    pub fn set(&mut self, name: &str, r: Option<Resolve>) {
        match name {
            "text" => self.text = r.unwrap_or_default(),
            "id" => self.id = r,
            "record" => self.record = r,
            "field" => self.field = r,
            "exclude" => self.exclude = r,
            "translation" => self.translation = r,
            _ => {}
        }
    }

    /// The resolutions a field may take for a type.
    pub fn choices(kind: &str, name: &str) -> &'static [&'static str] {
        match (kind, name) {
            ("csv", "text") => &["column"],
            (_, "text") => &["path"],
            ("csv", "id") => &["column", "auto", "fixed"],
            (_, "id") => &["property", "auto", "fixed"],
            ("csv", "record" | "field") => &["fixed", "column", "property-name"],
            (_, "record") => &["fixed", "property"],
            (_, "field") => &["property-name", "fixed", "property"],
            (_, "exclude") => &["regex"],
            ("csv", "translation") => &["copy", "column"],
            (_, "translation") => &["copy", "property"],
            _ => &[],
        }
    }

    /// The translation is written in the file itself (a sibling property,
    /// a column), not in a copy.
    pub fn in_place(&self) -> bool {
        self.translation
            .as_ref()
            .is_some_and(|t| matches!(t.by.as_str(), "property" | "column") && !t.value.is_empty())
    }
}

/// Why a description or a file is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoText,
    Path(String),
    Column(String),
    Regex(String),
    RegexGroup,
    /// The file does not read: line, column, what was expected.
    Parse(usize, usize, String),
}

impl Problem {
    /// The message (R-D08-07).
    pub fn message(&self) -> String {
        match self {
            Problem::NoText => crate::l10n!("fmt-err-no-text"),
            Problem::Path(p) => crate::l10n!("fmt-err-path", path = p),
            Problem::Column(c) => crate::l10n!("fmt-err-column", column = c),
            Problem::Regex(e) => crate::l10n!("formats-regex-invalid", err = e),
            Problem::RegexGroup => crate::l10n!("fmt-err-regex-group"),
            Problem::Parse(l, c, w) => crate::l10n!(
                "fmt-err-parse",
                line = l.to_string(),
                col = c.to_string(),
                what = w
            ),
        }
    }
}

/// How a value is written back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Esc {
    Json,
    XmlText,
    XmlAttr(char),
    Cdata,
    Csv { quoted: bool, sep: char },
}

impl Esc {
    pub fn apply(self, v: &str) -> String {
        match self {
            Esc::Json => json::escape(v),
            Esc::XmlText => xml::escape(v, None),
            Esc::XmlAttr(q) => xml::escape(v, Some(q)),
            Esc::Cdata => v.replace("]]>", "]]]]><![CDATA[>"),
            Esc::Csv { quoted, sep } => csv::quote(v, sep, quoted),
        }
    }
}

/// Where a translation kept in the file itself goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub span: (usize, usize),
    pub esc: Esc,
    /// Written before the value (the separators of missing CSV cells).
    pub prefix: String,
    /// What the slot holds now.
    pub current: String,
}

/// A value recognised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub record: String,
    pub field: String,
    /// The text to translate (the extracted part).
    pub value: String,
    /// The whole value, unescaped.
    pub full: String,
    /// Where `value` is in `full`.
    pub part: (usize, usize),
    /// The span of the escaped value in the text.
    pub span: (usize, usize),
    pub esc: Esc,
    /// The translation the file already gives.
    pub existing: Option<String>,
    pub slot: Option<Slot>,
}

/// The account of a reading (R-D08-05).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Text values of the file.
    pub read: usize,
    /// Read but not lines.
    pub excluded: usize,
    /// Lines per field, in the order met.
    pub by_field: Vec<(String, usize)>,
    /// The names of the values left out, the most frequent first.
    pub excluded_names: Vec<(String, usize)>,
    /// Distinct identifiers (per field).
    pub unique_ids: usize,
    /// Lines whose identifier another line of the same field has.
    pub dup_lines: usize,
}

/// A structured file read in a format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structured {
    pub text: String,
    pub items: Vec<Item>,
    pub stats: Stats,
    pub in_place: bool,
}

/// A value found before the exclusion.
struct Cand {
    full: String,
    span: (usize, usize),
    esc: Esc,
    id: String,
    record: String,
    field: String,
    existing: Option<String>,
    slot: Option<Slot>,
}

/// The checks of the editor's Enregistrer (R-D08-07), file aside.
pub fn validate(fmt: &TextFormat) -> Result<(), Problem> {
    let Some(f) = &fmt.fields else {
        return Err(Problem::NoText);
    };
    if f.text.value.trim().is_empty() {
        return Err(Problem::NoText);
    }
    match fmt.kind.as_str() {
        "json" => {
            json::Paths::parse(&f.text.value).map_err(|e| Problem::Path(e.0))?;
        }
        "xml" => {
            xml::XPaths::parse(&f.text.value).map_err(|e| Problem::Path(e.0))?;
        }
        _ => {}
    }
    if !f.text.regex.trim().is_empty() {
        let re = Regex::new(&f.text.regex).map_err(|e| Problem::Regex(e.to_string()))?;
        if !re.capture_names().flatten().any(|n| n == "text") {
            return Err(Problem::RegexGroup);
        }
    }
    if let Some(x) = &f.exclude
        && !x.value.trim().is_empty()
    {
        Regex::new(&x.value).map_err(|e| Problem::Regex(e.to_string()))?;
    }
    Ok(())
}

/// The columns of a CSV header a description names that it lacks
/// (R-D08-07, at the trial).
pub fn missing_columns(fmt: &TextFormat, text: &str) -> Option<String> {
    let f = fmt.fields.as_ref()?;
    if fmt.kind != "csv" {
        return None;
    }
    let sep = csv::sep_of(&fmt.separator, text);
    let table = csv::parse(text, sep);
    let header: Vec<String> = table
        .rows
        .first()
        .map(|r| r.iter().map(|c| c.value.trim().to_string()).collect())
        .unwrap_or_default();
    for name in FIELD_NAMES {
        let Some(r) = f.get(name) else { continue };
        if r.by == "column" && column_index(&r.value, &header, fmt.header).is_none() {
            return Some(r.value.clone());
        }
    }
    None
}

/// Read a structured file's text in a format.
pub fn parse(text: &str, fmt: &TextFormat) -> Result<Structured, Problem> {
    validate(fmt)?;
    let f = fmt.fields.clone().unwrap_or_default();
    let sig = fmt.sig();
    let (cands, read, names) = match fmt.kind.as_str() {
        "json" => collect_json(text, &f, &sig)?,
        "xml" => collect_xml(text, &f, &sig)?,
        _ => collect_csv(text, fmt, &f, &sig)?,
    };
    let extract = if f.text.regex.trim().is_empty() {
        None
    } else {
        Some(Regex::new(&f.text.regex).map_err(|e| Problem::Regex(e.to_string()))?)
    };
    let exclude = match &f.exclude {
        Some(x) if !x.value.trim().is_empty() => {
            Some(Regex::new(&x.value).map_err(|e| Problem::Regex(e.to_string()))?)
        }
        _ => None,
    };
    let mut items = Vec::new();
    let mut taken_spans = std::collections::HashSet::new();
    for c in cands {
        let (value, part) = match &extract {
            Some(re) => match re.captures(&c.full).and_then(|m| m.name("text")) {
                Some(m) => (m.as_str().to_string(), (m.start(), m.end())),
                None => continue,
            },
            None => (c.full.clone(), (0, c.full.len())),
        };
        if value.trim().is_empty() {
            continue;
        }
        if exclude.as_ref().is_some_and(|re| re.is_match(&value)) {
            continue;
        }
        taken_spans.insert(c.span.0);
        items.push(Item {
            id: c.id,
            record: c.record,
            field: c.field,
            value,
            full: c.full,
            part,
            span: c.span,
            esc: c.esc,
            existing: c.existing.filter(|e| !e.is_empty()),
            slot: c.slot,
        });
    }
    // The account.
    let mut stats = Stats {
        read,
        excluded: read.saturating_sub(items.len()),
        ..Default::default()
    };
    for it in &items {
        match stats.by_field.iter_mut().find(|(n, _)| *n == it.field) {
            Some(x) => x.1 += 1,
            None => stats.by_field.push((it.field.clone(), 1)),
        }
    }
    let mut ex: HashMap<String, usize> = HashMap::new();
    for (start, name) in names {
        if !taken_spans.contains(&start) {
            *ex.entry(name).or_default() += 1;
        }
    }
    let mut ex: Vec<(String, usize)> = ex.into_iter().collect();
    ex.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    stats.excluded_names = ex;
    let mut keys: HashMap<(&str, &str), usize> = HashMap::new();
    for it in &items {
        *keys.entry((it.field.as_str(), it.id.as_str())).or_default() += 1;
    }
    stats.unique_ids = keys.len();
    stats.dup_lines = keys.values().filter(|&&n| n > 1).sum();
    Ok(Structured {
        text: text.to_string(),
        items,
        stats,
        in_place: f.in_place(),
    })
}

impl Structured {
    /// The text with the translations (`trans(k)` for the k-th line, `None`
    /// to leave it) at their places (R-D08-04).
    pub fn write<'a>(&self, trans: impl Fn(usize) -> Option<&'a str>) -> String {
        let mut reps: Vec<((usize, usize), String)> = Vec::new();
        for (k, it) in self.items.iter().enumerate() {
            let Some(t) = trans(k) else { continue };
            let full = format!("{}{t}{}", &it.full[..it.part.0], &it.full[it.part.1..]);
            if self.in_place {
                if let Some(s) = &it.slot
                    && full != s.current
                {
                    reps.push((s.span, format!("{}{}", s.prefix, s.esc.apply(&full))));
                }
            } else if full != it.full {
                reps.push((it.span, it.esc.apply(&full)));
            }
        }
        reps.sort_by_key(|r| r.0.0);
        let mut out = String::with_capacity(self.text.len() + 64);
        let mut at = 0usize;
        for ((a, b), v) in reps {
            if a < at {
                continue;
            }
            out.push_str(&self.text[at..a]);
            out.push_str(&v);
            at = b;
        }
        out.push_str(&self.text[at..]);
        out
    }
}

/// The values of a file and what the paths took.
type Collected = (Vec<Cand>, usize, Vec<(usize, String)>);

fn parse_problem(text: &str, at: usize, what: &str) -> Problem {
    let before = &text[..at.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    Problem::Parse(line, col, what.to_string())
}

// ------------------------------------------------------------------ JSON

fn collect_json(text: &str, f: &Fields, sig: &str) -> Result<Collected, Problem> {
    let root = json::parse(text).map_err(|e| parse_problem(text, e.at, e.what))?;
    // Every string, for the counts.
    let mut names = Vec::new();
    all_strings(&root, "", &mut names);
    let read = names.len();
    let paths = json::Paths::parse(&f.text.value).map_err(|e| Problem::Path(e.0))?;
    let mut out = Vec::new();
    for trail in paths.select(&root) {
        let Some(node) = json::at(&root, &trail) else {
            continue;
        };
        let json::Value::Str(full) = &node.value else {
            continue;
        };
        // The property holding the value, the arrays between.
        let last_key = trail.iter().rposition(|s| matches!(s, json::Step::Key(_)));
        let name = match last_key {
            Some(k) => match &trail[k] {
                json::Step::Key(n) => n.clone(),
                json::Step::Index(_) => String::new(),
            },
            None => "$".to_string(),
        };
        let suffix: String = trail[last_key.map_or(0, |k| k + 1)..]
            .iter()
            .filter_map(|s| match s {
                json::Step::Index(i) => Some(format!(".{i}")),
                json::Step::Key(_) => None,
            })
            .collect();
        // The objects that enclose the value, the nearest first.
        let objects: Vec<usize> = (0..trail.len())
            .rev()
            .filter(|&n| {
                json::at(&root, &trail[..n])
                    .is_some_and(|o| matches!(o.value, json::Value::Object(_)))
            })
            .collect();
        let prop_up = |p: &str| -> Option<(usize, String)> {
            for &n in &objects {
                let o = json::at(&root, &trail[..n])?;
                if let json::Value::Object(m) = &o.value
                    && let Some((_, v)) = m.iter().find(|(k, _)| k == p)
                {
                    return match &v.value {
                        json::Value::Str(s) => Some((n, s.clone())),
                        json::Value::Other => Some((n, text[v.span.0..v.span.1].to_string())),
                        _ => None,
                    };
                }
            }
            None
        };
        let auto_id = {
            let base = json::trail_text(&trail[..last_key.unwrap_or(0)]);
            if base.is_empty() && suffix.is_empty() {
                name.clone()
            } else if base.is_empty() {
                format!("{name}{suffix}")
            } else {
                format!("{base}{suffix}")
            }
        };
        let id = match &f.id {
            Some(r) if r.by == "property" => match prop_up(&r.value) {
                Some((_, v)) => format!("{v}{suffix}"),
                None if r.otherwise == "auto" => auto_id,
                None => String::new(),
            },
            Some(r) if r.by == "fixed" => r.value.clone(),
            _ => auto_id,
        };
        let record = match &f.record {
            Some(r) if r.by == "fixed" && !r.value.is_empty() => r.value.clone(),
            Some(r) if r.by == "property" => prop_up(&r.value).map_or(sig.to_string(), |x| x.1),
            _ => sig.to_string(),
        };
        let field = resolve_field(f, &name, |p| prop_up(p).map(|x| x.1));
        // A sibling property for the translation.
        let (existing, slot) = match &f.translation {
            Some(t) if t.by == "property" && !t.value.is_empty() && suffix.is_empty() => {
                let obj = last_key.and_then(|k| json::at(&root, &trail[..k]));
                match obj.map(|o| &o.value) {
                    Some(json::Value::Object(m)) => match m.iter().find(|(k, _)| *k == t.value) {
                        Some((_, v)) => match &v.value {
                            json::Value::Str(s) => (
                                Some(s.clone()),
                                Some(Slot {
                                    span: v.span,
                                    esc: Esc::Json,
                                    prefix: String::new(),
                                    current: s.clone(),
                                }),
                            ),
                            _ => (None, None),
                        },
                        None => (None, None),
                    },
                    _ => (None, None),
                }
            }
            _ => (None, None),
        };
        out.push(Cand {
            full: full.clone(),
            span: node.span,
            esc: Esc::Json,
            id,
            record,
            field,
            existing,
            slot,
        });
    }
    // A sibling translation property is not a text left out.
    if let Some(t) = &f.translation
        && t.by == "property"
    {
        names.retain(|(_, n)| *n != t.value);
    }
    let _ = read;
    let read = names.len();
    Ok((out, read, names))
}

/// The field of a value: the description's, `TXT` without one.
fn resolve_field(f: &Fields, name: &str, prop: impl Fn(&str) -> Option<String>) -> String {
    match &f.field {
        Some(r) if r.by == "property-name" || r.by == "auto" => name.to_string(),
        Some(r) if r.by == "fixed" && !r.value.is_empty() => r.value.clone(),
        Some(r) if r.by == "property" => prop(&r.value).unwrap_or_else(|| FIELD.to_string()),
        _ => FIELD.to_string(),
    }
}

fn all_strings(n: &json::Node, name: &str, out: &mut Vec<(usize, String)>) {
    match &n.value {
        json::Value::Object(m) => {
            for (k, v) in m {
                all_strings(v, k, out);
            }
        }
        json::Value::Array(a) => {
            for v in a {
                all_strings(v, name, out);
            }
        }
        json::Value::Str(_) => out.push((n.span.0, name.to_string())),
        json::Value::Other => {}
    }
}

// ------------------------------------------------------------------- XML

fn collect_xml(text: &str, f: &Fields, sig: &str) -> Result<Collected, Problem> {
    let doc = xml::parse(text).map_err(|e| parse_problem(text, e.at, e.what))?;
    let name_of = |h: xml::Hit| -> String {
        match h {
            xml::Hit::Attr(e, r) => doc.elements[e].attrs[r].name.clone(),
            xml::Hit::Text(e, _) => doc.elements[e].name.clone(),
        }
    };
    let mut names: Vec<(usize, String)> = doc
        .all_values()
        .into_iter()
        .map(|h| (doc.span(h).0, name_of(h)))
        .collect();
    let paths = xml::XPaths::parse(&f.text.value).map_err(|e| Problem::Path(e.0))?;
    let mut out = Vec::new();
    for h in paths.select(&doc) {
        let (e, full, esc) = match h {
            xml::Hit::Attr(e, r) => {
                let a = &doc.elements[e].attrs[r];
                (e, a.value.clone(), Esc::XmlAttr(a.quote))
            }
            xml::Hit::Text(e, r) => match &doc.elements[e].children[r] {
                xml::Child::Text(t) => (
                    e,
                    t.value.clone(),
                    if t.cdata { Esc::Cdata } else { Esc::XmlText },
                ),
                xml::Child::Element(_) => continue,
            },
        };
        let name = name_of(h);
        let auto_id = match h {
            xml::Hit::Attr(_, _) => format!("{}/@{name}", doc.path_of(e)),
            xml::Hit::Text(_, _) => doc.path_of(e),
        };
        let id = match &f.id {
            Some(r) if r.by == "property" => match doc.attr_up(e, &r.value) {
                Some(v) => v,
                None if r.otherwise == "auto" => auto_id,
                None => String::new(),
            },
            Some(r) if r.by == "fixed" => r.value.clone(),
            _ => auto_id,
        };
        let record = match &f.record {
            Some(r) if r.by == "fixed" && !r.value.is_empty() => r.value.clone(),
            Some(r) if r.by == "property" => doc.attr_up(e, &r.value).unwrap_or(sig.to_string()),
            _ => sig.to_string(),
        };
        let field = resolve_field(f, &name, |p| doc.attr_up(e, p));
        let (existing, slot) = match &f.translation {
            Some(t) if t.by == "property" && !t.value.is_empty() => {
                match doc.elements[e].attrs.iter().find(|a| a.name == t.value) {
                    Some(a) => (
                        Some(a.value.clone()),
                        Some(Slot {
                            span: a.span,
                            esc: Esc::XmlAttr(a.quote),
                            prefix: String::new(),
                            current: a.value.clone(),
                        }),
                    ),
                    None => (None, None),
                }
            }
            _ => (None, None),
        };
        out.push(Cand {
            full,
            span: doc.span(h),
            esc,
            id,
            record,
            field,
            existing,
            slot,
        });
    }
    if let Some(t) = &f.translation
        && t.by == "property"
    {
        names.retain(|(_, n)| *n != t.value);
    }
    let read = names.len().max(out.len());
    Ok((out, read, names))
}

// ------------------------------------------------------------------- CSV

/// A column by its number (from 1) or its name in the header.
pub fn column_index(v: &str, header: &[String], has_header: bool) -> Option<usize> {
    let v = v.trim();
    if let Ok(n) = v.parse::<usize>() {
        return (n >= 1).then(|| n - 1);
    }
    if !has_header {
        return None;
    }
    header.iter().position(|h| h.eq_ignore_ascii_case(v))
}

fn collect_csv(text: &str, fmt: &TextFormat, f: &Fields, sig: &str) -> Result<Collected, Problem> {
    let sep = csv::sep_of(&fmt.separator, text);
    let table = csv::parse(text, sep);
    let skip = usize::from(fmt.header);
    let header: Vec<String> = if fmt.header {
        table
            .rows
            .first()
            .map(|r| r.iter().map(|c| c.value.trim().to_string()).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let col = |r: &Resolve| -> Result<usize, Problem> {
        column_index(&r.value, &header, fmt.header).ok_or_else(|| Problem::Column(r.value.clone()))
    };
    let text_col = col(&f.text)?;
    let text_name = header
        .get(text_col)
        .cloned()
        .unwrap_or_else(|| (text_col + 1).to_string());
    let id_col = match &f.id {
        Some(r) if r.by == "column" => Some(col(r)?),
        _ => None,
    };
    let trans_col = match &f.translation {
        Some(r) if r.by == "column" && !r.value.is_empty() => Some(col(r)?),
        _ => None,
    };
    let rec_col = match &f.record {
        Some(r) if r.by == "column" => Some(col(r)?),
        _ => None,
    };
    let field_col = match &f.field {
        Some(r) if r.by == "column" => Some(col(r)?),
        _ => None,
    };
    let mut out = Vec::new();
    let mut names = Vec::new();
    for (ri, row) in table.rows.iter().enumerate().skip(skip) {
        let Some(cell) = row.get(text_col) else {
            continue;
        };
        names.push((cell.span.0, text_name.clone()));
        let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|x| x.value.clone());
        let id = match &f.id {
            Some(r) if r.by == "fixed" => r.value.clone(),
            Some(r) if r.by == "column" => get(id_col).unwrap_or_default(),
            _ => (ri + 1).to_string(),
        };
        let record = match &f.record {
            Some(r) if r.by == "fixed" && !r.value.is_empty() => r.value.clone(),
            Some(r) if r.by == "column" => get(rec_col).unwrap_or(sig.to_string()),
            _ => sig.to_string(),
        };
        let field = match &f.field {
            Some(r) if r.by == "fixed" && !r.value.is_empty() => r.value.clone(),
            Some(r) if r.by == "column" => get(field_col).unwrap_or(FIELD.to_string()),
            Some(r) if r.by == "property-name" => text_name.clone(),
            _ => FIELD.to_string(),
        };
        let (existing, slot) = match trans_col {
            Some(tc) => match row.get(tc) {
                Some(c) => (
                    Some(c.value.clone()),
                    Some(Slot {
                        span: c.span,
                        esc: Esc::Csv {
                            quoted: c.quoted,
                            sep,
                        },
                        prefix: String::new(),
                        current: c.value.clone(),
                    }),
                ),
                None => {
                    let end = table.row_end[ri];
                    (
                        None,
                        Some(Slot {
                            span: (end, end),
                            esc: Esc::Csv { quoted: false, sep },
                            prefix: sep.to_string().repeat(tc + 1 - row.len()),
                            current: String::new(),
                        }),
                    )
                }
            },
            None => (None, None),
        };
        out.push(Cand {
            full: cell.value.clone(),
            span: cell.span,
            esc: Esc::Csv {
                quoted: cell.quoted,
                sep,
            },
            id,
            record,
            field,
            existing,
            slot,
        });
    }
    let read = out.len();
    Ok((out, read, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(kind: &str, fields: Fields) -> TextFormat {
        let mut f = TextFormat::new_user("t", "T");
        f.kind = kind.into();
        f.line = String::new();
        f.fields = Some(fields);
        f
    }

    #[test]
    fn json_by_paths_with_ids_and_options() {
        let t = "{\n\t\"content\": [{\"text\": \"About\", \"type\": \"section\"},\n\t {\"id\": \"opt\", \"text\": \"Diff\", \"help\": \"Choose\",\n\t  \"valueOptions\": {\"options\": [\"Easy\", \"Hard\"]}, \"n\": 3}],\n\t\"pages\": [{\"pageDisplayName\": \"Main\"}, {\"pageDisplayName\": \"$skip\"}]\n}\n";
        let f = fmt(
            "json",
            Fields {
                text: Resolve::new(
                    "path",
                    "$..text | $..help | $..pageDisplayName | $..options[*]",
                ),
                id: Some(Resolve {
                    by: "property".into(),
                    value: "id".into(),
                    otherwise: "auto".into(),
                    ..Default::default()
                }),
                record: Some(Resolve::new("fixed", "MCM")),
                field: Some(Resolve::new("property-name", "")),
                exclude: Some(Resolve::new("regex", r"^\$")),
                translation: Some(Resolve::new("copy", "")),
            },
        );
        let s = parse(t, &f).unwrap();
        let rows: Vec<(String, String, String)> = s
            .items
            .iter()
            .map(|i| (i.field.clone(), i.id.clone(), i.value.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("text".into(), "content[0]".into(), "About".into()),
                ("text".into(), "opt".into(), "Diff".into()),
                ("help".into(), "opt".into(), "Choose".into()),
                ("options".into(), "opt.0".into(), "Easy".into()),
                ("options".into(), "opt.1".into(), "Hard".into()),
                ("pageDisplayName".into(), "pages[0]".into(), "Main".into()),
            ]
        );
        assert_eq!(s.stats.read, 9);
        assert_eq!(s.stats.excluded, 3);
        assert_eq!(s.stats.dup_lines, 0);
        let out = s.write(|k| match k {
            0 => Some("À propos"),
            3 => Some("Facile \"x\""),
            _ => None,
        });
        assert_eq!(
            out,
            t.replace("\"About\"", "\"À propos\"")
                .replace("\"Easy\"", "\"Facile \\\"x\\\"\"")
        );
    }

    #[test]
    fn a_sibling_property_holds_the_translation() {
        let t = r#"[{"id":"a","original":"Sword","string":""},{"id":"b","original":"Bow","string":"Arc"}]"#;
        let f = fmt(
            "json",
            Fields {
                text: Resolve::new("path", "$[*].original"),
                id: Some(Resolve::new("property", "id")),
                translation: Some(Resolve::new("property", "string")),
                ..Default::default()
            },
        );
        let s = parse(t, &f).unwrap();
        assert!(s.in_place);
        assert_eq!(s.items[1].existing.as_deref(), Some("Arc"));
        assert_eq!(s.items[0].existing, None);
        let out = s.write(|k| [Some("Épée"), Some("Arc")][k]);
        assert_eq!(out, t.replace(r#""string":"""#, r#""string":"Épée""#));
    }

    #[test]
    fn csv_columns_in_place() {
        let t = "id;source;translation\r\nk1;\"Hello; you\";\r\nk2;Bye;Au revoir\r\nk3;Short\r\n";
        let mut f = fmt(
            "csv",
            Fields {
                text: Resolve::new("column", "source"),
                id: Some(Resolve::new("column", "id")),
                translation: Some(Resolve::new("column", "translation")),
                ..Default::default()
            },
        );
        f.header = true;
        let s = parse(t, &f).unwrap();
        assert_eq!(s.items.len(), 3);
        assert_eq!(s.items[0].value, "Hello; you");
        assert_eq!(s.items[1].existing.as_deref(), Some("Au revoir"));
        let out = s.write(|k| [Some("Salut; toi"), Some("Au revoir"), Some("Court")][k]);
        assert_eq!(
            out,
            "id;source;translation\r\nk1;\"Hello; you\";\"Salut; toi\"\r\nk2;Bye;Au revoir\r\nk3;Short;Court\r\n"
        );
        let mut bad = f.clone();
        bad.fields.as_mut().unwrap().translation = Some(Resolve::new("column", "trad"));
        assert_eq!(parse(t, &bad).unwrap_err(), Problem::Column("trad".into()));
        assert_eq!(missing_columns(&bad, t), Some("trad".into()));
    }

    #[test]
    fn xml_texts_and_attributes_keep_entities() {
        let t = "<strings>\n  <string id=\"a\">Fish &amp; chips</string>\n  <item label=\"Sword &lt;1&gt;\"/>\n</strings>\n";
        let f = fmt(
            "xml",
            Fields {
                text: Resolve::new("path", "//string/text() | //item/@label"),
                id: Some(Resolve {
                    by: "property".into(),
                    value: "id".into(),
                    otherwise: "auto".into(),
                    ..Default::default()
                }),
                field: Some(Resolve::new("property-name", "")),
                ..Default::default()
            },
        );
        let s = parse(t, &f).unwrap();
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[0].value, "Fish & chips");
        assert_eq!(s.items[1].id, "/strings/item/@label");
        assert_eq!(s.items[1].field, "label");
        let out = s.write(|k| [Some("Poisson & frites"), None][k]);
        assert_eq!(out, t.replace("Fish &amp; chips", "Poisson &amp; frites"));
    }

    #[test]
    fn extraction_and_validation() {
        let mut f = fmt(
            "json",
            Fields {
                text: Resolve {
                    by: "path".into(),
                    value: "$.a".into(),
                    regex: r"^\[(?P<text>[^\]]+)\]".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let s = parse(r#"{"a": "[Sword] of x"}"#, &f).unwrap();
        assert_eq!(s.items[0].value, "Sword");
        assert_eq!(s.write(|_| Some("Épée")), r#"{"a": "[Épée] of x"}"#);
        f.fields.as_mut().unwrap().text.regex = r"^\[([^\]]+)\]".into();
        assert_eq!(validate(&f), Err(Problem::RegexGroup));
        f.fields.as_mut().unwrap().text = Resolve::new("path", "$.[");
        assert!(matches!(validate(&f), Err(Problem::Path(_))));
        f.fields.as_mut().unwrap().text = Resolve::new("path", "");
        assert_eq!(validate(&f), Err(Problem::NoText));
        f.fields.as_mut().unwrap().text = Resolve::new("path", "$.a");
        assert!(matches!(
            parse("{\"a\": ", &f),
            Err(Problem::Parse(1, _, _))
        ));
    }
}
