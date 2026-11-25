//! Configurable keyboard shortcuts (E-15, D-123).
//!
//! Every command that has a shortcut has a stable identifier
//! (`transform_upper`, `journal`…), a place (the main window, the grid, the
//! edit window, the FOMOD window) and its shortcut of origin — those of the
//! manual's appendix. `[Shortcuts]` of Config.ini keeps the changes only:
//! `transform_upper=Ctrl+Alt+U` for a shortcut changed, `journal=none` for
//! one removed; a command absent keeps its origin. The table in force is
//! installed once at launch (and again when the Paramètres are saved) and
//! read by the menus, the tooltips and the handlers through [`get`].
//!
//! The text form is the same on every system: `Ctrl` is the command key (⌘
//! on macOS), then `Shift`, `Alt`, and egui's name of the key (`U`, `F5`,
//! `ArrowDown`, `Enter`, `Comma`).

use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

/// Where a command answers. The grid and the edit window are inside the main
/// window: their shortcuts must differ from the main window's (R-E15-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    Main,
    Grid,
    Edit,
    Fomod,
}

impl Place {
    /// The message key of its name.
    pub fn label_key(self) -> &'static str {
        match self {
            Place::Main => "sc-place-main",
            Place::Grid => "sc-place-grid",
            Place::Edit => "sc-place-edit",
            Place::Fomod => "sc-place-fomod",
        }
    }

    /// Whether a shortcut of `self` and one of `other` can both answer the
    /// same key press: same place, or the main window and a place inside
    /// it, or the grid and the edit window (which takes the grid's status
    /// keys). The FOMOD window is a window of its own.
    pub fn overlaps(self, other: Place) -> bool {
        use Place::*;
        match (self, other) {
            (Fomod, Fomod) => true,
            (Fomod, _) | (_, Fomod) => false,
            _ => true,
        }
    }
}

macro_rules! commands {
    ($( $v:ident => $id:literal, $place:ident, $mods:expr, $key:expr; )*) => {
        /// A command with a shortcut.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Cmd { $( $v, )* }

        impl Cmd {
            pub const ALL: &'static [Cmd] = &[ $( Cmd::$v, )* ];

            /// Its stable identifier (the key in `[Shortcuts]`).
            pub fn id(self) -> &'static str {
                match self { $( Cmd::$v => $id, )* }
            }

            pub fn place(self) -> Place {
                match self { $( Cmd::$v => Place::$place, )* }
            }

            /// Its shortcut of origin.
            pub fn origin(self) -> Option<KeyboardShortcut> {
                match self { $( Cmd::$v => Some(KeyboardShortcut::new($mods, $key)), )* }
            }
        }
    };
}

const C: Modifiers = Modifiers::COMMAND;
const CS: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const CA: Modifiers = Modifiers::COMMAND.plus(Modifiers::ALT);
const N: Modifiers = Modifiers::NONE;

