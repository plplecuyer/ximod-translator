//! Dérivation d'objets (B-21): parents in the document, the open files and
//! the bases, the tome's spell, transformations, other variables, follow,
//! ambiguity, validation.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ximod_translator::apply::Overwrite;
use ximod_translator::assist::derive::{
    self, DeriveFile, Derived, Deriver, Invalid, Origin, ParentBases, Parents, PerLang, Rule, Sigs,
};
use ximod_translator::assist::glossary::Forms;
use ximod_translator::assist::{
    self, Assistant, Class, DeriveCtx, Glossary, GlossaryFile, NoBases, Options, Patterns, Term,
};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

fn shipped() -> DeriveFile {
    let f = derive::shipped_file(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/assist"),
        "SkyrimSE",
    )
    .unwrap();
    f.validate().unwrap();
    f
}

fn assistant() -> Assistant {
    let mut g = GlossaryFile::default();
    g.add(
        "Degrés",
        Term {
            forms: Forms {
                m: Some("dilué".into()),
                f: Some("diluée".into()),
                mp: Some("dilués".into()),
                fp: Some("diluées".into()),
            },
            ..Term::new("Weak", "", Class::Adj)
        },
    )
    .unwrap();
    let p = assist::patterns::shipped_file(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/assist"),
        "ENG-FRA",
    )
    .unwrap();
    let mut a = Assistant::new(Glossary::new(&[(&g, false)]), p.composition.clone());
    a.patterns = Some(Arc::new(Patterns::new(p)));
    a
}

fn line(rec: &str, edid: &str, formid: u32, src: &str, tr: &str) -> TmEntry {
    TmEntry {
        rec_sig: rec.into(),
        field_sig: "FULL".into(),
        edid: edid.into(),
        formid,
        source: src.into(),
        trans: tr.into(),
        ..Default::default()
    }
}

/// A dictionary of texts, signed or not.
struct Texts(HashMap<String, String>);

impl ParentBases for Texts {
    fn lookup(&self, _: &str, _: &str, text: &str) -> Option<String> {
        self.0.get(text).cloned()
    }
}

fn plan(
    entries: &[TmEntry],
    open: Vec<&[TmEntry]>,
    pb: &dyn ParentBases,
    teaches: &HashMap<u32, u32>,
) -> assist::Plan {
    let f = shipped();
    let ctx = DeriveCtx {
        file: &f,
        lang: "fra",
        open,
        parent_bases: pb,
        teaches,
    };
    let lines: Vec<usize> = (0..entries.len()).collect();
    assist::plan_all(
        entries,
        &lines,
        &assistant(),
        &Options {
            glossary: false,
            patterns: false,
            derive: true,
            overwrite: Overwrite::None,
        },
        &NoBases,
        Some(&ctx),
    )
}

#[test]
fn a_scroll_from_a_spell_of_the_base() {
    // T-B21-01: the spell is only in the game's base.
    let entries = vec![line(
        "SCRL",
        "ModScrollFireball",
        0x0100_0800,
        "Scroll of Fireball",
        "",
    )];
    let base = Texts(HashMap::from([(
        "Fireball".to_string(),
        "Boule de feu".to_string(),
    )]));
    let p = plan(&entries, Vec::new(), &base, &HashMap::new());
    assert_eq!(p.filled(), 1);
    assert_eq!(p.changes[0].trans, "Parchemin - Boule de feu");
    assert_eq!(p.changes[0].prov, "rule:derive:scroll:Fireball");
    // Without the base: no parent.
    let p = plan(&entries, Vec::new(), &NoBases, &HashMap::new());
    assert_eq!((p.filled(), p.not_found), (0, 1));
}

