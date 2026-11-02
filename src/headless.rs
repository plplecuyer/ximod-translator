//! Opening a file without a window (D-07, D-06): what the application
//! does at the opening of a document — the lines read (plugin with its
//! STRINGS, text file in the format its name designates, translation
//! base), the dictionary tree of the document resolved as the interface
//! resolves it, the stack applied, the exclusion rules laid — so that the
//! analysis of a folder and the command line give what the interface would
//! give, without opening a tab nor writing anything.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::basedb::FileDbCache;
use crate::config::AppConfig;
use crate::dict_stack::{DictStack, StackReport};
use crate::dicts::{Discover, Role, Tree};
use crate::doc_settings::DocSettings;
use crate::exclusions::{Compiled, Deduced, Dirs, Plan};
use crate::games::Game;
use crate::status::{self, Status};
use crate::textfmt::{Formats, TextDoc, TextFormat};
use crate::tm::TranslationMemory;

/// What a file is for the opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A plugin (esp, esm, esl).
    Plugin,
    /// An interface or text file in a text format (C-05, D-08).
    Text,
    /// A DSD file (`SKSE/Plugins/DynamicStringDistributor/…/*.json`): a
    /// translation.
    Dsd,
    /// A compiled script (pex).
    Script,
    /// A dictionary the application wrote (ximdb, pivot): listed only.
    Dictionary,
}

/// A file read as the opening reads it.
pub struct Opened {
    pub path: PathBuf,
    pub kind: Kind,
    pub tm: TranslationMemory,
    pub settings: DocSettings,
    /// The text format of a text file.
    pub format: Option<TextFormat>,
    pub text: Option<TextDoc>,
    /// The STRINGS of a localized plugin, for the writers (D-06).
    pub strings: Option<crate::plugin::StringsBundle>,
}

/// The kind of a file by its place and name, `None` for what the opening
/// does not read (archives, STRINGS, images…).
pub fn kind_of(path: &Path, formats: &Formats) -> Option<Kind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if crate::games::registry().is_plugin_extension(&ext) {
        return Some(Kind::Plugin);
    }
    match ext.as_str() {
        "pex" => return Some(Kind::Script),
        "ximdb" => return Some(Kind::Dictionary),
        "bsa" | "ba2" | "zip" | "7z" | "rar" | "strings" | "dlstrings" | "ilstrings" => {
            return None;
        }
        _ => {}
    }
    let full = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if ext == "json" && full.contains("/skse/plugins/dynamicstringdistributor/") {
        return Some(Kind::Dsd);
    }
    // A text file a precise pattern takes (R-D08-06).
    let cands = formats.name_candidates(path);
    if formats.pick_for(path, &cands).is_some() {
        return Some(Kind::Text);
    }
    None
}

/// The STRINGS of a localized plugin, next to it or in its archives;
/// the number of strings resolved.
pub fn resolve_strings(
    path: &Path,
    tm: &mut TranslationMemory,
    source: &str,
) -> (usize, Option<crate::plugin::StringsBundle>) {
    let utf8 = Game::from_id(&tm.meta.game).uses_utf8();
    if let Ok(bundle) = crate::plugin::StringsBundle::from_plugin_path(path, source, utf8) {
        let n = bundle.resolve(tm);
        if n > 0 {
            return (n, Some(bundle));
        }
    }
    if let Ok(bundle) = crate::plugin::StringsBundle::from_archives(path, source, utf8) {
        let n = bundle.resolve(tm);
        if n > 0 {
            return (n, Some(bundle));
        }
    }
    (0, None)
}

