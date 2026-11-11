//! Dynamic font loading.
//!
//! XIMOD ships ~67 distinct Regular fonts (Noto family) covering every writing
//! system of the language table. Loading them all at once would waste a lot of
//! memory, so instead only the fonts actually needed are installed:
//!
//! - the font of the current interface language, and
//! - the fonts of every language listed for the selected country (that is what
//!   the language drop-down displays).
//!
//! In practice this is a single font for most countries (median 1, maximum 28
//! for India), so the cost stays small.
//!
//! The loaded fonts are appended *after* egui's built-in ones: Latin text keeps
//! using the default typeface, and any glyph missing from it falls through to
//! our fonts. That is what makes scripts such as Nyiakeng Puachue Hmong render
//! instead of showing tofu boxes (□).

use eframe::egui;
use std::path::PathBuf;

/// Locate the fonts directory (`assets/fonts`), production layout first.
pub fn fonts_dir() -> Option<PathBuf> {
    let rel = PathBuf::from("assets").join("fonts");
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&rel));
        if let Some(up) = dir.parent() {
            candidates.push(up.join("Resources").join(&rel));
        }
    }
    candidates.push(rel);

    candidates.into_iter().find(|p| p.is_dir())
}

/// Locate the book-fonts directory (`assets/fonts/books`), production layout
/// first. This folder is meant to receive **user-supplied** game fonts (e.g.
/// extracted from the user's own copy of the game); XIMOD never ships them.
pub fn book_fonts_dir() -> Option<PathBuf> {
    let rel = PathBuf::from("assets").join("fonts").join("books");
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&rel));
        if let Some(up) = dir.parent() {
            candidates.push(up.join("Resources").join(&rel));
        }
    }
    candidates.push(rel);
    candidates.into_iter().find(|p| p.is_dir())
}

/// Mapping from Bethesda font faces to the egui font families registered from the
/// user's book fonts. Empty when no book font is installed.
#[derive(Debug, Default, Clone)]
pub struct BookFonts {
    /// `(normalized file stem, egui family name)` for each loaded font.
    faces: Vec<(String, String)>,
}

/// Normalize a face/stem for tolerant matching: ASCII alphanumerics, lowercased,
/// with a trailing `"font"` dropped (`"$HandwrittenFont"` → `"handwritten"`).
fn normalize_face(s: &str) -> String {
    let t: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    t.strip_suffix("font").map(|x| x.to_string()).unwrap_or(t)
}

impl BookFonts {
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// The egui family name to use for a Bethesda face (e.g. `"$DaedricFont"`),
    /// matched tolerantly against the installed font file names.
    pub fn family_for(&self, face: &str) -> Option<&str> {
        let n = normalize_face(face);
        if n.is_empty() {
            return None;
        }
        self.faces
            .iter()
            .find(|(stem, _)| stem.contains(&n) || n.contains(stem.as_str()))
            .map(|(_, fam)| fam.as_str())
    }
}

/// Register every `.ttf`/`.otf` in the book-fonts folder into `fonts` (each as a
/// named family falling back to the default proportional fonts) and return the
/// face→family map. No-op (empty map) when the folder is absent or empty.
pub fn load_book_fonts(fonts: &mut egui::FontDefinitions) -> BookFonts {
    let mut faces = Vec::new();
    let Some(dir) = book_fonts_dir() else {
        return BookFonts { faces };
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return BookFonts { faces };
    };
    let defaults = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    for p in files {
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "ttf" && ext != "otf" {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Book font {:?} could not be read: {}", p, e);
                continue;
            }
        };
        let key = format!("book:{stem}");
        let family = format!("book-{stem}");
        fonts
            .font_data
            .insert(key.clone(), egui::FontData::from_owned(bytes));
        let mut chain = vec![key];
        chain.extend(defaults.iter().cloned());
        fonts
            .families
            .insert(egui::FontFamily::Name(family.clone().into()), chain);
        let stem_norm: String = stem
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        faces.push((stem_norm, family));
    }
    if !faces.is_empty() {
        tracing::info!("Loaded {} book font(s) from {:?}", faces.len(), dir);
    }
    BookFonts { faces }
}

