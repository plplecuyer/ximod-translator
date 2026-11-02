//! Motifs à variables (B-16): matching, resolution, agreement, elision and
//! contractions, order, validation, application.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use ximod_translator::apply::Overwrite;
use ximod_translator::assist::glossary::Forms;
use ximod_translator::assist::patterns::{self, Invalid, Pattern, Resolver, Side};
use ximod_translator::assist::{
    self, Assistant, Class, Gender, Glossary, GlossaryFile, How, NoBases, Options, Patterns,
    Refusal, Rule, Term,
};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

fn noun(src: &str, dst: &str, g: Gender) -> Term {
    Term {
        gender: Some(g),
        ..Term::new(src, dst, Class::Noun)
    }
}

fn proper(src: &str, dst: &str, class: Class, g: Option<Gender>) -> Term {
    Term {
        case: true,
        gender: g,
        ..Term::new(src, dst, class)
    }
}

fn adj(src: &str, m: &str, f: &str, mp: &str, fp: &str) -> Term {
    Term {
        forms: Forms {
            m: Some(m.into()),
            f: Some(f.into()),
            mp: Some(mp.into()),
            fp: Some(fp.into()),
        },
        ..Term::new(src, "", Class::Adj)
    }
}

fn shipped() -> assist::PatternsFile {
    patterns::shipped_file(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/assist"),
        "ENG-FRA",
    )
    .unwrap()
}

fn glossary() -> Glossary {
    let mut f = GlossaryFile::default();
    for t in [
        Term::new("Iron", "en fer", Class::Material),
        Term::new("Steel", "en acier", Class::Material),
        noun("Sword", "Épée", Gender::F),
        noun("Spear", "Lance", Gender::F),
        noun("Mace", "Masse", Gender::F),
        noun("Claymore", "Claymore", Gender::F),
        noun("Shield", "Bouclier", Gender::M),
        noun("Bow", "Arc", Gender::M),
        noun("Blade", "Lame", Gender::F),
        noun("Ice Blade", "Lame de glace", Gender::F),
        noun("Knight", "chevalier", Gender::M),
        proper("Monarch", "Monarque", Class::Noun, Some(Gender::M)),
        proper("Duke", "Duc", Class::Noun, Some(Gender::M)),
        proper("Night", "Nuit", Class::Noun, Some(Gender::F)),
        proper("Eagle", "Aigle", Class::Noun, Some(Gender::M)),
        proper("Molag Bal", "de Molag Bal", Class::Origin, None),
        proper("Sithis", "de Sithis", Class::Origin, None),
        proper("Aevar", "Aevar", Class::Free, None),
        Term::new("Burning", "feu", Class::Free),
        adj("Elven", "elfique", "elfique", "elfiques", "elfiques"),
        adj("Nordic", "nordique", "nordique", "nordiques", "nordiques"),
        adj("Antique", "antique", "antique", "antiques", "antiques"),
        adj("Honed", "affûté", "affûtée", "affûtés", "affûtées"),
        Term::new("Silver", "en argent", Class::Material),
    ] {
        f.add("Test", t).unwrap();
    }
    f.validate().unwrap();
    Glossary::new(&[(&f, false)])
}

fn assistant(file: assist::PatternsFile) -> Assistant {
    let comp = file.composition.clone();
    let mut a = Assistant::new(glossary(), comp);
    a.patterns = Some(Arc::new(Patterns::new(file)));
    a
}

fn run(a: &Assistant, s: &str) -> Result<assist::Outcome, Refusal> {
    a.run(s, true, true, &NoBases)
}

fn tr(a: &Assistant, s: &str) -> String {
    run(a, s)
        .map(|o| o.text)
        .unwrap_or_else(|e| format!("{e:?}"))
}

