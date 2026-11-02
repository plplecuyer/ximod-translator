//! The dictionaries applied to one document, in priority order (E-12, then
//! B-02): the active nodes of the document's dictionary tree, each indexed
//! on its own. The first level that knows a line wins — strict then relax
//! per dictionary — and each level only fills what the previous ones left
//! empty. Every filled line remembers where its translation came from.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::basedb::{BaseDb, FileDbCache, FillLevel, FormCache};
use crate::dicts::{Role, Tree};
use crate::doc_settings::DocSettings;
use crate::i18n::I18n;
use crate::tm::{TmEntry, TranslationMemory};

/// One active dictionary.
pub struct Level {
    pub role: Role,
    pub path: PathBuf,
    pub db: Arc<BaseDb>,
}

impl Level {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string()
    }
}

/// Where a translation came from (B-02); written on the line as its
/// provenance code (B-09).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    pub role: Role,
    pub level: FillLevel,
}

pub struct DictStack {
    pub levels: Vec<Level>,
    /// The normalized tier (B-19), on at opening unless
    /// `[Fuzzy] Normalize=0` (decision D-72).
    pub normalize: bool,
}

/// How many lines each level filled, in stack order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackReport {
    pub per_level: Vec<(String, usize)>,
    /// Lines left En attente because their source is ambiguous (B-08).
    pub ambiguous: usize,
    /// Lines matched on their normalized form, left En attente (B-19).
    pub normalized: usize,
}

impl StackReport {
    pub fn total(&self) -> usize {
        self.per_level.iter().map(|(_, n)| n).sum()
    }
}

impl DictStack {
    /// Path of the mod's own dictionary for a document (B-03), whether it
    /// exists or not.
    pub fn mod_dict_path(doc_path: &Path, settings: &DocSettings, mods_root: &Path) -> PathBuf {
        let stem = doc_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("mod");
        let game_sub: String = settings
            .game
            .trim()
            .chars()
            .map(|c| if std::path::is_separator(c) { '_' } else { c })
            .collect();
        let dir = if game_sub.is_empty() {
            mods_root.to_path_buf()
        } else {
            mods_root.join(game_sub)
        };
        dir.join(format!("{stem}_{}.ximdb", settings.pair_suffix()))
    }

    /// Path of the personal dictionary for a pair (B-04).
    pub fn personal_path(settings: &DocSettings, dict_dir: &Path) -> PathBuf {
        dict_dir.join(format!("personal_{}.ximdb", settings.pair_suffix()))
    }