/// Read a file as the opening does: the game detected in the file, else
/// `game`; the languages `source` / `target`.
pub fn read(
    path: &Path,
    kind: Kind,
    game: &str,
    source: &str,
    target: &str,
    cfg: &AppConfig,
    formats: &Formats,
) -> anyhow::Result<Opened> {
    let mut settings = DocSettings::defaults(cfg);
    settings.source = source.to_string();
    settings.target = target.to_string();
    settings.game = game.to_string();
    let mut strings = None;
    let (tm, format, text) = match kind {
        Kind::Plugin => {
            let hint = Game::from_id(game);
            let hint = (hint != Game::Unknown).then_some(hint);
            let mut tm = crate::plugin::read_file_as(path, hint)?;
            if !tm.meta.game.trim().is_empty() {
                settings.game = tm.meta.game.clone();
            }
            if tm.entries.iter().any(|e| e.idstexte >= 0) {
                strings = resolve_strings(path, &mut tm, source).1;
            }
            (tm, None, None)
        }
        Kind::Text => {
            let data = std::fs::read(path)?;
            let cands = formats.name_candidates(path);
            let idx = formats
                .pick_for(path, &cands)
                .ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-no-text-format")))?;
            let fmt = formats.formats[idx].clone();
            let mut doc = TextDoc::parse(&data, &fmt)?;
            doc.origin = crate::textfmt::stamp(path);
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let tm = doc.to_tm(&fmt, &name, &settings.game);
            (tm, Some(fmt), Some(doc))
        }
        Kind::Dsd | Kind::Dictionary => {
            let tm = if crate::basedb::is_ximdb_path(path) {
                crate::basedb::ximdb_to_tm(path).map_err(|e| anyhow::anyhow!(e))?
            } else {
                crate::tm::load(path)?
            };
            (tm, None, None)
        }
        Kind::Script => {
            let set = crate::scripts::ScriptSet::single(path)?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let tm = set.to_tm(Game::from_id(game), &name);
            (tm, None, None)
        }
    };
    Ok(Opened {
        path: path.to_path_buf(),
        kind,
        tm,
        settings,
        format,
        text,
        strings,
    })
}

/// The dictionary tree of a document (B-02, E-12): discovered from the
/// folders, merged with what the user saved for the game and pair, the
/// personal node following the document's switch.
pub fn dict_tree(cfg: &AppConfig, path: &Path, settings: &DocSettings) -> Tree {
    let mods_root = cfg.mod_base_dir();
    let dict_dir = crate::config::paths::get().dict_dir.clone();
    let game_dirs = cfg.base_dirs_for(&settings.game);
    let d = Discover {
        doc_path: path,
        settings,
        mods_root: &mods_root,
        dict_dir: &dict_dir,
        game_dirs: &game_dirs,
    };
    let mut tree = Tree::discover(&d);
    if let Some(saved) = cfg.dictionaries.get(&dicts_key(settings)) {
        tree.merge_saved(saved, settings.target.trim());
    }
    if let Some(g) = tree.group_mut(Role::Personal) {
        for n in &mut g.nodes {
            n.enabled = settings.use_personal;
        }
    }
    tree
}

/// `Dicts_<Game>_<SRC>-<DST>`'s key part.
pub fn dicts_key(settings: &DocSettings) -> String {
    let game: String = settings
        .game
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!(
        "{}_{}",
        if game.is_empty() {
            "any".to_string()
        } else {
            game
        },
        settings.pair_suffix()
    )
}

/// What the command line adds to the tree of a document (D-06, `apply
/// --dict / --base / --no-personal`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeExtra {
    /// Dictionaries added, as the user adds a file (External, or Text only).
    pub dicts: Vec<PathBuf>,
    /// Folders added: their dictionaries of the target language.
    pub bases: Vec<PathBuf>,
    pub no_personal: bool,
}

impl TreeExtra {
    pub fn add_to(&self, tree: &mut Tree, target: &str) {
        for d in &self.dicts {
            tree.add_file(d.clone());
        }
        for b in &self.bases {
            tree.add_dir(b.clone(), target);
        }
        if self.no_personal
            && let Some(g) = tree.group_mut(Role::Personal)
        {
            for n in &mut g.nodes {
                n.enabled = false;
            }
        }
    }
}

/// The stack of a document, its files read through `cache` (an index per
/// file, shared by a whole analysis).
pub fn stack(
    cfg: &AppConfig,
    path: &Path,
    settings: &DocSettings,
    cache: &mut FileDbCache,
) -> Option<DictStack> {
    stack_with(cfg, path, settings, cache, &TreeExtra::default())
}

