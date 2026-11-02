//! Conditions in clear (C-02): a CTDA decoded through the function table of
//! its family (`assets/data/conditions/<family>.json`) — the function's
//! name and parameter types — and worded in the interface language for the
//! common functions, generically otherwise (« Fonction(p1, p2) op valeur »).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::dialogue::Resolver;
use crate::i18n::I18n;
use crate::plugin::context::{Condition, Family};

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Function {
    pub index: u32,
    pub name: String,
    #[serde(default)]
    pub params: Vec<String>,
    /// The i18n key suffix of the phrase in clear (`cond-<phrase>`).
    #[serde(default)]
    pub phrase: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub family: String,
    /// Added to the stored index to reach the table's index.
    #[serde(default)]
    pub index_offset: i64,
    /// Enumerated parameter types: type → value → label (an i18n key when
    /// it starts with « cond- »).
    #[serde(default)]
    pub enums: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub functions: Vec<Function>,
    #[serde(skip)]
    by_index: HashMap<u32, usize>,
}

static TABLES: OnceLock<Mutex<HashMap<Family, Arc<Table>>>> = OnceLock::new();

impl Table {
    /// The table of a family, loaded once per session.
    pub fn load(family: Family) -> Arc<Table> {
        let cache = TABLES.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(t) = cache.lock().ok().and_then(|c| c.get(&family).cloned()) {
            return t;
        }
        let rel = format!("conditions/{}.json", family.stem());
        let mut t: Table = crate::data::find_data_file(&rel)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        t.index();
        let t = Arc::new(t);
        if let Ok(mut c) = cache.lock() {
            c.insert(family, t.clone());
        }
        t
    }

    /// A table from JSON text (tests).
    pub fn from_json(text: &str) -> Table {
        let mut t: Table = serde_json::from_str(text).unwrap_or_default();
        t.index();
        t
    }

    fn index(&mut self) {
        self.by_index = self
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.index, i))
            .collect();
    }

    pub fn function(&self, stored: u32) -> Option<&Function> {
        let idx = (stored as i64 + self.index_offset).max(0) as u32;
        self.by_index.get(&idx).map(|i| &self.functions[*i])
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

/// The operator as text.
pub fn op_text(op: u8) -> &'static str {
    match op {
        0 => "=",
        1 => "≠",
        2 => ">",
        3 => "≥",
        4 => "<",
        5 => "≤",
        _ => "?",
    }
}

/// A number without a useless decimal part.
fn num(v: f32) -> String {
    if v.fract() == 0.0 && v.abs() < 1.0e9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// One parameter, named through its type.
fn param(
    ptype: Option<&str>,
    raw: u32,
    cis: Option<&str>,
    table: &Table,
    res: &Resolver,
    i18n: &I18n,
) -> String {
    if let Some(s) = cis {
        return i18n.t_arg("text-quoted", "text", s);
    }
    match ptype {
        Some("Int") | Some("Float") => raw.to_string(),
        Some("String") | Some("Variable") => format!("#{raw}"),
        Some("Alias") => format!("#{raw}"),
        Some("Axis") => ["X", "Y", "Z"]
            .get(raw as usize)
            .map(|s| s.to_string())
            .unwrap_or_else(|| raw.to_string()),
        Some(t) if table.enums.contains_key(t) => {
            let label = table.enums[t]
                .get(&raw.to_string())
                .cloned()
                .unwrap_or_else(|| raw.to_string());
            if label.starts_with("cond-") {
                i18n.t(&label)
            } else {
                label
            }
        }
        Some("Quest") => {
            // Quests are known by their EDID.
            if raw == 0 {
                "—".to_string()
            } else {
                res.edid_or_full(raw)
            }
        }
        Some(_) => {
            // A form.
            if raw == 0 {
                "—".to_string()
            } else {
                res.display(raw)
            }
        }
        None => {
            // Unknown type: a form when the value looks like one and is known,
            // a number otherwise.
            if raw > 0xFF
                && res.display(raw).len() < 9 + 20
                && !looks_like_formid_only(&res.display(raw), raw)
            {
                res.display(raw)
            } else {
                raw.to_string()
            }
        }
    }
}

fn looks_like_formid_only(s: &str, raw: u32) -> bool {
    s.starts_with(&format!("{raw:08X}"))
}

/// The alias name of a quest, for GetIsAliasRef and the like.
fn alias_name(quest: Option<u32>, id: u32, res: &Resolver) -> String {
    quest
        .and_then(|q| res.quest(q))
        .and_then(|q| q.aliases.iter().find(|a| a.id == id))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| format!("#{id}"))
}