commands! {
    // The main window (menus, global handler).
    Open => "open", Main, C, Key::O;
    SaveAs => "save_as", Main, C, Key::S;
    Export => "export", Main, CS, Key::E;
    CloseDoc => "close_doc", Main, C, Key::W;
    Quit => "quit", Main, C, Key::Q;
    Settings => "settings", Main, C, Key::Comma;
    Manual => "manual", Main, N, Key::F1;
    Journal => "journal", Main, CS, Key::J;
    Replace => "replace", Main, C, Key::H;
    Quality => "quality", Main, N, Key::F12;
    Dialogues => "dialogues", Main, N, Key::F10;
    OnlineBatch => "online_batch", Main, CS, Key::G;
    Search => "search", Main, CA, Key::F;
    FilterFocus => "filter_focus", Main, C, Key::F;
    ColumnFilters => "column_filters", Main, CS, Key::F;
    // The grid, and the edit window for the same commands on its line.
    Validate => "status_validated", Grid, N, Key::F5;
    Pending => "status_pending", Grid, N, Key::F6;
    Custom => "status_custom", Grid, N, Key::F7;
    Ignored => "status_ignored", Grid, N, Key::F8;
    ValidateNext => "validate_next", Grid, C, Key::ArrowDown;
    ValidatePrev => "validate_prev", Grid, C, Key::ArrowUp;
    SameAsSource => "same_as_source", Grid, C, Key::F8;
    ApplySame => "apply_same_source", Grid, C, Key::F6;
    HistoryPrev => "history_previous", Grid, CA, Key::Z;
    ReviewAccept => "review_accept", Grid, CS, Key::R;
    OnlineLine => "online_line", Grid, C, Key::G;
    Concord => "concordance", Grid, C, Key::K;
    Personal => "personal_dict", Grid, C, Key::M;
    TransformUpper => "transform_upper", Grid, CS, Key::U;
    TransformLower => "transform_lower", Grid, CS, Key::L;
    TransformTitle => "transform_title", Grid, CS, Key::T;
    // The edit window only.
    EditValidateNext => "edit_validate_next", Edit, C, Key::Enter;
    InstantTranslate => "instant", Edit, N, Key::F9;
    InstantCursor => "instant_cursor", Edit, C, Key::F9;
    GlossaryAdd => "glossary_add", Edit, CA, Key::G;
    // The FOMOD window (F-03).
    FomodSave => "fomod_save", Fomod, C, Key::S;
    FomodNext => "fomod_next", Fomod, C, Key::Enter;
    FomodOnline => "fomod_online", Fomod, C, Key::G;
}

impl Cmd {
    /// The message key of its name.
    pub fn label_key(self) -> String {
        format!("sc-{}", self.id().replace('_', "-"))
    }

    pub fn from_id(id: &str) -> Option<Cmd> {
        Cmd::ALL.iter().copied().find(|c| c.id() == id.trim())
    }
}

/// The text of a shortcut in `[Shortcuts]` (« Ctrl+Alt+U »).
pub fn to_text(sc: &KeyboardShortcut) -> String {
    let m = sc.modifiers;
    let mut parts: Vec<&str> = Vec::new();
    if m.command || m.ctrl || m.mac_cmd {
        parts.push("Ctrl");
    }
    if m.shift {
        parts.push("Shift");
    }
    if m.alt {
        parts.push("Alt");
    }
    parts.push(sc.logical_key.name());
    parts.join("+")
}

/// A value of `[Shortcuts]`: `Some(None)` for « none », `None` when it
/// cannot be read (the origin then stays).
pub fn parse(text: &str) -> Option<Option<KeyboardShortcut>> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in t.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => mods = mods.plus(Modifiers::COMMAND),
            "shift" | "maj" => mods = mods.plus(Modifiers::SHIFT),
            "alt" => mods = mods.plus(Modifiers::ALT),
            "" => return None,
            _ => key = Some(Key::from_name(part)?),
        }
    }
    Some(Some(KeyboardShortcut::new(mods, key?)))
}

/// Why a shortcut cannot be given to a command (R-E15-02 / 04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Copy, paste, cut, select all, undo, redo, or Entrée, Échap, Tab, an
    /// arrow alone: they keep their meaning in the text fields.
    Reserved,
    /// A key without Ctrl or Alt: only F1 to F12 are accepted alone.
    NeedsModifier,
}

fn is_function_key(k: Key) -> bool {
    use Key::*;
    matches!(
        k,
        F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12
    )
}

/// Whether `sc` may be given to a command.
pub fn check(sc: &KeyboardShortcut) -> Result<(), Refusal> {
    use Key::*;
    let m = sc.modifiers;
    let cmd = m.command || m.ctrl || m.mac_cmd;
    if cmd && !m.shift && !m.alt && matches!(sc.logical_key, C | V | X | A | Z | Y) {
        return Err(Refusal::Reserved);
    }
    // Ctrl+Maj+Z is « Rétablir » too.
    if cmd && m.shift && !m.alt && sc.logical_key == Z {
        return Err(Refusal::Reserved);
    }
    let plain = !cmd && !m.alt;
    if plain
        && matches!(
            sc.logical_key,
            Enter | Escape | Tab | ArrowUp | ArrowDown | ArrowLeft | ArrowRight
        )
    {
        return Err(Refusal::Reserved);
    }
    if plain && !is_function_key(sc.logical_key) {
        return Err(Refusal::NeedsModifier);
    }
    Ok(())
}