/// [`stack`] with what the command line adds.
pub fn stack_with(
    cfg: &AppConfig,
    path: &Path,
    settings: &DocSettings,
    cache: &mut FileDbCache,
    extra: &TreeExtra,
) -> Option<DictStack> {
    if settings.target.trim().is_empty() {
        return None;
    }
    let mut tree = dict_tree(cfg, path, settings);
    extra.add_to(&mut tree, settings.target.trim());
    let mut s = DictStack::resolve(&tree, cache);
    s.normalize = cfg.normalize;
    Some(s)
}

/// What the mod's dictionary remembers of the file, as the opening takes
/// it back: the VMAD switch (C-08) and the rules learned and off (B-18).
pub fn remember(cfg: &AppConfig, o: &mut Opened) {
    if o.settings.target.trim().is_empty() {
        return;
    }
    let dict = DictStack::mod_dict_path(&o.path, &o.settings, &cfg.mod_base_dir());
    if dict.is_file() {
        let extra = crate::basedb::peek_extra(&dict);
        o.tm.meta.vmad = extra.vmad.unwrap_or(false);
        if let Some(r) = extra.rules {
            o.tm.meta.rules = r;
        }
        if let Some(r) = extra.rules_off {
            o.tm.meta.rules_off = r;
        }
    }
}

/// The exclusion rules of a document (B-18): the shipped and user rules,
/// and those its mod's dictionary remembers.
pub fn rules_plan(cfg: &AppConfig, o: &Opened) -> Plan {
    let file = Dirs::installed().load().0;
    let game = if o.settings.game.trim().is_empty() {
        o.tm.meta.game.clone()
    } else {
        o.settings.game.clone()
    };
    let mut local = o.tm.meta.rules.clone();
    let mut off: HashSet<String> = o.tm.meta.rules_off.iter().cloned().collect();
    let dict = DictStack::mod_dict_path(&o.path, &o.settings, &cfg.mod_base_dir());
    if dict.is_file() {
        let extra = crate::basedb::peek_extra(&dict);
        if let Some(r) = extra.rules {
            local = r;
        }
        if let Some(r) = extra.rules_off {
            off = r.into_iter().collect();
        }
    }
    let interface = if o.kind == Kind::Plugin {
        crate::textfmt::interface_files_of(&o.path, &o.settings.source, o.settings.game())
    } else {
        Vec::new()
    };
    Plan {
        compiled: Compiled::new(&file, &local, &game),
        deduced: Arc::new(Deduced::of(&o.tm.entries, Some(&o.path), &interface)),
        off,
        game,
        path: o.path.to_string_lossy().replace('\\', "/"),
        facts: None,
    }
}

/// What the stack and the rules did to a document's lines.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub report: StackReport,
    /// Lines (shown in game) after the stack.
    pub lines: usize,
    /// Translated by the stack.
    pub filled: usize,
    /// Still empty and not locked.
    pub left: usize,
    /// Locked by the rules, Ignoré.
    pub locked: usize,
}

/// Apply the stack (lifting the rules around it, laying them after, as at
/// the opening) on the lines of `o`, in memory only.
pub fn apply(o: &mut Opened, stack: Option<&DictStack>, plan: &Plan) -> Applied {
    let vmad_on = o.tm.meta.vmad;
    let es = &mut o.tm.entries;
    // The VMAD lines (C-08) and the exclusion rules (B-18): lifted around
    // the filling, laid after it.
    crate::vmad_lock::release(es);
    plan.release(es);
    let report = match stack {
        Some(s) => s.auto_translate_entries(es, &mut Vec::new()),
        None => StackReport::default(),
    };
    plan.lay(es);
    crate::vmad_lock::lay_file(es, vmad_on);
    let mut a = Applied {
        filled: report.total(),
        report,
        ..Default::default()
    };
    for e in es.iter() {
        if e.unused || e.source.trim().is_empty() {
            continue;
        }
        a.lines += 1;
        if status::derive(e) == Status::Ignored {
            a.locked += 1;
        } else if e.trans.trim().is_empty() {
            a.left += 1;
        }
    }
    a
}
