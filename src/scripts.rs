//! The scripts of a mod (C-06): finding the compiled scripts of a plugin
//! (the loose ones of `Scripts/` next to it, those of its archives), their
//! Papyrus sources, the lines of the « scripts » document, the locks of
//! the strings that are not texts, and the writing of the translated
//! scripts as loose files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::games::Game;
use crate::pex::{Pex, PexError};
use crate::script_rules::Rules;
use crate::status::{self, Status};
use crate::tm::{TmEntry, TmMeta, TranslationMemory};

/// The facts of the lines the locking rules need: `(script, index) →
/// (also an identifier, the calls the string is passed to)`.
pub type Facts = HashMap<(String, i32), (bool, Vec<String>)>;

/// The scripts rewritten: `(path under the export folder, bytes)`, or the
/// script that could not be written and why.
pub type Written = Result<Vec<(String, Vec<u8>)>, (String, PexError)>;

/// The record and field of a script line.
pub const REC: &str = "SCRIPT";
pub const FIELD: &str = "STR";
/// `source_format` of a scripts document.
pub const FORMAT: &str = "pex";

/// Where a file comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A loose file.
    Loose(PathBuf),
    /// A file of an archive: the archive and the entry's path.
    Archive(PathBuf, String),
}

impl Origin {
    /// Read the file.
    pub fn read(&self) -> Option<Vec<u8>> {
        match self {
            Origin::Loose(p) => std::fs::read(p).ok(),
            Origin::Archive(a, entry) => {
                let ar = crate::archive::Archive::open(a).ok()?;
                let i = ar
                    .entries()
                    .iter()
                    .position(|e| e.path.eq_ignore_ascii_case(entry))?;
                ar.extract(i).ok()
            }
        }
    }

    /// The archive's file name, or `None` for a loose file.
    pub fn archive_name(&self) -> Option<String> {
        match self {
            Origin::Archive(a, _) => a.file_name().map(|n| n.to_string_lossy().into_owned()),
            Origin::Loose(_) => None,
        }
    }
}

/// One compiled script.
#[derive(Debug, Clone)]
pub struct Script {
    /// The script's name: its object (`CMRQL:Fragments:Terminals:TERM__…`),
    /// else the file name without extension.
    pub name: String,
    /// The path under the data folder, `/`-separated, as found
    /// (`Scripts/MCM_skipper.pex`, `scripts/qf_x.pex`).
    pub rel: String,
    pub origin: Origin,
    pub bytes: Vec<u8>,
    pub pex: Result<Pex, PexError>,
    /// The Papyrus source found for it.
    pub source: Option<Origin>,
}

impl Script {
    /// The `.pex` path written under the export folder:
    /// `Scripts/<sub-folders>/<name>.pex`.
    pub fn out_rel(&self) -> String {
        let low = self.rel.to_lowercase();
        let under = match low.find("scripts/") {
            Some(i) => &self.rel[i + "scripts/".len()..],
            None => self.rel.rsplit('/').next().unwrap_or(&self.rel),
        };
        format!("Scripts/{under}")
    }
}

/// The scripts of a plugin, or one script opened alone.
#[derive(Debug, Clone, Default)]
pub struct ScriptSet {
    /// The plugin they belong to (`None` for a `.pex` opened alone).
    pub plugin: Option<PathBuf>,
    /// The folder searched (the plugin's).
    pub dir: PathBuf,
    /// The archives searched.
    pub archives: Vec<PathBuf>,
    pub scripts: Vec<Script>,
    /// The folder is a game's data folder: only the scripts the plugin
    /// names were kept.
    pub restricted: bool,
}

/// A folder named `name` (any case) in `dir`.
fn child_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
}

/// The files of `dir` and below with extension `ext`, skipping the
/// folders `skip` (any case).
fn walk(dir: &Path, ext: &str, skip: Option<&str>, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if skip.is_some_and(|s| name.eq_ignore_ascii_case(s)) {
                continue;
            }
            walk(&p, ext, skip, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case(ext))
        {
            out.push(p);
        }
    }
}

