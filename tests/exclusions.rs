//! B-18 — the exclusion rules: the shipped file, the learned rules and
//! their places, the rules off, the hand that wins, the deductions, the
//! editor's file, the suggestions (T-B18-01 … T-B18-10).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use ximod_translator::basedb::{self, BaseDb, BaseRow, HeaderExtra};
use ximod_translator::exclusions::{
    self, Choice, Compiled, Ctx, Deduced, Dirs, Invalid, Kind, Plan, Rule, RulesFile, Scope,
    Suggestion,
};
use ximod_translator::provenance::{self, Prov, ProvFilter};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

fn line(rec: &str, field: &str, edid: &str, src: &str) -> TmEntry {
    TmEntry {
        rec_sig: rec.into(),
        field_sig: field.into(),
        edid: edid.into(),
        formid: 0x800 + src.len() as u32,
        source: src.into(),
        index: 1,
        ..Default::default()
    }
}

fn plan(file: &RulesFile, local: &[Rule], off: &[&str], deduced: Deduced) -> Plan {
    Plan {
        compiled: Compiled::new(file, local, "Fallout4"),
        deduced: Arc::new(deduced),
        off: off.iter().map(|s| s.to_string()).collect(),
        game: "Fallout4".into(),
        path: "/mods/Test.esp".into(),
        facts: None,
    }
}

fn armors() -> Vec<TmEntry> {
    let mut es: Vec<TmEntry> = (0..12)
        .map(|i| {
            line(
                "ARMO",
                "FULL",
                &format!("zzzTpl{i}"),
                "zzz TEMPLATE - DO NOT USE",
            )
        })
        .collect();
    es.push(line("ARMO", "FULL", "zzzBlade1", "zzz Test Blade"));
    es.push(line("ARMO", "FULL", "zzzBlade2", "zzz Test Blade"));
    es.push(line("ARMO", "FULL", "IronHelmet", "Iron Helmet"));
    es.push(line("MESG", "ITXT", "Btn", "Next"));
    es
}

