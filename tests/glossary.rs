//! Glossaire de termes par groupe (B-15): matching, composition, files,
//! application.

use ximod_translator::apply::Overwrite;
use ximod_translator::assist::glossary::{Dirs, Forms, Invalid, file_name, read_table};
use ximod_translator::assist::{
    self, Assistant, Class, Gender, Glossary, GlossaryFile, Options, Pos, Refusal, Term,
};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

fn noun(src: &str, dst: &str, g: Gender) -> Term {
    Term {
        gender: Some(g),
        ..Term::new(src, dst, Class::Noun)
    }
}

fn adj(src: &str, m: &str, f: &str, mp: &str, fp: &str, class: Class) -> Term {
    Term {
        forms: Forms {
            m: Some(m.into()),
            f: Some(f.into()),
            mp: Some(mp.into()),
            fp: Some(fp.into()),
        },
        ..Term::new(src, "", class)
    }
}

fn file(terms: Vec<Term>) -> GlossaryFile {
    let mut f = GlossaryFile {
        source: "eng".into(),
        target: "fra".into(),
        ..Default::default()
    };
    for t in terms {
        f.add("Test", t).unwrap();
    }
    f
}

fn fra() -> Option<assist::Composition> {
    assist::compose::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/assist"),
        None,
        "ENG-FRA",
    )
}

fn sample() -> Assistant {
    let f = file(vec![
        Term {
            pos: Pos::Start,
            ..Term::new("Iron", "en fer", Class::Material)
        },
        Term::new("Silver", "en argent", Class::Material),
        Term::new("Ebonsteel", "en ébonite", Class::Material),
        Term::new("Dragonbone", "en os de dragon", Class::Material),
        noun("Sword", "Épée", Gender::F),
        noun("Boots", "Bottes", Gender::Fp),
        noun("Heavy Boots", "Solerets", Gender::Mp),
        noun("Helmet", "Casque", Gender::M),
        adj(
            "Elven",
            "elfique",
            "elfique",
            "elfiques",
            "elfiques",
            Class::Adj,
        ),
        adj(
            "Antique",
            "antique",
            "antique",
            "antiques",
            "antiques",
            Class::Adj,
        ),
        adj(
            "Honed",
            "affûté",
            "affûtée",
            "affûtés",
            "affûtées",
            Class::Adj,
        ),
        adj(
            "White",
            "blanc",
            "blanche",
            "blancs",
            "blanches",
            Class::Color,
        ),
        Term {
            case: true,
            ..Term::new("Tribunal", "du Tribunal", Class::Origin)
        },
        Term {
            case: true,
            ..adj("Dwemer", "dwemer", "dwemer", "dwemer", "dwemer", Class::Adj)
        },
        Term::new("Light", "", Class::Adj),
        Term::new("Unique", "unique", Class::Free),
    ]);
    f.validate().unwrap();
    Assistant::new(Glossary::new(&[(&f, false)]), fra())
}

fn tr(a: &Assistant, s: &str) -> Result<String, Refusal> {
    a.translate(s, true).map(|t| t.text)
}

#[test]
fn composition_orders_the_terms_and_agrees() {
    let a = sample();
    // R-B16-02: the noun first, the adjectives agreed, the material, the
    // origin.
    assert_eq!(
        tr(&a, "Antique Honed Silver Sword").unwrap(),
        "Épée antique affûtée en argent"
    );
    assert_eq!(
        tr(&a, "Tribunal Heavy Boots White").unwrap(),
        "Solerets blancs du Tribunal"
    );
    // T-B15-03: a lowercase source word, the capital of the result.
    assert_eq!(tr(&a, "Elven boots").unwrap(), "Bottes elfiques");
    assert_eq!(tr(&a, "Elven Helmet").unwrap(), "Casque elfique");
    // A multi-word term wins over its words; an empty translation drops
    // the word.
    assert_eq!(tr(&a, "Light Boots").unwrap(), "Bottes");
    // Two materials merged (the shipped composition of ENG-FRA).
    assert_eq!(
        tr(&a, "Dragonbone Ebonsteel Boots").unwrap(),
        "Bottes en os de dragon et ébonite"
    );
    // The separators of a variant are dropped.
    assert_eq!(
        tr(&a, "Elven Boots - White").unwrap(),
        "Bottes elfiques blanches"
    );
    let t = a.translate("Tribunal Heavy Boots White", true).unwrap();
    assert!(t.composed);
    assert_eq!(t.gender, Some(Gender::Mp));
    assert_eq!(t.classes, vec![Class::Noun, Class::Color, Class::Origin]);
}

