//! The profiles of the OpenAI-compatible connections (D-03, decisions D-85
//! and D-86): one JSON file per profile — endpoint, model, headers, the
//! prompt with its variables, the form of the answer, the sampling
//! parameters and the profile's own guards. Shipped in
//! `assets/data/online/profiles/`, the user's copies and new profiles in
//! `data/online/profiles/` of the configuration, which win.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};

pub const FORMAT: u32 = 1;

/// The folder under `assets/data/` and `data/`.
pub const DIR: &str = "online/profiles";

/// The form of the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    /// The texts sent and received as a JSON array.
    #[default]
    Json,
    /// The texts separated by a numbered marker or the separator.
    Lines,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// One profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub format: u32,
    /// The file's name, stable (the service is `openai:<id>`).
    #[serde(default)]
    pub id: String,
    /// The label shown.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub answer: Answer,
    /// `lines`: the separator (empty: a numbered marker « ⟨⟨n⟩⟩ »).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub separator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// The profile's guards; `None`: the common values of `[Online]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rpm: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Usable without a key (a local server).
    #[serde(default, skip_serializing_if = "is_default")]
    pub key_optional: bool,
    /// Where the profile was read from (not in the file).
    #[serde(skip)]
    pub origin: Origin,
}

/// Where a profile comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    /// The shipped file.
    #[default]
    Shipped,
    /// The user's copy of a shipped profile.
    UserCopy,
    /// A profile of the user's only.
    User,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            format: FORMAT,
            id: String::new(),
            name: String::new(),
            endpoint: String::new(),
            model: String::new(),
            headers: BTreeMap::new(),
            prompt: String::new(),
            answer: Answer::Json,
            separator: String::new(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            frequency_penalty: None,
            presence_penalty: None,
            max_lines: None,
            max_chars: None,
            max_rpm: None,
            timeout: None,
            key_optional: false,
            origin: Origin::Shipped,
        }
    }
}

/// Why a profile is refused, or what it should mention (R-D03-07).
#[derive(Debug, Clone, PartialEq)]
pub enum Invalid {
    Json(String),
    Format(u32),
    /// An id that cannot be a file name.
    Id(String),
    Endpoint,
    Model,
    /// A number out of its bounds: the field, the bounds.
    Range {
        field: &'static str,
        min: f64,
        max: f64,
    },
}

/// Something worth saying that does not refuse the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The prompt names no target language.
    NoTarget,
}

/// A valid profile id: letters, digits, `-` and `_`.
pub fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn check(field: &'static str, v: Option<f64>, min: f64, max: f64) -> Result<(), Invalid> {
    match v {
        Some(x) if !(min..=max).contains(&x) => Err(Invalid::Range { field, min, max }),
        _ => Ok(()),
    }
}

impl Profile {
    pub fn parse(text: &str) -> Result<Profile, Invalid> {
        let p: Profile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| Invalid::Json(e.to_string()))?;
        if p.format != FORMAT {
            return Err(Invalid::Format(p.format));
        }
        Ok(p)
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// R-D03-07: the endpoint, the model, the bounds; the warnings.
    pub fn validate(&self) -> Result<Vec<Warning>, Invalid> {
        if self.format != FORMAT {
            return Err(Invalid::Format(self.format));
        }
        if !is_id(&self.id) {
            return Err(Invalid::Id(self.id.clone()));
        }
        let e = self.endpoint.trim();
        if !(e.starts_with("http://") || e.starts_with("https://")) || e.len() < 10 {
            return Err(Invalid::Endpoint);
        }
        if self.model.trim().is_empty() {
            return Err(Invalid::Model);
        }
        check("temperature", self.temperature.map(f64::from), 0.0, 2.0)?;
        check("topP", self.top_p.map(f64::from), 0.0, 1.0)?;
        check(
            "frequencyPenalty",
            self.frequency_penalty.map(f64::from),
            -2.0,
            2.0,
        )?;
        check(
            "presencePenalty",
            self.presence_penalty.map(f64::from),
            -2.0,
            2.0,
        )?;
        check(
            "maxTokens",
            self.max_tokens.map(f64::from),
            1.0,
            1_000_000.0,
        )?;
        check("maxLines", self.max_lines.map(|v| v as f64), 1.0, 500.0)?;
        check(
            "maxChars",
            self.max_chars.map(|v| v as f64),
            100.0,
            100_000.0,
        )?;
        check("maxRpm", self.max_rpm.map(f64::from), 1.0, 600.0)?;
        check("timeout", self.timeout.map(|v| v as f64), 5.0, 600.0)?;
        let mut w = Vec::new();
        if !self.prompt.contains("%lang_dest%") && !self.prompt.contains("{dst}") {
            w.push(Warning::NoTarget);
        }
        Ok(w)
    }

