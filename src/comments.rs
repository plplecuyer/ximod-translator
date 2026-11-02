//! The Commentaire column (C-03): a flat summary of what the dialogue map
//! (C-01, C-02) and the context pass know about a line — « Ralof (h) · MQ101
//! · MQ101RalofTopic01 · Colère 60 · réponse 1 / 2 » for a reply, « modèle :
//! Weapons\Iron\IronWarAxe.nif » for a weapon, « maître : Skyrim.esm » for
//! an override — behind the author's own comment when the file carries one.
//! The text is generated on demand and cached per line (R-C03-01).

use std::sync::Arc;

use crate::dialogue::{DialogueMap, Gender};
use crate::i18n::I18n;
use crate::tm::TmEntry;

/// Longest text shown in the cell (R-C03-02); the tooltip has it whole.
pub const MAX_SHOWN: usize = 200;

/// The separator between the parts of a comment.
pub const SEP: &str = " · ";

/// What the generator needs: the dialogue map of the document (none while
/// the context pass runs), the messages, and — C-13 — the voice map, which
/// says whether a response has its voice file.
pub struct Generator<'a> {
    pub map: Option<&'a DialogueMap>,
    pub i18n: &'a I18n,
    pub voices: Option<Arc<crate::voices::VoiceMap>>,
}