/// Two shortcuts that answer the same key press.
fn same(a: &KeyboardShortcut, b: &KeyboardShortcut) -> bool {
    let norm = |m: Modifiers| (m.command || m.ctrl || m.mac_cmd, m.shift, m.alt);
    a.logical_key == b.logical_key && norm(a.modifiers) == norm(b.modifiers)
}

/// The shortcuts in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    map: HashMap<Cmd, Option<KeyboardShortcut>>,
}

impl Default for Table {
    fn default() -> Self {
        Table {
            map: Cmd::ALL.iter().map(|c| (*c, c.origin())).collect(),
        }
    }
}

impl Table {
    /// The origins with the changes of `[Shortcuts]`; an unknown command or
    /// an unreadable value is ignored.
    pub fn with_overrides(overrides: &BTreeMap<String, String>) -> Table {
        let mut t = Table::default();
        for (id, v) in overrides {
            if let (Some(c), Some(sc)) = (Cmd::from_id(id), parse(v)) {
                t.map.insert(c, sc);
            }
        }
        t
    }

    /// What `[Shortcuts]` keeps: the commands whose shortcut is not their
    /// origin.
    pub fn overrides(&self) -> BTreeMap<String, String> {
        Cmd::ALL
            .iter()
            .filter(|c| !self.is_origin(**c))
            .map(|c| {
                let v = self
                    .get(*c)
                    .map(|s| to_text(&s))
                    .unwrap_or_else(|| "none".into());
                (c.id().to_string(), v)
            })
            .collect()
    }

    pub fn get(&self, c: Cmd) -> Option<KeyboardShortcut> {
        self.map.get(&c).copied().flatten()
    }

    pub fn set(&mut self, c: Cmd, sc: Option<KeyboardShortcut>) {
        self.map.insert(c, sc);
    }

    /// Whether `c` has its shortcut of origin.
    pub fn is_origin(&self, c: Cmd) -> bool {
        match (self.get(c), c.origin()) {
            (None, None) => true,
            (Some(a), Some(b)) => same(&a, &b),
            _ => false,
        }
    }

    /// The command that already answers `sc` where `c` answers (R-E15-03).
    pub fn conflict(&self, c: Cmd, sc: &KeyboardShortcut) -> Option<Cmd> {
        Cmd::ALL.iter().copied().find(|o| {
            *o != c && o.place().overlaps(c.place()) && self.get(*o).is_some_and(|s| same(&s, sc))
        })
    }
}

static ACTIVE: RwLock<Option<Table>> = RwLock::new(None);

/// Put `t` in force (launch, Paramètres saved).
pub fn install(t: Table) {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(t);
}