    /// The base of the endpoint, without its final `/`.
    pub fn base(&self) -> String {
        let e = self.endpoint.trim().trim_end_matches('/');
        if e.starts_with("http") {
            e.to_string()
        } else {
            format!("https://{e}")
        }
    }

    /// The prompt sent (R-D03-02): `%lang_source%` / `{src}`,
    /// `%lang_dest%` / `{dst}` — the English names of the languages —,
    /// `%game%`, `%n%`.
    pub fn render_prompt(&self, src: &str, dst: &str, game: &str, n: usize) -> String {
        let (s, d) = (lang_name(src), lang_name(dst));
        self.prompt
            .replace("%lang_source%", &s)
            .replace("{src}", &s)
            .replace("%lang_dest%", &d)
            .replace("{dst}", &d)
            .replace("%game%", game)
            .replace("%n%", &n.to_string())
    }
}

/// The English name of a language (ISO 639-1), which every model knows;
/// the code in capitals otherwise.
pub fn lang_name(iso1: &str) -> String {
    match crate::lang::STRINGS_LANGUAGES
        .iter()
        .find(|(i1, _, _)| i1.eq_ignore_ascii_case(iso1))
    {
        Some((_, _, name)) => name.to_string(),
        None => iso1.to_uppercase(),
    }
}

// ----------------------------------------------------------------------
// The files
// ----------------------------------------------------------------------

const SHIPPED_IDS: [&str; 3] = ["openai", "local", "mistral"];

fn embedded(id: &str) -> Option<&'static str> {
    Some(match id {
        "openai" => include_str!("../../assets/data/online/profiles/openai.json"),
        "local" => include_str!("../../assets/data/online/profiles/local.json"),
        "mistral" => include_str!("../../assets/data/online/profiles/mistral.json"),
        _ => return None,
    })
}

/// The folders of the installation.
pub fn shipped_dir() -> PathBuf {
    crate::config::assets_data_dir().join(DIR)
}

pub fn user_dir() -> PathBuf {
    crate::config::paths::get().data_dir.join(DIR)
}

fn read(path: &Path) -> Option<Profile> {
    let text = std::fs::read_to_string(path).ok()?;
    match Profile::parse(&text) {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!("{}: {e:?}", path.display());
            None
        }
    }
}

/// The shipped profile of an id (the file, else the embedded copy).
pub fn shipped(dir: &Path, id: &str) -> Option<Profile> {
    read(&dir.join(format!("{id}.json")))
        .or_else(|| embedded(id).and_then(|t| Profile::parse(t).ok()))
        .map(|mut p| {
            p.id = id.to_string();
            p.origin = Origin::Shipped;
            p
        })
}