impl Generator<'_> {
    /// The generated part alone (empty when nothing is known).
    pub fn generated(&self, e: &TmEntry) -> String {
        let Some(map) = self.map else {
            return String::new();
        };
        let i18n = self.i18n;
        let res = &map.res;
        let ctx = &res.ctx;
        let mut parts: Vec<String> = Vec::new();
        match e.rec_sig.as_str() {
            "INFO" => {
                if let Some(info) = ctx.infos.get(&e.formid) {
                    // Speaker and gender.
                    let name = map.speaker_label(i18n, e.formid);
                    if !name.is_empty() {
                        let gender = map
                            .speaker(e.formid)
                            .map(|s| s.gender)
                            .unwrap_or(Gender::Unknown);
                        parts.push(match gender {
                            Gender::Male => i18n.paren(&name, &i18n.t("gender-m-short")),
                            Gender::Female => i18n.paren(&name, &i18n.t("gender-f-short")),
                            Gender::Unknown => name,
                        });
                    }
                    if let Some(t) = ctx.topics.get(&info.topic) {
                        if let Some(q) = t.quest {
                            parts.push(res.edid_or_full(q));
                        }
                        push_nonempty(&mut parts, topic_kind(map, t, i18n));
                    }
                    if e.field_sig != "RNAM" {
                        let n = e.index.max(1) as usize;
                        let total = info.responses.len();
                        if let Some(r) = info.responses.get(n - 1) {
                            push_nonempty(
                                &mut parts,
                                map.emotion_label(i18n, r.emotion, r.emotion_value),
                            );
                        }
                        if total > 0 {
                            parts.push(i18n.t_args(
                                "dialog-response-of",
                                &[("n", &n.to_string()), ("total", &total.to_string())],
                            ));
                        }
                        if let Some(voiced) = self
                            .voices
                            .as_ref()
                            .and_then(|v| v.voiced(e.formid, n as u8))
                        {
                            parts.push(i18n.t(if voiced {
                                "comment-voiced"
                            } else {
                                "comment-unvoiced"
                            }));
                        }
                    }
                }
            }
            "DIAL" => {
                if let Some(t) = ctx.topics.get(&e.formid) {
                    if let Some(q) = t.quest {
                        parts.push(res.edid_or_full(q));
                    }
                    push_nonempty(&mut parts, topic_kind(map, t, i18n));
                    if !t.infos.is_empty() {
                        parts.push(i18n.t_arg("dialog-replies", "n", &t.infos.len().to_string()));
                    }
                }
            }
            "QUST" => {
                if let Some(q) = ctx.quests.get(&e.formid) {
                    let n = e.index.max(1) as usize;
                    match e.field_sig.as_str() {
                        "CNAM" => {
                            parts.push(i18n.t_arg("comment-quest", "name", &q.edid));
                            if let Some(stage) = q.stages.get(n - 1) {
                                parts.push(i18n.t_arg("comment-journal", "n", &stage.to_string()));
                            }
                        }
                        "NNAM" => {
                            parts.push(i18n.t_arg("comment-quest", "name", &q.edid));
                            if let Some(obj) = q.objectives.get(n - 1) {
                                parts.push(i18n.t_arg("comment-objective", "n", &obj.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            "NPC_" => {
                if let Some(n) = ctx.npcs.get(&e.formid) {
                    parts.push(i18n.t(if n.female { "gender-f" } else { "gender-m" }));
                    // The race only when its record is known (an unresolved
                    // FormID says nothing to the translator).
                    if let Some(r) = n.race.filter(|&r| res.named(r).is_some()) {
                        push_nonempty(&mut parts, res.edid_or_full(r));
                    }
                    if let Some(v) = n.voice
                        && let Some(name) = res.voice(v)
                    {
                        parts.push(i18n.t_arg("speaker-voice", "name", name));
                    }
                }
            }
            _ => {
                if let Some(n) = ctx.names.get(&e.formid) {
                    if let Some(q) = n.quest {
                        parts.push(i18n.t_arg("comment-quest", "name", &res.edid_or_full(q)));
                    }
                    if let Some(m) = &n.model {
                        parts.push(i18n.t_arg("comment-model", "path", m));
                    }
                }
            }
        }
        // An override of a master's record (R-C03-02).
        if e.formid != 0
            && let Some(master) = ctx.master_of(e.formid)
        {
            parts.push(i18n.t_arg("comment-master", "file", master));
        }
        parts.join(SEP)
    }

    /// The whole comment: the author's text first (R-C03-03), then the
    /// generated part.
    pub fn full(&self, e: &TmEntry) -> String {
        join(&self.i18n.localize(&e.comment), &self.generated(e))
    }
}

/// « author · generated », either side possibly empty.
pub fn join(author: &str, generated: &str) -> String {
    let author = author.trim();
    match (author.is_empty(), generated.is_empty()) {
        (true, _) => generated.to_string(),
        (false, true) => author.to_string(),
        (false, false) => format!("{author}{SEP}{generated}"),
    }
}

fn push_nonempty(parts: &mut Vec<String>, s: String) {
    if !s.is_empty() {
        parts.push(s);
    }
}

/// The topic's EDID, else its « catégorie / sous-type ».
fn topic_kind(map: &DialogueMap, t: &crate::plugin::context::Topic, i18n: &I18n) -> String {
    if !t.edid.is_empty() {
        return t.edid.clone();
    }
    let sub = map
        .spec
        .subtype_name(t.subtype)
        .map(str::to_string)
        .unwrap_or_else(|| t.snam.trim().to_string());
    let cat = map
        .spec
        .category_key(t.category)
        .map(|k| i18n.t(k))
        .unwrap_or_default();
    if sub.is_empty() {
        cat
    } else if cat.is_empty() {
        sub
    } else {
        format!("{cat} / {sub}")
    }
}

/// The text shown in the cell: cut at [`MAX_SHOWN`] characters with an
/// ellipsis; `None` when it fits.
pub fn shown(full: &str) -> Option<String> {
    if full.chars().count() <= MAX_SHOWN {
        return None;
    }
    let cut: String = full.chars().take(MAX_SHOWN - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// The per-document cache of the column (R-C03-01): one slot per line,
/// filled on first display, dropped when the document or its map changes.
#[derive(Default)]
pub struct Cache {
    key: Option<CacheKey>,
    cells: Vec<Option<Arc<str>>>,
}

/// (document serial, map key, voices generation).
pub type CacheKey = (u64, Option<(u64, u64, usize, u64)>, u64);

impl Cache {
    /// Align the cache on a document: a new key clears it.
    pub fn align(&mut self, key: CacheKey, lines: usize) {
        if self.key != Some(key) || self.cells.len() != lines {
            self.key = Some(key);
            self.cells.clear();
            self.cells.resize(lines, None);
        }
    }

    /// The comment of a line, generated once.
    pub fn get(&mut self, idx: usize, e: &TmEntry, g: &Generator<'_>) -> Arc<str> {
        if idx >= self.cells.len() {
            return Arc::from(g.full(e));
        }
        if let Some(c) = &self.cells[idx] {
            return c.clone();
        }
        let c: Arc<str> = Arc::from(g.full(e));
        self.cells[idx] = Some(c.clone());
        c
    }

    /// Whether a line is already generated.
    pub fn is_ready(&self, idx: usize) -> bool {
        self.cells.get(idx).is_some_and(Option::is_some)
    }

    /// Drop everything (the voices changed, a reload…).
    pub fn clear(&mut self) {
        self.key = None;
        self.cells.clear();
    }
}

/// Whether an output file carries a comment per line (EET, EET XML, XIMOD
/// XML): the formats the generated comments may go to (R-C03-06).
pub fn format_carries_comment(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("eet") | Some("xml")
    )
}

/// A copy of the memory whose empty comments receive the generated text,
/// for the formats that carry a comment (R-C03-06).
pub fn with_generated(
    tm: &crate::tm::TranslationMemory,
    g: &Generator<'_>,
) -> crate::tm::TranslationMemory {
    let mut out = tm.clone();
    for e in &mut out.entries {
        let text = g.generated(e);
        if !text.is_empty() {
            e.comment = join(&e.comment, &text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogue::DialogueMap;
    use crate::plugin::context::{ContextMap, Family};

    fn fixture_map(i18n: &I18n) -> (Arc<DialogueMap>, Vec<TmEntry>) {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/skyrim_dialogue.esp");
        let tm = crate::plugin::read_file(&p).unwrap();
        let ctx = Arc::new(ContextMap::read_file(&p, Family::Skyrim).unwrap());
        let map = DialogueMap::build(ctx, Vec::new(), &tm.entries, i18n);
        (Arc::new(map), tm.entries)
    }

    fn fra() -> I18n {
        let mut i = I18n::new();
        i.set_locale("fra");
        i
    }

    fn find<'a>(
        entries: &'a [TmEntry],
        sig: &str,
        field: &str,
        formid: u32,
        index: i32,
    ) -> &'a TmEntry {
        entries
            .iter()
            .find(|e| {
                e.rec_sig == sig && e.field_sig == field && e.formid == formid && e.index == index
            })
            .unwrap_or_else(|| panic!("{sig}:{field} {formid:08X} #{index}"))
    }

    /// T-C03-01: a reply, a weapon, a journal entry, an override.
    #[test]
    fn generates_per_line_kind() {
        let i18n = fra();
        let (map, entries) = fixture_map(&i18n);
        let g = Generator {
            map: Some(&map),
            i18n: &i18n,
            voices: None,
        };
        let reply = g.generated(find(&entries, "INFO", "NAM1", 0x01000301, 1));
        assert!(reply.starts_with("Ralof (h) · MQ101 · "), "{reply}");
        assert!(reply.contains("réponse 1 / "), "{reply}");
        assert!(!reply.contains("voix"), "no voice map yet: {reply}");
        let axe = g.generated(find(&entries, "WEAP", "FULL", 0x01000E01, 1));
        assert_eq!(axe, "modèle : Weapons\\Iron\\IronWarAxe.nif");
        let msg = g.generated(find(&entries, "MESG", "FULL", 0x01000F01, 1));
        assert_eq!(msg, "quête MQ101");
        let j1 = g.generated(find(&entries, "QUST", "CNAM", 0x01000101, 1));
        assert_eq!(j1, "quête MQ101 · entrée de journal, étape 10");
        let j3 = g.generated(find(&entries, "QUST", "CNAM", 0x01000101, 3));
        assert_eq!(j3, "quête MQ101 · entrée de journal, étape 20");
        let obj = g.generated(find(&entries, "QUST", "NNAM", 0x01000101, 1));
        assert_eq!(obj, "quête MQ101 · objectif 10");
        let hadvar = g.generated(find(&entries, "NPC_", "FULL", 0x0002BF9D, 1));
        assert!(hadvar.starts_with("homme · "), "{hadvar}");
        assert!(
            hadvar.ends_with(" · voix MaleNord · maître : Skyrim.esm"),
            "{hadvar}"
        );
        let gerdur = g.generated(find(&entries, "NPC_", "FULL", 0x01000A03, 1));
        assert!(gerdur.starts_with("femme · "), "{gerdur}");
        let topic = g.generated(find(&entries, "DIAL", "FULL", 0x01000201, 1));
        assert!(topic.starts_with("MQ101 · "), "{topic}");
        assert!(topic.ends_with(" · 3 répliques"), "{topic}");
    }

    /// T-C03-02: the author's comment comes first; the export copy fills
    /// the empty comments only when asked.
    #[test]
    fn author_comment_first_and_export_copy() {
        let i18n = fra();
        let (map, mut entries) = fixture_map(&i18n);
        let g = Generator {
            map: Some(&map),
            i18n: &i18n,
            voices: None,
        };
        let i = entries
            .iter()
            .position(|e| e.rec_sig == "WEAP" && e.field_sig == "FULL")
            .unwrap();
        entries[i].comment = "à vérifier".into();
        assert_eq!(
            g.full(&entries[i]),
            "à vérifier · modèle : Weapons\\Iron\\IronWarAxe.nif"
        );
        assert!(format_carries_comment(std::path::Path::new("a/b.EET")));
        assert!(format_carries_comment(std::path::Path::new("b.xml")));
        assert!(!format_carries_comment(std::path::Path::new("b.sst")));
        assert_eq!(join("", ""), "");
        assert_eq!(join("a", ""), "a");
        assert_eq!(join("", "b"), "b");
        let tm = crate::tm::TranslationMemory {
            entries: entries.clone(),
            ..Default::default()
        };
        let out = with_generated(&tm, &g);
        assert_eq!(
            out.entries[i].comment,
            "à vérifier · modèle : Weapons\\Iron\\IronWarAxe.nif"
        );
        // The original memory is untouched.
        assert_eq!(tm.entries[i].comment, "à vérifier");
        // Without a map nothing is generated.
        let none = Generator {
            map: None,
            i18n: &i18n,
            voices: None,
        };
        assert_eq!(none.full(&entries[i]), "à vérifier");
    }

    /// T-C03-02 (files): the EET and XIMOD XML writers carry the generated
    /// comments of the copy, and the author's alone otherwise.
    #[test]
    fn eet_and_xml_round_trip_generated_comments() {
        let i18n = fra();
        let (map, mut entries) = fixture_map(&i18n);
        let g = Generator {
            map: Some(&map),
            i18n: &i18n,
            voices: None,
        };
        let i = entries
            .iter()
            .position(|e| e.rec_sig == "WEAP" && e.field_sig == "FULL")
            .unwrap();
        entries[i].comment = "à vérifier".into();
        let tm = crate::tm::TranslationMemory {
            entries,
            ..Default::default()
        };
        let (bytes, _) = crate::tm::eet::write_bytes(&tm);
        let back = crate::tm::eet::read_bytes(&bytes).unwrap();
        let w = back.entries.iter().find(|e| e.edid == "RalofAxe").unwrap();
        assert_eq!(w.comment, "à vérifier", "author alone without the option");
        let full = with_generated(&tm, &g);
        let (bytes, _) = crate::tm::eet::write_bytes(&full);
        let back = crate::tm::eet::read_bytes(&bytes).unwrap();
        let w = back.entries.iter().find(|e| e.edid == "RalofAxe").unwrap();
        assert_eq!(
            w.comment,
            "à vérifier · modèle : Weapons\\Iron\\IronWarAxe.nif"
        );
        let xml = crate::tm::xml::write_ximodxml(&full);
        let back = crate::tm::xml::read_ximodxml(&xml).unwrap();
        let r = back
            .entries
            .iter()
            .find(|e| e.rec_sig == "INFO" && e.formid == 0x01000301 && e.index == 1)
            .unwrap();
        assert!(r.comment.starts_with("Ralof (h) · MQ101"), "{}", r.comment);
    }

    /// T-C03-03 / C-13: « voix ✔ / ✘ » from the voice map.
    #[test]
    fn voice_marks_follow_the_map() {
        let i18n = fra();
        let data =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voices/Data");
        let plugin = data.join("MonMod.esp");
        let tm = crate::plugin::read_file(&plugin).unwrap();
        let ctx = Arc::new(ContextMap::read_file(&plugin, Family::Skyrim).unwrap());
        let map = DialogueMap::build(ctx, Vec::new(), &tm.entries, &i18n);
        let spec = crate::dialogue::DialogueSpec::load(Family::Skyrim).voice;
        let voices = Arc::new(crate::voices::VoiceMap::build(
            &plugin,
            Family::Skyrim,
            &spec,
        ));
        let g = Generator {
            map: Some(&map),
            i18n: &i18n,
            voices: Some(voices),
        };
        let a = g.generated(find(&tm.entries, "INFO", "NAM1", 0x01000301, 1));
        assert!(a.ends_with(" · voix ✔"), "{a}");
        let b = g.generated(find(&tm.entries, "INFO", "NAM1", 0x01000303, 1));
        assert!(b.ends_with(" · voix ✘"), "{b}");
    }

    /// The cache fills lazily and follows its key; long texts are cut.
    #[test]
    fn cache_and_truncation() {
        let i18n = fra();
        let (map, entries) = fixture_map(&i18n);
        let g = Generator {
            map: Some(&map),
            i18n: &i18n,
            voices: None,
        };
        let mut cache = Cache::default();
        cache.align((1, None, 0), entries.len());
        assert!(!cache.is_ready(0));
        let a = cache.get(0, &entries[0], &g);
        assert!(cache.is_ready(0));
        let b = cache.get(0, &entries[0], &g);
        assert!(Arc::ptr_eq(&a, &b));
        cache.align((2, None, 0), entries.len());
        assert!(!cache.is_ready(0), "a new key clears the cache");
        assert_eq!(shown("court"), None);
        let long = "x".repeat(250);
        let s = shown(&long).unwrap();
        assert_eq!(s.chars().count(), MAX_SHOWN);
        assert!(s.ends_with('…'));
    }
}