/// The shortcut in force of `c` (`None`: removed).
pub fn get(c: Cmd) -> Option<KeyboardShortcut> {
    let g = ACTIVE.read().unwrap_or_else(|e| e.into_inner());
    match g.as_ref() {
        Some(t) => t.get(c),
        None => c.origin(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_origins_free_of_conflicts() {
        let mut ids: Vec<&str> = Cmd::ALL.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Cmd::ALL.len());
        let t = Table::default();
        for c in Cmd::ALL {
            let sc = t.get(*c).unwrap();
            assert_eq!(t.conflict(*c, &sc), None, "{}", c.id());
            assert_eq!(Cmd::from_id(c.id()), Some(*c));
            // Every origin reads back from its text.
            assert_eq!(parse(&to_text(&sc)), Some(Some(sc)), "{}", c.id());
        }
        assert!(t.overrides().is_empty());
    }

    #[test]
    fn a_changed_shortcut_round_trips() {
        // T-E15-01 / 03 / 04.
        let mut t = Table::default();
        let alt_u = KeyboardShortcut::new(CA, Key::U);
        t.set(Cmd::TransformUpper, Some(alt_u));
        t.set(Cmd::Journal, None);
        let o = t.overrides();
        assert_eq!(
            o.get("transform_upper").map(String::as_str),
            Some("Ctrl+Alt+U")
        );
        assert_eq!(o.get("journal").map(String::as_str), Some("none"));
        assert_eq!(o.len(), 2);
        let back = Table::with_overrides(&o);
        assert_eq!(back, t);
        assert_eq!(back.get(Cmd::Journal), None);
        // Back to the origin: nothing kept.
        let mut t2 = back;
        t2.set(Cmd::TransformUpper, Cmd::TransformUpper.origin());
        t2.set(Cmd::Journal, Cmd::Journal.origin());
        assert!(t2.overrides().is_empty());
        // Unreadable values and unknown commands are ignored.
        let mut bad = BTreeMap::new();
        bad.insert("frobnicate".to_string(), "Ctrl+U".to_string());
        bad.insert("journal".to_string(), "Ctrl+Nope".to_string());
        assert_eq!(Table::with_overrides(&bad), Table::default());
    }

    #[test]
    fn conflicts_follow_the_places() {
        // T-E15-02: Ctrl+Maj+J for Concordance is the Journal's.
        let t = Table::default();
        let j = KeyboardShortcut::new(CS, Key::J);
        assert_eq!(t.conflict(Cmd::Concord, &j), Some(Cmd::Journal));
        // The FOMOD window is apart: Ctrl+S there is not « Enregistrer sous ».
        let s = KeyboardShortcut::new(C, Key::S);
        assert_eq!(t.conflict(Cmd::FomodSave, &s), None);
        assert_eq!(t.conflict(Cmd::Concord, &s), Some(Cmd::SaveAs));
        // T-E15-05: the three of the FOMOD window between themselves.
        let g = KeyboardShortcut::new(C, Key::G);
        assert_eq!(t.conflict(Cmd::FomodSave, &g), Some(Cmd::FomodOnline));
    }

    #[test]
    fn every_command_and_place_is_named_in_every_language() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales");
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let Ok(ftl) = std::fs::read_to_string(e.path().join("main.ftl")) else {
                continue;
            };
            n += 1;
            let has = |k: &str| ftl.lines().any(|l| l.starts_with(&format!("{k} = ")));
            for c in Cmd::ALL {
                assert!(has(&c.label_key()), "{} in {:?}", c.label_key(), e.path());
                assert!(has(c.place().label_key()), "{:?}", e.path());
            }
        }
        assert_eq!(n, 33);
    }

    #[test]
    fn reserved_combinations_are_refused() {
        // T-E15-03.
        for (m, k) in [
            (C, Key::C),
            (C, Key::V),
            (C, Key::X),
            (C, Key::A),
            (C, Key::Z),
            (C, Key::Y),
            (CS, Key::Z),
            (N, Key::Enter),
            (N, Key::Escape),
            (N, Key::Tab),
            (N, Key::ArrowDown),
        ] {
            assert_eq!(
                check(&KeyboardShortcut::new(m, k)),
                Err(Refusal::Reserved),
                "{k:?}"
            );
        }
        assert_eq!(
            check(&KeyboardShortcut::new(N, Key::U)),
            Err(Refusal::NeedsModifier)
        );
        assert_eq!(check(&KeyboardShortcut::new(N, Key::F11)), Ok(()));
        assert_eq!(check(&KeyboardShortcut::new(CA, Key::U)), Ok(()));
        assert_eq!(check(&KeyboardShortcut::new(C, Key::ArrowDown)), Ok(()));
        assert_eq!(
            parse("Maj+Ctrl+u"),
            Some(Some(KeyboardShortcut::new(CS, Key::U)))
        );
        assert_eq!(parse("none"), Some(None));
        assert_eq!(parse("Ctrl+"), None);
    }
}
