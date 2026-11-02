//! The dialogues of a document (C-01, C-02): the tree quest → scene /
//! category → topic → reply built from the context map and the document's
//! lines, the order of the conversation, the speaker of each reply and the
//! names of the forms the replies refer to (the plugin's own, then its
//! masters' name indexes).

use std::collections::HashMap;
use std::sync::Arc;

use crate::i18n::I18n;
use crate::plugin::context::{Condition, ContextMap, Family, Info, Named, Npc, Quest};
use crate::tm::TmEntry;

/// The data of a family (`assets/data/dialogue/<family>.json`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogueSpec {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub family: String,
    /// Category index → i18n key.
    #[serde(default)]
    pub categories: HashMap<String, String>,
    /// The category of the scene topics (-1 when the family has none).
    #[serde(default = "minus_one")]
    pub scene_category: i32,
    /// Subtype index → Creation Kit name.
    #[serde(default)]
    pub subtypes: HashMap<String, String>,
    /// Emotion index → i18n key (empty: the family has no emotion).
    #[serde(default)]
    pub emotions: Vec<String>,
    #[serde(default)]
    pub quest_link: String,
    #[serde(default)]
    pub voice: VoiceSpec,
}

fn minus_one() -> i32 {
    -1
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSpec {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    /// `formid-suffix`, `snam-path`, `hash`.
    #[serde(default)]
    pub naming: String,
    #[serde(default)]
    pub folders: String,
}

impl DialogueSpec {
    pub fn load(family: Family) -> DialogueSpec {
        let rel = format!("dialogue/{}.json", family.stem());
        crate::data::find_data_file(&rel)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn category_key(&self, cat: u8) -> Option<&str> {
        self.categories.get(&cat.to_string()).map(String::as_str)
    }

    pub fn subtype_name(&self, subtype: u16) -> Option<&str> {
        self.subtypes.get(&subtype.to_string()).map(String::as_str)
    }

    pub fn emotion_key(&self, emotion: u32) -> Option<&str> {
        self.emotions.get(emotion as usize).map(String::as_str)
    }
}

// ---------------------------------------------------------------- names

/// The names a document can reach: its own context map, then the name
/// indexes of its masters, by master index.
#[derive(Clone, Default)]
pub struct Resolver {
    pub ctx: Arc<ContextMap>,
    /// Parallel to `ctx.masters`; `None` while a master is unknown.
    pub masters: Vec<Option<Arc<ContextMap>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gender {
    Male,
    Female,
    Unknown,
}

impl Resolver {
    pub fn new(ctx: Arc<ContextMap>) -> Resolver {
        let n = ctx.masters.len();
        Resolver {
            ctx,
            masters: vec![None; n],
        }
    }

    /// The map holding a form: the plugin's own, or a master's.
    fn map_of(&self, formid: u32) -> Option<(&ContextMap, u32)> {
        let idx = (formid >> 24) as usize;
        if idx < self.ctx.masters.len() {
            self.masters
                .get(idx)
                .and_then(|m| m.as_deref())
                .map(|m| (m, formid & 0x00FF_FFFF))
        } else {
            Some((&self.ctx, formid))
        }
    }

    pub fn npc(&self, formid: u32) -> Option<&Npc> {
        let (m, local) = self.map_of(formid)?;
        m.npcs.get(&local).or_else(|| m.npcs.get(&formid))
    }

    pub fn quest(&self, formid: u32) -> Option<&Quest> {
        let (m, local) = self.map_of(formid)?;
        m.quests.get(&local).or_else(|| m.quests.get(&formid))
    }

    pub fn voice(&self, formid: u32) -> Option<&str> {
        let (m, local) = self.map_of(formid)?;
        m.voices
            .get(&local)
            .or_else(|| m.voices.get(&formid))
            .map(String::as_str)
    }

    pub fn named(&self, formid: u32) -> Option<&Named> {
        let (m, local) = self.map_of(formid)?;
        m.names.get(&local).or_else(|| m.names.get(&formid))
    }

    /// The base NPC of a reference, when known.
    pub fn npc_of_ref(&self, formid: u32) -> Option<u32> {
        let (m, local) = self.map_of(formid)?;
        let base = m
            .refs
            .get(&local)
            .or_else(|| m.refs.get(&formid))
            .copied()?;
        // A base in the master keeps the master's index.
        let idx = formid & 0xFF00_0000;
        let base = if (base >> 24) as usize >= m.masters.len() {
            (base & 0x00FF_FFFF) | idx
        } else {
            base
        };
        Some(base)
    }

    /// The display name of any form: FULL, then EDID, then the FormID with
    /// its master.
    pub fn display(&self, formid: u32) -> String {
        if let Some(n) = self.npc(formid) {
            return pick(&n.full, &n.edid, formid, &self.ctx);
        }
        if let Some(q) = self.quest(formid) {
            return pick(&q.full, &q.edid, formid, &self.ctx);
        }
        if let Some(v) = self.voice(formid) {
            return v.to_string();
        }
        if let Some(n) = self.named(formid) {
            return pick(&n.full, &n.edid, formid, &self.ctx);
        }
        if let Some((m, local)) = self.map_of(formid)
            && let Some(t) = m.topics.get(&local).or_else(|| m.topics.get(&formid))
        {
            return pick(&t.full, &t.edid, formid, &self.ctx);
        }
        unresolved(formid, &self.ctx)
    }

    /// EDID first (the identifier translators know), then FULL.
    pub fn edid_or_full(&self, formid: u32) -> String {
        if let Some(n) = self.npc(formid) {
            return pick(&n.edid, &n.full, formid, &self.ctx);
        }
        if let Some(q) = self.quest(formid) {
            return pick(&q.edid, &q.full, formid, &self.ctx);
        }
        if let Some(n) = self.named(formid) {
            return pick(&n.edid, &n.full, formid, &self.ctx);
        }
        self.display(formid)
    }

    pub fn gender_of_npc(&self, formid: u32) -> Gender {
        match self.npc(formid) {
            Some(n) if n.female => Gender::Female,
            Some(_) => Gender::Male,
            None => Gender::Unknown,
        }
    }

    /// Gender from a voice type's EDID (Male… / Female…).
    pub fn gender_of_voice(&self, formid: u32) -> Gender {
        match self.voice(formid) {
            Some(v) => gender_of_voice_name(v),
            None => Gender::Unknown,
        }
    }
}

pub fn gender_of_voice_name(name: &str) -> Gender {
    let l = name.to_ascii_lowercase();
    if l.starts_with("female") || l.contains("female") {
        Gender::Female
    } else if l.starts_with("male") || l.contains("male") {
        Gender::Male
    } else {
        Gender::Unknown
    }
}

fn pick(a: &str, b: &str, formid: u32, ctx: &ContextMap) -> String {
    if !a.is_empty() {
        a.to_string()
    } else if !b.is_empty() {
        b.to_string()
    } else {
        unresolved(formid, ctx)
    }
}

/// « 000A2C8E (Skyrim.esm) » / « 01000A01 ».
pub fn unresolved(formid: u32, ctx: &ContextMap) -> String {
    match ctx.master_of(formid) {
        Some(m) => format!("{formid:08X} ({m})"),
        None => format!("{formid:08X}"),
    }
}

// ---------------------------------------------------------------- speaker

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeakerKind {
    Npc(u32),
    /// An alias of a quest that no actor resolves.
    Alias {
        quest: Option<u32>,
        name: String,
    },
    Voice(u32),
    Faction(u32),
    Race(u32),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeakerOrigin {
    Anam,
    Scene,
    Condition,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    pub kind: SpeakerKind,
    pub name: String,
    pub gender: Gender,
    pub origin: SpeakerOrigin,
    /// The voice type's EDID, when known.
    pub voice: Option<String>,
}

impl Speaker {
    fn unknown() -> Speaker {
        Speaker {
            kind: SpeakerKind::Unknown,
            name: String::new(),
            gender: Gender::Unknown,
            origin: SpeakerOrigin::None,
            voice: None,
        }
    }

    /// A stable key for the PNJ filter (C-02).
    pub fn key(&self) -> String {
        match &self.kind {
            SpeakerKind::Npc(f) => format!("npc:{f:08X}"),
            SpeakerKind::Alias { quest, name } => {
                format!("alias:{:08X}:{name}", quest.unwrap_or(0))
            }
            SpeakerKind::Voice(f) => format!("voice:{f:08X}"),
            SpeakerKind::Faction(f) => format!("faction:{f:08X}"),
            SpeakerKind::Race(f) => format!("race:{f:08X}"),
            SpeakerKind::Unknown => "unknown".to_string(),
        }
    }
}

/// Condition functions the speaker resolution reads (Skyrim / Fallout 4
/// numbering, the classic games share the low ones).
pub const FN_GET_IS_ID: u32 = 72;
pub const FN_GET_IS_RACE: u32 = 69;
pub const FN_GET_IS_SEX: u32 = 70;
pub const FN_GET_IN_FACTION: u32 = 71;
pub const FN_GET_IS_VOICE_TYPE: u32 = 426;
pub const FN_GET_IS_ALIAS_REF: u32 = 566;

/// The speaker of a reply (R-C02-01): ANAM, then the actor of the scene
/// playing its topic, then its conditions on the subject.
pub fn speaker_of(info: &Info, res: &Resolver, scenes: &SceneIndex) -> Vec<Speaker> {
    // (a) ANAM.
    if let Some(npc) = info.speaker {
        return vec![speaker_npc(npc, res, SpeakerOrigin::Anam)];
    }
    // (b) The scene.
    if let Some((scene, alias)) = scenes.actor_of_topic(info.topic) {
        let quest = res.ctx.scenes.get(&scene).and_then(|s| s.quest);
        if let Some(sp) = speaker_alias(quest, alias, res, SpeakerOrigin::Scene) {
            return vec![sp];
        }
    }
    // (c) The conditions on the subject.
    let topic_quest = res.ctx.topics.get(&info.topic).and_then(|t| t.quest);
    let mut out: Vec<Speaker> = Vec::new();
    for c in &info.conditions {
        if c.run_on != 0 {
            continue;
        }
        let sp = match c.function {
            FN_GET_IS_ID => Some(speaker_npc(c.param1, res, SpeakerOrigin::Condition)),
            FN_GET_IS_ALIAS_REF => {
                speaker_alias(topic_quest, c.param1, res, SpeakerOrigin::Condition)
            }
            FN_GET_IS_VOICE_TYPE => Some(Speaker {
                kind: SpeakerKind::Voice(c.param1),
                name: res.display(c.param1),
                gender: res.gender_of_voice(c.param1),
                origin: SpeakerOrigin::Condition,
                voice: res.voice(c.param1).map(str::to_string),
            }),
            FN_GET_IN_FACTION => Some(Speaker {
                kind: SpeakerKind::Faction(c.param1),
                name: res.display(c.param1),
                gender: Gender::Unknown,
                origin: SpeakerOrigin::Condition,
                voice: None,
            }),
            FN_GET_IS_RACE => Some(Speaker {
                kind: SpeakerKind::Race(c.param1),
                name: res.display(c.param1),
                gender: Gender::Unknown,
                origin: SpeakerOrigin::Condition,
                voice: None,
            }),
            _ => None,
        };
        if let Some(sp) = sp {
            // `== 0` excludes rather than designates.
            if c.op == 0 && c.value == 0.0 && c.global.is_none() {
                continue;
            }
            if !out.iter().any(|o| o.kind == sp.kind) {
                out.push(sp);
            }
        }
    }
    if out.is_empty() {
        out.push(Speaker::unknown());
    }
    out
}

fn speaker_npc(npc: u32, res: &Resolver, origin: SpeakerOrigin) -> Speaker {
    let voice = res
        .npc(npc)
        .and_then(|n| n.voice)
        .and_then(|v| res.voice(v))
        .map(str::to_string);
    Speaker {
        kind: SpeakerKind::Npc(npc),
        name: res.display(npc),
        gender: res.gender_of_npc(npc),
        origin,
        voice,
    }
}

/// An alias resolved to its actor when the quest says so.
fn speaker_alias(
    quest: Option<u32>,
    alias_id: u32,
    res: &Resolver,
    origin: SpeakerOrigin,
) -> Option<Speaker> {
    let q = quest.and_then(|q| res.quest(q));
    let alias = q.and_then(|q| q.aliases.iter().find(|a| a.id == alias_id));
    if let Some(a) = alias {
        if let Some(npc) = a.unique_actor {
            let mut sp = speaker_npc(npc, res, origin);
            if sp.name.is_empty() {
                sp.name = a.name.clone();
            }
            return Some(sp);
        }
        if let Some(r) = a.forced_ref
            && let Some(npc) = res.npc_of_ref(r)
        {
            return Some(speaker_npc(npc, res, origin));
        }
        return Some(Speaker {
            kind: SpeakerKind::Alias {
                quest,
                name: a.name.clone(),
            },
            name: a.name.clone(),
            gender: Gender::Unknown,
            origin,
            voice: None,
        });
    }
    Some(Speaker {
        kind: SpeakerKind::Alias {
            quest,
            name: format!("#{alias_id}"),
        },
        name: format!("#{alias_id}"),
        gender: Gender::Unknown,
        origin,
        voice: None,
    })
}

/// Which scene plays which topic, and with which actor alias.
#[derive(Debug, Clone, Default)]
pub struct SceneIndex {
    /// topic → (scene, alias of the action)
    pub by_topic: HashMap<u32, (u32, u32)>,
}

impl SceneIndex {
    pub fn build(ctx: &ContextMap) -> SceneIndex {
        let mut by_topic = HashMap::new();
        for (sid, sc) in &ctx.scenes {
            for a in &sc.actions {
                if let (Some(t), Some(al)) = (a.topic, a.alias) {
                    by_topic.entry(t).or_insert((*sid, al));
                }
            }
        }
        SceneIndex { by_topic }
    }

    pub fn actor_of_topic(&self, topic: u32) -> Option<(u32, u32)> {
        self.by_topic.get(&topic).copied()
    }
}

// ---------------------------------------------------------------- tree

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Quest(u32),
    NoQuest,
    Scene(u32),
    /// (scene, phase index)
    Phase(u32, usize),
    /// (category, subtype) under a quest
    Category(u8, u16),
    Topic(u32),
    Info(u32),
    /// (info, response position)
    Response(u32, usize),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub depth: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// The label, ready to show.
    pub label: String,
    /// Right-aligned detail (counts, emotion).
    pub extra: String,
    /// The document line the node stands for (first response, FULL…).
    pub line: Option<usize>,
    /// Reply lines translated / total in the subtree.
    pub done: usize,
    pub total: usize,
    /// Lines of the subtree without a voice file (C-13), when known.
    pub unvoiced: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineKey {
    pub sig: String,
    pub field: String,
    pub formid: u32,
    pub index: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Sequence {
    pub prev: Option<u32>,
    pub next: Option<u32>,
}

/// The dialogues of one document.
pub struct DialogueMap {
    pub res: Resolver,
    pub spec: DialogueSpec,
    pub scenes: SceneIndex,
    pub lines: HashMap<LineKey, usize>,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub info_node: HashMap<u32, usize>,
    /// Document line → node (replies, topics, quests).
    pub line_node: HashMap<usize, usize>,
    pub sequence: HashMap<u32, Sequence>,
    /// Speakers of every reply, cached.
    pub speakers: HashMap<u32, Vec<Speaker>>,
    /// Reply lines of the document (NAM1 / RNAM).
    pub reply_lines: usize,
}

impl DialogueMap {
    pub fn line(&self, sig: &str, field: &str, formid: u32, index: i32) -> Option<usize> {
        self.lines
            .get(&LineKey {
                sig: sig.to_string(),
                field: field.to_string(),
                formid,
                index,
            })
            .copied()
    }

    pub fn speaker(&self, info: u32) -> Option<&Speaker> {
        self.speakers.get(&info).and_then(|v| v.first())
    }

    pub fn is_empty(&self) -> bool {
        self.res.ctx.infos.is_empty()
    }

    /// The emotion label (« Colère 60 »), empty for a neutral or absent one.
    pub fn emotion_label(&self, i18n: &I18n, emotion: u32, value: u32) -> String {
        match self.spec.emotion_key(emotion) {
            Some(k) if emotion != 0 || value != 0 => {
                let name = i18n.t(k);
                if value > 0 {
                    format!("{name} {value}")
                } else {
                    name
                }
            }
            _ => String::new(),
        }
    }

    /// The name of a speaker list for a label: « Ralof », « Ralof ou Lokir »,
    /// or the i18n « indéterminé ».
    pub fn speaker_label(&self, i18n: &I18n, info: u32) -> String {
        let Some(list) = self.speakers.get(&info) else {
            return String::new();
        };
        let names: Vec<String> = list
            .iter()
            .map(|s| match &s.kind {
                SpeakerKind::Unknown => i18n.t("speaker-unknown"),
                SpeakerKind::Voice(_) => i18n.t_arg("speaker-voice", "name", &s.name),
                SpeakerKind::Alias { .. } => i18n.t_arg("speaker-alias", "name", &s.name),
                SpeakerKind::Faction(_) => i18n.t_arg("speaker-faction", "name", &s.name),
                SpeakerKind::Race(_) => i18n.t_arg("speaker-race", "name", &s.name),
                SpeakerKind::Npc(_) => s.name.clone(),
            })
            .collect();
        names.join(&format!(" {} ", i18n.t("cond-or")))
    }

    /// Build the map of a document.
    pub fn build(
        ctx: Arc<ContextMap>,
        masters: Vec<Option<Arc<ContextMap>>>,
        entries: &[TmEntry],
        i18n: &I18n,
    ) -> DialogueMap {
        Self::build_with(ctx, masters, entries, i18n, None)
    }

    /// [`build`](Self::build) with the voice map (C-13): the « non doublée »
    /// counts of the nodes.
    pub fn build_with(
        ctx: Arc<ContextMap>,
        masters: Vec<Option<Arc<ContextMap>>>,
        entries: &[TmEntry],
        i18n: &I18n,
        voices: Option<&crate::voices::VoiceMap>,
    ) -> DialogueMap {
        let family = ctx.family.unwrap_or(Family::Skyrim);
        let spec = DialogueSpec::load(family);
        let mut res = Resolver::new(ctx.clone());
        for (i, m) in masters.into_iter().enumerate() {
            if i < res.masters.len() {
                res.masters[i] = m;
            }
        }
        let scenes = SceneIndex::build(&ctx);
        // Lines of the document by (sig, field, formid, index).
        let mut lines = HashMap::new();
        let mut reply_lines = 0;
        for (i, e) in entries.iter().enumerate() {
            if !matches!(e.rec_sig.as_str(), "INFO" | "DIAL" | "QUST" | "NPC_") {
                continue;
            }
            if e.rec_sig == "INFO" {
                reply_lines += 1;
            }
            lines.insert(
                LineKey {
                    sig: e.rec_sig.clone(),
                    field: e.field_sig.clone(),
                    formid: e.formid,
                    index: e.index,
                },
                i,
            );
        }
        let mut speakers = HashMap::new();
        for (fid, info) in &ctx.infos {
            speakers.insert(*fid, speaker_of(info, &res, &scenes));
        }
        let mut map = DialogueMap {
            res,
            spec,
            scenes,
            lines,
            nodes: Vec::new(),
            roots: Vec::new(),
            info_node: HashMap::new(),
            line_node: HashMap::new(),
            sequence: HashMap::new(),
            speakers,
            reply_lines,
        };
        map.build_tree(entries, i18n);
        map.build_sequence();
        if let Some(v) = voices {
            map.apply_voices(v);
        }
        map
    }

    /// The « non doublée » counts (R-C13-03): a reply counts once when any
    /// of its responses has no voice file; its response nodes say which;
    /// the parents add their replies up.
    pub fn apply_voices(&mut self, voices: &crate::voices::VoiceMap) {
        let ctx = self.res.ctx.clone();
        for i in (0..self.nodes.len()).rev() {
            let n = match self.nodes[i].kind {
                NodeKind::Response(fid, pos) => ctx
                    .infos
                    .get(&fid)
                    .and_then(|inf| inf.responses.get(pos))
                    .map(|r| usize::from(voices.voiced(fid, r.number) == Some(false)))
                    .unwrap_or(0),
                NodeKind::Info(fid) => ctx
                    .infos
                    .get(&fid)
                    .map(|inf| {
                        usize::from(
                            inf.responses
                                .iter()
                                .any(|r| voices.voiced(fid, r.number) == Some(false)),
                        )
                    })
                    .unwrap_or(0),
                _ => self.nodes[i]
                    .children
                    .iter()
                    .map(|c| self.nodes[*c].unvoiced)
                    .sum(),
            };
            self.nodes[i].unvoiced = n;
        }
    }

    /// The replies with a response that has no voice file, in tree order,
    /// as « FormID ⇥ sujet ⇥ locuteur ⇥ texte » lines (R-C13-03, « Copier
    /// la liste »).
    pub fn unvoiced_list(&self, entries: &[TmEntry], i18n: &I18n) -> Vec<String> {
        let ctx = self.res.ctx.clone();
        let mut out = Vec::new();
        for node in &self.nodes {
            let NodeKind::Info(fid) = node.kind else {
                continue;
            };
            if node.unvoiced == 0 {
                continue;
            }
            let topic = ctx
                .infos
                .get(&fid)
                .and_then(|inf| ctx.topics.get(&inf.topic))
                .map(|t| t.edid.clone())
                .unwrap_or_default();
            let speaker = self.speaker_label(i18n, fid);
            let text = Self::text_of_line(entries, node.line, 200);
            out.push(format!("{fid:08X}\t{topic}\t{speaker}\t{text}"));
        }
        out
    }

    fn push(
        &mut self,
        parent: Option<usize>,
        kind: NodeKind,
        label: String,
        extra: String,
        line: Option<usize>,
    ) -> usize {
        let depth = parent.map(|p| self.nodes[p].depth + 1).unwrap_or(0);
        let idx = self.nodes.len();
        self.nodes.push(Node {
            kind,
            depth,
            parent,
            children: Vec::new(),
            label,
            extra,
            line,
            done: 0,
            total: 0,
            unvoiced: 0,
        });
        match parent {
            Some(p) => self.nodes[p].children.push(idx),
            None => self.roots.push(idx),
        }
        idx
    }

    fn text_of_line(entries: &[TmEntry], line: Option<usize>, max: usize) -> String {
        match line.and_then(|l| entries.get(l)) {
            Some(e) => crate::ui::edit_window::one_line(&e.source, max),
            None => String::new(),
        }
    }

    fn build_tree(&mut self, entries: &[TmEntry], i18n: &I18n) {
        let ctx = self.res.ctx.clone();
        // Quests in file order, then « Sans quête ».
        let mut quests: Vec<&Quest> = ctx.quests.values().collect();
        quests.sort_by_key(|q| q.order);
        // Quests of the masters the topics refer to come after, by formid.
        let mut foreign: Vec<u32> = ctx
            .topics
            .values()
            .filter_map(|t| t.quest)
            .filter(|q| !ctx.quests.contains_key(q))
            .collect();
        foreign.sort_unstable();
        foreign.dedup();
        let scene_cat = self.spec.scene_category;
        let mut consumed: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let mut quest_ids: Vec<(Option<u32>, String)> = quests
            .iter()
            .map(|q| {
                let line = self.line("QUST", "FULL", q.formid, 1);
                let full = if line.is_some() {
                    Self::text_of_line(entries, line, 60)
                } else {
                    q.full.clone()
                };
                let label = if full.is_empty() {
                    q.edid.clone()
                } else {
                    format!("{} — {}", q.edid, full)
                };
                (Some(q.formid), label)
            })
            .collect();
        for f in foreign {
            quest_ids.push((Some(f), self.res.display(f)));
        }
        let has_orphans = ctx.topics.values().any(|t| t.quest.is_none());
        if has_orphans {
            quest_ids.push((None, i18n.t("dialog-no-quest")));
        }
        for (quest, label) in quest_ids {
            let line = quest.and_then(|q| self.line("QUST", "FULL", q, 1));
            let kind = match quest {
                Some(q) => NodeKind::Quest(q),
                None => NodeKind::NoQuest,
            };
            let qn = self.push(None, kind, label, String::new(), line);
            if let Some(l) = line {
                self.line_node.insert(l, qn);
            }
            // Scenes of the quest.
            if let Some(q) = quest {
                let mut scenes: Vec<_> =
                    ctx.scenes.values().filter(|s| s.quest == Some(q)).collect();
                scenes.sort_by_key(|s| s.order);
                for sc in scenes {
                    let n_phases = sc.phases.len().max(
                        sc.actions
                            .iter()
                            .map(|a| a.start_phase as usize + 1)
                            .max()
                            .unwrap_or(0),
                    );
                    let label = i18n.t_args(
                        "dialog-scene",
                        &[("name", &sc.edid), ("n", &n_phases.to_string())],
                    );
                    let sn = self.push(
                        Some(qn),
                        NodeKind::Scene(sc.formid),
                        label,
                        String::new(),
                        None,
                    );
                    for phase in 0..n_phases {
                        let mut actions: Vec<_> = sc
                            .actions
                            .iter()
                            .filter(|a| {
                                a.kind == 0 && a.start_phase as usize == phase && a.topic.is_some()
                            })
                            .collect();
                        if actions.is_empty() {
                            continue;
                        }
                        actions.sort_by_key(|a| a.index);
                        let actor = actions
                            .first()
                            .and_then(|a| a.alias)
                            .and_then(|al| {
                                speaker_alias(Some(q), al, &self.res, SpeakerOrigin::Scene)
                            })
                            .map(|s| s.name)
                            .unwrap_or_default();
                        let mut label = i18n.t_args(
                            "dialog-phase",
                            &[("n", &(phase + 1).to_string()), ("actor", &actor)],
                        );
                        if let Some(name) = sc.phases.get(phase).filter(|n| !n.is_empty()) {
                            label = i18n.dot(&label, name);
                        }
                        let pn = self.push(
                            Some(sn),
                            NodeKind::Phase(sc.formid, phase),
                            label,
                            String::new(),
                            None,
                        );
                        for a in actions {
                            let topic = a.topic.unwrap();
                            consumed.insert(topic);
                            for fid in ctx
                                .topics
                                .get(&topic)
                                .map(|t| t.infos.clone())
                                .unwrap_or_default()
                            {
                                self.push_info(pn, fid, entries, i18n);
                            }
                        }
                    }
                }
            }
            // Topics by category and subtype.
            let mut topics: Vec<_> = ctx
                .topics
                .values()
                .filter(|t| t.quest == quest && !consumed.contains(&t.formid))
                .collect();
            topics.sort_by_key(|t| (t.category, t.subtype, t.order));
            let mut cat_node: Option<(u8, u16, usize)> = None;
            for t in topics {
                let need_new = match cat_node {
                    Some((c, s, _)) => c != t.category || s != t.subtype,
                    None => true,
                };
                if need_new {
                    let cat = self
                        .spec
                        .category_key(t.category)
                        .map(|k| i18n.t(k))
                        .unwrap_or_else(|| format!("#{}", t.category));
                    let sub = self
                        .spec
                        .subtype_name(t.subtype)
                        .map(str::to_string)
                        .unwrap_or_else(|| t.snam.trim().to_string());
                    let label =
                        if sub.is_empty() || (t.category as i32 == scene_cat && sub == "Custom") {
                            cat
                        } else {
                            i18n.dash(&cat, &sub)
                        };
                    let cn = self.push(
                        Some(qn),
                        NodeKind::Category(t.category, t.subtype),
                        label,
                        String::new(),
                        None,
                    );
                    cat_node = Some((t.category, t.subtype, cn));
                }
                let cn = cat_node.unwrap().2;
                let line = self.line("DIAL", "FULL", t.formid, 1);
                let full = if line.is_some() {
                    Self::text_of_line(entries, line, 60)
                } else {
                    t.full.clone()
                };
                let label = if full.is_empty() {
                    t.edid.clone()
                } else {
                    i18n.t_args("dialog-edid-name", &[("edid", &t.edid), ("name", &full)])
                };
                let extra = i18n.t_arg("dialog-replies", "n", &t.infos.len().to_string());
                let tn = self.push(Some(cn), NodeKind::Topic(t.formid), label, extra, line);
                if let Some(l) = line {
                    self.line_node.insert(l, tn);
                }
                for fid in &t.infos {
                    self.push_info(tn, *fid, entries, i18n);
                }
            }
        }
        // Counts, bottom-up.
        for i in (0..self.nodes.len()).rev() {
            let (done, total) = match self.nodes[i].kind {
                NodeKind::Info(_) | NodeKind::Response(_, _) => {
                    (self.nodes[i].done, self.nodes[i].total)
                }
                _ => self.nodes[i].children.iter().fold((0, 0), |acc, c| {
                    (acc.0 + self.nodes[*c].done, acc.1 + self.nodes[*c].total)
                }),
            };
            self.nodes[i].done = done;
            self.nodes[i].total = total;
            if !matches!(
                self.nodes[i].kind,
                NodeKind::Info(_) | NodeKind::Response(_, _)
            ) {
                self.nodes[i].extra = i18n.t_args(
                    "dialog-counts",
                    &[("done", &done.to_string()), ("total", &total.to_string())],
                );
            }
        }
    }

    fn push_info(&mut self, parent: usize, fid: u32, entries: &[TmEntry], i18n: &I18n) {
        let ctx = self.res.ctx.clone();
        let Some(info) = ctx.infos.get(&fid) else {
            return;
        };
        let speaker = self.speaker_label(i18n, fid);
        let first = info.responses.first();
        let first_line = first
            .and_then(|r| r.line_index)
            .and_then(|ix| self.line("INFO", "NAM1", fid, ix));
        let prompt_line = self.line("INFO", "RNAM", fid, 1);
        let line = first_line.or(prompt_line);
        let text = Self::text_of_line(entries, line, 60);
        let emotion = first
            .map(|r| self.emotion_label(i18n, r.emotion, r.emotion_value))
            .unwrap_or_default();
        let label = if speaker.is_empty() {
            text
        } else {
            i18n.dash(&speaker, &text)
        };
        let extra = if info.responses.len() > 1 {
            i18n.t_arg("dialog-responses-n", "n", &info.responses.len().to_string())
        } else {
            emotion.clone()
        };
        let n = self.push(Some(parent), NodeKind::Info(fid), label, extra, line);
        self.info_node.insert(fid, n);
        // Count the reply lines of the INFO.
        let mut done = 0;
        let mut total = 0;
        let mut lines: Vec<usize> = Vec::new();
        if let Some(l) = prompt_line {
            lines.push(l);
        }
        for (pos, r) in info.responses.iter().enumerate() {
            let l = r
                .line_index
                .and_then(|ix| self.line("INFO", "NAM1", fid, ix));
            if let Some(l) = l {
                lines.push(l);
            }
            if info.responses.len() > 1 {
                let em = self.emotion_label(i18n, r.emotion, r.emotion_value);
                let t = Self::text_of_line(entries, l, 60);
                let label = format!("{} · {}", r.number, t);
                let rn = self.push(Some(n), NodeKind::Response(fid, pos), label, em, l);
                if let Some(l) = l {
                    let d = entries.get(l).map(|e| !e.trans.is_empty()).unwrap_or(false);
                    self.nodes[rn].done = usize::from(d);
                    self.nodes[rn].total = 1;
                    self.line_node.insert(l, rn);
                }
            }
        }
        for l in &lines {
            total += 1;
            if entries
                .get(*l)
                .map(|e| !e.trans.is_empty())
                .unwrap_or(false)
            {
                done += 1;
            }
            self.line_node.entry(*l).or_insert(n);
        }
        self.nodes[n].done = done;
        self.nodes[n].total = total;
    }

    fn build_sequence(&mut self) {
        let ctx = self.res.ctx.clone();
        // Topics: file order, PNAM for the previous, the first linked topic after the last.
        for t in ctx.topics.values() {
            for (i, fid) in t.infos.iter().enumerate() {
                let next = t.infos.get(i + 1).copied().or_else(|| {
                    ctx.infos
                        .get(fid)
                        .and_then(|inf| inf.links.first())
                        .and_then(|lt| ctx.topics.get(lt))
                        .and_then(|lt| lt.infos.first().copied())
                });
                let prev = ctx
                    .infos
                    .get(fid)
                    .and_then(|inf| inf.prev)
                    .filter(|p| ctx.infos.contains_key(p))
                    .or_else(|| {
                        if i > 0 {
                            t.infos.get(i - 1).copied()
                        } else {
                            None
                        }
                    });
                self.sequence.insert(*fid, Sequence { prev, next });
            }
        }
        // Scenes: the actions in order override.
        for sc in ctx.scenes.values() {
            let mut actions: Vec<_> = sc
                .actions
                .iter()
                .filter(|a| a.kind == 0 && a.topic.is_some())
                .collect();
            actions.sort_by_key(|a| (a.start_phase, a.index));
            let chain: Vec<u32> = actions
                .iter()
                .flat_map(|a| {
                    ctx.topics
                        .get(&a.topic.unwrap())
                        .map(|t| t.infos.clone())
                        .unwrap_or_default()
                })
                .collect();
            for (i, fid) in chain.iter().enumerate() {
                let prev = if i > 0 { Some(chain[i - 1]) } else { None };
                let next = chain.get(i + 1).copied();
                self.sequence.insert(*fid, Sequence { prev, next });
            }
        }
    }

    /// The reply after / before one, in the conversation.
    pub fn step(&self, info: u32, dir: i32) -> Option<u32> {
        let s = self.sequence.get(&info)?;
        if dir > 0 { s.next } else { s.prev }
    }

    /// The reply a document line belongs to.
    pub fn info_of_line(&self, line: usize) -> Option<u32> {
        let mut n = *self.line_node.get(&line)?;
        loop {
            match self.nodes[n].kind {
                NodeKind::Info(f) | NodeKind::Response(f, _) => return Some(f),
                _ => n = self.nodes[n].parent?,
            }
        }
    }

    /// The conditions of a reply.
    pub fn conditions(&self, info: u32) -> &[Condition] {
        self.res
            .ctx
            .infos
            .get(&info)
            .map(|i| i.conditions.as_slice())
            .unwrap_or(&[])
    }

    /// The quest and topic labels of a reply (« MQ101 « Unbound » », « MQ101RalofTopic01 (Custom) »).
    pub fn where_of(&self, info: u32, i18n: &I18n) -> (String, String) {
        let ctx = &self.res.ctx;
        let Some(inf) = ctx.infos.get(&info) else {
            return (String::new(), String::new());
        };
        let topic = ctx.topics.get(&inf.topic);
        let quest = topic.and_then(|t| t.quest).map(|q| {
            let name = self.res.display(q);
            let edid = self.res.edid_or_full(q);
            if name == edid {
                name
            } else {
                i18n.t_args("dialog-edid-name", &[("edid", &edid), ("name", &name)])
            }
        });
        let topic_label = topic
            .map(|t| {
                let sub = self
                    .spec
                    .subtype_name(t.subtype)
                    .map(str::to_string)
                    .unwrap_or_else(|| t.snam.trim().to_string());
                let cat = self
                    .spec
                    .category_key(t.category)
                    .map(|k| i18n.t(k))
                    .unwrap_or_default();
                let kind = if sub.is_empty() {
                    cat
                } else if cat.is_empty() {
                    sub
                } else {
                    format!("{cat} / {sub}")
                };
                if t.edid.is_empty() {
                    kind
                } else {
                    i18n.paren(&t.edid, &kind)
                }
            })
            .unwrap_or_default();
        (
            quest.unwrap_or_else(|| i18n.t("dialog-no-quest")),
            topic_label,
        )
    }

    /// Distinct speakers of the document with their reply counts, most
    /// frequent first (the PNJ list, C-02).
    pub fn speaker_counts(&self, i18n: &I18n) -> Vec<(String, String, Gender, usize)> {
        let mut counts: HashMap<String, (String, Gender, usize)> = HashMap::new();
        for (fid, list) in &self.speakers {
            for s in list {
                let key = s.key();
                let label = match &s.kind {
                    SpeakerKind::Unknown => i18n.t("speaker-unknown"),
                    SpeakerKind::Voice(_) => i18n.t_arg("speaker-voice", "name", &s.name),
                    SpeakerKind::Alias { .. } => i18n.t_arg("speaker-alias", "name", &s.name),
                    SpeakerKind::Faction(_) => i18n.t_arg("speaker-faction", "name", &s.name),
                    SpeakerKind::Race(_) => i18n.t_arg("speaker-race", "name", &s.name),
                    SpeakerKind::Npc(_) => s.name.clone(),
                };
                let n = self
                    .res
                    .ctx
                    .infos
                    .get(fid)
                    .map(|i| i.responses.len().max(1))
                    .unwrap_or(1);
                let e = counts.entry(key).or_insert((label, s.gender, 0));
                e.2 += n;
            }
        }
        let mut out: Vec<(String, String, Gender, usize)> = counts
            .into_iter()
            .map(|(k, (l, g, n))| (k, l, g, n))
            .collect();
        out.sort_by(|a, b| b.3.cmp(&a.3).then(a.1.cmp(&b.1)));
        out
    }

    /// Replies whose speaker has this key (C-02 filter).
    pub fn infos_of_speaker(&self, key: &str) -> Vec<u32> {
        self.speakers
            .iter()
            .filter(|(_, l)| l.iter().any(|s| s.key() == key))
            .map(|(f, _)| *f)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> (Arc<ContextMap>, Vec<TmEntry>) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/skyrim_dialogue.esp");
        let ctx = Arc::new(ContextMap::read_file(&p, Family::Skyrim).unwrap());
        let tm = crate::plugin::read_file(&p).unwrap();
        (ctx, tm.entries)
    }

    fn i18n() -> I18n {
        let mut i = I18n::new();
        i.set_locale("fra");
        i
    }

    /// T-C13-01 / T-C13-04: the voice fixture — counts per node, the
    /// unvoiced list for the mod's author, and the filter « Non doublées ».
    #[test]
    fn voices_count_unvoiced_replies() {
        let data =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voices/Data");
        let plugin = data.join("MonMod.esp");
        let ctx = Arc::new(ContextMap::read_file(&plugin, Family::Skyrim).unwrap());
        let tm = crate::plugin::read_file(&plugin).unwrap();
        let i18n = i18n();
        let spec = DialogueSpec::load(Family::Skyrim).voice;
        let voices = crate::voices::VoiceMap::build(&plugin, Family::Skyrim, &spec);
        assert_eq!(voices.sources.len(), 2, "{:?}", voices.sources);
        let map = DialogueMap::build_with(ctx, Vec::new(), &tm.entries, &i18n, Some(&voices));
        let unvoiced: usize = map.roots.iter().map(|r| map.nodes[*r].unvoiced).sum();
        assert_eq!(unvoiced, 4, "303 (lips alone), 304, 306, 307");
        let n301 = map.info_node[&0x01000301];
        assert_eq!(
            map.nodes[n301].unvoiced, 0,
            "both responses voiced (loose + BSA)"
        );
        let n302 = map.info_node[&0x01000302];
        assert_eq!(map.nodes[n302].unvoiced, 0, "load index 05 matched");
        let n303 = map.info_node[&0x01000303];
        assert_eq!(map.nodes[n303].unvoiced, 1);
        let list = map.unvoiced_list(&tm.entries, &i18n);
        assert_eq!(list.len(), 4);
        let mut ids: Vec<&str> = list.iter().map(|l| l.split('\t').next().unwrap()).collect();
        ids.sort();
        assert_eq!(ids, vec!["01000303", "01000304", "01000306", "01000307"]);
        assert!(list.iter().all(|l| l.split('\t').count() == 4), "{list:?}");
        // Without any source, nothing is unvoiced.
        let lonely = DialogueMap::build_with(
            map.res.ctx.clone(),
            Vec::new(),
            &tm.entries,
            &i18n,
            Some(&crate::voices::VoiceMap::default()),
        );
        assert_eq!(
            lonely
                .roots
                .iter()
                .map(|r| lonely.nodes[*r].unvoiced)
                .sum::<usize>(),
            0
        );
    }

    /// T-C01-01: the tree of the synthetic quest mod.
    #[test]
    fn tree_of_the_fixture() {
        let (ctx, entries) = load();
        let i18n = i18n();
        let map = DialogueMap::build(ctx, Vec::new(), &entries, &i18n);
        let labels: Vec<(usize, &str)> = map
            .nodes
            .iter()
            .map(|n| (n.depth, n.label.as_str()))
            .collect();
        assert_eq!(labels[0], (0, "MQ101 — Unbound"));
        assert!(
            labels[1].1.starts_with("Scène MQ101IntroScene"),
            "{:?}",
            labels[1]
        );
        assert!(
            labels[2].1.starts_with("Phase 1 — Ralof"),
            "{:?}",
            labels[2]
        );
        assert_eq!(labels[3].0, 3);
        assert!(
            labels[3].1.starts_with("Ralof — You were trying"),
            "{:?}",
            labels[3]
        );
        assert!(
            labels[5].1.contains("Lokir — Damn you Stormcloaks"),
            "{:?}",
            labels[5]
        );
        // Custom topics after the scene; the scene topics are not repeated.
        let topic_nodes: Vec<&Node> = map
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Topic(_)))
            .collect();
        assert_eq!(topic_nodes.len(), 3, "two custom topics and the hello one");
        assert!(
            topic_nodes[0]
                .label
                .starts_with("MQ101RalofTopic01 « What's going on? »")
        );
        // The reply with two responses expands.
        let i1 = map.info_node[&0x01000301];
        assert_eq!(map.nodes[i1].children.len(), 2);
        assert!(map.nodes[i1].label.starts_with("Ralof — We're being taken"));
        assert_eq!(map.nodes[i1].extra, "2 réponses");
        let r2 = map.nodes[i1].children[1];
        assert_eq!(map.nodes[r2].extra, "Peur 40");
        // « Sans quête » holds the hello topic, with its category.
        let no_quest = map
            .nodes
            .iter()
            .position(|n| n.kind == NodeKind::NoQuest)
            .unwrap();
        let cat = map.nodes[no_quest].children[0];
        assert_eq!(map.nodes[cat].label, "Divers — Hello");
        // Counts: 8 NAM1 and 1 RNAM in the document; MQ101 holds 8 of them.
        assert_eq!(map.reply_lines, 9);
        assert_eq!(
            map.nodes[0].total, 8,
            "MQ101: 2 scene lines + 5 + 1 custom lines"
        );
        assert_eq!(map.nodes[0].done, 0);
        assert_eq!(map.nodes[0].extra, "0 / 8");
    }

    /// R-C01-05: lines ↔ nodes, both ways.
    #[test]
    fn lines_and_nodes() {
        let (ctx, entries) = load();
        let i18n = i18n();
        let map = DialogueMap::build(ctx, Vec::new(), &entries, &i18n);
        let l = map.line("INFO", "NAM1", 0x01000301, 2).unwrap();
        assert_eq!(entries[l].source, "Don't worry, I'm sure it's a mistake.");
        assert_eq!(map.info_of_line(l), Some(0x01000301));
        let prompt = map.line("INFO", "RNAM", 0x01000303, 1).unwrap();
        assert_eq!(map.info_of_line(prompt), Some(0x01000303));
        let full = map.line("DIAL", "FULL", 0x01000201, 1).unwrap();
        assert!(matches!(
            map.nodes[map.line_node[&full]].kind,
            NodeKind::Topic(0x01000201)
        ));
    }

    /// R-C01-04 / T-C01-03: the conversation order.
    #[test]
    fn conversation_order() {
        let (ctx, entries) = load();
        let map = DialogueMap::build(ctx, Vec::new(), &entries, &i18n());
        assert_eq!(map.step(0x01000301, 1), Some(0x01000302));
        assert_eq!(map.step(0x01000302, 1), Some(0x01000303));
        // The last reply of the topic goes to the linked topic (TCLT of I1 → topic 2).
        assert_eq!(map.step(0x01000303, 1), None, "I3 has no link");
        assert_eq!(map.step(0x01000302, -1), Some(0x01000301), "PNAM");
        // Scenes chain their actions across phases.
        assert_eq!(map.step(0x01000306, 1), Some(0x01000307));
        assert_eq!(map.step(0x01000307, -1), Some(0x01000306));
    }

    /// R-C02-01: speakers by ANAM, scene alias, conditions, and unknown.
    #[test]
    fn speakers() {
        let (ctx, entries) = load();
        let i18n = i18n();
        let map = DialogueMap::build(ctx, Vec::new(), &entries, &i18n);
        let s = map.speaker(0x01000301).unwrap();
        assert_eq!(
            (s.name.as_str(), s.gender, s.origin),
            ("Ralof", Gender::Male, SpeakerOrigin::Anam)
        );
        assert_eq!(s.voice.as_deref(), Some("MaleNord"));
        let s = map.speaker(0x01000306).unwrap();
        assert_eq!((s.name.as_str(), s.origin), ("Ralof", SpeakerOrigin::Scene));
        let s = map.speaker(0x01000307).unwrap();
        assert_eq!((s.name.as_str(), s.origin), ("Lokir", SpeakerOrigin::Scene));
        let s = map.speaker(0x01000302).unwrap();
        assert_eq!(
            (s.name.as_str(), s.origin),
            ("Lokir", SpeakerOrigin::Condition)
        );
        let s = map.speaker(0x01000304).unwrap();
        assert_eq!(
            (s.name.as_str(), s.origin),
            ("Ralof", SpeakerOrigin::Condition),
            "alias → unique actor"
        );
        let s = map.speaker(0x01000305).unwrap();
        assert_eq!((s.name.as_str(), s.gender), ("Gerdur", Gender::Female));
        assert_eq!(map.speaker(0x01000303).unwrap().kind, SpeakerKind::Unknown);
        assert_eq!(map.speaker_label(&i18n, 0x01000303), "locuteur indéterminé");
        // An NPC of the master, without the master: the FormID and the file.
        let s = map.speaker(0x01000307);
        assert!(s.is_some());
        let counts = map.speaker_counts(&i18n);
        assert_eq!(counts[0].1, "Ralof");
        assert_eq!(counts[0].3, 4, "Ralof: 2 responses of I1, I4, I6");
        assert_eq!(map.infos_of_speaker("npc:01000A02").len(), 2);
    }

    #[test]
    fn unresolved_master_form() {
        let (ctx, entries) = load();
        let map = DialogueMap::build(ctx, Vec::new(), &entries, &i18n());
        assert_eq!(map.res.display(0x000A2C8E), "000A2C8E (Skyrim.esm)");
        assert_eq!(map.res.display(0x01000C01), "Whiterun Guards");
        assert_eq!(map.res.edid_or_full(0x01000C01), "GuardFaction");
    }
}
