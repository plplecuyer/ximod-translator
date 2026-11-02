//! Recherche avancée (A-19): six criteria combined with AND — source,
//! translation, EDID or FormID, record, field, keyword — each a « contains »
//! text or a regular expression, with or without case; the comparison of
//! the groups captured in the source and in the translation; the statuses;
//! and the named presets kept in `data/search_presets.json`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::filter::{CommentTexts, parse_formid};
use crate::status;
use crate::tm::TmEntry;

/// The six fields, in the order of the window.
pub const N_FIELDS: usize = 6;
pub const F_SOURCE: usize = 0;
pub const F_TRANS: usize = 1;
pub const F_ID: usize = 2;
pub const F_RECORD: usize = 3;
pub const F_FIELD: usize = 4;
pub const F_KEYWORD: usize = 5;

/// The comparison of the captured groups (R-A19-02, decision D-52).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Groups {
    #[default]
    None,
    Same,
    Differ,
}

/// What the window holds and a preset keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spec {
    /// Source, Traduction, EDID ou FormID, GRUP, Champ, Mot-clé.
    pub texts: [String; N_FIELDS],
    pub regex: [bool; N_FIELDS],
    pub case: [bool; N_FIELDS],
    pub groups: Groups,
    /// Indexed by [`Status::index`].
    pub statuses: [bool; status::COUNT],
    /// The lines never shown in game too.
    pub unused: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            texts: Default::default(),
            regex: [false; N_FIELDS],
            case: [false; N_FIELDS],
            groups: Groups::None,
            statuses: [true; status::COUNT],
            unused: false,
        }
    }
}

impl Spec {
    /// Whether the groups comparison applies: source and translation in
    /// regex mode (the groups themselves are checked when compiling).
    pub fn groups_possible(&self) -> bool {
        self.regex[F_SOURCE]
            && self.regex[F_TRANS]
            && !self.texts[F_SOURCE].trim().is_empty()
            && !self.texts[F_TRANS].trim().is_empty()
    }
}

/// One compiled field.
#[derive(Debug, Clone)]
enum Field {
    Any,
    /// The needle, lowercased unless `case`.
    Plain {
        needle: String,
        case: bool,
    },
    Regex(regex::Regex),
}

impl Field {
    fn hits(&self, s: &str) -> bool {
        match self {
            Field::Any => true,
            Field::Plain { needle, case: true } => s.contains(needle.as_str()),
            Field::Plain {
                needle,
                case: false,
            } => s.to_lowercase().contains(needle.as_str()),
            Field::Regex(re) => re.is_match(s),
        }
    }

    fn is_any(&self) -> bool {
        matches!(self, Field::Any)
    }
}

/// A spec ready to run, or the errors of its fields.
#[derive(Debug, Clone)]
pub struct Compiled {
    fields: [Field; N_FIELDS],
    /// The FormID of the EDID field, when it reads as one.
    formid: Option<u32>,
    groups: Groups,
    statuses: [bool; status::COUNT],
    unused: bool,
}

/// The errors of a spec, per field (`None` = fine).
pub type Errors = Box<[Option<String>; N_FIELDS]>;

/// Compile a spec (R-A19-01): an invalid expression is an error of its
/// field; the comparison of the groups needs a group in both expressions.
pub fn compile(spec: &Spec) -> Result<Compiled, Errors> {
    let mut errors: [Option<String>; N_FIELDS] = Default::default();
    let mut fields: [Field; N_FIELDS] = std::array::from_fn(|_| Field::Any);
    for i in 0..N_FIELDS {
        let t = spec.texts[i].trim();
        if t.is_empty() {
            continue;
        }
        if spec.regex[i] {
            match regex::RegexBuilder::new(t)
                .case_insensitive(!spec.case[i])
                .size_limit(1 << 20)
                .build()
            {
                Ok(re) => fields[i] = Field::Regex(re),
                Err(e) => errors[i] = Some(e.to_string()),
            }
        } else {
            fields[i] = Field::Plain {
                needle: if spec.case[i] {
                    t.to_string()
                } else {
                    t.to_lowercase()
                },
                case: spec.case[i],
            };
        }
    }
    if errors.iter().any(Option::is_some) {
        return Err(Box::new(errors));
    }
    let formid = if spec.regex[F_ID] {
        None
    } else {
        parse_formid(spec.texts[F_ID].trim())
    };
    let groups = if spec.groups != Groups::None && has_groups(&fields) {
        spec.groups
    } else {
        Groups::None
    };
    Ok(Compiled {
        fields,
        formid,
        groups,
        statuses: spec.statuses,
        unused: spec.unused,
    })
}

