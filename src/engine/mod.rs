//! Plugin engines (S-05).
//!
//! An [`Engine`] is the only thing the rest of the application talks to when
//! it needs to *read* a game file into translatable lines, know which output
//! modes make sense, or label the grid columns. Two engines exist:
//! [`Tes4Engine`] (Oblivion → Starfield, localized plugins included) and
//! [`Tes3Engine`] (Morrowind). Their implementations are the existing modules
//! `plugin::reader`, `plugin::tes3` and `plugin::write`; this module is the
//! seam that keeps format knowledge out of the UI and of the pivot model.
//!
//! The engine of a *file* is decided by its magic ([`for_data`]); the engine
//! of a *game* by its sheet ([`for_game`], `GameSpec::engine`).

use crate::games::{EngineKind, Game, GameSpec, OutputModeId, registry};
use crate::tm::TranslationMemory;
use anyhow::Result;

/// i18n keys of the three engine-dependent grid columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnLabels {
    /// Record group / type column (GRUP for TES4, record type for TES3).
    pub group: &'static str,
    /// Field column.
    pub field: &'static str,
    /// Identifier column (EDID for TES4, NAME / INAM for TES3).
    pub id: &'static str,
}

pub trait Engine: Send + Sync {
    fn kind(&self) -> EngineKind;

    /// Column labels (i18n keys) for this engine's files.
    fn columns(&self) -> ColumnLabels;

    /// Read a whole file (already in memory or memory-mapped) into lines.
    fn read(&self, data: &[u8], plugin_name: &str) -> Result<TranslationMemory>;

    /// [`read`](Self::read) with the game imposed rather than detected
    /// (E-12); engines with a single game ignore it.
    fn read_as(
        &self,
        data: &[u8],
        plugin_name: &str,
        game: Option<Game>,
    ) -> Result<TranslationMemory> {
        let _ = game;
        self.read(data, plugin_name)
    }

    /// Whether an output mode can be produced for a file of this engine, given
    /// the game sheet and whether the source plugin is localized.
    fn supports(&self, mode: OutputModeId, spec: &GameSpec, localized: bool) -> bool;
}

/// The TES4 family: Oblivion, Skyrim LE / SE, Fallout 3 / NV / 4 / 76, Starfield.
pub struct Tes4Engine;

impl Engine for Tes4Engine {
    fn kind(&self) -> EngineKind {
        EngineKind::Tes4
    }

    fn columns(&self) -> ColumnLabels {
        ColumnLabels {
            group: "col-grup",
            field: "col-field",
            id: "col-edid",
        }
    }

    fn read(&self, data: &[u8], plugin_name: &str) -> Result<TranslationMemory> {
        crate::plugin::reader::extract_streaming(data, plugin_name)
    }

    fn read_as(
        &self,
        data: &[u8],
        plugin_name: &str,
        game: Option<Game>,
    ) -> Result<TranslationMemory> {
        crate::plugin::reader::extract_streaming_as(data, plugin_name, game)
    }

    fn supports(&self, mode: OutputModeId, spec: &GameSpec, localized: bool) -> bool {
        if !spec.output_modes.contains(&mode) {
            return false;
        }
        match mode {
            // STRINGS need string ids, i.e. a localized source.
            OutputModeId::Strings | OutputModeId::Hybrid => spec.localized_strings && localized,
            // The override patch of a localized plugin is written delocalized
            // (S-01), like the full plugin: both need the resolved STRINGS.
            OutputModeId::OverridePatch | OutputModeId::FullPlugin | OutputModeId::Dsd => true,
        }
    }
}

/// Morrowind (TES3 container).
pub struct Tes3Engine;

impl Engine for Tes3Engine {
    fn kind(&self) -> EngineKind {
        EngineKind::Tes3
    }

    fn columns(&self) -> ColumnLabels {
        ColumnLabels {
            group: "col-grup-tes3",
            field: "col-field",
            id: "col-edid-tes3",
        }
    }

