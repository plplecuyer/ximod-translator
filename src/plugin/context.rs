//! The context pass (C-01): a second streaming walk of a TES4-family plugin
//! that reads what the dialogues need — quests and their aliases, topics,
//! replies with their responses and conditions, scenes, NPCs, voice types
//! and the names of the forms the conditions refer to. The extraction of
//! the translatable text is untouched: the map only carries structure and
//! identifiers, the text comes from the document's lines.

use std::collections::HashMap;

use anyhow::{Result, bail};

use super::reader::{decompress_record, detect_header_len, parse_subrecords, sig4};
use super::record::{FLAG_COMPRESSED, FLAG_LOCALIZED, SubRecord};

/// The family of record layouts, which decides a few field sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Family {
    /// Skyrim LE / SE / VR.
    Skyrim,
    Fallout4,
    /// Fallout 3 and New Vegas.
    Fallout3,
    /// Oblivion and Oblivion Remastered.
    Oblivion,
    Starfield,
    Morrowind,
}

impl Family {
    pub fn of(game: crate::games::Game) -> Family {
        use crate::games::Game as G;
        match game {
            G::Morrowind => Family::Morrowind,
            G::Oblivion | G::OblivionRD => Family::Oblivion,
            G::Fallout3 | G::FalloutNV => Family::Fallout3,
            G::Fallout4 | G::Fallout76 => Family::Fallout4,
            G::Starfield => Family::Starfield,
            _ => Family::Skyrim,
        }
    }

    /// File stem of the data files of the family (`dialogue/<stem>.json`,
    /// `conditions/<stem>.json`).
    pub fn stem(self) -> &'static str {
        match self {
            Family::Skyrim => "skyrim",
            Family::Fallout4 => "fallout4",
            Family::Fallout3 => "fallout3",
            Family::Oblivion => "oblivion",
            Family::Starfield => "starfield",
            Family::Morrowind => "morrowind",
        }
    }
}

/// One response of a reply (TRDT + NAM1 + NAM2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Response {
    pub number: u8,
    pub emotion: u32,
    pub emotion_value: u32,
    /// The text, inline plugins only (empty for a localized one).
    pub text: String,
    /// The string id, localized plugins only.
    pub text_id: Option<u32>,
    /// NAM2, the author's note.
    pub note: String,
    /// 1-based index of the NAM1 among the lines the extraction keeps for
    /// this record (an empty inline response makes no line).
    pub line_index: Option<i32>,
}

/// One CTDA, raw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Condition {
    /// Comparison operator: 0 ==, 1 !=, 2 >, 3 >=, 4 <, 5 <=.
    pub op: u8,
    /// Flags: 0x01 OR, 0x02 aliases, 0x04 use global, 0x08 pack data, 0x10 swap.
    pub flags: u8,
    pub value: f32,
    pub global: Option<u32>,
    pub function: u32,
    pub param1: u32,
    pub param2: u32,
    pub run_on: u32,
    pub reference: u32,
    pub param3: i32,
    pub cis1: Option<String>,
    pub cis2: Option<String>,
}

impl Condition {
    pub fn is_or(&self) -> bool {
        self.flags & 0x01 != 0
    }
    pub fn uses_aliases(&self) -> bool {
        self.flags & 0x02 != 0
    }
}

/// A reply (INFO).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    pub formid: u32,
    pub topic: u32,
    /// Position in the file among the replies (the game's priority order).
    pub order: usize,
    pub prev: Option<u32>,
    /// TCLT: the topics that follow.
    pub links: Vec<u32>,
    /// DNAM: the reply whose responses are shared.
    pub shared: Option<u32>,
    /// ANAM.
    pub speaker: Option<u32>,
    /// RNAM: the player's own words (has a line RNAM #1 when not empty).
    pub prompt: bool,
    pub responses: Vec<Response>,
    pub conditions: Vec<Condition>,
}

