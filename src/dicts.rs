//! The dictionary tree of a document (B-02): the dictionaries that may fill
//! its lines, each with a role, a check box and a place; the order is the
//! priority — the first active dictionary that knows a line wins. The tree
//! is discovered from the folders, then merged with what the user saved for
//! the (game, pair) in `[Dictionaries] Dicts_<Game>_<SRC>-<DST>`.

use std::path::{Path, PathBuf};

use crate::doc_settings::DocSettings;

/// The roles, in their initial order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// The document's own dictionary (B-03).
    Mod,
    /// The user's personal dictionary of the pair (B-04).
    Personal,
    /// Text-only bases (B-05).
    TextOnly,
    /// The game's bases, `jeux/<jeu>`.
    Game,
    /// Other mods' dictionaries, `mods/<jeu>`, and what the user added.
    External,
    /// The timestamped copies of the mod's dictionary, off by default.
    Backups,
}

impl Role {
    pub const ALL: [Role; 6] = [
        Role::Mod,
        Role::Personal,
        Role::TextOnly,
        Role::Game,
        Role::External,
        Role::Backups,
    ];

    /// Token in the configuration.
    pub fn key(self) -> &'static str {
        match self {
            Role::Mod => "mod",
            Role::Personal => "personal",
            Role::TextOnly => "textonly",
            Role::Game => "game",
            Role::External => "external",
            Role::Backups => "backups",
        }
    }

    pub fn from_key(s: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.key() == s)
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Role::Mod => "dicts-role-mod",
            Role::Personal => "dicts-role-personal",
            Role::TextOnly => "dicts-role-textonly",
            Role::Game => "dicts-role-game",
            Role::External => "dicts-role-external",
            Role::Backups => "dicts-role-backups",
        }
    }

    /// The i18n key of the short label for the undo tooltip and the status
    /// line.
    pub fn short_key(self) -> &'static str {
        match self {
            Role::Mod => "dict-role-mod",
            Role::Personal => "dict-role-personal",
            Role::TextOnly => "dict-role-text",
            Role::Game => "dict-role-game",
            Role::External => "dict-role-external",
            Role::Backups => "dict-role-backups",
        }
    }

    /// Whether the role's files are checked when first seen.
    pub fn default_enabled(self) -> bool {
        self != Role::Backups
    }
}

/// One dictionary file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub role: Role,
    pub path: PathBuf,
    pub enabled: bool,
    /// Added by the user (outside the XIMOD folders): may be removed.
    pub added: bool,
}

impl Node {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string()
    }

    /// What identifies the node in the configuration: the file name inside
    /// a XIMOD folder, the full path otherwise.
    fn token_name(&self) -> String {
        if self.added {
            self.path.display().to_string()
        } else {
            self.name()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub role: Role,
    pub enabled: bool,
    pub nodes: Vec<Node>,
    /// Folders the user added (External): expanded at each discovery.
    pub added_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    pub groups: Vec<Group>,
}

/// What the discovery needs.
pub struct Discover<'a> {
    pub doc_path: &'a Path,
    pub settings: &'a DocSettings,
    /// `mods/<jeu>` root (without the game).
    pub mods_root: &'a Path,
    pub dict_dir: &'a Path,
    /// The game folders (`AppConfig::base_dirs_for`): `jeux/<jeu>`,
    /// `mods/<jeu>`, then the shipped copies in user mode.
    pub game_dirs: &'a [PathBuf],
}

/// Whether a `.ximdb` declares the target language (or none).
fn lang_matches(path: &Path, target: &str) -> bool {
    match crate::basedb::peek_lang(path) {
        Some(l) => l.is_empty() || l.eq_ignore_ascii_case(target),
        None => true,
    }
}

