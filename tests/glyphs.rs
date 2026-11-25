//! T-D104-02 (recette du lot 2) : aucun carré vide dans l'interface. Chaque
//! caractère des 33 fichiers de langue, et chaque symbole ou icône écrit
//! dans le code (flèches, coches, icônes Phosphor, emoji), a un glyphe dans
//! les polices que l'application charge : celles d'egui, les icônes et les
//! polices citées par `Languages.json`.

use std::path::{Path, PathBuf};

use eframe::egui;
use egui::epaint::text::{Fonts, TextOptions};
use ximod_translator::data::LanguagesData;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The fonts of the interface, as the application builds them.
fn fonts() -> Fonts {
    let all = LanguagesData::load().all_fonts();
    let defs = ximod_translator::fonts::build_font_definitions_with_preview(&all, None);
    Fonts::new(TextOptions::default(), defs)
}

/// Drawn: neither a control, a format character (isolates, joiners) nor a
/// space, which have no glyph to show.
fn drawn(c: char) -> bool {
    !(c.is_control()
        || c.is_whitespace()
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'))
}

/// A symbol or an icon: arrows, technical and geometric signs, dingbats,
/// emoji, the private area of the icon font.
fn symbol(c: char) -> bool {
    matches!(c as u32, 0x2190..=0x2BFF | 0xE000..=0xF8FF | 0x1F000..=0x1FAFF)
}

/// The characters of the string and character literals of a Rust file
/// (escapes `\u{…}` decoded), comments left out.
fn literal_chars(text: &str) -> Vec<char> {
    let mut out = Vec::new();
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    let unescape = |cs: &[char], i: &mut usize, out: &mut Vec<char>| {
        // At a backslash: decode `\u{…}`, skip the others.
        if cs.get(*i + 1) == Some(&'u') && cs.get(*i + 2) == Some(&'{') {
            let mut j = *i + 3;
            let mut hex = String::new();
            while j < cs.len() && cs[j] != '}' {
                hex.push(cs[j]);
                j += 1;
            }
            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push(c);
            }
            *i = j + 1;
        } else {
            *i += 2;
        }
    };
    while i < cs.len() {
        match cs[i] {
            '/' if cs.get(i + 1) == Some(&'/') => {
                while i < cs.len() && cs[i] != '\n' {
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    if cs[i] == '\\' {
                        unescape(&cs, &mut i, &mut out);
                    } else {
                        out.push(cs[i]);
                        i += 1;
                    }
                }
                i += 1;
            }
            '\'' => {
                // A character literal, not a lifetime.
                if cs.get(i + 1) == Some(&'\\') {
                    let mut j = i + 1;
                    let before = out.len();
                    unescape(&cs, &mut j, &mut out);
                    if cs.get(j) == Some(&'\'') {
                        i = j + 1;
                    } else {
                        out.truncate(before);
                        i += 1;
                    }
                } else if cs.get(i + 2) == Some(&'\'') {
                    out.push(cs[i + 1]);
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_character_of_the_interface_files_has_a_glyph() {
    let mut fonts = fonts();
    let font = egui::FontId::proportional(14.0);
    let mut missing: Vec<String> = Vec::new();
    let mut locales = 0;
    for entry in std::fs::read_dir(root().join("assets/locales")).unwrap() {
        let dir = entry.unwrap().path();
        let Ok(text) = std::fs::read_to_string(dir.join("main.ftl")) else {
            continue;
        };
        locales += 1;
        let loc = dir.file_name().unwrap().to_string_lossy().to_string();
        for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
            for c in line.chars().filter(|&c| drawn(c)) {
                if !fonts.has_glyph(&font, c) {
                    missing.push(format!("{loc}: U+{:04X} {c}", c as u32));
                }
            }
        }
    }
    assert!(locales >= 33, "{locales}");
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "sans glyphe : {missing:?}");
}

#[test]
fn every_symbol_of_the_code_has_a_glyph() {
    let mut fonts = fonts();
    let font = egui::FontId::proportional(14.0);
    let mut files = Vec::new();
    rust_files(&root().join("src"), &mut files);
    let mut seen = 0;
    let mut missing: Vec<String> = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for c in literal_chars(&text).into_iter().filter(|&c| symbol(c)) {
            seen += 1;
            if !fonts.has_glyph(&font, c) {
                let name = f.strip_prefix(root()).unwrap().display().to_string();
                missing.push(format!("{name}: U+{:04X} {c}", c as u32));
            }
        }
    }
    // The icons of src/icons.rs at least.
    assert!(seen >= 30, "{seen}");
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "sans glyphe : {missing:?}");
}

#[test]
fn the_literals_are_read_from_the_code() {
    let src = r#"let a = "→ x"; // ✘ comment
let b = '\u{E244}'; let c = "\u{1F3F3}"; fn f<'a>(x: &'a str) {} let d = '✔';"#;
    let got: String = literal_chars(src)
        .into_iter()
        .filter(|&c| symbol(c))
        .collect();
    assert_eq!(got, "→\u{E244}\u{1F3F3}✔");
}