#[test]
fn transformations_variables_and_the_tome() {
    let mut entries = vec![
        line(
            "SPEL",
            "WB_ConjureLich",
            0x10,
            "Conjure Lich",
            "Invocation - Liche",
        ),
        line("SCRL", "WB_ScrollLich", 0x11, "Scroll of Conjure Lich", ""),
        line(
            "MGEF",
            "AlchFortOneHanded",
            0x12,
            "Fortify One-handed",
            "Renfort d'armes à une main",
        ),
        line(
            "ALCH",
            "PotionWeak1H",
            0x13,
            "Weak Potion of Fortify One-handed",
            "",
        ),
        line(
            "ALCH",
            "PoisonWeak1H",
            0x14,
            "Weak Poison of Fortify One-handed",
            "",
        ),
        line("MGEF", "AlchEther", 0x15, "Etherealness", "Éthérisation"),
        line("ALCH", "PotionEther", 0x16, "Potion of Etherealness", ""),
        // The tome's spell has another name than the tome says.
        line(
            "SPEL",
            "WB_BloodForBlood",
            0x20,
            "Blood For Blood",
            "Sang pour sang",
        ),
        line(
            "BOOK",
            "WB_TomeBlood",
            0x21,
            "Spell Tome: Blood for Blood (old)",
            "",
        ),
    ];
    let teaches = HashMap::from([(0x21u32, 0x20u32)]);
    let p = plan(&entries, Vec::new(), &NoBases, &teaches);
    let get = |i: usize| {
        p.changes
            .iter()
            .find(|c| c.idx == i)
            .map(|c| c.trans.clone())
    };
    // « Invocation - » taken off the spell.
    assert_eq!(get(1).as_deref(), Some("Parchemin - Liche"));
    // The degree agreed with the noun the rule writes, the effect lowered.
    assert_eq!(
        get(3).as_deref(),
        Some("Potion de renfort d'armes à une main - Diluée")
    );
    assert_eq!(
        get(4).as_deref(),
        Some("Poison de renfort d'armes à une main - Dilué")
    );
    // Elision at the junction.
    assert_eq!(get(6).as_deref(), Some("Potion d'éthérisation"));
    // The tome takes the spell it teaches.
    assert_eq!(get(8).as_deref(), Some("Livre de sort - Sang pour sang"));
    assert_eq!(p.derived.get("tome"), Some(&1));
    // Commit: En attente, provenance with the parent.
    assist::commit(&mut entries, &p);
    assert_eq!(entries[1].prov, "rule:derive:scroll:WB_ConjureLich");
    assert_eq!(status::derive(&entries[1]), Status::Pending);
}

#[test]
fn untranslated_and_ambiguous_parents() {
    let entries = vec![
        line("SPEL", "ModFrost", 1, "Frost", ""),
        line("SCRL", "ModScrollFrost", 2, "Scroll of Frost", ""),
    ];
    // T-B21-05: the parent has no translation.
    let p = plan(&entries, Vec::new(), &NoBases, &HashMap::new());
    assert_eq!((p.filled(), p.untranslated), (0, 1));
    // Two open files translate « Frost » differently: ambiguous.
    let a = vec![line("SPEL", "AFrost", 1, "Frost", "Givre")];
    let b = vec![line("SPEL", "BFrost", 1, "Frost", "Gel")];
    let p = plan(&entries, vec![&a, &b], &NoBases, &HashMap::new());
    assert_eq!(p.filled(), 1);
    assert_eq!(p.ambiguous, 1);
    assert_eq!(p.changes[0].trans, "Parchemin - Givre");
    assert!(p.changes[0].prov.ends_with("|amb:2"));
}

#[test]
fn the_derived_lines_follow_their_parent() {
    // T-B21-04.
    let mut entries = vec![
        line("SPEL", "ModFireball", 1, "Fireball", "Boule de feu"),
        line("SCRL", "ModScroll1", 2, "Scroll of Fireball", ""),
        line("SCRL", "ModScroll2", 3, "Scroll of Fireball", ""),
    ];
    let p = plan(&entries, Vec::new(), &NoBases, &HashMap::new());
    assist::commit(&mut entries, &p);
    // The second one is validated: it no longer follows.
    status::set(&mut entries[2], Status::Validated);
    entries[0].trans = "Boule de feu majeure".into();
    let f = shipped();
    let a = assistant();
    let mut parents = Parents::default();
    parents.add(
        &entries,
        Origin::Document,
        &derive::parent_sigs(&f, "fra"),
        &|_, e| e.trans.clone(),
    );
    let teaches = HashMap::new();
    let d = Deriver {
        file: &f,
        lang: "fra",
        parents: &parents,
        parent_bases: &NoBases,
        assistant: &a,
        bases: &NoBases,
        teaches: &teaches,
    };
    let again = derive::refollow(&d, &entries, &[0]);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].0, 1);
    assert_eq!(again[0].1, "Parchemin - Boule de feu majeure");
    assert!(matches!(
        d.derive(&entries[1], Some("scroll")),
        Some(Derived::Text { .. })
    ));
}

