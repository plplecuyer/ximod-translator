//! D-104 (taille livrée) : les polices livrées sont exactement celles que
//! citent les données, avec leurs licences ; les polices de jeu déposées
//! dans fonts/books ne partent jamais ; sous Linux et macOS, ximod-cli est
//! un lien vers ximod-translator.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ximod_translator::data::LanguagesData;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The fonts the data cite: `Languages.json` and the `@font` header of the
/// interface languages, as files of `assets/fonts`.
fn cited() -> BTreeSet<String> {
    let dir = root().join("assets/fonts");
    let mut rels: Vec<String> = LanguagesData::load().all_fonts();
    for entry in std::fs::read_dir(root().join("assets/locales")).unwrap() {
        let ftl = entry.unwrap().path().join("main.ftl");
        let Ok(text) = std::fs::read_to_string(&ftl) else {
            continue;
        };
        for line in text.lines().take(20) {
            if let Some(f) = line.strip_prefix("# @font = ") {
                rels.push(f.trim().to_string());
            }
        }
    }
    rels.into_iter()
        .map(|r| {
            let f = ximod_translator::fonts::font_file(&dir, &r);
            assert!(f.is_file(), "police citée absente : {r}");
            rel(&dir, &f)
        })
        .collect()
}

fn rel(base: &Path, p: &Path) -> String {
    p.strip_prefix(base)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

fn is_game_font(r: &str) -> bool {
    let l = r.to_lowercase();
    l.starts_with("books/") && (l.ends_with(".ttf") || l.ends_with(".otf"))
}

/// R-D104-01 : chaque police citée existe, et `assets/fonts` ne contient
/// rien d'autre que les polices citées, leurs licences OFL et le dossier
/// des polices de livre.
#[test]
fn the_fonts_are_those_the_data_cite() {
    let dir = root().join("assets/fonts");
    let cited = cited();
    assert!(cited.len() >= 60, "{}", cited.len());
    for f in files(&dir) {
        let r = rel(&dir, &f);
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let ok = cited.contains(&r)
            || name == "OFL.txt"
            || name == ".gitkeep"
            || r == "books/README.md"
            || is_game_font(&r);
        assert!(ok, "fichier de assets/fonts non cité : {r}");
        // Regular only (16 novembre) : ni graisse, ni police variable.
        if cited.contains(&r) {
            assert!(!r.contains("static/") && !r.contains("Variable"), "{r}");
        }
    }
}

/// R-D104-01 et R-D104-03 : le dossier livré (Linux, macOS) porte les
/// polices citées et leurs licences, aucune police de jeu, et ximod-cli
/// en lien vers ximod-translator.
#[cfg(unix)]
#[test]
fn the_staged_folder_carries_the_cited_fonts_and_the_link() {
    let tmp = std::env::temp_dir().join(format!("ximod-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let release = tmp.join("release");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(release.join("ximod-translator"), b"bin").unwrap();
    std::fs::write(release.join("ximod-cli"), b"cli").unwrap();
    let dest = tmp.join("stage");
    let out = std::process::Command::new("bash")
        .arg("packaging/stage.sh")
        .arg(&release)
        .arg(&dest)
        .env("XIMOD_STAGE_SKIP", "dictionaries images locales data icons")
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The link, never a second copy of the executable.
    let link = dest.join("ximod-cli");
    let meta = std::fs::symlink_metadata(&link).unwrap();
    assert!(meta.file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        PathBuf::from("ximod-translator")
    );
    // The fonts: those cited, their licences, the README of books.
    let fonts = dest.join("assets/fonts");
    let shipped: Vec<String> = files(&fonts).iter().map(|f| rel(&fonts, f)).collect();
    let cited = cited();
    for c in &cited {
        assert!(shipped.contains(c), "police citée non livrée : {c}");
    }
    for s in &shipped {
        assert!(!is_game_font(s), "police de jeu livrée : {s}");
        assert!(
            cited.contains(s) || s.ends_with("OFL.txt") || s == "books/README.md",
            "livrée sans être citée : {s}"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