/// Both expressions have at least one capture group.
fn has_groups(fields: &[Field; N_FIELDS]) -> bool {
    matches!(
        (&fields[F_SOURCE], &fields[F_TRANS]),
        (Field::Regex(a), Field::Regex(b)) if a.captures_len() > 1 && b.captures_len() > 1
    )
}

/// Whether the groups comparison is available for a spec (the option is
/// greyed otherwise).
pub fn groups_available(spec: &Spec) -> bool {
    spec.groups_possible()
        && compile(&Spec {
            groups: Groups::None,
            ..spec.clone()
        })
        .is_ok_and(|c| has_groups(&c.fields))
}

/// The captured groups of a text, in order, trimmed; `None` when the
/// expression does not match.
fn captured(re: &regex::Regex, s: &str) -> Option<Vec<String>> {
    let caps = re.captures(s)?;
    Some(
        (1..caps.len())
            .map(|i| {
                caps.get(i)
                    .map(|m| m.as_str().trim())
                    .unwrap_or("")
                    .to_string()
            })
            .collect(),
    )
}

impl Compiled {
    /// Both expressions have a group: the comparison can be chosen.
    pub fn groups_ready(&self) -> bool {
        has_groups(&self.fields)
    }

    /// Whether the keyword is looked for in the comments (the generated
    /// texts are then wanted).
    pub fn needs_comments(&self) -> bool {
        !self.fields[F_KEYWORD].is_any()
    }

    /// The groups of the source and the translation of a line, when the
    /// comparison is on and both match.
    pub fn groups_of(&self, e: &TmEntry) -> Option<(Vec<String>, Vec<String>)> {
        match (&self.fields[F_SOURCE], &self.fields[F_TRANS]) {
            (Field::Regex(a), Field::Regex(b)) if self.groups != Groups::None => {
                Some((captured(a, &e.source)?, captured(b, &e.trans)?))
            }
            _ => None,
        }
    }

    /// Whether line `i` passes every criterion (R-A19-01/02).
    pub fn matches(&self, e: &TmEntry, i: usize, comments: Option<&CommentTexts>) -> bool {
        if e.unused && !self.unused {
            return false;
        }
        if !self.statuses[status::derive(e).index()] {
            return false;
        }
        let f = &self.fields;
        if !f[F_SOURCE].hits(&e.source) || !f[F_TRANS].hits(&e.trans) {
            return false;
        }
        if !f[F_ID].is_any() {
            let by_id = self
                .formid
                .is_some_and(|v| e.formid == v || (e.formid & 0x00FF_FFFF) == (v & 0x00FF_FFFF));
            if !by_id && !f[F_ID].hits(&e.edid) && !f[F_ID].hits(&format!("{:08X}", e.formid)) {
                return false;
            }
        }
        if !f[F_RECORD].hits(&e.rec_sig) || !f[F_FIELD].hits(&e.field_sig) {
            return false;
        }
        if !f[F_KEYWORD].is_any() {
            let k = &f[F_KEYWORD];
            let comment = comments
                .and_then(|c| c.0.get(i))
                .map(String::as_str)
                .unwrap_or(&e.comment);
            let sig = format!("{}:{}", e.rec_sig, e.field_sig);
            if !(k.hits(&e.source)
                || k.hits(&e.trans)
                || k.hits(&e.edid)
                || k.hits(&sig)
                || k.hits(comment))
            {
                return false;
            }
        }
        match self.groups {
            Groups::None => true,
            g => match self.groups_of(e) {
                None => false,
                Some((a, b)) => (a == b) == (g == Groups::Same),
            },
        }
    }