/// The dictionaries of one folder, sorted: the `.ximdb` of the target
/// language, or, without any `.ximdb`, the raw bases (EET, SST…).
pub fn dictionaries_in(dir: &Path, target: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let ximdb: Vec<PathBuf> = files
        .iter()
        .filter(|p| crate::basedb::is_ximdb_path(p))
        .cloned()
        .collect();
    if !ximdb.is_empty() {
        ximdb
            .into_iter()
            .filter(|p| lang_matches(p, target))
            .collect()
    } else {
        files
            .into_iter()
            .filter(|p| crate::basedb::is_raw_base_path(p))
            .collect()
    }
}

impl Tree {
    /// The tree as the folders give it: initial order, every file checked
    /// except the backups.
    pub fn discover(d: &Discover) -> Tree {
        let target = d.settings.target.trim();
        let mod_path =
            crate::dict_stack::DictStack::mod_dict_path(d.doc_path, d.settings, d.mods_root);
        let personal = crate::dict_stack::DictStack::personal_path(d.settings, d.dict_dir);
        let mut groups: Vec<Group> = Role::ALL
            .iter()
            .map(|&role| Group {
                role,
                enabled: true,
                nodes: Vec::new(),
                added_dirs: Vec::new(),
            })
            .collect();
        let push = |groups: &mut Vec<Group>, role: Role, path: PathBuf| {
            let g = groups.iter_mut().find(|g| g.role == role).unwrap();
            if !g.nodes.iter().any(|n| n.path == path) {
                g.nodes.push(Node {
                    role,
                    path,
                    enabled: role.default_enabled(),
                    added: false,
                });
            }
        };
        if mod_path.is_file() {
            push(&mut groups, Role::Mod, mod_path.clone());
        }
        if personal.is_file() {
            push(&mut groups, Role::Personal, personal);
        }
        // Game folders first, mods folders next (the order of `game_dirs`).
        let mods_sub = {
            let game_sub: String = d
                .settings
                .game
                .trim()
                .chars()
                .map(|c| if std::path::is_separator(c) { '_' } else { c })
                .collect();
            if game_sub.is_empty() {
                d.mods_root.to_path_buf()
            } else {
                d.mods_root.join(game_sub)
            }
        };
        for dir in d.game_dirs {
            let is_mods = dir.starts_with(d.mods_root)
                || dir.ends_with("mods")
                || dir.parent().is_some_and(|p| p.ends_with("mods"));
            let role = if is_mods { Role::External } else { Role::Game };
            for p in dictionaries_in(dir, target) {
                if p == mod_path {
                    continue;
                }
                if crate::basedb::is_text_only_hint(&p) {
                    push(&mut groups, Role::TextOnly, p);
                } else {
                    push(&mut groups, role, p);
                }
            }
        }
        // The copies of this mod's dictionary.
        let stem = mod_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let backups = mods_sub.join("backups");
        if !stem.is_empty() {
            let mut copies: Vec<PathBuf> = std::fs::read_dir(&backups)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file() && crate::basedb::is_ximdb_path(p))
                        .filter(|p| {
                            p.file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n.starts_with(&stem))
                        })
                        .collect()
                })
                .unwrap_or_default();
            copies.sort();
            copies.reverse();
            for p in copies {
                push(&mut groups, Role::Backups, p);
            }
        }
        if let Some(g) = groups.iter_mut().find(|g| g.role == Role::Backups) {
            g.enabled = false;
        }
        Tree { groups }
    }

    /// Merge what the user saved: order of the groups and of the files,
    /// cases, added files and folders. Unknown files are dropped, new ones
    /// keep their discovered place.
    pub fn merge_saved(&mut self, saved: &str, target: &str) {
        if saved.trim().is_empty() {
            return;
        }
        let mut order: Vec<Group> = Vec::new();
        for tok in saved.split(';').map(str::trim).filter(|t| !t.is_empty()) {
            if let Some((role, rest)) = tok.split_once(':') {
                let Some(role) = Role::from_key(role) else {
                    continue;
                };
                let (enabled, rest) = match rest.strip_prefix('!') {
                    Some(r) => (false, r),
                    None => (true, rest),
                };
                if let Some(dir) = rest.strip_prefix('+') {
                    // An added folder: remembered, and its files that the
                    // save does not name (new since) come in at the end.
                    let dir = PathBuf::from(dir);
                    let Some(g) = order.iter_mut().find(|g| g.role == role) else {
                        continue;
                    };
                    if !g.added_dirs.contains(&dir) {
                        g.added_dirs.push(dir.clone());
                    }
                    for p in dictionaries_in(&dir, target) {
                        if !g.nodes.iter().any(|n| n.path == p) {
                            g.nodes.push(Node {
                                role,
                                path: p,
                                enabled: true,
                                added: true,
                            });
                        }
                    }
                    continue;
                }
                let Some(g) = order.iter_mut().find(|g| g.role == role) else {
                    continue;
                };
                let abs = Path::new(rest).is_absolute();
                if abs {
                    // An added file, named by its full path.
                    if g.nodes.iter().any(|n| n.path == Path::new(rest)) {
                        continue;
                    }
                    if Path::new(rest).is_file() {
                        g.nodes.push(Node {
                            role,
                            path: PathBuf::from(rest),
                            enabled,
                            added: true,
                        });
                    }
                    continue;
                }
                // A discovered file, named by its file name.
                let found = self
                    .groups
                    .iter_mut()
                    .find(|dg| dg.role == role)
                    .and_then(|dg| {
                        let pos = dg.nodes.iter().position(|n| !n.added && n.name() == rest)?;
                        Some(dg.nodes.remove(pos))
                    });
                if let Some(mut n) = found {
                    n.enabled = enabled;
                    g.nodes.push(n);
                }
            } else {
                let (enabled, key) = match tok.strip_suffix('!') {
                    Some(k) => (false, k),
                    None => (true, tok),
                };
                let Some(role) = Role::from_key(key) else {
                    continue;
                };
                if order.iter().any(|g| g.role == role) {
                    continue;
                }
                order.push(Group {
                    role,
                    enabled,
                    nodes: Vec::new(),
                    added_dirs: Vec::new(),
                });
            }
        }
        // Groups the save did not mention keep their place; discovered files
        // not mentioned go after the mentioned ones of their group.
        for g in self.groups.drain(..) {
            match order.iter_mut().find(|o| o.role == g.role) {
                Some(o) => o.nodes.extend(g.nodes),
                None => order.push(g),
            }
        }
        // The roles missing from the save (older save) come in initial order.
        for role in Role::ALL {
            if !order.iter().any(|g| g.role == role) {
                order.push(Group {
                    role,
                    enabled: role.default_enabled(),
                    nodes: Vec::new(),
                    added_dirs: Vec::new(),
                });
            }
        }
        self.groups = order;
    }

    /// The configuration value (R-B02-08).
    pub fn to_config(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        for g in &self.groups {
            out.push(format!(
                "{}{}",
                g.role.key(),
                if g.enabled { "" } else { "!" }
            ));
            for n in &g.nodes {
                out.push(format!(
                    "{}:{}{}",
                    g.role.key(),
                    if n.enabled { "" } else { "!" },
                    n.token_name()
                ));
            }
            // The folders come last: their files are already named above,
            // the token only brings in what appeared since.
            for d in &g.added_dirs {
                out.push(format!("{}:+{}", g.role.key(), d.display()));
            }
        }
        out.join(";")
    }

    /// The active dictionaries, in priority order.
    pub fn active(&self) -> Vec<&Node> {
        self.groups
            .iter()
            .filter(|g| g.enabled)
            .flat_map(|g| g.nodes.iter().filter(|n| n.enabled))
            .collect()
    }

    pub fn group_mut(&mut self, role: Role) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.role == role)
    }

    /// Move a group up (`-1`) or down (`+1`).
    pub fn move_group(&mut self, role: Role, dir: i32) -> bool {
        let Some(i) = self.groups.iter().position(|g| g.role == role) else {
            return false;
        };
        let j = i as i32 + dir;
        if j < 0 || j as usize >= self.groups.len() {
            return false;
        }
        self.groups.swap(i, j as usize);
        true
    }

    /// Move a file inside its group.
    pub fn move_node(&mut self, role: Role, path: &Path, dir: i32) -> bool {
        let Some(g) = self.group_mut(role) else {
            return false;
        };
        let Some(i) = g.nodes.iter().position(|n| n.path == path) else {
            return false;
        };
        let j = i as i32 + dir;
        if j < 0 || j as usize >= g.nodes.len() {
            return false;
        }
        g.nodes.swap(i, j as usize);
        true
    }

    /// Add a file the user chose (External, or Text only when the file is
    /// one). Returns false when it is already there.
    pub fn add_file(&mut self, path: PathBuf) -> bool {
        if self
            .groups
            .iter()
            .any(|g| g.nodes.iter().any(|n| n.path == path))
        {
            return false;
        }
        let role = if crate::basedb::is_text_only_hint(&path) {
            Role::TextOnly
        } else {
            Role::External
        };
        if let Some(g) = self.group_mut(role) {
            g.nodes.push(Node {
                role,
                path,
                enabled: true,
                added: true,
            });
            true
        } else {
            false
        }
    }

    /// Add a folder: its dictionaries of the target language, as External.
    pub fn add_dir(&mut self, dir: PathBuf, target: &str) -> usize {
        let files = dictionaries_in(&dir, target);
        let Some(g) = self.group_mut(Role::External) else {
            return 0;
        };
        if !g.added_dirs.contains(&dir) {
            g.added_dirs.push(dir);
        }
        let mut n = 0;
        for p in files {
            if !g.nodes.iter().any(|x| x.path == p) {
                g.nodes.push(Node {
                    role: Role::External,
                    path: p,
                    enabled: true,
                    added: true,
                });
                n += 1;
            }
        }
        n
    }

    /// Remove an added file (or every file of an added folder).
    pub fn remove(&mut self, role: Role, path: &Path) -> bool {
        let Some(g) = self.group_mut(role) else {
            return false;
        };
        if let Some(i) = g.added_dirs.iter().position(|d| d == path) {
            g.added_dirs.remove(i);
            g.nodes.retain(|n| !n.path.starts_with(path));
            return true;
        }
        let before = g.nodes.len();
        g.nodes.retain(|n| !(n.added && n.path == path));
        g.nodes.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basedb::{BaseRow, write_ximdb};
    use crate::config::OutputMode;

    fn settings() -> DocSettings {
        DocSettings {
            game: "SkyrimSE".into(),
            source: "eng".into(),
            target: "fra".into(),
            output: Some(OutputMode::Strings),
            use_personal: true,
        }
    }

    fn row(src: &str, tr: &str) -> BaseRow {
        BaseRow {
            rec: "WEAP".into(),
            field: "FULL".into(),
            formid: 1,
            edid: "A".into(),
            source: src.into(),
            trans: tr.into(),
            status: None,
            ..Default::default()
        }
    }

    fn fixture() -> (PathBuf, Discover<'static>) {
        let root = std::env::temp_dir().join(format!(
            "ximod-dicts-{}-{}",
            std::process::id(),
            rand_suffix()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let jeux = root.join("jeux").join("SkyrimSE");
        let mods = root.join("mods").join("SkyrimSE");
        let dict = root.join("dict");
        std::fs::create_dir_all(&jeux).unwrap();
        std::fs::create_dir_all(mods.join("backups")).unwrap();
        std::fs::create_dir_all(&dict).unwrap();
        write_ximdb(
            vec![row("a", "jeu")],
            "fra",
            &jeux.join("Skyrim_ENG-FRA.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("a", "dlc")],
            "fra",
            &jeux.join("Dawnguard_ENG-FRA.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("a", "deu")],
            "deu",
            &jeux.join("Skyrim_ENG-DEU.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("a", "mod")],
            "fra",
            &mods.join("MonMod_ENG-FRA.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("a", "autre")],
            "fra",
            &mods.join("AutreMod_ENG-FRA.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("a", "old")],
            "fra",
            &mods
                .join("backups")
                .join("MonMod_ENG-FRA_2026-10-08_17-02.ximdb"),
        )
        .unwrap();
        write_ximdb(
            vec![row("a", "perso")],
            "fra",
            &dict.join("personal_ENG-FRA.ximdb"),
        )
        .unwrap();
        // Leaked on purpose: the test fixture lives for the process.
        let s: &'static DocSettings = Box::leak(Box::new(settings()));
        let doc: &'static Path = Box::leak(root.join("MonMod.esp").into_boxed_path());
        let mods_root: &'static Path = Box::leak(root.join("mods").into_boxed_path());
        let dict_dir: &'static Path = Box::leak(dict.into_boxed_path());
        let dirs: &'static [PathBuf] = Box::leak(vec![jeux, mods].into_boxed_slice());
        (
            root,
            Discover {
                doc_path: doc,
                settings: s,
                mods_root,
                dict_dir,
                game_dirs: dirs,
            },
        )
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as u64
    }

    fn names(t: &Tree) -> Vec<String> {
        t.active().iter().map(|n| n.name()).collect()
    }

    /// T-B02-01/02: discovery in the initial order, backups off, other
    /// languages left out; the save reorders, unchecks and round-trips.
    #[test]
    fn discover_merge_and_round_trip() {
        let (root, d) = fixture();
        let t = Tree::discover(&d);
        assert_eq!(
            names(&t),
            vec![
                "MonMod_ENG-FRA.ximdb",
                "personal_ENG-FRA.ximdb",
                "Dawnguard_ENG-FRA.ximdb",
                "Skyrim_ENG-FRA.ximdb",
                "AutreMod_ENG-FRA.ximdb"
            ]
        );
        let backups = t.groups.iter().find(|g| g.role == Role::Backups).unwrap();
        assert!(!backups.enabled && backups.nodes.len() == 1 && !backups.nodes[0].enabled);
        let cfg = t.to_config();
        assert!(cfg.starts_with("mod;mod:MonMod_ENG-FRA.ximdb;personal;personal:personal_ENG-FRA.ximdb;textonly;game;game:Dawnguard_ENG-FRA.ximdb;game:Skyrim_ENG-FRA.ximdb;external;external:AutreMod_ENG-FRA.ximdb;backups!;backups:!MonMod"), "{cfg}");

        // Saved: externals above the game, Skyrim before Dawnguard which is unchecked.
        let mut t2 = Tree::discover(&d);
        t2.merge_saved("mod;personal;external;external:AutreMod_ENG-FRA.ximdb;game;game:Skyrim_ENG-FRA.ximdb;game:!Dawnguard_ENG-FRA.ximdb", "fra");
        assert_eq!(
            names(&t2),
            vec![
                "MonMod_ENG-FRA.ximdb",
                "personal_ENG-FRA.ximdb",
                "AutreMod_ENG-FRA.ximdb",
                "Skyrim_ENG-FRA.ximdb"
            ]
        );
        let again = {
            let mut t3 = Tree::discover(&d);
            t3.merge_saved(&t2.to_config(), "fra");
            t3
        };
        assert_eq!(again, t2, "the configuration round-trips");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T-B02-03: an added folder brings its target-language files as
    /// External, added; one of them unchecked stays unchecked after a
    /// round-trip; a removed folder takes its files away.
    #[test]
    fn added_folder_and_file() {
        let (root, d) = fixture();
        let extra = root.join("extra");
        std::fs::create_dir_all(&extra).unwrap();
        write_ximdb(vec![row("a", "x")], "fra", &extra.join("X_ENG-FRA.ximdb")).unwrap();
        write_ximdb(vec![row("a", "y")], "fra", &extra.join("Y_ENG-FRA.ximdb")).unwrap();
        write_ximdb(vec![row("a", "z")], "deu", &extra.join("Z_ENG-DEU.ximdb")).unwrap();
        let mut t = Tree::discover(&d);
        assert_eq!(t.add_dir(extra.clone(), "fra"), 2);
        let g = t.group_mut(Role::External).unwrap();
        g.nodes
            .iter_mut()
            .find(|n| n.name() == "Y_ENG-FRA.ximdb")
            .unwrap()
            .enabled = false;
        let cfg = t.to_config();
        assert!(
            cfg.contains(&format!("external:+{}", extra.display())),
            "{cfg}"
        );
        assert!(
            cfg.contains(&format!(
                "external:!{}",
                extra.join("Y_ENG-FRA.ximdb").display()
            )),
            "{cfg}"
        );
        let mut t2 = Tree::discover(&d);
        t2.merge_saved(&cfg, "fra");
        assert_eq!(t2, t);
        assert!(t2.active().iter().any(|n| n.name() == "X_ENG-FRA.ximdb"));
        assert!(!t2.active().iter().any(|n| n.name() == "Y_ENG-FRA.ximdb"));
        assert!(t2.remove(Role::External, &extra));
        assert!(!t2.groups.iter().any(|g| g.nodes.iter().any(|n| n.added)));
        // A single added file.
        let single = root.join("single_ENG-FRA.ximdb");
        write_ximdb(vec![row("a", "s")], "fra", &single).unwrap();
        assert!(t2.add_file(single.clone()));
        assert!(!t2.add_file(single.clone()), "not twice");
        assert!(
            t2.to_config()
                .contains(&format!("external:{}", single.display()))
        );
        assert!(
            !t2.remove(Role::External, Path::new("AutreMod_ENG-FRA.ximdb")),
            "a folder file is not removable"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// T-B05-02: a text-only file in a XIMOD folder is classified under
    /// Texte seul, whatever the folder; an added one too.
    #[test]
    fn text_only_files_take_their_group() {
        let (root, d) = fixture();
        let unsigned = |s: &str, t: &str| BaseRow {
            rec: String::new(),
            field: String::new(),
            formid: 0,
            edid: String::new(),
            source: s.into(),
            trans: t.into(),
            status: None,
            ..Default::default()
        };
        write_ximdb(
            vec![unsigned("of Burning", "de feu")],
            "fra",
            &root
                .join("mods")
                .join("SkyrimSE")
                .join("fragments_ENG-FRA.ximdb"),
        )
        .unwrap();
        let mut t = Tree::discover(&d);
        let g = t.groups.iter().find(|g| g.role == Role::TextOnly).unwrap();
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.nodes[0].name(), "fragments_ENG-FRA.ximdb");
        assert!(
            !t.groups
                .iter()
                .find(|g| g.role == Role::External)
                .unwrap()
                .nodes
                .iter()
                .any(|n| n.name() == "fragments_ENG-FRA.ximdb")
        );
        let extra = root.join("more_ENG-FRA.ximdb");
        write_ximdb(vec![unsigned("Steel", "acier")], "fra", &extra).unwrap();
        assert!(t.add_file(extra));
        assert_eq!(
            t.groups
                .iter()
                .find(|g| g.role == Role::TextOnly)
                .unwrap()
                .nodes
                .len(),
            2
        );
        let cfg = t.to_config();
        assert!(cfg.contains("textonly:fragments_ENG-FRA.ximdb"), "{cfg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn moves_are_bounded() {
        let (root, d) = fixture();
        let mut t = Tree::discover(&d);
        assert!(!t.move_group(Role::Mod, -1));
        assert!(t.move_group(Role::Game, -1));
        assert_eq!(t.groups[2].role, Role::Game);
        let p = t.groups[2].nodes[1].path.clone();
        assert!(t.move_node(Role::Game, &p, -1));
        assert_eq!(t.groups[2].nodes[0].path, p);
        assert!(!t.move_node(Role::Game, &p, -1));
        let _ = std::fs::remove_dir_all(&root);
    }
}
