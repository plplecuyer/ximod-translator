//! Locate the user manual shipped with the application (F-01, F1).
//!
//! Manuals are files named `XIMOD_Translator_<word>_<CODE>.<ext>` where
//! `<CODE>` is the ISO 639-3 code of their language in capitals (`FRA`,
//! `ENG`, the convention of XIMOD Architect, R-E07-06) and `<ext>` is `pdf`
//! or `docx`. They live in a `Manuals/` folder: next to the executable for a
//! release, at the repository root for a source checkout. The former English
//! name (`_GBR`) is still found.

use std::path::{Path, PathBuf};

/// Code of the manual written in an interface locale (ISO 639-3, upper
/// case); a locale without a manual falls back to English.
pub fn manual_code(locale: &str) -> String {
    let code = crate::i18n::normalize_locale(locale);
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
        code.to_ascii_uppercase()
    } else {
        "ENG".to_string()
    }
}

/// Candidate `Manuals/` folders, most specific first.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        dirs.push(exe_dir.join("Manuals"));
        dirs.push(exe_dir.clone());
        // `cargo run`: target/<profile>/ximod-translator → repository root.
        for up in [1, 2, 3] {
            let mut d = exe_dir.clone();
            for _ in 0..up {
                d.pop();
            }
            dirs.push(d.join("Manuals"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("Manuals"));
    }
    dirs
}

/// Whether a file name is a manual (`XIMOD…_<cc>.pdf|docx`); `cc` empty
/// accepts any country.
fn is_manual_name(name: &str, cc: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if !name.starts_with("XIMOD") {
        return false;
    }
    let stem = match lower
        .strip_suffix(".pdf")
        .or_else(|| lower.strip_suffix(".docx"))
    {
        Some(s) => s,
        None => return false,
    };
    cc.is_empty() || stem.ends_with(&format!("_{}", cc.to_ascii_lowercase()))
}

/// The manuals inside `dir` for a country (PDF before DOCX, then by name).
fn manuals_in(dir: &Path, cc: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| is_manual_name(n, cc))
        })
        .collect();
    found.sort_by_key(|p| {
        let pdf = p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        (!pdf, p.clone())
    });
    found
}

/// Find the manual for `locale`, falling back to English, then to any.
pub fn find_manual(locale: &str) -> Option<PathBuf> {
    find_manual_in(&candidate_dirs(), locale)
}

/// [`find_manual`] over explicit folders (tests).
pub fn find_manual_in(dirs: &[PathBuf], locale: &str) -> Option<PathBuf> {
    let cc = manual_code(locale);
    for want in [cc.as_str(), "ENG", "GBR", ""] {
        for dir in dirs {
            if let Some(p) = manuals_in(dir, want).into_iter().next() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_locale_then_english_then_any_pdf_first() {
        let dir = std::env::temp_dir().join(format!("ximod_manual_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fra_docx = dir.join("XIMOD_Translator_Manuel_FRA.docx");
        let fra_pdf = dir.join("XIMOD_Translator_Manuel_FRA.pdf");
        let eng_pdf = dir.join("XIMOD_Translator_Manual_ENG.pdf");
        std::fs::write(&fra_docx, b"x").unwrap();
        std::fs::write(&eng_pdf, b"x").unwrap();
        let dirs = vec![dir.clone()];
        assert_eq!(find_manual_in(&dirs, "fra"), Some(fra_docx.clone()));
        assert_eq!(find_manual_in(&dirs, "deu"), Some(eng_pdf.clone()));
        std::fs::write(&fra_pdf, b"x").unwrap();
        assert_eq!(find_manual_in(&dirs, "fra"), Some(fra_pdf));
        std::fs::remove_file(&eng_pdf).unwrap();
        // No English: any manual rather than nothing.
        assert!(find_manual_in(&dirs, "deu").is_some());
        assert!(find_manual_in(&[dir.join("nope")], "fra").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_codes_and_former_english_name() {
        assert_eq!(manual_code("fra"), "FRA");
        assert_eq!(manual_code("deu"), "DEU");
        assert_eq!(manual_code("en"), "ENG");
        let dir = std::env::temp_dir().join(format!("ximod_manual_gbr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gbr = dir.join("XIMOD_Translator_Manual_GBR.docx");
        std::fs::write(&gbr, b"x").unwrap();
        assert_eq!(find_manual_in(std::slice::from_ref(&dir), "pol"), Some(gbr));
        let eng = dir.join("XIMOD_Translator_Manual_ENG.docx");
        std::fs::write(&eng, b"x").unwrap();
        assert_eq!(find_manual_in(std::slice::from_ref(&dir), "pol"), Some(eng));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