/// Build a `FontDefinitions` containing egui's defaults plus every font in
/// `rel_paths` (paths relative to `assets/fonts`), appended as fallbacks.
///
/// Unreadable or missing files are skipped, so a broken entry in
/// `Languages.json` degrades to tofu instead of crashing the application.
/// Named font family used by the translation editor to preview a language in
/// its own typeface, independently of the interface font.
pub const PREVIEW_FAMILY: &str = "ximod-preview";

/// Same as [`build_font_definitions`], plus an optional font registered under
/// [`PREVIEW_FAMILY`] so a single widget can be drawn with it.
pub fn build_font_definitions_with_preview(
    rel_paths: &[String],
    preview: Option<&str>,
) -> egui::FontDefinitions {
    let mut fonts = build_defs(rel_paths);

    // The preview family always falls back to the default proportional fonts so
    // Latin text and punctuation still render if the chosen font lacks a glyph.
    let mut chain: Vec<String> = Vec::new();
    if let Some(rel) = preview.filter(|s| !s.is_empty()) {
        let key = rel.to_string();
        if !fonts.font_data.contains_key(&key)
            && let Some(dir) = fonts_dir()
        {
            let path = dir.join(rel.replace('\\', "/"));
            match std::fs::read(&path) {
                Ok(bytes) => {
                    fonts
                        .font_data
                        .insert(key.clone(), egui::FontData::from_owned(bytes));
                }
                Err(e) => tracing::warn!("Preview font {:?}: {}", path, e),
            }
        }
        if fonts.font_data.contains_key(&key) {
            chain.push(key);
        }
    }
    if let Some(defaults) = fonts.families.get(&egui::FontFamily::Proportional) {
        chain.extend(defaults.iter().cloned());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name(PREVIEW_FAMILY.into()), chain);

    fonts
}

fn build_defs(rel_paths: &[String]) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    // The Phosphor icons (arrows, undo / redo…), as a fallback of every family.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let Some(dir) = fonts_dir() else {
        tracing::warn!("assets/fonts not found: falling back to built-in fonts");
        return fonts;
    };

    for rel in rel_paths {
        if rel.is_empty() {
            continue;
        }
        let path = dir.join(rel.replace('\\', "/"));
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("Font {:?} could not be read: {}", path, e);
                continue;
            }
        };

        // The relative path doubles as a unique key.
        let key = rel.clone();
        fonts
            .font_data
            .insert(key.clone(), egui::FontData::from_owned(bytes));

        // Append as a fallback to both families.
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(key.clone());
        }
    }

    fonts
}

/// Open a URL in the user's default browser (Google Translate with the
/// line, Google Fonts, a mail).
///
/// Through the system's own call (ShellExecute on Windows, `open` on macOS,
/// `xdg-open` and the like on Linux), so the whole address arrives: on
/// Windows, `cmd /C start` read each « & » of the query string as the end
/// of the command, and Google Translate opened without the text nor the
/// target language. Failure is not an error worth interrupting the user
/// for: it is logged and ignored, since the address is also shown in the
/// interface.
pub fn open_url(url: &str) {
    if let Err(e) = webbrowser::open(url) {
        tracing::warn!("Could not open {}: {}", url, e);
    }
}

/// Reveal a folder in the system file manager (used after building the
/// translation archive, so the user can attach it to their e-mail).
pub fn open_path(path: &std::path::Path) {
    let s = path.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(&s).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&s).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(&s).spawn();

    if let Err(e) = result {
        tracing::warn!("Could not open {}: {}", s, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_face_strips_font_suffix() {
        assert_eq!(normalize_face("$HandwrittenFont"), "handwritten");
        assert_eq!(normalize_face("SkyrimBooks"), "skyrimbooks");
        assert_eq!(normalize_face("$Daedric_Font"), "daedric");
    }

    #[test]
    fn face_mapping_is_tolerant() {
        let bf = BookFonts {
            faces: vec![
                ("handwritten".into(), "book-handwritten".into()),
                ("skyrimbooks".into(), "book-SkyrimBooks".into()),
                ("daedric".into(), "book-daedric".into()),
            ],
        };
        assert_eq!(bf.family_for("$HandwrittenFont"), Some("book-handwritten"));
        assert_eq!(bf.family_for("$SkyrimBooks"), Some("book-SkyrimBooks"));
        assert_eq!(bf.family_for("$DaedricFont"), Some("book-daedric"));
        assert_eq!(bf.family_for("$UnknownFont"), None);
        assert!(
            BookFonts::default()
                .family_for("$HandwrittenFont")
                .is_none()
        );
    }
}
