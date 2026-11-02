//! B-17 — replacement matrices: the shipped files, the user's copy, the
//! two directions, the lines a matrix may touch (T-B17-01 … T-B17-07).

use std::path::{Path, PathBuf};

use ximod_translator::matrices::{
    ALL, Direction, Dirs, Invalid, Inverse, Matrix, MatrixFile, MatrixRule, Origin, apply_text,
    not_idempotent, plan,
};
use ximod_translator::provenance::{self, Prov, ProvFilter};
use ximod_translator::replace::{Mode, Rule, RulesFile};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

fn dirs(tag: &str) -> Dirs {
    let user = std::env::temp_dir().join(format!("ximod-matrices-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    Dirs {
        shipped: Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/matrices"),
        user: Some(user),
    }
}

fn entry(trans: &str) -> TmEntry {
    let mut e = TmEntry {
        source: "source".into(),
        trans: trans.into(),
        ..Default::default()
    };
    e.set_translated_flags(true);
    status::set(&mut e, Status::Validated);
    e
}

fn typo() -> Matrix {
    let (f, _) = dirs("typo").load("fra");
    f.find("Typographie française").cloned().unwrap()
}

/// T-B17-01: the shipped matrix edited, saved in the user's copy, offered
/// again without restart; « Restaurer la version livrée » removes the copy.
#[test]
fn user_copy_wins_and_restores() {
    let d = dirs("copy");
    let (mut f, origin) = d.load("fra");
    assert_eq!(origin, Origin::Shipped);
    let m = f
        .matrices
        .iter_mut()
        .find(|m| m.name == "Typographie française")
        .unwrap();
    let mut oe = MatrixRule::new("oe", "œ", Mode::Normal);
    oe.rule.whole = false;
    m.rules.push(oe);
    let saved: PathBuf = d.save("fra", &f).unwrap();
    assert!(saved.is_file());
    let (again, origin) = d.load("fra");
    assert_eq!(origin, Origin::User);
    let typo = again.find("Typographie française").unwrap();
    assert_eq!(typo.rules.last().unwrap().rule.find, "oe");
    let offered = d.offered("fra");
    assert!(
        offered
            .iter()
            .any(|(l, m)| l == "fra" && m.name == "Typographie française")
    );
    assert!(
        offered.iter().any(|(l, _)| l == ALL),
        "the common matrices follow"
    );
    let c = typo.compile(Direction::Direct).unwrap();
    assert_eq!(apply_text(&c, "Un coeur").0, "Un cœur");
    d.restore("fra").unwrap();
    assert_eq!(d.load("fra").1, Origin::Shipped);
    let _ = std::fs::remove_dir_all(d.user.unwrap());
}

/// The French typography of the shipped matrix, and its inverse.
#[test]
fn french_typography() {
    let m = typo();
    let c = m.compile(Direction::Direct).unwrap();
    let nnbsp = '\u{202f}';
    let nbsp = '\u{a0}';
    let (t, _) = apply_text(
        &c,
        "Bonjour ! Qui êtes-vous? Attention : danger... Il est 10:30.",
    );
    assert_eq!(
        t,
        format!("Bonjour{nnbsp}! Qui êtes-vous{nnbsp}? Attention{nbsp}: danger… Il est 10:30.")
    );
    // Tags and addresses are left alone.
    let tag = "<font face='$MAIN_Font'>Voir img://Textures/a.dds</font>";
    assert_eq!(apply_text(&c, tag).0, tag);
    let inv = m.compile(Direction::Inverse).unwrap();
    assert_eq!(
        apply_text(&inv, &t).0,
        "Bonjour! Qui êtes-vous? Attention: danger... Il est 10:30."
    );
    // Quotes and apostrophes.
    let (f, _) = dirs("quotes").load("fra");
    let q = f.find("Guillemets français « »").unwrap();
    let qc = q.compile(Direction::Direct).unwrap();
    assert_eq!(
        apply_text(&qc, "Il dit \"oui\" <p align=\"center\">").0,
        format!("Il dit «{nbsp}oui{nbsp}» <p align=\"center\">")
    );
    let a = f.find("Apostrophes typographiques").unwrap();
    let ac = a.compile(Direction::Direct).unwrap();
    assert_eq!(apply_text(&ac, "l'épée d'acier").0, "l’épée d’acier");
}

/// T-B17-02: direct then inverse gives the texts back (Normal rules); a
/// Regex rule without inverse keeps the matrix to the direct direction.
#[test]
fn direct_then_inverse_round_trip() {
    let m = Matrix {
        name: "Ponctuation".into(),
        rules: vec![
            MatrixRule::new("...", "…", Mode::Normal),
            MatrixRule::new("--", "—", Mode::Normal),
            MatrixRule::new("\\t", "→", Mode::Extended),
        ],
    };
    assert!(m.blocking_inverse().is_none());
    let texts: Vec<String> = (0..1000)
        .map(|i| format!("Ligne {i}... suite -- fin\tok"))
        .collect();
    let d = m.compile(Direction::Direct).unwrap();
    let i = m.compile(Direction::Inverse).unwrap();
    for t in &texts {
        let (once, n) = apply_text(&d, t);
        assert_eq!(n, 3);
        assert_eq!(&apply_text(&i, &once).0, t);
    }
    let mut no_inv = m.clone();
    no_inv
        .rules
        .push(MatrixRule::new("(\\d+)", "#$1", Mode::Regex));
    assert_eq!(no_inv.blocking_inverse(), Some(3));
    assert!(no_inv.compile(Direction::Inverse).is_err());
    // An explicit inverse lifts the block.
    no_inv.rules[3].inverse = Some(Inverse {
        find: "#(\\d+)".into(),
        replace: "$1".into(),
    });
    assert!(no_inv.blocking_inverse().is_none());
}

/// T-B17-03 / T-B17-04: only the translated lines that are not locked are
/// touched; the provenance code reads « remplacement (<matrice>) » and
/// files under the Remplacement filter.
#[test]
fn lines_touched_and_provenance() {
    let mut locked = entry("Bonjour !");
    status::set(&mut locked, Status::Ignored);
    let mut unused = entry("Bonjour !");
    unused.unused = true;
    let es = vec![entry("Bonjour !"), locked, unused, entry(""), entry("Rien")];
    let c = typo().compile(Direction::Direct).unwrap();
    let p = plan(&es, &[0, 1, 2, 3, 4], &c);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].0, 0);
    let code = provenance::replace_matrix("Typographie française");
    assert_eq!(code, "replace:Typographie française");
    assert_eq!(
        provenance::parse(&code),
        Prov::ReplaceMatrix("Typographie française".into())
    );
    let mut e = entry(&p[0].1);
    e.prov = code;
    assert_eq!(ProvFilter::of(&e), Some(ProvFilter::Replace));
    assert_eq!(status::derive(&e), Status::Validated, "the status is kept");
}