    fn read(&self, data: &[u8], plugin_name: &str) -> Result<TranslationMemory> {
        let plugin = crate::plugin::tes3::parse_tes3(data)?;
        Ok(crate::plugin::tes3::extract_tes3(&plugin, plugin_name))
    }

    fn supports(&self, mode: OutputModeId, spec: &GameSpec, _localized: bool) -> bool {
        spec.output_modes.contains(&mode)
    }
}

/// Key / value text files (C-05): the lines come from `textfmt`, the
/// columns are Format / Champ / Clé, and the only output is the text file.
pub struct TextEngine;

impl Engine for TextEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Text
    }

    fn columns(&self) -> ColumnLabels {
        ColumnLabels {
            group: "col-format",
            field: "col-field",
            id: "col-key",
        }
    }

    /// The file in the first shipped format whose name patterns match
    /// (the application asks when they do not decide).
    fn read(&self, data: &[u8], plugin_name: &str) -> Result<TranslationMemory> {
        let formats = crate::textfmt::Formats::load();
        let path = std::path::Path::new(plugin_name);
        let cands = formats.candidates(path, data);
        let idx = formats
            .pick(&cands)
            .or_else(|| cands.first().map(|c| c.index))
            .ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-no-text-format")))?;
        let fmt = &formats.formats[idx];
        let doc = crate::textfmt::TextDoc::parse(data, fmt)?;
        Ok(doc.to_tm(fmt, plugin_name, ""))
    }

    fn supports(&self, mode: OutputModeId, _spec: &GameSpec, _localized: bool) -> bool {
        let _ = mode;
        false
    }
}

static TES4: Tes4Engine = Tes4Engine;
static TES3: Tes3Engine = Tes3Engine;
static TEXT: TextEngine = TextEngine;

/// Engine for a raw file, from its magic (`TES3` → Morrowind, else TES4).
pub fn for_data(data: &[u8]) -> &'static dyn Engine {
    if data.len() >= 4 && &data[0..4] == b"TES3" {
        &TES3
    } else {
        &TES4
    }
}

/// Engine of a game, from its sheet.
pub fn for_game(game: Game) -> &'static dyn Engine {
    match registry().spec(game).engine {
        EngineKind::Tes3 => &TES3,
        EngineKind::Tes4 => &TES4,
        EngineKind::Text => &TEXT,
    }
}

/// Engine of a translation memory: the text engine for a text document
/// (C-05), else the game recorded in its metadata (TES4 when unknown).
pub fn for_tm(tm: &TranslationMemory) -> &'static dyn Engine {
    if tm.meta.source_format == "text" {
        return &TEXT;
    }
    for_game(Game::from_id(&tm.meta.game))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_routes_to_engine() {
        assert_eq!(for_data(b"TES3....").kind(), EngineKind::Tes3);
        assert_eq!(for_data(b"TES4....").kind(), EngineKind::Tes4);
        assert_eq!(for_game(Game::Morrowind).kind(), EngineKind::Tes3);
        assert_eq!(for_game(Game::Starfield).kind(), EngineKind::Tes4);
    }

    #[test]
    fn supports_follows_sheet_and_localization() {
        let sse = GameSpec::builtin(Game::SkyrimSE);
        let e = for_game(Game::SkyrimSE);
        assert!(e.supports(OutputModeId::Strings, &sse, true));
        assert!(!e.supports(OutputModeId::Strings, &sse, false));
        assert!(e.supports(OutputModeId::Dsd, &sse, false));
        assert!(e.supports(OutputModeId::OverridePatch, &sse, true)); // delocalized patch (S-01)
        let obl = GameSpec::builtin(Game::Oblivion);
        assert!(!for_game(Game::Oblivion).supports(OutputModeId::Dsd, &obl, false));
        let mw = GameSpec::builtin(Game::Morrowind);
        assert!(for_game(Game::Morrowind).supports(OutputModeId::FullPlugin, &mw, false));
        assert!(!for_game(Game::Morrowind).supports(OutputModeId::OverridePatch, &mw, false));
    }
}