#[test]
fn of_with_a_composed_variable() {
    // T-B16-01.
    let a = assistant(shipped());
    let o = run(&a, "Iron Sword of Burning").unwrap();
    assert_eq!(o.text, "Épée en fer de feu");
    assert_eq!(o.rule, Rule::Pattern(2));
    assert_eq!(o.rule.prov(), "rule:pattern:2");
    assert_eq!(o.steps.len(), 2);
    assert_eq!(
        (o.steps[0].var.as_str(), &o.steps[0].how),
        ("A", &How::Compose)
    );
    assert_eq!(o.steps[0].text, "Épée en fer");
    assert_eq!(
        (o.steps[1].capture.as_str(), &o.steps[1].how),
        ("Burning", &How::Glossary)
    );
    // The composition comes first when the text is covered.
    let o = run(&a, "Steel Nordic Spear").unwrap();
    assert_eq!(
        (o.text.as_str(), o.rule),
        ("Lance nordique en acier", Rule::Compose)
    );
    assert_eq!(
        tr(&a, "Antique Honed Silver Sword"),
        "Épée antique affûtée en argent"
    );
}

#[test]
fn articles_elision_and_contractions() {
    let a = assistant(shipped());
    // « de le » → « du »; an origin keeps its « de ».
    assert_eq!(
        tr(&a, "Ice Blade of the Monarch"),
        "Lame de glace du Monarque"
    );
    assert_eq!(tr(&a, "Mace of Molag Bal"), "Masse de Molag Bal");
    // Elision before a vowel; the article takes the gender.
    assert_eq!(tr(&a, "Mace of Aevar"), "Masse d'Aevar");
    assert_eq!(tr(&a, "Shield of the Night"), "Bouclier de la Nuit");
    assert_eq!(tr(&a, "Bow of the Eagle"), "Arc de l'Aigle");
    // The possessive: the article before a noun, none before a name.
    assert_eq!(tr(&a, "Duke's Sword"), "Épée du Duc");
    assert_eq!(tr(&a, "Knight's Claymore"), "Claymore du chevalier");
    assert_eq!(tr(&a, "Sithis's Blade"), "Lame de Sithis");
    // Recursion: two patterns, the second through the composition.
    let o = run(&a, "Scroll of Elven Sword").unwrap();
    assert_eq!(o.text, "Parchemin - Épée elfique");
    assert_eq!(o.rule, Rule::Pattern(4));
    assert_eq!(o.steps[0].how, How::Compose);
}

#[test]
fn agreement_threshold_and_failure() {
    let mut f = assist::PatternsFile {
        composition: None,
        ..shipped()
    };
    f.patterns = vec![Pattern {
        src: "%ADJ% %N%".into(),
        dst: "%N% %ADJ%".into(),
        vars: BTreeMap::from([("ADJ".to_string(), vec![Resolver::Adjective])]),
        ..Default::default()
    }];
    let a = assistant(f.clone());
    // The adjective takes the gender of the head (the last variable of the
    // target by default is ADJ; the head is set to N).
    f.patterns[0].head = Some("N".into());
    let a2 = assistant(f.clone());
    assert_eq!(tr(&a2, "Elven Sword"), "Épée elfique");
    assert_eq!(tr(&a2, "Honed Sword"), "Épée affûtée");
    assert_eq!(tr(&a2, "Honed Bow"), "Arc affûté");
    // Without a head of known gender: the m. form.
    assert_eq!(tr(&a, "Honed Sword"), "Épée affûté");
    // A forced form and a capital.
    f.patterns[0].dst = "%ADJ:fp:cap% %N%".into();
    let p = Patterns::new(f.clone());
    let m = p.translate("Honed Bow", &glossary(), None, &NoBases);
    assert!(m.is_some());
    assert_eq!(m.unwrap().text, "Affûtées arc");
    // The threshold: a pattern of 11 characters at most is not tried on a
    // longer text (the glossary alone takes it, in the source order).
    f.patterns[0].dst = "%N% %ADJ%".into();
    f.patterns[0].max_len = 11;
    let a3 = assistant(f.clone());
    assert_eq!(run(&a3, "Honed Sword").unwrap().rule, Rule::Pattern(2));
    assert_eq!(run(&a3, "Honed Claymore").unwrap().rule, Rule::Glossary);
    // A variable that does not resolve: the line is left as it is.
    let a = assistant(shipped());
    assert!(matches!(run(&a, "Sword of Doom"), Err(Refusal::Partial(_))));
}