#[test]
fn whole_words_position_and_case() {
    let a = sample();
    // « Iron » (start) does not take « Ironhand » (whole word)…
    assert!(matches!(tr(&a, "Ironhand"), Err(Refusal::NoTerm)));
    // …nor « Sword of Iron » (position): « of » and « Iron » stay uncovered.
    match tr(&a, "Sword of Iron") {
        Err(Refusal::Partial(w)) => assert_eq!(w, vec!["of", "Iron"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(tr(&a, "Iron Sword").unwrap(), "Épée en fer");
    // A term whose case is kept does not take another case.
    assert!(tr(&a, "Dwemer Helmet").is_ok());
    assert!(matches!(tr(&a, "dwemer Helmet"), Err(Refusal::Partial(_))));
    // A word inside a word, with whole false.
    let f = file(vec![
        Term {
            whole: false,
            ..Term::new("Dragon", "dragon", Class::Free)
        },
        Term {
            whole: false,
            ..Term::new("bone", "os", Class::Free)
        },
    ]);
    let g = Glossary::new(&[(&f, false)]);
    assert!(g.cover("Dragonbone").complete());
    assert!(!g.cover("Dragonbones").complete());
}

#[test]
fn without_a_noun_the_terms_keep_the_source_order() {
    let a = sample();
    let t = a.translate("Elven Silver", true).unwrap();
    assert!(!t.composed);
    assert_eq!(t.not_composed, Some(assist::NotComposed::NoNoun));
    assert_eq!(t.text, "Elfique en argent");
    // A free term keeps its place among the classed ones.
    let t = a.translate("Unique Silver Sword", true).unwrap();
    assert_eq!(t.text, "Unique épée en argent");
    // Without composition (« motifs » unchecked): the source order.
    let t = a.translate("Iron Sword", false).unwrap();
    assert_eq!(t.text, "En fer épée");
    assert!(!t.composed);
}

#[test]
fn chinese_keeps_the_source_order_without_spaces() {
    let a = Assistant::load_from(
        &Dirs {
            shipped: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/assist"),
            user: None,
        },
        "ENG-ZHO",
        "SkyrimSE",
    );
    assert_eq!(a.glossary.len(), 40);
    assert_eq!(tr(&a, "Nord Steel Warhammer").unwrap(), "诺德钢制战锤");
    assert_eq!(tr(&a, "Elven Dagger").unwrap(), "精灵匕首");
}

#[test]
fn the_game_glossary_comes_first() {
    let dir = std::env::temp_dir().join(format!("ximod-b15-game-{}", std::process::id()));
    let shipped = dir.join("shipped");
    std::fs::create_dir_all(&shipped).unwrap();
    let pair = file(vec![
        Term::new("Iron", "en fer", Class::Material),
        noun("Sword", "Épée", Gender::F),
    ]);
    let game = file(vec![Term::new("Iron", "de fer", Class::Material)]);
    std::fs::write(shipped.join(file_name("ENG-FRA", None)), pair.to_json()).unwrap();
    std::fs::write(
        shipped.join(file_name("ENG-FRA", Some("Oblivion"))),
        game.to_json(),
    )
    .unwrap();
    let dirs = Dirs {
        shipped,
        user: Some(dir.join("user")),
    };
    let ob = Assistant::load_from(&dirs, "ENG-FRA", "Oblivion");
    let sk = Assistant::load_from(&dirs, "ENG-FRA", "SkyrimSE");
    assert_eq!(tr(&ob, "Iron Sword").unwrap(), "Épée de fer");
    assert_eq!(tr(&sk, "Iron Sword").unwrap(), "Épée en fer");
    // The user's copy wins over the shipped file.
    let mut user = pair.clone();
    user.groups[0].terms[0].dst = "ferreux".into();
    dirs.save(&file_name("ENG-FRA", None), &user).unwrap();
    let sk = Assistant::load_from(&dirs, "ENG-FRA", "SkyrimSE");
    assert_eq!(tr(&sk, "Iron Sword").unwrap(), "Épée ferreux");
    dirs.restore(&file_name("ENG-FRA", None)).unwrap();
    let sk = Assistant::load_from(&dirs, "ENG-FRA", "SkyrimSE");
    assert_eq!(tr(&sk, "Iron Sword").unwrap(), "Épée en fer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_and_validation() {
    // A TSV of 50 terms, with a header, into a new group.
    let mut text = String::from("original\ttraduit\n");
    for i in 0..50 {
        text.push_str(&format!("Word{i}\tMot{i}\n"));
    }
    let rows = read_table(&text);
    assert_eq!(rows.len(), 50);
    let mut f = GlossaryFile::default();
    for (s, d) in &rows {
        f.add("Importés", Term::new(s, d, Class::Free)).unwrap();
    }
    assert_eq!(f.term_count(), 50);
    // A duplicate original is refused with its group.
    assert_eq!(
        f.add("Importés", Term::new("word7", "x", Class::Free)),
        Err(Invalid::Duplicate {
            src: "word7".into(),
            group: "Importés".into()
        })
    );
    assert!(matches!(
        f.add("Importés", Term::new("  ", "x", Class::Free)),
        Err(Invalid::EmptySrc(_))
    ));
    // CSV with semicolons and quotes.
    assert_eq!(
        read_table("\"Heavy Boots\";\"Solerets\"\nIron;en fer\n"),
        vec![
            ("Heavy Boots".to_string(), "Solerets".to_string()),
            ("Iron".to_string(), "en fer".to_string())
        ]
    );
    // Forms without the masculine one.
    let mut g = GlossaryFile::default();
    g.add(
        "A",
        Term {
            forms: Forms {
                f: Some("x".into()),
                ..Default::default()
            },
            ..Term::new("X", "", Class::Adj)
        },
    )
    .unwrap();
    assert!(matches!(g.validate(), Err(Invalid::Forms { .. })));
    // The shipped files are valid and round-trip.
    for name in ["glossary_ENG-FRA.json", "glossary_ENG-ZHO.json"] {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/data/assist")
            .join(name);
        let f = GlossaryFile::parse(&std::fs::read_to_string(p).unwrap()).unwrap();
        f.validate().unwrap();
        assert_eq!(GlossaryFile::parse(&f.to_json()).unwrap(), f);
    }
}

fn entry(src: &str, tr: &str, st: Option<Status>) -> TmEntry {
    let mut e = TmEntry {
        rec_sig: "ARMO".into(),
        field_sig: "FULL".into(),
        source: src.into(),
        trans: tr.into(),
        ..Default::default()
    };
    if let Some(s) = st {
        status::set(&mut e, s);
    }
    e
}

#[test]
fn application_fills_whole_lines_only() {
    let a = sample();
    let mut entries = vec![
        entry("Elven Boots", "", None),
        entry("Elven Boots of Doom", "", None),
        entry("Iron Sword", "Épée de fer", Some(Status::Validated)),
        entry("Silver Sword", "", Some(Status::Ignored)),
        entry("Elven Silver", "", None),
        entry("Helmet", "Casque ancien", Some(Status::BySimilarity)),
    ];
    entries[3].prov = "lock:word".into();
    let lines: Vec<usize> = (0..entries.len()).collect();
    let opts = Options::default();
    let plan = assist::plan(&entries, &lines, &a, &opts, &assist::NoBases);
    assert_eq!(plan.filled(), 2);
    assert_eq!(
        (plan.composed, plan.glossary, plan.partial, plan.locked),
        (1, 1, 1, 1)
    );
    let touched = assist::commit(&mut entries, &plan);
    assert_eq!(touched, vec![0, 4]);
    assert_eq!(entries[0].trans, "Bottes elfiques");
    assert_eq!(entries[0].prov, "rule:compose");
    assert_eq!(entries[4].prov, "rule:glossary");
    assert_eq!(status::derive(&entries[0]), Status::Pending);
    assert!(entries[1].trans.is_empty());
    assert_eq!(entries[2].trans, "Épée de fer");
    assert!(entries[3].trans.is_empty());
    // « Tout sauf Validé »: the line translated by similarity is replaced,
    // the Validé one is not.
    let opts = Options {
        overwrite: Overwrite::KeepValidated,
        ..Options::default()
    };
    let plan = assist::plan(&entries, &lines, &a, &opts, &assist::NoBases);
    assert_eq!(plan.filled(), 1);
    assert_eq!(plan.overwritten, 1);
    assert_eq!(plan.changes[0].idx, 5);
    assert_eq!(plan.changes[0].trans, "Casque");
}