/// Every profile: the shipped ones in their order (the user's copy
/// instead when there is one), then the user's own, by name.
pub fn load_all(shipped_dir: &Path, user_dir: Option<&Path>) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    let mut ids: Vec<String> = SHIPPED_IDS.iter().map(|s| s.to_string()).collect();
    if let Ok(rd) = std::fs::read_dir(shipped_dir) {
        let mut more: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".json")
                    .map(str::to_string)
            })
            .filter(|id| is_id(id) && !ids.contains(id))
            .collect();
        more.sort();
        ids.extend(more);
    }
    for id in &ids {
        let user = user_dir.and_then(|u| read(&u.join(format!("{id}.json"))));
        match user {
            Some(mut p) => {
                p.id = id.clone();
                p.origin = Origin::UserCopy;
                out.push(p);
            }
            None => {
                if let Some(p) = shipped(shipped_dir, id) {
                    out.push(p);
                }
            }
        }
    }
    if let Some(u) = user_dir
        && let Ok(rd) = std::fs::read_dir(u)
    {
        let mut own: Vec<Profile> = rd
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                let id = n.strip_suffix(".json")?.to_string();
                if !is_id(&id) || ids.contains(&id) {
                    return None;
                }
                let mut p = read(&e.path())?;
                p.id = id;
                p.origin = Origin::User;
                Some(p)
            })
            .collect();
        own.sort_by_key(|p| p.name.to_lowercase());
        out.extend(own);
    }
    out
}

/// Write a profile to the user's folder, validated first.
pub fn save(user_dir: &Path, p: &Profile) -> Result<PathBuf, Invalid> {
    p.validate()?;
    let _ = std::fs::create_dir_all(user_dir);
    let path = user_dir.join(format!("{}.json", p.id));
    crate::config::paths::write_atomic(&path, p.to_json().as_bytes())
        .map_err(|e| Invalid::Json(e.to_string()))?;
    Ok(path)
}

/// Remove the user's copy (a shipped profile comes back as shipped).
pub fn remove(user_dir: &Path, id: &str) -> std::io::Result<()> {
    let p = user_dir.join(format!("{id}.json"));
    if p.is_file() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

/// A free id from a name (`Mon serveur` → `mon-serveur`, `-2`…).
pub fn new_id(name: &str, taken: &[String]) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        "profile".to_string()
    } else {
        base
    };
    let base: String = base.chars().take(32).collect();
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(c))
        .unwrap_or(base)
}

// ----------------------------------------------------------------------
// The profiles of the session
// ----------------------------------------------------------------------

static PROFILES: RwLock<Option<Vec<Profile>>> = RwLock::new(None);

/// The endpoint and model of `[Online]` (étape 1), applied to the shipped
/// « openai » profile while the user has no copy of it (R-D03-05).
static LEGACY: Mutex<Option<(String, String)>> = Mutex::new(None);

pub fn set_legacy(endpoint: &str, model: &str) {
    let d = super::OnlineConfig::default();
    let e = (endpoint.trim() != d.openai_endpoint && !endpoint.trim().is_empty())
        .then(|| endpoint.trim().to_string());
    let m = (model.trim() != d.openai_model && !model.trim().is_empty())
        .then(|| model.trim().to_string());
    if let Ok(mut l) = LEGACY.lock() {
        *l = (e.is_some() || m.is_some()).then(|| (e.unwrap_or_default(), m.unwrap_or_default()));
    }
    reload();
}

fn load_installed() -> Vec<Profile> {
    let mut all = load_all(&shipped_dir(), Some(&user_dir()));
    if let Ok(l) = LEGACY.lock()
        && let Some((e, m)) = l.as_ref()
        && let Some(p) = all
            .iter_mut()
            .find(|p| p.id == "openai" && p.origin == Origin::Shipped)
    {
        if !e.is_empty() {
            p.endpoint = e.clone();
        }
        if !m.is_empty() {
            p.model = m.clone();
        }
    }
    all
}

/// The profiles of the session, read once.
pub fn all() -> Vec<Profile> {
    if let Ok(g) = PROFILES.read()
        && let Some(v) = g.as_ref()
    {
        return v.clone();
    }
    let v = load_installed();
    if let Ok(mut g) = PROFILES.write() {
        *g = Some(v.clone());
    }
    v
}

/// Read the files again (after the settings saved them).
pub fn reload() {
    if let Ok(mut g) = PROFILES.write() {
        *g = None;
    }
}

/// Replace the profiles of the session (tests, the command line).
pub fn set_all(v: Vec<Profile>) {
    if let Ok(mut g) = PROFILES.write() {
        *g = Some(v);
    }
}

/// A profile of the session by id.
pub fn get(id: &str) -> Option<Profile> {
    all().into_iter().find(|p| p.id == id)
}