    /// The lines that pass, in order.
    pub fn apply(&self, entries: &[TmEntry], comments: Option<&CommentTexts>) -> Vec<usize> {
        (0..entries.len())
            .filter(|&i| self.matches(&entries[i], i, comments))
            .collect()
    }
}

/// The result of a search over a document: the lines and the first
/// differing group pairs (the preview, R-A19-03).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub lines: Vec<usize>,
    pub pairs: Vec<(String, String)>,
}

pub fn find(c: &Compiled, entries: &[TmEntry], comments: Option<&CommentTexts>) -> Found {
    let lines = c.apply(entries, comments);
    let mut pairs = Vec::new();
    if c.groups == Groups::Differ {
        for &i in &lines {
            if let Some((a, b)) = c.groups_of(&entries[i]) {
                let pair = (a.join(" "), b.join(" "));
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
            if pairs.len() == 3 {
                break;
            }
        }
    }
    Found { lines, pairs }
}

/// « FormID ⇥ EDID ⇥ source ⇥ traduction », one line per result.
pub fn copy_list(entries: &[TmEntry], lines: &[usize]) -> String {
    let one = |s: &str| s.replace(['\r', '\n'], " ");
    lines
        .iter()
        .filter_map(|&i| entries.get(i))
        .map(|e| {
            format!(
                "{:08X}\t{}\t{}\t{}",
                e.formid,
                e.edid,
                one(&e.source),
                one(&e.trans)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------- presets

/// A named spec. A shipped preset carries the message key of its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    pub spec: Spec,
    /// From the shipped file (read-only), not the user's copy.
    #[serde(skip)]
    pub shipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PresetFile {
    format: u32,
    presets: Vec<Preset>,
}

pub const PRESETS_FILE: &str = "search_presets.json";
pub const PRESETS_FORMAT: u32 = 1;

fn read_file(path: &Path) -> Vec<Preset> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PresetFile>(&s).ok())
        .map(|f| f.presets)
        .unwrap_or_default()
}

/// The presets: the shipped ones, then the user's, a user preset winning
/// over a shipped one of the same name (R-A19-05).
pub fn load(shipped: Option<&Path>, user: &Path) -> Vec<Preset> {
    let mut out: Vec<Preset> = shipped
        .map(read_file)
        .unwrap_or_default()
        .into_iter()
        .map(|p| Preset { shipped: true, ..p })
        .collect();
    for p in read_file(user) {
        match out.iter_mut().find(|q| q.name == p.name) {
            Some(q) => *q = p,
            None => out.push(p),
        }
    }
    out
}

/// Write the user's presets (the non-shipped ones).
pub fn save_user(presets: &[Preset], user: &Path) -> Result<(), String> {
    let file = PresetFile {
        format: PRESETS_FORMAT,
        presets: presets.iter().filter(|p| !p.shipped).cloned().collect(),
    };
    if let Some(dir) = user.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(user, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rec: &str, field: &str, edid: &str, src: &str, trans: &str) -> TmEntry {
        let mut e = TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            edid: edid.into(),
            source: src.into(),
            trans: trans.into(),
            formid: 0x0A00_1234,
            ..Default::default()
        };
        e.set_translated_flags(!trans.is_empty());
        e
    }

    fn spec(texts: [&str; N_FIELDS], regex: [bool; N_FIELDS]) -> Spec {
        Spec {
            texts: texts.map(String::from),
            regex,
            ..Default::default()
        }
    }

    #[test]
    fn criteria_combine_and_groups_compare() {
        let es = vec![
            entry("ARMO", "FULL", "IronHelm", "Iron Helmet", "Casque en fer"),
            entry("WEAP", "FULL", "IronSword", "Iron Sword", "Épée en fer"),
            entry(
                "MESG",
                "DESC",
                "MsgDays",
                "You have 3 days left",
                "Il reste 5 jours",
            ),
            entry("MESG", "DESC", "MsgGold", "12 gold", "12 pièces"),
        ];
        // T-A19-01: GRUP, Champ and translation together.
        let s = spec(["", "fer", "", "armo", "FULL", ""], [false; N_FIELDS]);
        assert_eq!(compile(&s).unwrap().apply(&es, None), vec![0]);
        // T-A19-02: numbers compared.
        let mut s = spec(
            [r"(\d+)", r"(\d+)", "", "", "", ""],
            [true, true, false, false, false, false],
        );
        assert!(groups_available(&s));
        s.groups = Groups::Differ;
        let c = compile(&s).unwrap();
        let f = find(&c, &es, None);
        assert_eq!(f.lines, vec![2]);
        assert_eq!(f.pairs, vec![("3".to_string(), "5".to_string())]);
        s.groups = Groups::Same;
        assert_eq!(compile(&s).unwrap().apply(&es, None), vec![3]);
        // T-A19-04: an invalid expression is the error of its field.
        let s = spec(
            ["[", "", "", "", "", ""],
            [true, false, false, false, false, false],
        );
        let err = compile(&s).unwrap_err();
        assert!(err[F_SOURCE].is_some() && err[F_TRANS].is_none());
        // Case, FormID, keyword in the EDID.
        let mut s = spec(["iron", "", "", "", "", ""], [false; N_FIELDS]);
        s.case[F_SOURCE] = true;
        assert!(compile(&s).unwrap().apply(&es, None).is_empty());
        let s = spec(["", "", "001234", "", "", ""], [false; N_FIELDS]);
        assert_eq!(compile(&s).unwrap().apply(&es, None).len(), 4);
        let s = spec(["", "", "", "", "", "msggold"], [false; N_FIELDS]);
        assert_eq!(compile(&s).unwrap().apply(&es, None), vec![3]);
        // No group in an expression: the comparison does not apply.
        let s = Spec {
            groups: Groups::Differ,
            ..spec(
                ["Iron", "fer", "", "", "", ""],
                [true, true, false, false, false, false],
            )
        };
        assert!(!groups_available(&s));
        assert_eq!(compile(&s).unwrap().apply(&es, None), vec![0, 1]);
        assert_eq!(
            copy_list(&es, &[3]),
            "0A001234\tMsgGold\t12 gold\t12 pièces"
        );
    }

    #[test]
    fn presets_load_merge_and_save() {
        let dir = std::env::temp_dir().join(format!("ximod-presets-{}", std::process::id()));
        let shipped = dir.join("shipped.json");
        let user = dir.join("data").join(PRESETS_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        let file = PresetFile {
            format: 1,
            presets: vec![
                Preset {
                    name: "A".into(),
                    key: "k-a".into(),
                    spec: Spec::default(),
                    shipped: false,
                },
                Preset {
                    name: "B".into(),
                    key: String::new(),
                    spec: Spec::default(),
                    shipped: false,
                },
            ],
        };
        std::fs::write(&shipped, serde_json::to_string(&file).unwrap()).unwrap();
        let mut ps = load(Some(&shipped), &user);
        assert_eq!(ps.len(), 2);
        assert!(ps.iter().all(|p| p.shipped));
        // A user preset named like a shipped one replaces it.
        let mut mine = Spec::default();
        mine.texts[F_SOURCE] = "x".into();
        ps.push(Preset {
            name: "B".into(),
            key: String::new(),
            spec: mine.clone(),
            shipped: false,
        });
        ps.push(Preset {
            name: "C".into(),
            key: String::new(),
            spec: Spec::default(),
            shipped: false,
        });
        save_user(&ps, &user).unwrap();
        let again = load(Some(&shipped), &user);
        assert_eq!(
            again.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
        assert_eq!(again[1].spec, mine);
        assert!(!again[1].shipped && again[0].shipped);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
