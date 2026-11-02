//! Every text the user can read lives in the `.ftl` files: this test scans
//! the sources (outside the test modules) for texts written in the code —
//! a sentence given to a widget, an error built with `bail!`/`anyhow!`
//! without `l10n!`, French typography — and fails on the first new one.
//!
//! What stays in the code on purpose: log lines (tracing, startup log),
//! file-format content (XML tags, Config.ini keys, metadata headers),
//! technical identifiers (record signatures, URLs, file names, extensions,
//! Creation Kit function names) and the endonyms of the built-in language
//! table.

use regex::Regex;
use std::path::{Path, PathBuf};

/// Literals accepted in a widget call: values and examples, not language.
const ALLOWED: &[&str] = &[
    "https://api.openai.com/v1",
    "westeurope",
    "http://proxy:3128",
    "REC:FLD",
    "DNAM",
    "NAM2",
    "HTTPS_PROXY",
    "7z",
    "{name} {version} {LANG} {lang} {lang3} {langname}",
    "CSV",
    "SST",
    "DSD",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The source without its test module and comments (lines kept).
fn code_of(src: &str) -> String {
    let src = match src.find("#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
    };
    src.lines()
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with("//") { "" } else { l }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_user_text_in_the_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    // A widget given a sentence literal.
    let widget = Regex::new(
        r#"(\.label|\.button|hover_text|hint_text|RichText::new|Button::new|Label::new|shortcut_text|add_filter|with_title|\.heading|small_button|colored_label\([^,]+,)\(\s*"([^"\\]*)""#,
    )
    .unwrap();
    // An error built from a sentence instead of an i18n key.
    let error = Regex::new(
        r#"(bail!|anyhow!|Error::Format|Error::Network)\(\s*"([^"\\{]*[A-Za-z]{2}[^"\\]*)""#,
    )
    .unwrap();
    // French typography or accents in a literal.
    let french = Regex::new(r#""[^"\n]*([«»]|[éèêàçùÉÈ][a-z])[^"\n]*""#).unwrap();
    let mut found = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap().display().to_string();
        let code = code_of(&std::fs::read_to_string(f).unwrap());
        for (n, line) in code.lines().enumerate() {
            if line.contains("tracing::") || line.contains("log.push") {
                continue;
            }
            for c in widget.captures_iter(line) {
                let t = &c[2];
                if t.chars().filter(|c| c.is_alphabetic()).count() >= 2 && !ALLOWED.contains(&t) {
                    found.push(format!("{rel}:{}: {t}", n + 1));
                }
            }
            for c in error.captures_iter(line) {
                found.push(format!("{rel}:{}: {}", n + 1, &c[2]));
            }
            if french.is_match(line) && !rel.ends_with("i18n/mod.rs") {
                found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "texts written in the code (move them to the .ftl files):\n{}",
        found.join("\n")
    );
}