/// The subject the condition runs on, as a prefix (« sur l'interlocuteur : »).
fn target_prefix(c: &Condition, res: &Resolver, i18n: &I18n) -> String {
    match c.run_on {
        0 => String::new(),
        1 => format!("{} ", i18n.t("cond-on-target")),
        2 => format!(
            "{} ",
            i18n.t_arg("cond-on-ref", "ref", &res.display(c.reference))
        ),
        3 => format!("{} ", i18n.t("cond-on-combat-target")),
        4 => format!("{} ", i18n.t("cond-on-linked-ref")),
        5 => format!(
            "{} ",
            i18n.t_arg("cond-on-alias", "n", &c.param3.to_string())
        ),
        _ => format!("{} ", i18n.t("cond-on-other")),
    }
}

/// Describe the conditions of a reply: (text in clear, raw form), the
/// « et » / « ou » chaining carried by the text of each line.
pub fn describe(
    conds: &[Condition],
    res: &Resolver,
    table: &Table,
    i18n: &I18n,
) -> Vec<(String, String)> {
    describe_in(conds, None, res, table, i18n)
}

/// [`describe`] with the quest the aliases belong to.
pub fn describe_in(
    conds: &[Condition],
    quest: Option<u32>,
    res: &Resolver,
    table: &Table,
    i18n: &I18n,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, c) in conds.iter().enumerate() {
        let f = table.function(c.function);
        let name = f
            .map(|f| f.name.clone())
            .unwrap_or_else(|| i18n.t_arg("cond-unknown-fn", "index", &c.function.to_string()));
        let p1t = f.and_then(|f| f.params.first()).map(String::as_str);
        let p2t = f.and_then(|f| f.params.get(1)).map(String::as_str);
        let value = match c.global {
            Some(g) => i18n.t_arg("cond-global-value", "name", &res.display(g)),
            None => num(c.value),
        };
        let op = op_text(c.op);
        // Raw form: GetIsID [NPC_:01000A01 Ralof] == 1.
        let raw_p = |t: Option<&str>, raw: u32, cis: Option<&str>| -> String {
            if let Some(s) = cis {
                return format!("\"{s}\"");
            }
            match t {
                Some("Int") | Some("Float") | Some("Axis") | Some("Alias") | Some("Sex") => {
                    raw.to_string()
                }
                Some(_) if raw != 0 => format!("[{raw:08X} {}]", res.display(raw)),
                None if raw > 0xFF => format!("[{raw:08X}]"),
                _ => raw.to_string(),
            }
        };
        let raw_op = ["==", "!=", ">", ">=", "<", "<="]
            .get(c.op as usize)
            .copied()
            .unwrap_or("?");
        let raw = format!(
            "{}{name}({}, {}) {raw_op} {value}{}",
            match c.run_on {
                0 => String::new(),
                n => format!("{} ", i18n.t_arg("cond-run-on", "n", &n.to_string())),
            },
            raw_p(p1t, c.param1, c.cis1.as_deref()),
            raw_p(p2t, c.param2, c.cis2.as_deref()),
            if c.is_or() { " OR" } else { "" }
        );
        // In clear.
        let p1 = if p1t == Some("Alias") {
            alias_name(quest, c.param1, res)
        } else {
            param(p1t, c.param1, c.cis1.as_deref(), table, res, i18n)
        };
        let p2 = param(p2t, c.param2, c.cis2.as_deref(), table, res, i18n);
        let prefix = target_prefix(c, res, i18n);
        let truthy = c.global.is_none()
            && ((c.op == 0 && c.value != 0.0)
                || (c.op == 1 && c.value == 0.0)
                || (c.op == 2 && c.value == 0.0)
                || (c.op == 3 && c.value > 0.0));
        let falsy =
            c.global.is_none() && ((c.op == 0 && c.value == 0.0) || (c.op == 1 && c.value != 0.0));
        let phrase = f.and_then(|f| f.phrase.as_deref());
        let mut text = None;
        if let Some(ph) = phrase {
            let key = if falsy {
                format!("cond-{ph}-not")
            } else {
                format!("cond-{ph}")
            };
            let subject = if c.run_on == 1 {
                i18n.t("cond-subject-target")
            } else {
                i18n.t("cond-subject-speaker")
            };
            let args: Vec<(&str, &str)> = vec![
                ("p1", &p1),
                ("p2", &p2),
                ("op", op),
                ("value", &value),
                ("subject", &subject),
            ];
            if truthy
                || falsy
                || !matches!(
                    ph,
                    "GetIsID"
                        | "GetIsSex"
                        | "GetIsRace"
                        | "GetInFaction"
                        | "GetIsAliasRef"
                        | "GetIsVoiceType"
                        | "GetDead"
                        | "IsInCombat"
                        | "IsSneaking"
                        | "IsWeaponOut"
                        | "GetSleeping"
                        | "IsChild"
                        | "GetIsPlayableRace"
                        | "IsBribedbyPlayer"
                        | "IsIntimidatedbyPlayer"
                        | "GetTalkedToPC"
                        | "GetIsCreature"
                        | "GetQuestCompleted"
                        | "GetQuestRunning"
                        | "GetStageDone"
                        | "HasKeyword"
                        | "HasPerk"
                        | "HasSpell"
                        | "HasMagicEffect"
                        | "IsInList"
                        | "GetIsReference"
                        | "IsInInterior"
                        | "GetIsCrimeFaction"
                        | "GetObjectiveCompleted"
                        | "GetObjectiveDisplayed"
                        | "IsInDialogueWithPlayer"
                        | "GetIsEditorLocation"
                        | "WornHasKeyword"
                        | "IsRaining"
                        | "IsSnowing"
                        | "SameSex"
                        | "SameRace"
                        | "SameFaction"
                        | "GetIsAlignment"
                        | "GetPCIsSex"
                        | "GetIsClass"
                        | "GetInCurrentLoc"
                        | "GetInCell"
                        | "GetInWorldspace"
                        | "GetIsCurrentPackage"
                        | "GetEquipped"
                )
            {
                let t = i18n.t_args(&key, &args);
                // A missing key gives the key back: fall through to the generic form.
                if t != key {
                    text = Some(format!("{prefix}{t}"));
                }
            }
        }
        let text = text.unwrap_or_else(|| {
            let params: Vec<String> = [(p1t, &p1, c.param1), (p2t, &p2, c.param2)]
                .iter()
                .filter(|(t, _, raw)| t.is_some() || *raw != 0)
                .map(|(_, s, _)| s.to_string())
                .collect();
            let fname = name.clone();
            format!(
                "{prefix}{}",
                i18n.t_args(
                    "cond-generic",
                    &[
                        ("fn", &fname),
                        ("params", &params.join(", ")),
                        ("op", op),
                        ("value", &value)
                    ]
                )
            )
        });
        // The chaining word: « ou » when this line is OR-ed with the next.
        let joiner = if i + 1 < conds.len() {
            if c.is_or() {
                format!(" {}", i18n.t("cond-or"))
            } else {
                format!(" {}", i18n.t("cond-and"))
            }
        } else {
            String::new()
        };
        out.push((format!("{text}{joiner}"), raw));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::context::ContextMap;

    fn fixture() -> (Resolver, Arc<Table>, I18n) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/skyrim_dialogue.esp");
        let ctx = Arc::new(ContextMap::read_file(&p, Family::Skyrim).unwrap());
        let res = Resolver::new(ctx);
        let mut i18n = I18n::new();
        i18n.set_locale("fra");
        (res, Table::load(Family::Skyrim), i18n)
    }

    /// T-C02-02: the conditions of the fixture in clear and raw.
    #[test]
    fn conditions_in_clear() {
        let (res, table, i18n) = fixture();
        assert!(!table.is_empty());
        assert_eq!(table.function(72).unwrap().name, "GetIsID");
        let i1 = &res.ctx.infos[&0x01000301];
        let d = describe_in(&i1.conditions, Some(0x01000101), &res, &table, &i18n);
        assert_eq!(d[0].0, "si le locuteur est Ralof et");
        assert_eq!(d[0].1, "GetIsID([01000A01 Ralof], 0) == 1");
        assert_eq!(d[1].0, "si la quête MQ101 est à l'étape ≥ 20");
        let i2 = &res.ctx.infos[&0x01000302];
        let d = describe_in(&i2.conditions, Some(0x01000101), &res, &table, &i18n);
        assert_eq!(d[0].0, "si le locuteur est Lokir ou");
        assert_eq!(
            d[1].0,
            "sur l'interlocuteur : si l'interlocuteur est une femme"
        );
        let i4 = &res.ctx.infos[&0x01000304];
        let d = describe_in(&i4.conditions, Some(0x01000101), &res, &table, &i18n);
        assert_eq!(d[0].0, "si le locuteur est l'alias « Ralof » et");
        // HasKeyword on a keyword of the master, unresolved: the FormID and the file.
        assert_eq!(
            d[1].0,
            "sur l'interlocuteur : si l'interlocuteur a le mot-clé 0001F3A0 (Skyrim.esm)"
        );
        // A function out of the table.
        let c = Condition {
            function: 4096,
            op: 0,
            value: 1.0,
            ..Default::default()
        };
        let d = describe(&[c], &res, &table, &i18n);
        assert_eq!(d[0].0, "Fonction #4096() = 1");
        // A generic one with a known name but no phrase.
        let c = Condition {
            function: 77,
            op: 4,
            value: 50.0,
            ..Default::default()
        };
        let d = describe(&[c], &res, &table, &i18n);
        assert_eq!(d[0].0, "si un tirage aléatoire est < 50");
        // A known name without a phrase: the generic form.
        let c = Condition {
            function: 80 + 1,
            op: 0,
            value: 1.0,
            ..Default::default()
        };
        let d = describe(&[c], &res, &table, &i18n);
        assert_eq!(d[0].0, "IsRotating() = 1");
        // A global value.
        let c = Condition {
            function: 58,
            op: 3,
            flags: 0x04,
            global: Some(0x01000D01),
            param1: 0x01000101,
            ..Default::default()
        };
        let d = describe(&[c], &res, &table, &i18n);
        assert_eq!(
            d[0].0,
            "si la quête MQ101 est à l'étape ≥ la valeur globale MQ101Progress"
        );
    }

    #[test]
    fn tables_load_for_every_family() {
        for f in [
            Family::Skyrim,
            Family::Fallout4,
            Family::Fallout3,
            Family::Oblivion,
            Family::Starfield,
            Family::Morrowind,
        ] {
            let t = Table::load(f);
            assert!(!t.is_empty(), "{f:?}");
        }
        assert_eq!(
            Table::load(Family::Fallout4).function(231).unwrap().name,
            "GetPlayerDialogueInput"
        );
        assert_eq!(
            Table::load(Family::Starfield).function(960).unwrap().name,
            "EPIsResistanceActorValue"
        );
    }
}