/// T-B18-02: F8 on a template proposes the exact text (11 others) and
/// the prefix « zzz » (13 others); the local rule locks the 11 lines,
/// travels in the header of the mod's dictionary, and the origin line keeps
/// its own state. The generalization by field is refused on ARMO:FULL.
#[test]
fn generalization_and_learned_rules() {
    let file = RulesFile::embedded();
    let mut es = armors();
    status::set(&mut es[0], Status::Ignored);
    let p = exclusions::propose(&es, 0, &file).unwrap();
    assert_eq!(
        (p.same_text, p.prefix.clone()),
        (11, Some(("zzz".into(), 13)))
    );
    assert!(p.displayed, "ARMO:FULL is shown in game");
    assert!(exclusions::learned_rule(&p, &Choice::FieldLocal, "x", "d", "Fallout4", &[]).is_none());
    let rule = exclusions::learned_rule(
        &p,
        &Choice::TextLocal,
        "zzzTpl0",
        "2026-10-28",
        "Fallout4",
        &[],
    )
    .unwrap();
    assert_eq!(rule.learned.as_ref().unwrap().from, "zzzTpl0");
    let pl = plan(&file, std::slice::from_ref(&rule), &[], Deduced::default());
    pl.release(&mut es);
    assert_eq!(pl.lay(&mut es), 11);
    let code = format!("lock:{}", rule.id);
    assert_eq!(es.iter().filter(|e| e.prov == code).count(), 11);
    assert!(es[0].prov.is_empty(), "the origin line is not the rule's");
    assert_eq!(ProvFilter::of(&es[1]), Some(ProvFilter::Locked));
    assert!(matches!(provenance::parse(&es[1].prov), Prov::Lock(r) if r == rule.id));
    // The header of the mod's dictionary keeps the local rule and the off.
    let dir = std::env::temp_dir().join(format!("ximod-b18-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dict = dir.join("Test_ENG-FRA.ximdb");
    let extra = HeaderExtra {
        rules: Some(vec![rule.clone()]),
        rules_off: Some(vec!["tag-only".into()]),
        ..Default::default()
    };
    let rows = vec![BaseRow {
        rec: "ARMO".into(),
        field: "FULL".into(),
        formid: 1,
        edid: "IronHelmet".into(),
        source: "Iron Helmet".into(),
        trans: "Casque de fer".into(),
        ..Default::default()
    }];
    basedb::write_ximdb_with(rows, Vec::new(), "fra", &dict, 0, None, &extra).unwrap();
    let back = basedb::peek_extra(&dict);
    assert_eq!(back.rules, Some(vec![rule.clone()]));
    assert_eq!(back.rules_off, Some(vec!["tag-only".to_string()]));
    assert_eq!(basedb::read_rows(&dict).unwrap().1.len(), 1);
    // Reopened: the rule acts again.
    let mut again = armors();
    status::set(&mut again[0], Status::Ignored);
    let pl = plan(&file, &back.rules.unwrap(), &[], Deduced::default());
    assert_eq!(pl.lay(&mut again), 11);
    // The prefix, global: 13 lines, on another mod too.
    let g = exclusions::learned_rule(&p, &Choice::PrefixGlobal, "zzzTpl0", "d", "Fallout4", &[])
        .unwrap();
    assert!(exclusions::is_global(&Choice::PrefixGlobal));
    let mut f2 = file.clone();
    f2.rules.push(g);
    let mut other = armors();
    let pl = plan(&f2, &[], &[], Deduced::default());
    assert_eq!(pl.lay(&mut other), 14);
    let _ = std::fs::remove_dir_all(dir);
}

/// T-B18-04 / T-B18-05: a rule off unlocks its lines (À traduire, for the
/// dictionaries); a line unlocked by hand is never locked again.
#[test]
fn rules_off_and_the_hand() {
    let file = RulesFile::embedded();
    let mut es = vec![
        line("WEAP", "FULL", "A", "xCACO_FireGrenadeEffect01Impact"),
        line("WEAP", "FULL", "B", "ITMPotionUse"),
        line("WEAP", "FULL", "C", "Iron Sword"),
    ];
    let pl = plan(&file, &[], &[], Deduced::default());
    assert_eq!(pl.lay(&mut es), 2);
    let ids: HashSet<String> = ["ident-text".to_string()].into();
    let back = exclusions::unlock_rules(&mut es, &ids);
    assert_eq!(back, vec![0, 1]);
    assert!(es.iter().all(|e| status::derive(e) == Status::ToTranslate));
    let pl = plan(&file, &[], &["ident-text"], Deduced::default());
    assert_eq!(pl.lay(&mut es), 0);
    // Back on, then a status by hand on one line: it stays free.
    let pl = plan(&file, &[], &[], Deduced::default());
    pl.lay(&mut es);
    status::set(&mut es[1], Status::Pending);
    ximod_translator::vmad_lock::status_by_hand(&mut es[1]);
    assert_eq!(es[1].prov, provenance::TYPED);
    pl.release(&mut es);
    pl.lay(&mut es);
    assert_eq!(status::derive(&es[0]), Status::Ignored);
    assert_eq!(status::derive(&es[1]), Status::Pending);
    let c = exclusions::counts(&es, &pl.compiled, &pl.ctx());
    assert_eq!(c.locked, 1);
    assert_eq!(c.hand, 1);
    assert_eq!(c.per_rule.get("ident-text").map(|x| x.0), Some(1));
}

/// T-B18-06: the deductions — a script string that is an EDID of the
/// plugin, a key of its interface files; never a displayed field.
#[test]
fn deductions() {
    let file = RulesFile::embedded();
    let mut es = vec![
        line("QUST", "VMAD", "MyQuest", "Bandit Chief"),
        line("NPC_", "FULL", "Bandit Chief", "Bandit Chief"),
        line("QUST", "VMAD", "MyQuest", "Bandit leader is here"),
    ];
    let dir = std::env::temp_dir().join(format!("ximod-b18d-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Interface/Translations")).unwrap();
    let iface = dir.join("Interface/Translations/Test_english.txt");
    std::fs::write(&iface, "$Akaviri Samurai Armor\tAkaviri Samurai Armor\n").unwrap();
    es.push(line("QUST", "VMAD", "MyQuest", "$Akaviri Samurai Armor"));
    let d = Deduced::of(
        &es,
        Some(&dir.join("Test.esp")),
        std::slice::from_ref(&iface),
    );
    assert!(d.edids.contains("bandit chief"));
    assert!(d.keys.contains("$Akaviri Samurai Armor"));
    let pl = plan(&file, &[], &[], d);
    pl.lay(&mut es);
    assert_eq!(es[0].prov, "lock:edid");
    assert_eq!(
        status::derive(&es[1]),
        Status::ToTranslate,
        "a FULL is never locked by an EDID"
    );
    assert!(es[2].prov.is_empty());
    // The key rule comes first; ifkey counts it too.
    assert_eq!(es[3].prov, "lock:key");
    let c = exclusions::counts(&es, &pl.compiled, &pl.ctx());
    assert_eq!(c.per_rule.get("ifkey").map(|x| x.0), Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

/// T-B18-08: the editor's file — an invalid rule is never written, the
/// user's copy wins, the restoration finds the shipped rules.
#[test]
fn the_users_file() {
    let tmp = std::env::temp_dir().join(format!("ximod-b18f-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let dirs = Dirs {
        shipped: Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/rules"),
        user: Some(tmp.clone()),
    };
    let (mut f, src, err) = dirs.load();
    assert_eq!((src, err), (exclusions::Source::Shipped, None));
    f.rules
        .push(Rule::new("broken", Kind::Regex, "(a", vec![Scope::Text]));
    assert!(matches!(dirs.save(&f), Err(Invalid::Regex { id, .. }) if id == "broken"));
    assert!(!dirs.user_path().unwrap().exists());
    f.rules.pop();
    f.rules
        .iter_mut()
        .find(|r| r.id == "noletter")
        .unwrap()
        .enabled = false;
    f.rules.push(Rule::new(
        "mine",
        Kind::Field,
        "WEAP:MODL",
        vec![Scope::Field],
    ));
    dirs.save(&f).unwrap();
    let (g, src, _) = dirs.load();
    assert_eq!(src, exclusions::Source::User);
    let c = Compiled::new(&g, &[], "SkyrimSE");
    let d = Deduced::default();
    let off = HashSet::new();
    let ctx = Ctx {
        game: "SkyrimSE",
        path: "x.esp",
        deduced: &d,
        off: &off,
        facts: None,
    };
    assert_eq!(c.lock_of(&line("WEAP", "FULL", "A", "100"), &ctx), None);
    assert_eq!(
        c.lock_of(&line("WEAP", "MODL", "A", "Weapons\\Iron Sword"), &ctx)
            .as_deref(),
        Some("mine")
    );
    dirs.restore().unwrap();
    assert!(dirs.load().0.find("noletter").unwrap().enabled);
    // A copy broken by hand: the shipped rules, and the error said.
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(dirs.user_path().unwrap(), "{ \"format\": 1, \"rules\": [ ").unwrap();
    let (f, src, err) = dirs.load();
    assert_eq!(src, exclusions::Source::Shipped);
    assert!(err.is_some());
    assert!(f.find("key").is_some());
    let _ = std::fs::remove_dir_all(tmp);
}

/// T-B18-09: the suggestions — a field the game's base never translates,
/// a group of lines Ignoré by hand.
#[test]
fn suggestions() {
    let mut base = BaseDb::default();
    let rows: Vec<BaseRow> = (0..60)
        .map(|i| BaseRow {
            rec: "WEAP".into(),
            field: "MODL".into(),
            formid: i,
            edid: format!("W{i}"),
            source: format!("Weapons\\Model{i}.nif"),
            trans: format!("Weapons\\Model{i}.nif"),
            ..Default::default()
        })
        .collect();
    base.add_rows(&rows);
    let es = vec![line("WEAP", "MODL", "A", "Weapons\\Mine.nif")];
    let s = exclusions::suggest_fields(&es, [&base]);
    assert_eq!(s, vec![Suggestion::FieldBase("WEAP:MODL".into(), 60)]);
    let mut es: Vec<TmEntry> = (0..10)
        .map(|i| {
            line(
                "STAT",
                "FULL",
                &format!("S{i}"),
                &format!("Bld Deco Stairs{i}"),
            )
        })
        .collect();
    for e in &mut es {
        status::set(e, Status::Ignored);
    }
    let s = exclusions::suggest_groups(&es, &[]);
    assert!(s.iter().any(|x| matches!(x, Suggestion::Group(r, 10) if r.kind == Kind::Field && r.pattern == "STAT:FULL")));
    assert!(
        s.iter()
            .any(|x| matches!(x, Suggestion::Group(r, 10) if r.pattern == "^Bld\\s"))
    );
}

/// T-B18-10: 50 000 lines, twenty rules — under 200 ms (measured in
/// release; the debug build only reports it).
#[test]
fn cost() {
    let mut file = RulesFile::embedded();
    for k in 0..8 {
        file.rules.push(Rule::new(
            &format!("extra{k}"),
            Kind::Regex,
            &format!("^zz{k}"),
            vec![Scope::Text],
        ));
    }
    let mut es: Vec<TmEntry> = (0..50_000)
        .map(|i| {
            line(
                "WEAP",
                "FULL",
                &format!("E{i}"),
                &format!("Some weapon number {i}"),
            )
        })
        .collect();
    let pl = plan(&file, &[], &[], Deduced::default());
    let t = std::time::Instant::now();
    pl.lay(&mut es);
    let el = t.elapsed();
    eprintln!("rules on 50 000 lines: {el:?}");
    if !cfg!(debug_assertions) {
        assert!(el.as_millis() < 200, "{el:?}");
    }
}