#[test]
fn the_order_of_the_patterns_decides() {
    let mut f = shipped();
    f.patterns = vec![
        Pattern {
            src: "%A% of %B%".into(),
            dst: "%A% de %B%".into(),
            ..Default::default()
        },
        Pattern {
            src: "%A% of %B%".into(),
            dst: "%B% : %A%".into(),
            ..Default::default()
        },
    ];
    let a = assistant(f.clone());
    let o = run(&a, "Mace of Aevar").unwrap();
    assert_eq!(
        (o.text.as_str(), o.rule),
        ("Masse d'Aevar", Rule::Pattern(2))
    );
    f.patterns.swap(0, 1);
    let a = assistant(f.clone());
    let o = run(&a, "Mace of Aevar").unwrap();
    assert_eq!(
        (o.text.as_str(), o.rule),
        ("Aevar : masse", Rule::Pattern(2))
    );
    // The order is kept by the file.
    let back = assist::PatternsFile::parse(&f.to_json()).unwrap();
    assert_eq!(back.patterns, f.patterns);
}

#[test]
fn validation() {
    let mut f = shipped();
    f.validate().unwrap();
    f.patterns.push(Pattern {
        src: "%A% with %B%".into(),
        dst: "%A% avec".into(),
        ..Default::default()
    });
    let n = f.patterns.len() + 1;
    assert_eq!(
        f.validate(),
        Err(Invalid::VarMissing {
            n,
            var: "B".into(),
            side: Side::Dst
        })
    );
    f.patterns.last_mut().unwrap().dst = "%A% avec %B% %C%".into();
    assert!(matches!(
        f.validate(),
        Err(Invalid::VarMissing {
            side: Side::Src,
            ..
        })
    ));
    f.patterns.last_mut().unwrap().dst = "%A% avec %B%".into();
    f.patterns.last_mut().unwrap().max_len = 0;
    assert_eq!(f.validate(), Err(Invalid::MaxLen(n)));
    // Two patterns that call each other.
    let mut g = shipped();
    g.patterns = vec![
        Pattern {
            src: "%A% x".into(),
            dst: "%A% x".into(),
            recursive: true,
            vars: BTreeMap::from([("A".to_string(), vec![Resolver::Pattern(3)])]),
            ..Default::default()
        },
        Pattern {
            src: "%A% y".into(),
            dst: "%A% y".into(),
            recursive: true,
            vars: BTreeMap::from([("A".to_string(), vec![Resolver::Pattern(2)])]),
            ..Default::default()
        },
    ];
    assert!(matches!(g.validate(), Err(Invalid::Cycle { .. })));
    // Resolvers read and written as their codes.
    assert_eq!(Resolver::parse("pattern:3"), Some(Resolver::Pattern(3)));
    assert_eq!(Resolver::Adjective.code(), "glossary:adjective");
}

#[test]
fn application_by_patterns() {
    let a = assistant(shipped());
    let entry = |src: &str| TmEntry {
        rec_sig: "WEAP".into(),
        field_sig: "FULL".into(),
        source: src.into(),
        ..Default::default()
    };
    let mut entries = vec![
        entry("Iron Sword of Burning"),
        entry("Steel Nordic Spear"),
        entry("Sword of Doom"),
        entry("Mace of Molag Bal"),
    ];
    let lines: Vec<usize> = (0..entries.len()).collect();
    let plan = assist::plan(&entries, &lines, &a, &Options::default(), &NoBases);
    assert_eq!(plan.filled(), 3);
    assert_eq!(plan.composed, 1);
    assert_eq!(plan.per_pattern.get(&2), Some(&2));
    assert_eq!(plan.partial, 1);
    assist::commit(&mut entries, &plan);
    assert_eq!(entries[0].prov, "rule:pattern:2");
    assert_eq!(status::derive(&entries[0]), Status::Pending);
    assert!(entries[2].trans.is_empty());
    // Patterns unchecked: the glossary alone (nothing here is fully
    // covered but the composed line, assembled in the source order).
    let opts = Options {
        patterns: false,
        overwrite: Overwrite::None,
        ..Options::default()
    };
    let fresh: Vec<TmEntry> = lines.iter().map(|&i| entry(&entries[i].source)).collect();
    let plan = assist::plan(&fresh, &lines, &a, &opts, &NoBases);
    assert_eq!(plan.filled(), 1);
    assert_eq!(plan.glossary, 1);
}