/// T-B17-05: an invalid file is never saved and the shipped version is
/// read instead of an invalid user copy.
#[test]
fn invalid_files() {
    let d = dirs("invalid");
    let bad = MatrixFile {
        format: 1,
        matrices: vec![Matrix {
            name: "Cassée".into(),
            rules: vec![MatrixRule::new("(a|", "b", Mode::Regex)],
        }],
    };
    match d.save("fra", &bad) {
        Err(Invalid::Regex { rule, matrix, .. }) => {
            assert_eq!(rule, 1);
            assert_eq!(matrix, "Cassée");
        }
        other => panic!("{other:?}"),
    }
    assert!(!d.user_path("fra").unwrap().exists());
    // A user copy broken by hand: the shipped matrices apply.
    let p = d.user_path("fra").unwrap();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "{ \"format\": 1, \"matrices\": [ ").unwrap();
    let (f, origin) = d.load("fra");
    assert_eq!(origin, Origin::Shipped);
    assert!(f.find("Typographie française").is_some());
    assert!(matches!(
        MatrixFile::parse("{\"format\": 2, \"matrices\": []}"),
        Err(Invalid::Format(2))
    ));
    let _ = std::fs::remove_dir_all(d.user.unwrap());
}

/// T-B17-06: an A-11 rules file imported as a matrix.
#[test]
fn rules_file_imported() {
    let f = RulesFile::from_json(
        r#"{ "format": 1, "rules": [
            { "find": "Dragonborn", "replace": "Enfant de dragon", "case": true, "whole": true },
            { "find": "\\s+$", "replace": "", "mode": "regex" } ] }"#,
    )
    .unwrap();
    let m = Matrix::from_rules("lore", &f.rules);
    assert_eq!(m.rules.len(), 2);
    assert_eq!(
        m.rules[0].rule,
        Rule {
            find: "Dragonborn".into(),
            replace: "Enfant de dragon".into(),
            mode: Mode::Normal,
            case_sensitive: true,
            whole: true,
        }
    );
    assert_eq!(m.blocking_inverse(), Some(1));
    let c = m.compile(Direction::Direct).unwrap();
    assert_eq!(
        apply_text(&c, "Le Dragonborn arrive   ").0,
        "Le Enfant de dragon arrive"
    );
}

/// T-B17-07: every shipped matrix is idempotent.
#[test]
fn shipped_matrices_idempotent() {
    let d = dirs("idem");
    let samples = [
        "Bonjour ! Qui êtes-vous ? Attention : « danger ».",
        "Il dit \"oui\"... puis s'en va ; voilà !",
        "Quoi ?! Vraiment ? Il est 10:30.",
        "Hallo \"Welt\", wie geht's?",
        "¿Qué? ¡Hola! Bien. ¿Y tú? \"comillas\"",
        "A  B   C",
    ];
    for lang in d.languages() {
        let (f, _) = d.load(&lang);
        f.validate().unwrap();
        for m in &f.matrices {
            let c = m.compile(Direction::Direct).unwrap();
            assert_eq!(not_idempotent(&c, &samples), 0, "{lang}: {}", m.name);
        }
    }
}