/// A topic (DIAL).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Topic {
    pub formid: u32,
    pub edid: String,
    pub full: String,
    /// The string id of FULL, localized plugins.
    #[serde(default)]
    pub full_id: Option<u32>,
    pub quest: Option<u32>,
    pub branch: Option<u32>,
    pub category: u8,
    pub subtype: u16,
    /// SNAM, the four-letter subtype code (Skyrim+).
    pub snam: String,
    pub order: usize,
    /// Its replies, in file order.
    pub infos: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Alias {
    pub id: u32,
    pub name: String,
    /// ALFR: the forced reference (a REFR / ACHR).
    pub forced_ref: Option<u32>,
    /// ALUA: the unique actor (an NPC_).
    pub unique_actor: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Quest {
    pub formid: u32,
    pub edid: String,
    pub full: String,
    #[serde(default)]
    pub full_id: Option<u32>,
    pub order: usize,
    pub aliases: Vec<Alias>,
    /// The stage (INDX) of each journal entry (CNAM) the reader keeps, in
    /// file order: the n-th CNAM line of the grid belongs to `stages[n-1]`.
    pub stages: Vec<u16>,
    /// The objective (QOBJ) of each NNAM line, in file order.
    pub objectives: Vec<u16>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneAction {
    /// ANAM: 0 dialogue, 1 package, 2 timer.
    pub kind: u16,
    pub name: String,
    pub alias: Option<u32>,
    pub index: u32,
    pub start_phase: u32,
    pub end_phase: u32,
    /// DATA of a dialogue action: the topic.
    pub topic: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub formid: u32,
    pub edid: String,
    pub quest: Option<u32>,
    pub order: usize,
    pub phases: Vec<String>,
    pub actions: Vec<SceneAction>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Npc {
    pub edid: String,
    pub full: String,
    #[serde(default)]
    pub full_id: Option<u32>,
    pub female: bool,
    pub voice: Option<u32>,
    pub race: Option<u32>,
}

/// A form the conditions may name.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Named {
    pub sig: String,
    pub edid: String,
    pub full: String,
    #[serde(default)]
    pub full_id: Option<u32>,
    /// MODL path of an object (C-03), kept by the full pass only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// QNAM owner quest of a MESG, PERK, SPEL… (C-03).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest: Option<u32>,
}

/// The dialogue context of one plugin.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContextMap {
    pub plugin_name: String,
    pub family: Option<Family>,
    pub masters: Vec<String>,
    pub localized: bool,
    pub quests: HashMap<u32, Quest>,
    pub topics: HashMap<u32, Topic>,
    #[serde(skip)]
    pub infos: HashMap<u32, Info>,
    #[serde(skip)]
    pub scenes: HashMap<u32, Scene>,
    pub npcs: HashMap<u32, Npc>,
    /// VTYP: EDID.
    pub voices: HashMap<u32, String>,
    /// FACT, KYWD, GLOB, RACE, LCTN, CELL, WRLD, CLAS, PERK, SPEL, MGEF…
    pub names: HashMap<u32, Named>,
    /// ACHR / REFR: the base form.
    #[serde(skip)]
    pub refs: HashMap<u32, u32>,
    /// Records the context pass looked at.
    pub records: usize,
}

const NAMED: &[&str] = &[
    "FACT", "KYWD", "GLOB", "RACE", "LCTN", "CELL", "WRLD", "CLAS", "PERK", "SPEL", "MGEF", "FLST",
    "PACK", "WEAP", "ARMO", "MISC", "ALCH", "BOOK", "KEYM", "INGR", "SHOU", "WOOP", "FURN", "ACTI",
    "AVIF", "ASTP", "ENCH", "SCRL", "AMMO", "LVLI", "LVLN", "IDLE", "WTHR", "REGN", "PLYR", "MESG",
    "CONT", "FLOR", "LIGH", "DOOR", "NOTE", "TERM", "SLGM", "APPA", "TACT", "HAZD", "EXPL", "PROJ",
    "TREE", "STAT",
];

/// Whether a subrecord carries text the reader would keep as a line: a
/// non-empty zstring, or a string id in a localized plugin (C-03: the
/// n-th kept CNAM / NNAM is the n-th line of the grid).
fn is_text_line(s: &SubRecord, localized: bool) -> bool {
    if localized {
        s.as_u32().is_some()
    } else {
        !s.as_zstring().is_empty()
    }
}

fn read_u16(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn read_i32(d: &[u8], p: usize) -> i32 {
    i32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn read_f32(d: &[u8], p: usize) -> f32 {
    f32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

fn text_of(sub: &SubRecord, localized: bool) -> (String, Option<u32>) {
    if localized {
        (String::new(), sub.as_u32())
    } else {
        (sub.as_zstring(), None)
    }
}

impl ContextMap {
    /// Whether a record type takes part in the context; the light pass (the
    /// name index of a master) leaves the replies, scenes and references out.
    pub fn wants(sig: &str, light: bool) -> bool {
        if light {
            matches!(sig, "QUST" | "DIAL" | "NPC_" | "VTYP") || NAMED.contains(&sig)
        } else {
            matches!(
                sig,
                "QUST" | "DIAL" | "INFO" | "SCEN" | "NPC_" | "VTYP" | "ACHR" | "REFR"
            ) || NAMED.contains(&sig)
        }
    }

    /// Read the context of a plugin's bytes.
    pub fn read(data: &[u8], plugin_name: &str, family: Family) -> Result<ContextMap> {
        Self::read_with(data, plugin_name, family, false, &mut |_, _| true)
    }

    /// [`read`](Self::read) with the light mode (names only) and a progress
    /// callback `(bytes done, bytes total) -> keep going`, called per
    /// top-level group.
    pub fn read_with(
        data: &[u8],
        plugin_name: &str,
        family: Family,
        light: bool,
        progress: &mut dyn FnMut(usize, usize) -> bool,
    ) -> Result<ContextMap> {
        if data.len() < 16 {
            bail!(crate::l10n!("err-plugin-short"));
        }
        let header_len = detect_header_len(data);
        let hdr_flags = read_u32(data, 8);
        let mut map = ContextMap {
            plugin_name: plugin_name.to_string(),
            family: Some(family),
            localized: hdr_flags & FLAG_LOCALIZED != 0,
            ..Default::default()
        };
        // Masters from the TES4 header.
        let hdr_size = read_u32(data, 4) as usize;
        if header_len + hdr_size <= data.len() {
            for s in parse_subrecords(&data[header_len..header_len + hdr_size]) {
                if s.sig == "MAST" {
                    map.masters.push(s.as_zstring());
                }
            }
        }
        let mut order = 0usize;
        // Top level: one group at a time, so the progress can be reported.
        let mut p = 0usize;
        while p + header_len <= data.len() {
            let sig = sig4(&data[p..p + 4]);
            let size = read_u32(data, p + 4) as usize;
            if sig == "GRUP" {
                if size < header_len || p + size > data.len() {
                    break;
                }
                let label = sig4(&data[p + 8..p + 12]);
                if ContextMap::wants(&label, light) {
                    walk(
                        &data[p + header_len..p + size],
                        1,
                        header_len,
                        None,
                        family,
                        light,
                        &mut map,
                        &mut order,
                    )?;
                }
                p += size;
            } else {
                p += header_len + size;
            }
            if !progress(p.min(data.len()), data.len()) {
                bail!(crate::l10n!("err-cancelled"));
            }
        }
        Ok(map)
    }

    /// Resolve the FULL names of a localized plugin from its STRINGS.
    pub fn resolve_strings(&mut self, bundle: &super::StringsBundle) {
        let get = |id: Option<u32>| id.and_then(|i| bundle.get(i)).map(str::to_string);
        for q in self.quests.values_mut() {
            if let Some(t) = get(q.full_id) {
                q.full = t;
            }
        }
        for t in self.topics.values_mut() {
            if let Some(s) = get(t.full_id) {
                t.full = s;
            }
        }
        for n in self.npcs.values_mut() {
            if let Some(t) = get(n.full_id) {
                n.full = t;
            }
        }
        for n in self.names.values_mut() {
            if let Some(t) = get(n.full_id) {
                n.full = t;
            }
        }
    }

    /// Read the context of a plugin file (memory-mapped).
    pub fn read_file(path: &std::path::Path, family: Family) -> Result<ContextMap> {
        Self::read_file_with(path, family, false, &mut |_, _| true)
    }

    /// [`read_file`](Self::read_file) in light mode and with progress.
    pub fn read_file_with(
        path: &std::path::Path,
        family: Family,
        light: bool,
        progress: &mut dyn FnMut(usize, usize) -> bool,
    ) -> Result<ContextMap> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the file is only read; a concurrent modification would at
        // worst yield garbage that the parser rejects.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plugin")
            .to_string();
        Self::read_with(&mmap, &name, family, light, progress)
    }

    pub fn is_empty(&self) -> bool {
        self.infos.is_empty() && self.topics.is_empty()
    }

    /// The master a form belongs to (`None` for the plugin itself).
    pub fn master_of(&self, formid: u32) -> Option<&str> {
        let idx = (formid >> 24) as usize;
        self.masters.get(idx).map(String::as_str)
    }

    /// The NPC behind a reference (ACHR / REFR), when known.
    pub fn npc_of_ref(&self, formid: u32) -> Option<u32> {
        self.refs
            .get(&formid)
            .copied()
            .filter(|b| self.npcs.contains_key(b))
    }

    /// Replies of a topic in file order.
    pub fn topic_infos(&self, topic: u32) -> Vec<&Info> {
        self.topics
            .get(&topic)
            .map(|t| t.infos.iter().filter_map(|i| self.infos.get(i)).collect())
            .unwrap_or_default()
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    d: &[u8],
    depth: u32,
    header_len: usize,
    parent_topic: Option<u32>,
    family: Family,
    light: bool,
    map: &mut ContextMap,
    order: &mut usize,
) -> Result<()> {
    if depth > 32 {
        bail!(crate::l10n!("err-plugin-nesting"));
    }
    let mut p = 0usize;
    while p + header_len <= d.len() {
        let sig = sig4(&d[p..p + 4]);
        let size = read_u32(d, p + 4) as usize;
        if sig == "GRUP" {
            if size < header_len || p + size > d.len() {
                break;
            }
            let label = read_u32(d, p + 8);
            let gtype = read_i32(d, p + 12);
            // Type 7: the replies of the topic `label` (skipped in light mode).
            let topic = if gtype == 7 {
                Some(label)
            } else {
                parent_topic
            };
            if !(light && gtype == 7) {
                walk(
                    &d[p + header_len..p + size],
                    depth + 1,
                    header_len,
                    topic,
                    family,
                    light,
                    map,
                    order,
                )?;
            }
            p += size;
        } else {
            let data_start = p + header_len;
            let data_end = data_start + size;
            if data_end > d.len() {
                break;
            }
            if sig != "TES4" && ContextMap::wants(&sig, light) {
                let flags = read_u32(d, p + 8);
                let formid = read_u32(d, p + 12);
                let raw = &d[data_start..data_end];
                let subs = if flags & FLAG_COMPRESSED != 0 {
                    match decompress_record(raw) {
                        Ok(bytes) => parse_subrecords(&bytes),
                        Err(_) => Vec::new(),
                    }
                } else {
                    parse_subrecords(raw)
                };
                *order += 1;
                map.records += 1;
                take_record(
                    map,
                    &sig,
                    formid,
                    &subs,
                    parent_topic,
                    family,
                    light,
                    *order,
                );
            }
            p += header_len + size;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn take_record(
    map: &mut ContextMap,
    sig: &str,
    formid: u32,
    subs: &[SubRecord],
    parent_topic: Option<u32>,
    family: Family,
    light: bool,
    order: usize,
) {
    let localized = map.localized;
    let edid = subs
        .iter()
        .find(|s| s.sig == "EDID")
        .map(|s| s.as_zstring())
        .unwrap_or_default();
    let (full, full_id) = subs
        .iter()
        .find(|s| s.sig == "FULL")
        .map(|s| text_of(s, localized))
        .unwrap_or_default();
    match sig {
        "QUST" => {
            let mut q = Quest {
                formid,
                edid,
                full,
                full_id,
                order,
                ..Default::default()
            };
            let mut alias: Option<Alias> = None;
            let mut stage = 0u16;
            let mut objective = 0u16;
            for s in subs {
                match s.sig.as_str() {
                    "INDX" if s.data.len() >= 2 => stage = read_u16(&s.data, 0),
                    // Oblivion: a one-byte stage index.
                    "INDX" if s.data.len() == 1 => stage = s.data[0] as u16,
                    "CNAM" if is_text_line(s, localized) => q.stages.push(stage),
                    "QOBJ" if s.data.len() >= 2 => objective = read_u16(&s.data, 0),
                    "NNAM" if is_text_line(s, localized) => q.objectives.push(objective),
                    "ALST" | "ALLS" => {
                        if let Some(a) = alias.take() {
                            q.aliases.push(a);
                        }
                        alias = Some(Alias {
                            id: s.as_u32().unwrap_or(0),
                            ..Default::default()
                        });
                    }
                    "ALID" => {
                        if let Some(a) = alias.as_mut() {
                            a.name = s.as_zstring();
                        }
                    }
                    "ALFR" => {
                        if let Some(a) = alias.as_mut() {
                            a.forced_ref = s.as_u32();
                        }
                    }
                    "ALUA" => {
                        if let Some(a) = alias.as_mut() {
                            a.unique_actor = s.as_u32();
                        }
                    }
                    "ALED" => {
                        if let Some(a) = alias.take() {
                            q.aliases.push(a);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(a) = alias.take() {
                q.aliases.push(a);
            }
            map.quests.insert(formid, q);
        }
        "DIAL" => {
            let mut t = Topic {
                formid,
                edid,
                full,
                full_id,
                order,
                ..Default::default()
            };
            for s in subs {
                match s.sig.as_str() {
                    "QNAM" => t.quest = s.as_u32(),
                    // Oblivion / Fallout 3: the quests the topic belongs to.
                    "QSTI" if t.quest.is_none() => t.quest = s.as_u32(),
                    "BNAM" => t.branch = s.as_u32(),
                    "SNAM" if s.data.len() >= 4 => t.snam = sig4(&s.data),
                    "DATA" => match family {
                        Family::Skyrim | Family::Fallout4 | Family::Starfield
                            if s.data.len() >= 4 =>
                        {
                            t.category = s.data[1];
                            t.subtype = read_u16(&s.data, 2);
                        }
                        _ if !s.data.is_empty() => {
                            // Oblivion / Fallout 3: the type byte is the category.
                            t.category = s.data[0];
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            map.topics.insert(formid, t);
        }
        "INFO" => {
            let Some(topic) = parent_topic else {
                return;
            };
            let mut info = Info {
                formid,
                topic,
                order,
                ..Default::default()
            };
            let mut kept = 0i32;
            let mut last_cond: Option<usize> = None;
            for s in subs {
                match s.sig.as_str() {
                    "PNAM" => info.prev = s.as_u32().filter(|&v| v != 0),
                    "TCLT" | "NAME" => {
                        if let Some(v) = s.as_u32() {
                            info.links.push(v);
                        }
                    }
                    "DNAM" if s.data.len() == 4 => info.shared = s.as_u32().filter(|&v| v != 0),
                    "ANAM" if s.data.len() == 4 => info.speaker = s.as_u32().filter(|&v| v != 0),
                    "RNAM" => {
                        let (text, id) = text_of(s, localized);
                        info.prompt = id.is_some() || !text.is_empty();
                    }
                    "TRDT" if s.data.len() >= 13 => info.responses.push(Response {
                        emotion: read_u32(&s.data, 0),
                        emotion_value: read_u32(&s.data, 4),
                        number: s.data[12],
                        ..Default::default()
                    }),
                    "NAM1" => {
                        let (text, id) = text_of(s, localized);
                        let keep = id.is_some() || !text.is_empty();
                        if keep {
                            kept += 1;
                        }
                        let r = match info.responses.last_mut() {
                            Some(r) if r.text.is_empty() && r.text_id.is_none() => r,
                            _ => {
                                info.responses.push(Response::default());
                                info.responses.last_mut().unwrap()
                            }
                        };
                        r.text = text;
                        r.text_id = id;
                        r.line_index = keep.then_some(kept);
                    }
                    "NAM2" => {
                        if let Some(r) = info.responses.last_mut() {
                            r.note = s.as_zstring();
                        }
                    }
                    "CTDA" => {
                        if let Some(c) = parse_ctda(&s.data, family) {
                            info.conditions.push(c);
                            last_cond = Some(info.conditions.len() - 1);
                        }
                    }
                    "CIS1" => {
                        if let Some(i) = last_cond {
                            info.conditions[i].cis1 = Some(s.as_zstring());
                        }
                    }
                    "CIS2" => {
                        if let Some(i) = last_cond {
                            info.conditions[i].cis2 = Some(s.as_zstring());
                        }
                    }
                    _ => {}
                }
            }
            if let Some(t) = map.topics.get_mut(&topic) {
                t.infos.push(formid);
            }
            map.infos.insert(formid, info);
        }
        "SCEN" => {
            let mut sc = Scene {
                formid,
                edid,
                order,
                ..Default::default()
            };
            // Phases come first (HNAM … HNAM), then the actors (ALID LNAM DNAM),
            // then the actions (ANAM …), then PNAM the quest.
            let mut in_phase = false;
            let mut action: Option<SceneAction> = None;
            for s in subs {
                match s.sig.as_str() {
                    "HNAM" => in_phase = !in_phase,
                    "NAM0" if in_phase => sc.phases.push(s.as_zstring()),
                    // ANAM with the type opens an action; the empty ANAM closes it.
                    "ANAM" if s.data.len() >= 2 => {
                        if let Some(a) = action.take() {
                            sc.actions.push(a);
                        }
                        action = Some(SceneAction {
                            kind: read_u16(&s.data, 0),
                            ..Default::default()
                        });
                    }
                    "ANAM" => {
                        if let Some(a) = action.take() {
                            sc.actions.push(a);
                        }
                    }
                    "NAM0" => {
                        if let Some(a) = action.as_mut() {
                            a.name = s.as_zstring();
                        }
                    }
                    "ALID" => {
                        if let Some(a) = action.as_mut() {
                            a.alias = s.as_u32();
                        }
                    }
                    "INAM" => {
                        if let Some(a) = action.as_mut() {
                            a.index = s.as_u32().unwrap_or(0);
                        }
                    }
                    "SNAM" => {
                        if let Some(a) = action.as_mut() {
                            a.start_phase = s.as_u32().unwrap_or(0);
                        }
                    }
                    "ENAM" => {
                        if let Some(a) = action.as_mut() {
                            a.end_phase = s.as_u32().unwrap_or(0);
                        }
                    }
                    "DATA" if s.data.len() == 4 => {
                        if let Some(a) = action.as_mut() {
                            a.topic = s.as_u32().filter(|&v| v != 0);
                        }
                    }
                    "PNAM" if action.is_none() => sc.quest = s.as_u32(),
                    _ => {}
                }
            }
            if let Some(a) = action.take() {
                sc.actions.push(a);
            }
            map.scenes.insert(formid, sc);
        }
        "NPC_" => {
            let mut n = Npc {
                edid,
                full,
                full_id,
                ..Default::default()
            };
            for s in subs {
                match s.sig.as_str() {
                    "ACBS" if s.data.len() >= 4 => n.female = read_u32(&s.data, 0) & 0x1 != 0,
                    "VTCK" => n.voice = s.as_u32().filter(|&v| v != 0),
                    "RNAM" if s.data.len() == 4 => n.race = s.as_u32(),
                    _ => {}
                }
            }
            map.npcs.insert(formid, n);
        }
        "VTYP" => {
            map.voices.insert(formid, edid);
        }
        "ACHR" | "REFR" => {
            if let Some(base) = subs
                .iter()
                .find(|s| s.sig == "NAME")
                .and_then(|s| s.as_u32())
            {
                map.refs.insert(formid, base);
            }
        }
        _ => {
            let model = if light {
                None
            } else {
                subs.iter()
                    .find(|s| s.sig == "MODL")
                    .map(|s| s.as_zstring())
                    .filter(|m| !m.is_empty())
            };
            let quest = match sig {
                "MESG" | "PERK" | "SPEL" | "SHOU" | "WOOP" | "PACK" | "LCTN" => subs
                    .iter()
                    .find(|s| s.sig == "QNAM" && s.data.len() == 4)
                    .and_then(|s| s.as_u32())
                    .filter(|&q| q != 0),
                _ => None,
            };
            map.names.insert(
                formid,
                Named {
                    sig: sig.to_string(),
                    edid,
                    full,
                    full_id,
                    model,
                    quest,
                },
            );
        }
    }
}

/// Decode a CTDA by its size and family.
pub fn parse_ctda(d: &[u8], family: Family) -> Option<Condition> {
    if d.len() < 20 {
        return None;
    }
    let t = d[0];
    let mut c = Condition {
        op: t >> 5,
        flags: t & 0x1F,
        ..Default::default()
    };
    let use_global = c.flags & 0x04 != 0;
    if use_global {
        c.global = Some(read_u32(d, 4));
    } else {
        c.value = read_f32(d, 4);
    }
    match (family, d.len()) {
        // Oblivion: type, unused[3], value, function u32, p1, p2, unused.
        (Family::Oblivion, _) => {
            c.function = read_u32(d, 8);
            c.param1 = read_u32(d, 12);
            c.param2 = read_u32(d, 16);
            if d.len() >= 28 {
                c.run_on = read_u32(d, 20);
                c.reference = read_u32(d, 24);
            }
        }
        // Fallout 3 / NV: … function u32, p1, p2, run on, reference.
        (Family::Fallout3, _) => {
            c.function = read_u32(d, 8);
            c.param1 = read_u32(d, 12);
            c.param2 = read_u32(d, 16);
            if d.len() >= 28 {
                c.run_on = read_u32(d, 20);
                c.reference = read_u32(d, 24);
            }
        }
        // Starfield: function on 32 bits.
        (Family::Starfield, n) if n >= 32 => {
            c.function = read_u32(d, 8);
            c.param1 = read_u32(d, 12);
            c.param2 = read_u32(d, 16);
            c.run_on = read_u32(d, 20);
            c.reference = read_u32(d, 24);
            c.param3 = read_i32(d, 28);
        }
        // Skyrim / Fallout 4: function u16 + padding, p1, p2, run on, reference, p3.
        (_, n) if n >= 32 => {
            c.function = read_u16(d, 8) as u32;
            c.param1 = read_u32(d, 12);
            c.param2 = read_u32(d, 16);
            c.run_on = read_u32(d, 20);
            c.reference = read_u32(d, 24);
            c.param3 = read_i32(d, 28);
        }
        _ => {
            c.function = read_u16(d, 8) as u32;
            c.param1 = read_u32(d, 12);
            c.param2 = read_u32(d, 16);
        }
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ContextMap {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/skyrim_dialogue.esp");
        ContextMap::read_file(&p, Family::Skyrim).unwrap()
    }

    /// T-C01-01 (data): the synthetic plugin's structure is read whole.
    #[test]
    fn reads_the_dialogue_fixture() {
        let m = fixture();
        assert_eq!(m.masters, vec!["Skyrim.esm".to_string()]);
        assert!(!m.localized);
        assert_eq!(m.quests.len(), 1);
        let q = &m.quests[&0x01000101];
        assert_eq!((q.edid.as_str(), q.full.as_str()), ("MQ101", "Unbound"));
        assert_eq!(q.stages, vec![10, 20, 20]);
        assert_eq!(q.objectives, vec![10]);
        assert_eq!(q.aliases.len(), 3);
        assert_eq!(q.aliases[0].name, "Ralof");
        assert_eq!(q.aliases[0].unique_actor, Some(0x01000A01));
        assert_eq!(q.aliases[2].unique_actor, None);
        assert_eq!(m.topics.len(), 5);
        let t = &m.topics[&0x01000201];
        assert_eq!(t.full, "What's going on?");
        assert_eq!(t.quest, Some(0x01000101));
        assert_eq!((t.category, t.subtype, t.snam.as_str()), (0, 0, "CUST"));
        assert_eq!(t.infos, vec![0x01000301, 0x01000302, 0x01000303]);
        let hello = &m.topics[&0x01000203];
        assert_eq!(
            (hello.category, hello.subtype, hello.snam.as_str()),
            (7, 82, "HELO")
        );
        assert_eq!(hello.quest, None);
        let i1 = &m.infos[&0x01000301];
        assert_eq!(i1.speaker, Some(0x01000A01));
        assert_eq!(i1.links, vec![0x01000202]);
        assert_eq!(i1.responses.len(), 2);
        assert_eq!(i1.responses[0].text, "We're being taken to Helgen.");
        assert_eq!(i1.responses[0].note, "said quietly, resigned");
        assert_eq!(
            (i1.responses[1].emotion, i1.responses[1].emotion_value),
            (3, 40)
        );
        assert_eq!(i1.responses[1].line_index, Some(2));
        assert_eq!(i1.conditions.len(), 2);
        assert_eq!(i1.conditions[0].function, 72);
        assert_eq!(i1.conditions[0].param1, 0x01000A01);
        assert_eq!((i1.conditions[1].function, i1.conditions[1].op), (58, 3));
        assert_eq!(i1.conditions[1].value, 20.0);
        let i2 = &m.infos[&0x01000302];
        assert_eq!(i2.prev, Some(0x01000301));
        assert!(i2.conditions[0].is_or());
        assert_eq!(i2.conditions[1].run_on, 1);
        let i3 = &m.infos[&0x01000303];
        assert!(i3.prompt);
        assert!(m.infos[&0x01000304].conditions[0].uses_aliases());
        assert_eq!(m.npcs.len(), 4);
        assert!(m.npcs[&0x01000A03].female);
        assert_eq!(m.npcs[&0x01000A01].voice, Some(0x01000B01));
        assert_eq!(m.voices[&0x01000B01], "MaleNord");
        assert_eq!(m.names[&0x01000C01].full, "Whiterun Guards");
        let sc = &m.scenes[&0x01000401];
        assert_eq!(sc.quest, Some(0x01000101));
        assert_eq!(
            sc.phases,
            vec!["Ralof talks".to_string(), "Lokir answers".to_string()]
        );
        assert_eq!(sc.actions.len(), 2);
        assert_eq!(
            (sc.actions[0].alias, sc.actions[0].topic),
            (Some(0), Some(0x01000204))
        );
        assert_eq!(
            (sc.actions[1].alias, sc.actions[1].start_phase),
            (Some(1), 1)
        );
        assert_eq!(m.master_of(0x000A2C8E), Some("Skyrim.esm"));
        assert_eq!(m.master_of(0x01000A01), None);
    }

    /// A plugin without dialogue gives an empty map, quickly.
    #[test]
    fn plugin_without_dialogue_is_empty() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/skyrimle_riverwood.esp");
        let m = ContextMap::read_file(&p, Family::Skyrim).unwrap();
        assert!(m.is_empty());
        assert!(m.records > 0, "cells and references are named");
    }

    /// T-C03-01 (data): the model path, the owner quest and the stage of
    /// each journal entry are read by the full pass.
    #[test]
    fn reads_models_quests_and_stages() {
        let m = fixture();
        let axe = &m.names[&0x01000E01];
        assert_eq!((axe.sig.as_str(), axe.edid.as_str()), ("WEAP", "RalofAxe"));
        assert_eq!(axe.model.as_deref(), Some("Weapons\\Iron\\IronWarAxe.nif"));
        assert_eq!(axe.quest, None);
        let msg = &m.names[&0x01000F01];
        assert_eq!(msg.sig, "MESG");
        assert_eq!(msg.quest, Some(0x01000101));
        assert_eq!(msg.model, None);
        // Two journal entries at stage 20: the third CNAM line is stage 20.
        assert_eq!(m.quests[&0x01000101].stages, vec![10, 20, 20]);
        // The override of a master's NPC is read like any other.
        assert_eq!(m.npcs[&0x0002BF9D].edid, "Hadvar");
        assert_eq!(m.master_of(0x0002BF9D), Some("Skyrim.esm"));
    }

    /// The light pass keeps the names and drops the replies.
    #[test]
    fn light_pass_keeps_names_only() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/skyrim_dialogue.esp");
        let mut steps = 0;
        let m = ContextMap::read_file_with(&p, Family::Skyrim, true, &mut |_, _| {
            steps += 1;
            true
        })
        .unwrap();
        assert!(steps > 0);
        assert!(m.infos.is_empty() && m.scenes.is_empty());
        assert_eq!(m.npcs.len(), 4);
        assert_eq!(m.topics.len(), 5);
        assert_eq!(m.quests[&0x01000101].aliases.len(), 3);
        // The model paths are not kept by the light pass (cache size).
        assert_eq!(m.names[&0x01000E01].model, None);
        assert_eq!(m.names[&0x01000F01].quest, Some(0x01000101));
        // Round trip through JSON (the disk cache of a master's names).
        let json = serde_json::to_string(&m).unwrap();
        let back: ContextMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back.npcs, m.npcs);
        assert_eq!(back.quests, m.quests);
        assert!(back.infos.is_empty());
        // Cancelling stops the walk.
        assert!(ContextMap::read_file_with(&p, Family::Skyrim, true, &mut |_, _| false).is_err());
    }

    #[test]
    fn ctda_sizes() {
        // Oblivion: 24 bytes, function on 32 bits.
        let mut d = vec![0u8; 24];
        d[0] = 0b0110_0001; // op 3 (>=), OR
        d[4..8].copy_from_slice(&1.5f32.to_le_bytes());
        d[8..12].copy_from_slice(&72u32.to_le_bytes());
        d[12..16].copy_from_slice(&0xABCDu32.to_le_bytes());
        let c = parse_ctda(&d, Family::Oblivion).unwrap();
        assert_eq!(
            (c.op, c.is_or(), c.function, c.param1),
            (3, true, 72, 0xABCD)
        );
        assert_eq!(c.value, 1.5);
        // Skyrim with a global.
        let mut d = vec![0u8; 32];
        d[0] = 0x04;
        d[4..8].copy_from_slice(&0x01000D01u32.to_le_bytes());
        d[8..10].copy_from_slice(&58u16.to_le_bytes());
        d[28..32].copy_from_slice(&(-1i32).to_le_bytes());
        let c = parse_ctda(&d, Family::Skyrim).unwrap();
        assert_eq!((c.global, c.function, c.param3), (Some(0x01000D01), 58, -1));
        assert!(parse_ctda(&[0u8; 8], Family::Skyrim).is_none());
    }
}