/// `a/b/c` of a path under `base`.
fn rel_of(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The key that matches a script and its source: the path under
/// `scripts/` (or under `source/`, `source/user/`, `source/scripts/` for a
/// source), without extension, lowercase.
fn match_key(rel: &str) -> String {
    let low = rel.to_lowercase().replace('\\', "/");
    let stem = low.rsplit_once('.').map(|(a, _)| a).unwrap_or(&low);
    let mut s = stem;
    // The last `source/` segment, then `user/` or `scripts/`.
    if let Some(i) = s.rfind("source/") {
        s = &s[i + "source/".len()..];
        if let Some(r) = s.strip_prefix("user/") {
            s = r;
        } else if let Some(r) = s.strip_prefix("scripts/") {
            s = r;
        }
    } else if let Some(i) = s.find("scripts/") {
        s = &s[i + "scripts/".len()..];
    }
    s.to_string()
}

/// The last component of a key.
fn stem_key(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// The sibling archives of a plugin: same rule as its STRINGS
/// (`<plugin>.bsa`, `<plugin> - Main.ba2`…).
pub fn archives_of(plugin: &Path) -> Vec<PathBuf> {
    let stem = plugin.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let stem_norm = crate::lang::normalize_name(stem);
    let dir = plugin.parent().unwrap_or_else(|| Path::new("."));
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let astem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            (ext == "bsa" || ext == "ba2")
                && crate::lang::normalize_name(astem).starts_with(&stem_norm)
        })
        .collect();
    out.sort();
    out
}

/// The scripts a plugin names in its VMAD blocks, lowercase.
pub fn plugin_script_names(plugin: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(bytes) = std::fs::read(plugin) else {
        return out;
    };
    let Ok(p) = crate::plugin::parse(&bytes) else {
        return out;
    };
    for r in &p.records {
        for s in r.subrecords.iter().filter(|s| s.sig == "VMAD") {
            for n in crate::plugin::vmad::script_names(&r.sig, &s.data) {
                if !n.is_empty() {
                    out.insert(n.to_lowercase().replace(':', "/"));
                }
            }
        }
    }
    out
}

impl ScriptSet {
    /// The scripts of a plugin: the loose `.pex` of `Scripts/` (sources
    /// folders aside), then those of its archives (a loose file wins over
    /// the archive's at equal path, as for the engine), and their sources.
    /// In a game's data folder (the game's master is there), only the
    /// scripts the plugin names are kept.
    pub fn discover(plugin: &Path, game: Game) -> ScriptSet {
        let dir = plugin
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut set = ScriptSet {
            plugin: Some(plugin.to_path_buf()),
            dir: dir.clone(),
            archives: archives_of(plugin),
            ..Default::default()
        };
        let masters = crate::games::registry().spec(game).masters.clone();
        set.restricted = masters
            .first()
            .is_some_and(|m| dir.join(m).is_file() && !plugin.ends_with(m));
        let wanted = set.restricted.then(|| plugin_script_names(plugin));
        let keep = |key: &str| -> bool {
            match &wanted {
                Some(w) => w.contains(key) || w.contains(stem_key(key)),
                None => true,
            }
        };
        let mut found: Vec<(String, String, Origin, Option<Vec<u8>>)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut sources: HashMap<String, Origin> = HashMap::new();
        // Loose.
        if let Some(sd) = child_dir(&dir, "scripts") {
            let mut files = Vec::new();
            walk(&sd, "pex", Some("source"), &mut files);
            files.sort();
            for f in files {
                let rel = rel_of(&f, &dir);
                let key = match_key(&rel);
                if keep(&key) && seen.insert(rel.to_lowercase()) {
                    found.push((rel, key, Origin::Loose(f), None));
                }
            }
            if let Some(src) = child_dir(&sd, "source") {
                let mut psc = Vec::new();
                walk(&src, "psc", None, &mut psc);
                for f in psc {
                    let key = match_key(&rel_of(&f, &dir));
                    sources.entry(key).or_insert(Origin::Loose(f));
                }
            }
        }
        if let Some(src) = child_dir(&dir, "source") {
            let mut psc = Vec::new();
            walk(&src, "psc", None, &mut psc);
            for f in psc {
                let key = match_key(&rel_of(&f, &dir));
                sources.entry(key).or_insert(Origin::Loose(f));
            }
        }
        // Archives.
        for a in &set.archives {
            let Ok(ar) = crate::archive::Archive::open(a) else {
                continue;
            };
            for (i, e) in ar.entries().iter().enumerate() {
                let low = e.path.to_lowercase();
                if low.ends_with(".psc") {
                    sources
                        .entry(match_key(&e.path))
                        .or_insert_with(|| Origin::Archive(a.clone(), e.path.clone()));
                } else if low.ends_with(".pex") {
                    let key = match_key(&e.path);
                    if keep(&key) && seen.insert(low.replace('\\', "/")) {
                        found.push((
                            e.path.replace('\\', "/"),
                            key,
                            Origin::Archive(a.clone(), e.path.clone()),
                            Some(ar.extract(i).unwrap_or_default()),
                        ));
                    }
                }
            }
        }
        // By file name too, for a source kept elsewhere than its script.
        let by_stem: HashMap<String, Origin> = sources
            .iter()
            .map(|(k, o)| (stem_key(k).to_string(), o.clone()))
            .collect();
        for (rel, key, origin, bytes) in found {
            let bytes = match (&origin, bytes) {
                (_, Some(b)) => b,
                (Origin::Loose(p), None) => std::fs::read(p).unwrap_or_default(),
                (Origin::Archive(..), None) => Vec::new(),
            };
            let source = sources
                .get(&key)
                .or_else(|| by_stem.get(stem_key(&key)))
                .cloned();
            set.scripts.push(Script::new(rel, origin, bytes, source));
        }
        set
    }