    /// The stack of a tree: its active nodes, indexed through the cache
    /// (a file that cannot be read is skipped).
    pub fn resolve(tree: &Tree, cache: &mut FileDbCache) -> DictStack {
        let levels = tree
            .active()
            .into_iter()
            .filter_map(|n| {
                cache.get(&n.path).map(|db| Level {
                    role: n.role,
                    path: n.path.clone(),
                    db,
                })
            })
            .collect();
        DictStack {
            levels,
            normalize: true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// Fill the still-untranslated lines, level after level.
    pub fn auto_translate(&self, tm: &mut TranslationMemory) -> StackReport {
        self.auto_translate_entries(&mut tm.entries, &mut Vec::new())
    }

    /// [`auto_translate`](Self::auto_translate) on a slice of entries,
    /// collecting the provenance of each filled line.
    pub fn auto_translate_entries(
        &self,
        entries: &mut [TmEntry],
        origins: &mut Vec<(usize, Origin)>,
    ) -> StackReport {
        let mut r = StackReport::default();
        let mut forms = FormCache::new();
        for level in &self.levels {
            let name = level.name();
            // The personal dictionary gives Personnalisé (B-04, decision D-13).
            let over = (level.role == Role::Personal).then_some(crate::status::Status::Custom);
            let first = origins.len();
            let mut found: std::collections::HashMap<usize, String> = Default::default();
            let norm = self.normalize.then_some(&mut forms);
            let n = level
                .db
                .auto_translate_entries_norm(entries, over, norm, |idx, fill, src| {
                    if let Some(src) = src {
                        found.insert(idx, src.to_string());
                    }
                    origins.push((
                        idx,
                        Origin {
                            file: name.clone(),
                            role: level.role,
                            level: fill,
                        },
                    ));
                });
            // The provenance of the lines filled (B-09): the one stored
            // with the mod's own dictionary, else this level.
            for (idx, o) in &origins[first..] {
                if let Some(e) = entries.get_mut(*idx)
                    && e.prov.is_empty()
                    && !e.trans.is_empty()
                {
                    e.prov = match o.level {
                        FillLevel::Normalized(rep) => {
                            r.normalized += 1;
                            let src = found.get(idx).map(String::as_str).unwrap_or("");
                            crate::provenance::normalized(o.role, &o.file, rep, src)
                        }
                        _ => crate::provenance::dict(o.role, o.level, &o.file),
                    };
                }
            }
            r.per_level.push((name, n));
        }
        r.ambiguous = self.mark_ambiguous(entries, origins);
        r
    }

    /// The candidates of a source across the stack (B-08), in stack order.
    pub fn candidates(&self, source: &str) -> Vec<crate::ambiguity::Candidate> {
        let names: Vec<String> = self.levels.iter().map(Level::name).collect();
        crate::ambiguity::candidates(
            source,
            names
                .iter()
                .zip(&self.levels)
                .map(|(n, l)| (n.as_str(), l.role, &*l.db)),
        )
    }

    /// Whether `source` is ambiguous in the stack: at least two distinct
    /// candidates and no personal dictionary deciding (R-B08-01/03).
    /// Returns the number of candidates.
    pub fn ambiguity_of(&self, source: &str) -> Option<usize> {
        if crate::ambiguity::personal_decides(source, self.levels.iter().map(|l| (l.role, &*l.db)))
        {
            return None;
        }
        let n = self.candidates(source).len();
        (n >= 2).then_some(n)
    }

    /// Mark the lines just filled at a text level by an ambiguous source
    /// (R-B08-02, decision D-44): En attente, provenance « ambiguë (n) ».
    fn mark_ambiguous(&self, entries: &mut [TmEntry], origins: &[(usize, Origin)]) -> usize {
        // One lookup per distinct source: many lines share theirs.
        let mut cache: std::collections::HashMap<String, Option<usize>> = Default::default();
        let mut n = 0;
        for (idx, o) in origins {
            if !crate::ambiguity::level_can_be(o.level) || o.role == Role::Personal {
                continue;
            }
            let Some(e) = entries.get(*idx) else { continue };
            if e.trans.is_empty() || e.unused {
                continue;
            }
            let amb = match cache.get(e.source.as_str()) {
                Some(a) => *a,
                None => {
                    let a = self.ambiguity_of(&e.source);
                    cache.insert(e.source.clone(), a);
                    a
                }
            };
            if let Some(count) = amb {
                let e = &mut entries[*idx];
                e.prov = crate::ambiguity::mark(&e.prov, count);
                crate::status::set(e, crate::status::Status::Pending);
                n += 1;
            }
        }
        n
    }

    /// Short name of the stack for the undo tooltip (« mod + perso + jeu »).
    pub fn label(&self, i18n: &I18n) -> String {
        let mut keys: Vec<&str> = Vec::new();
        for l in &self.levels {
            let k = l.role.short_key();
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        if keys.is_empty() {
            return "—".to_string();
        }
        let parts: Vec<String> = keys.iter().map(|k| i18n.t(k)).collect();
        parts.join(" + ")
    }

    /// « Dictionnaires : MonMod (212), Skyrim (1 004) — 1 221 lignes
    /// pré-remplies » (R-B02-07): the dictionaries that filled something,
    /// in order; « aucun » when none did.
    pub fn status_line(&self, i18n: &I18n, report: &StackReport) -> String {
        let list: Vec<String> = report
            .per_level
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(name, n)| {
                format!(
                    "{} ({})",
                    stem_of(name),
                    crate::ui::export_dialog::fmt_count(i18n, *n)
                )
            })
            .collect();
        let list = if list.is_empty() {
            i18n.t("dict-stack-none")
        } else {
            list.join(", ")
        };
        let line = i18n.t_args(
            "dict-stack-status",
            &[
                ("list", &list),
                (
                    "count",
                    &crate::ui::export_dialog::fmt_count(i18n, report.total()),
                ),
            ],
        );
        let line = if report.normalized > 0 {
            i18n.dash(
                &line,
                &i18n.t_arg(
                    "status-normalized-count",
                    "n",
                    &crate::ui::export_dialog::fmt_count(i18n, report.normalized),
                ),
            )
        } else {
            line
        };
        if report.ambiguous > 0 {
            i18n.dash(
                &line,
                &i18n.t_arg(
                    "apply-ambiguous",
                    "n",
                    &crate::ui::export_dialog::fmt_count(i18n, report.ambiguous),
                ),
            )
        } else {
            line
        }
    }
}

/// « MonMod_ENG-FRA.ximdb » → « MonMod_ENG-FRA ».
pub fn stem_of(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basedb::{BaseRow, write_ximdb};
    use crate::config::OutputMode;
    use crate::dicts::Discover;

    fn entry(rec: &str, field: &str, formid: u32, edid: &str, src: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            ..Default::default()
        }
    }

    fn row(rec: &str, field: &str, formid: u32, edid: &str, src: &str, tr: &str) -> BaseRow {
        BaseRow {
            rec: rec.into(),
            field: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            trans: tr.into(),
            status: None,
            ..Default::default()
        }
    }

    fn settings(use_personal: bool) -> DocSettings {
        DocSettings {
            game: "SkyrimSE".into(),
            source: "eng".into(),
            target: "fra".into(),
            output: Some(OutputMode::Strings),
            use_personal,
        }
    }

    #[test]
    fn paths_follow_the_conventions() {
        let s = settings(false);
        let p =
            DictStack::mod_dict_path(Path::new("/mods/MonMod.esp"), &s, Path::new("/bases/mods"));
        assert_eq!(
            p,
            PathBuf::from("/bases/mods/SkyrimSE/MonMod_ENG-FRA.ximdb")
        );
        assert_eq!(
            DictStack::personal_path(&s, Path::new("/cfg/dict")),
            PathBuf::from("/cfg/dict/personal_ENG-FRA.ximdb")
        );
    }

    /// The tree's order is the priority; reordering changes the winner;
    /// the origins name the dictionary and the level.
    #[test]
    fn priority_follows_the_tree_and_origins_are_kept() {
        let root = std::env::temp_dir().join(format!("ximod-stack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mods = root.join("mods");
        let jeux = root.join("jeux").join("SkyrimSE");
        let dict = root.join("dict");
        std::fs::create_dir_all(mods.join("SkyrimSE")).unwrap();
        std::fs::create_dir_all(&jeux).unwrap();
        std::fs::create_dir_all(&dict).unwrap();
        // Game base: A, B, C ; personal: A, B ; mod: A.
        write_ximdb(
            vec![
                row("WEAP", "FULL", 1, "A", "a", "jeu-A"),
                row("WEAP", "FULL", 2, "B", "b", "jeu-B"),
                row("WEAP", "FULL", 3, "C", "c", "jeu-C"),
            ],
            "fra",
            &jeux.join("Skyrim_ENG-FRA.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![
                row("WEAP", "FULL", 1, "A", "a", "perso-A"),
                row("WEAP", "FULL", 2, "B", "b", "perso-B"),
            ],
            "fra",
            &dict.join("personal_ENG-FRA.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("WEAP", "FULL", 1, "A", "a", "mod-A")],
            "fra",
            &mods.join("SkyrimSE").join("MonMod_ENG-FRA.ximdb"),
        )
        .unwrap();

        let game_dirs = vec![jeux.clone(), mods.join("SkyrimSE")];
        let mut cache = FileDbCache::default();
        let doc = root.join("MonMod.esp");
        let mk = || TranslationMemory {
            meta: Default::default(),
            entries: vec![
                entry("WEAP", "FULL", 1, "A", "a"),
                entry("WEAP", "FULL", 2, "B", "b"),
                // Same text, another record: the loose tier.
                entry("WEAP", "FULL", 9, "C2", "c"),
                {
                    let mut e = entry("WEAP", "FULL", 4, "D", "d");
                    e.trans = "déjà".into();
                    e
                },
            ],
            history: Vec::new(),
        };
        let s = settings(true);
        let d = Discover {
            doc_path: &doc,
            settings: &s,
            mods_root: &mods,
            dict_dir: &dict,
            game_dirs: &game_dirs,
        };

        // Initial order: mod, personal, game.
        let tree = Tree::discover(&d);
        let stack = DictStack::resolve(&tree, &mut cache);
        assert_eq!(stack.levels.len(), 3);
        let mut tm = mk();
        let mut origins = Vec::new();
        let r = stack.auto_translate_entries(&mut tm.entries, &mut origins);
        assert_eq!(
            r.per_level,
            vec![
                ("MonMod_ENG-FRA.ximdb".to_string(), 1),
                ("personal_ENG-FRA.ximdb".to_string(), 1),
                ("Skyrim_ENG-FRA.ximdb".to_string(), 1)
            ]
        );
        let t: Vec<&str> = tm.entries.iter().map(|e| e.trans.as_str()).collect();
        assert_eq!(t, vec!["mod-A", "perso-B", "jeu-C", "déjà"]);
        // The personal dictionary gives Personnalisé (D-13), the game Validé.
        assert_eq!(
            crate::status::derive(&tm.entries[1]),
            crate::status::Status::Custom
        );
        assert_eq!(
            crate::status::derive(&tm.entries[0]),
            crate::status::Status::Validated
        );
        assert_eq!(
            origins[0],
            (
                0,
                Origin {
                    file: "MonMod_ENG-FRA.ximdb".into(),
                    role: Role::Mod,
                    level: FillLevel::Exact
                }
            )
        );
        assert_eq!(origins[2].1.level, FillLevel::Text);
        let mut i18n = I18n::new();
        i18n.set_locale("fra");
        assert_eq!(stack.label(&i18n), "mod + perso + jeu");

        // The game above the personal one: B comes from the game.
        let mut tree = Tree::discover(&d);
        tree.merge_saved("mod;game;personal", "fra");
        let stack = DictStack::resolve(&tree, &mut cache);
        let mut tm = mk();
        stack.auto_translate_entries(&mut tm.entries, &mut Vec::new());
        assert_eq!(tm.entries[1].trans, "jeu-B");
        assert_eq!(cache.len(), 3, "each file is indexed once");

        // Unchecking the game: C stays empty.
        let mut tree = Tree::discover(&d);
        tree.merge_saved("mod;personal;game!", "fra");
        let stack = DictStack::resolve(&tree, &mut cache);
        let mut tm = mk();
        stack.auto_translate_entries(&mut tm.entries, &mut Vec::new());
        assert_eq!(tm.entries[2].trans, "");

        // No mod dictionary for another file.
        let d2 = Discover {
            doc_path: &root.join("Autre.esp"),
            ..d
        };
        let tree = Tree::discover(&d2);
        assert!(!tree.active().iter().any(|n| n.role == Role::Mod));
        let _ = std::fs::remove_dir_all(&root);
    }
}