#[test]
fn languages_and_validation() {
    let f = shipped();
    // T-B21-07: no target pattern for German: the rules are inactive.
    assert_eq!(f.active("deu").filter(|r| r.id == "scroll").count(), 0);
    assert_eq!(f.active("fra").filter(|r| r.id == "scroll").count(), 1);
    let mut bad = f.clone();
    bad.rules.push(Rule {
        id: "scroll".into(),
        ..f.rules[0].clone()
    });
    assert_eq!(bad.validate(), Err(Invalid::DuplicateId("scroll".into())));
    let mut bad = f.clone();
    bad.rules[0].target = "SCRLFULL".into();
    assert!(matches!(bad.validate(), Err(Invalid::Sig { .. })));
    let mut bad = f.clone();
    bad.rules[0].dst = PerLang::All("Parchemin - %B%".into());
    assert!(matches!(bad.validate(), Err(Invalid::VarMissing { .. })));
    let mut bad = f.clone();
    bad.rules[0].parent = Sigs::Many(vec!["SPEL:FULL".into(), "MGEF".into()]);
    assert!(bad.validate().is_err());
    // An invalid user's copy is not written.
    let dir = std::env::temp_dir().join(format!("ximod-b21-{}", std::process::id()));
    assert!(derive::save(&dir, "SkyrimSE", &bad).is_err());
    assert!(!dir.join(derive::file_name("SkyrimSE")).exists());
    derive::save(&dir, "SkyrimSE", &f).unwrap();
    let (back, user) =
        derive::load_file(Path::new("/nonexistent"), Some(&dir), "SkyrimSE").unwrap();
    assert!(user);
    assert_eq!(back, f);
    let _ = std::fs::remove_dir_all(&dir);
    // Every shipped file is valid.
    for g in ["Skyrim", "SkyrimSE", "Fallout4", "Oblivion", "Starfield"] {
        derive::shipped_file(Path::new("/nonexistent"), g)
            .unwrap()
            .validate()
            .unwrap();
    }
}

impl assist::Bases for Texts {
    fn lookup(&self, text: &str) -> Option<String> {
        self.0.get(text).cloned()
    }
}

#[test]
fn the_derivation_wins_over_the_pattern() {
    // The shipped pattern « Scroll of %A% » would translate the scroll by
    // the bases; the derivation takes it, with its parent and its strip.
    let entries = vec![
        line(
            "SPEL",
            "ModConjureLich",
            1,
            "Conjure Lich",
            "Invocation - Liche",
        ),
        line("SCRL", "ModScrollLich", 2, "Scroll of Conjure Lich", ""),
    ];
    let base = Texts(HashMap::from([(
        "Conjure Lich".to_string(),
        "Invocation - Liche".to_string(),
    )]));
    let f = shipped();
    let teaches = HashMap::new();
    let run = |derive: bool| {
        let ctx = DeriveCtx {
            file: &f,
            lang: "fra",
            open: Vec::new(),
            parent_bases: &NoBases,
            teaches: &teaches,
        };
        assist::plan_all(
            &entries,
            &[0, 1],
            &assistant(),
            &Options {
                derive,
                ..Options::default()
            },
            &base,
            Some(&ctx),
        )
    };
    let p = run(false);
    assert_eq!(p.patterned(), 1);
    assert_eq!(p.changes[0].trans, "Parchemin - Invocation - Liche");
    let p = run(true);
    assert_eq!((p.patterned(), p.derived_n(), p.filled()), (0, 1, 1));
    assert_eq!(p.changes[0].trans, "Parchemin - Liche");
    assert_eq!(p.changes[0].prov, "rule:derive:scroll:ModConjureLich");
}