    /// One script opened alone (Fichier ▸ Ouvrir a `.pex`).
    pub fn single(path: &Path) -> std::io::Result<ScriptSet> {
        let bytes = std::fs::read(path)?;
        let dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        // Its source: next to it, or in a Source folder of the data folder.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut source = None;
        let mut cands = vec![dir.join(format!("{stem}.psc"))];
        for base in [
            dir.clone(),
            dir.parent().map(Path::to_path_buf).unwrap_or_default(),
        ] {
            if let Some(s) = child_dir(&base, "source") {
                let mut psc = Vec::new();
                walk(&s, "psc", None, &mut psc);
                cands.extend(psc);
            }
        }
        for c in cands {
            if c.is_file()
                && c.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(stem))
            {
                source = Some(Origin::Loose(c));
                break;
            }
        }
        Ok(ScriptSet {
            plugin: None,
            dir,
            archives: Vec::new(),
            scripts: vec![Script::new(
                name,
                Origin::Loose(path.to_path_buf()),
                bytes,
                source,
            )],
            restricted: false,
        })
    }

    /// The script of a line (by its EDID).
    pub fn script(&self, name: &str) -> Option<&Script> {
        self.scripts.iter().find(|s| s.name == name)
    }

    /// The lines of the document: per script, its literals in table order
    /// (an empty one aside), field `STR`, index = the string's index, EDID
    /// = the script; comment: the function of the first use, and the call
    /// it is an argument of.
    pub fn to_tm(&self, game: Game, plugin_name: &str) -> TranslationMemory {
        let utf8 = game.uses_utf8();
        let mut entries = Vec::new();
        for s in &self.scripts {
            let Ok(p) = &s.pex else {
                continue;
            };
            for i in p.literals() {
                let text = crate::tm::encoding::decode(&p.strings[i], utf8);
                if text.is_empty() {
                    continue;
                }
                let u = &p.uses[i][0];
                let comment = match &u.call {
                    Some(c) => crate::l10n!("script-comment-call", function = u.function, call = c),
                    None => crate::l10n!("script-comment", function = u.function),
                };
                entries.push(TmEntry {
                    rec_sig: REC.to_string(),
                    field_sig: FIELD.to_string(),
                    edid: s.name.clone(),
                    index: i as i32,
                    source: text,
                    comment,
                    idstexte: -1,
                    icon: -1,
                    ..Default::default()
                });
            }
        }
        let meta = TmMeta {
            game: game.id().to_string(),
            source_format: FORMAT.to_string(),
            plugin_name: plugin_name.to_string(),
            ..Default::default()
        };
        TranslationMemory::new(entries, meta)
    }

    /// The scripts with exported lines, rewritten: `(path under the export
    /// folder, bytes, original existed)`. A script without exported line is
    /// not written; one whose translations give back its own bytes neither.
    pub fn translated(&self, exported: &TranslationMemory, game: Game) -> Written {
        let utf8 = game.uses_utf8();
        let mut by_script: HashMap<&str, HashMap<usize, Vec<u8>>> = HashMap::new();
        for e in &exported.entries {
            if e.field_sig != FIELD || !e.is_translated() || e.index < 0 {
                continue;
            }
            by_script.entry(e.edid.as_str()).or_default().insert(
                e.index as usize,
                crate::tm::encoding::encode_capped(&e.trans, utf8, u16::MAX as usize),
            );
        }
        let mut out = Vec::new();
        for s in &self.scripts {
            let (Ok(p), Some(t)) = (&s.pex, by_script.get(s.name.as_str())) else {
                continue;
            };
            let bytes = crate::pex::write(&s.bytes, p, t).map_err(|e| (s.rel.clone(), e))?;
            if bytes != s.bytes {
                out.push((s.out_rel(), bytes));
            }
        }
        Ok(out)
    }

    /// The Papyrus sources of the scripts written, with the translations
    /// of their exported lines put in place of the literals (« Reporter
    /// aussi la traduction dans la source .psc », R-C07-04): `(path under
    /// the export folder, bytes)`, the source keeping its own place
    /// (`Scripts/Source/…`, `Source/Scripts/…`).
    pub fn translated_sources(
        &self,
        exported: &TranslationMemory,
        game: Game,
    ) -> Vec<(String, Vec<u8>)> {
        let utf8 = game.uses_utf8();
        let mut out = Vec::new();
        for s in &self.scripts {
            let (Some(src), Ok(_)) = (&s.source, &s.pex) else {
                continue;
            };
            let lines: Vec<&TmEntry> = exported
                .entries
                .iter()
                .filter(|e| e.edid == s.name && e.field_sig == FIELD && e.is_translated())
                .collect();
            if lines.is_empty() {
                continue;
            }
            let Some(bytes) = src.read() else {
                continue;
            };
            let mut text = crate::tm::encoding::decode(&bytes, utf8);
            let mut changed = false;
            for e in lines {
                let from = format!("\"{}\"", papyrus_escape(&e.source));
                let to = format!("\"{}\"", papyrus_escape(&e.trans));
                if text.contains(&from) {
                    text = text.replace(&from, &to);
                    changed = true;
                }
            }
            if !changed {
                continue;
            }
            let rel = match src {
                Origin::Loose(p) => p
                    .strip_prefix(&self.dir)
                    .map(|r| {
                        r.components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .unwrap_or_else(|_| {
                        format!(
                            "Scripts/Source/{}",
                            p.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        )
                    }),
                Origin::Archive(_, entry) => entry.replace('\\', "/"),
            };
            out.push((rel, crate::tm::encoding::encode(&text, utf8)));
        }
        out
    }

    /// The facts of each line the locking rules need:
    /// `(script, index) → (also an identifier, the calls it is passed to)`.
    pub fn facts(&self) -> Facts {
        let mut out = HashMap::new();
        for s in &self.scripts {
            let Ok(p) = &s.pex else {
                continue;
            };
            for i in p.literals() {
                let calls: Vec<String> = p.uses[i].iter().filter_map(|u| u.call.clone()).collect();
                out.insert((s.name.clone(), i as i32), (p.ident[i], calls));
            }
        }
        out
    }

    /// The text of a script's Papyrus source, if found.
    pub fn source_text(&self, name: &str, game: Game) -> Option<(String, String)> {
        let s = self.script(name)?;
        let src = s.source.as_ref()?;
        let bytes = src.read()?;
        let file = match src {
            Origin::Loose(p) => p.file_name()?.to_string_lossy().into_owned(),
            Origin::Archive(_, e) => e.rsplit('/').next().unwrap_or(e).to_string(),
        };
        Some((file, crate::tm::encoding::decode(&bytes, game.uses_utf8())))
    }
}

impl Script {
    fn new(rel: String, origin: Origin, bytes: Vec<u8>, source: Option<Origin>) -> Script {
        let pex = crate::pex::parse(&bytes);
        let name = match &pex {
            Ok(p) if !p.object.is_empty() => p.object.clone(),
            _ => rel
                .rsplit('/')
                .next()
                .unwrap_or(&rel)
                .rsplit_once('.')
                .map(|(a, _)| a.to_string())
                .unwrap_or_else(|| rel.clone()),
        };
        Script {
            name,
            rel,
            origin,
            bytes,
            pex,
            source,
        }
    }

    /// The format, `3.2 · Skyrim`.
    pub fn format(&self) -> Option<String> {
        let p = self.pex.as_ref().ok()?;
        let game = match p.game_id {
            1 => "Skyrim",
            2 => "Fallout 4",
            3 => "Fallout 76",
            4 => "Starfield",
            _ => "?",
        };
        Some(format!("{} · {game}", p.version()))
    }
}

/// The lock of a script line (C-06): `ident` (the string is also a name of
/// the script), `function:<call>` (an argument of a function whose string
/// arguments are identifiers), or the first pattern of the rules.
pub fn lock_of(source: &str, fact: Option<&(bool, Vec<String>)>, rules: &Rules) -> Option<String> {
    if let Some((ident, calls)) = fact {
        if *ident {
            return Some("ident".to_string());
        }
        for c in calls {
            let bare = c.rsplit('.').next().unwrap_or(c);
            if rules.locks_function(c) || rules.locks_function(bare) {
                return Some(format!("function:{c}"));
            }
        }
    }
    rules.lock_of(source).map(str::to_string)
}

/// Lift the rule locks of the script lines without translation before the
/// dictionaries fill them.
pub fn release(entries: &mut [TmEntry]) {
    for e in entries
        .iter_mut()
        .filter(|e| e.field_sig == FIELD && e.rec_sig == REC)
    {
        if e.trans.is_empty() && e.prov.starts_with(crate::vmad_lock::LOCK) {
            status::set(e, Status::ToTranslate);
            e.prov.clear();
        }
    }
}

/// Lay the rule locks on the script lines still À traduire without
/// translation (« jamais sur une ligne déjà traduite »). Returns the number
/// of lines locked.
pub fn apply(entries: &mut [TmEntry], facts: &Facts, rules: &Rules) -> usize {
    let mut n = 0;
    for e in entries
        .iter_mut()
        .filter(|e| e.field_sig == FIELD && e.rec_sig == REC)
    {
        if !(e.trans.is_empty() && e.prov.is_empty() && status::derive(e) == Status::ToTranslate) {
            continue;
        }
        let fact = facts.get(&(e.edid.clone(), e.index));
        if let Some(rule) = lock_of(&e.source, fact, rules) {
            status::set(e, Status::Ignored);
            e.prov = format!("{}{rule}", crate::vmad_lock::LOCK);
            n += 1;
        }
    }
    n
}

/// A text as Papyrus writes it between quotes.
pub fn papyrus_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_scripts_and_sources() {
        assert_eq!(match_key("Scripts/MCM_skipper.pex"), "mcm_skipper");
        assert_eq!(
            match_key("Scripts/CMRQL/UpdateOneDotSevenFixScript.pex"),
            "cmrql/updateonedotsevenfixscript"
        );
        assert_eq!(
            match_key("Scripts/Source/User/CMRQL/UpdateOneDotSevenFixScript.psc"),
            "cmrql/updateonedotsevenfixscript"
        );
        assert_eq!(match_key("source/scripts/QF_X_0100.psc"), "qf_x_0100");
        assert_eq!(match_key("scripts/source/Menu.psc"), "menu");
    }

    #[test]
    fn locks_of_the_rules() {
        let r = Rules::builtin();
        let none = (false, vec![]);
        assert_eq!(
            lock_of("$MyMod_Title", Some(&none), &r).as_deref(),
            Some("key")
        );
        assert_eq!(
            lock_of("MyMod.esp", Some(&none), &r).as_deref(),
            Some("path")
        );
        assert_eq!(lock_of("42", Some(&none), &r).as_deref(), Some("noletter"));
        assert_eq!(lock_of("Default", Some(&none), &r).as_deref(), Some("word"));
        let call = (false, vec!["Game.GetFormFromFile".to_string()]);
        assert_eq!(
            lock_of("My Mod Name", Some(&call), &r).as_deref(),
            Some("function:Game.GetFormFromFile")
        );
        let both = (true, vec![]);
        assert_eq!(
            lock_of("A real sentence.", Some(&both), &r).as_deref(),
            Some("ident")
        );
        assert_eq!(lock_of("A real sentence.", Some(&none), &r), None);
    }
}
