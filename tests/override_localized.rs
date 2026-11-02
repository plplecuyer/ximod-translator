//! S-01 — the override patch of a localized plugin is written delocalized:
//! translated records only, texts inline, localized flag removed, original as
//! last master, unresolved ids counted, ESL flag by eligibility. Checked on a
//! synthetic Skyrim SE plugin and on the same layout at Starfield's form
//! version; STRINGS file naming follows each game's convention (R-S01-04).

use ximod_translator::games::{Game, LangStyle};
use ximod_translator::plugin::{self, OverrideOptions, StringsBundle};
use ximod_translator::tm::strings::{self, StringsKind};

const LOCALIZED: u32 = 0x0000_0080;
const ESL: u32 = 0x0000_0200;

fn sub_u16(sig: &str, data: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(data.len() as u16).to_le_bytes());
    v.extend_from_slice(data);
    v
}
fn zstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}
fn rec24(sig: &str, formid: u32, flags: u32, form_version: u16, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes());
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&formid.to_le_bytes());
    v.extend_from_slice(&[0u8; 4]);
    v.extend_from_slice(&form_version.to_le_bytes());
    v.extend_from_slice(&[0u8; 2]);
    v.extend_from_slice(subs);
    v
}
fn top_grup(label: &str, body: &[u8]) -> Vec<u8> {
    let mut v = b"GRUP".to_vec();
    v.extend_from_slice(&((24 + body.len()) as u32).to_le_bytes());
    v.extend_from_slice(label.as_bytes());
    v.extend_from_slice(&0i32.to_le_bytes());
    v.extend_from_slice(&[0u8; 8]);
    v.extend_from_slice(body);
    v
}

/// A localized plugin with two weapons (ids 1/2 and 3/4) and one armour
/// (id 5) whose DESC id 6 has no STRINGS entry.
fn localized_plugin(form_version: u16, master: &str) -> Vec<u8> {
    let tes4 = {
        let mut s = sub_u16("HEDR", &[0u8; 12]);
        s.extend(sub_u16("CNAM", &zstr("Author")));
        s.extend(sub_u16("MAST", &zstr(master)));
        rec24("TES4", 0, LOCALIZED, form_version, &s)
    };
    let weap = |formid: u32, edid: &str, full: u32, desc: u32| {
        let mut s = sub_u16("EDID", &zstr(edid));
        s.extend(sub_u16("FULL", &full.to_le_bytes()));
        s.extend(sub_u16("DESC", &desc.to_le_bytes()));
        rec24("WEAP", formid, 0, form_version, &s)
    };
    let armo = {
        let mut s = sub_u16("EDID", &zstr("LeatherHat"));
        s.extend(sub_u16("FULL", &5u32.to_le_bytes()));
        s.extend(sub_u16("DESC", &6u32.to_le_bytes()));
        rec24("ARMO", 0x0100_0900, 0, form_version, &s)
    };
    let mut src = tes4;
    let mut weaps = weap(0x0100_0801, "IronSword", 1, 2);
    weaps.extend(weap(0x0100_0802, "SteelSword", 3, 4));
    src.extend(top_grup("WEAP", &weaps));
    src.extend(top_grup("ARMO", &armo));
    src
}

fn bundle() -> StringsBundle {
    let strings = strings::write(
        &[
            (1, "Iron Sword".into()),
            (3, "Steel Sword".into()),
            (5, "Leather Hat".into()),
        ],
        StringsKind::Strings,
        true,
    );
    let dl = strings::write(
        &[(2, "A basic sword.".into()), (4, "A better sword.".into())],
        StringsKind::DlStrings,
        true,
    );
    StringsBundle::from_files(Some(&strings), Some(&dl), None, true).unwrap()
}

fn check_game(form_version: u16, master: &str, expected: Game) {
    let src = localized_plugin(form_version, master);
    let parsed = plugin::parse(&src).unwrap();
    assert!(parsed.localized);
    assert_eq!(parsed.game, expected);
    let mut tm = plugin::extract_strings(&parsed, "MyMod.esm");
    let b = bundle();
    b.resolve(&mut tm);
    // Translate IronSword FULL and LeatherHat FULL; SteelSword untouched.
    for e in &mut tm.entries {
        match (e.edid.as_str(), e.field_sig.as_str()) {
            ("IronSword", "FULL") => e.trans = "Épée de fer".into(),
            ("LeatherHat", "FULL") => e.trans = "Chapeau de cuir".into(),
            _ => {}
        }
    }

    // Without the bundle: refused with a clear message.
    let err = plugin::write_override_esp(&src, &tm, "MyMod.esm")
        .unwrap_err()
        .to_string();
    assert!(err.contains("err-write-no-strings"), "{err}");

    // With the bundle, ESL asked.
    let opts = OverrideOptions {
        bundle: Some(&b),
        esl_if_eligible: true,
    };
    let plan = plugin::plan_override(&src, &tm, &opts).unwrap();
    assert!(plan.localized);
    assert_eq!(plan.records, 2, "two records carry an exported field");
    assert_eq!(
        plan.unresolved,
        vec![("ARMO".to_string(), "DESC".to_string(), 6)]
    );
    assert!(plan.esl_eligible && plan.esl);

    let (out, report) = plugin::write_override_esp_with(&src, &tm, "MyMod.esm", &opts).unwrap();
    assert_eq!(report, plan);
    let re = plugin::parse(&out).unwrap();
    assert!(!re.localized, "localized flag removed (R-S01-01)");
    assert_eq!(
        re.masters,
        vec![master.to_string(), "MyMod.esm".to_string()],
        "original as last master"
    );
    assert!(
        re.records
            .iter()
            .any(|r| r.sig == "TES4" && r.flags & ESL != 0),
        "ESL flag set (R-S01-03)"
    );
    // Re-extract WITHOUT STRINGS: the texts are inline.
    let tm2 = plugin::extract_strings(&re, "patch");
    let get = |edid: &str, fld: &str| {
        tm2.entries
            .iter()
            .find(|e| e.edid == edid && e.field_sig == fld)
            .map(|e| e.source.clone())
    };
    assert_eq!(
        get("IronSword", "FULL").as_deref(),
        Some("Épée de fer"),
        "exported field: translation"
    );
    assert_eq!(
        get("IronSword", "DESC").as_deref(),
        Some("A basic sword."),
        "other text field: resolved original"
    );
    assert_eq!(
        get("LeatherHat", "FULL").as_deref(),
        Some("Chapeau de cuir")
    );
    assert!(
        get("LeatherHat", "DESC").is_none(),
        "unresolved id written empty, hence not a line"
    );
    assert!(
        get("SteelSword", "FULL").is_none(),
        "untranslated record absent from the patch"
    );
    assert!(
        tm2.entries.iter().all(|e| e.idstexte < 0),
        "no string id left"
    );

    // ESL not asked: no flag.
    let opts2 = OverrideOptions {
        bundle: Some(&b),
        esl_if_eligible: false,
    };
    let (out2, r2) = plugin::write_override_esp_with(&src, &tm, "MyMod.esm", &opts2).unwrap();
    assert!(!r2.esl);
    let re2 = plugin::parse(&out2).unwrap();
    assert!(
        re2.records
            .iter()
            .any(|r| r.sig == "TES4" && r.flags & ESL == 0)
    );
}

#[test]
fn skyrim_se_localized_override_patch() {
    check_game(44, "Skyrim.esm", Game::SkyrimSE);
}

#[test]
fn starfield_localized_override_patch() {
    check_game(555, "Starfield.esm", Game::Starfield);
}

#[test]
fn non_localized_override_is_unchanged() {
    // The existing behaviour on the Skyrim LE fixture: plain patch, no ESL
    // (the game has no light plugins).
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/skyrimle_riverwood.esp");
    let src = std::fs::read(&path).unwrap();
    let mut tm = plugin::read_file(&path).unwrap();
    for e in tm.entries.iter_mut().filter(|e| e.formid != 0) {
        e.trans = format!("FR {}", e.source);
    }
    let opts = OverrideOptions {
        bundle: None,
        esl_if_eligible: true,
    };
    let (out, r) =
        plugin::write_override_esp_with(&src, &tm, "skyrimle_riverwood.esp", &opts).unwrap();
    assert!(!r.localized && !r.esl_eligible && !r.esl && r.unresolved.is_empty());
    let legacy = plugin::write_override_esp(&src, &tm, "skyrimle_riverwood.esp").unwrap();
    assert_eq!(
        out, legacy,
        "the compatibility entry point writes the same bytes"
    );
}

#[test]
fn strings_suffix_follows_the_game() {
    use ximod_translator::lang::{strings_suffix, strings_suffix_for};
    assert_eq!(strings_suffix(LangStyle::Name, "fra"), "French");
    assert_eq!(strings_suffix(LangStyle::Iso2, "fra"), "fr");
    assert_eq!(strings_suffix(LangStyle::Iso2, "fr"), "fr");
    assert_eq!(strings_suffix(LangStyle::Name, "xyz"), "xyz");
    assert_eq!(strings_suffix_for(Game::SkyrimSE, "deu"), "German");
    assert_eq!(strings_suffix_for(Game::Fallout4, "fra"), "French");
    assert_eq!(strings_suffix_for(Game::Starfield, "fra"), "fr");
    // The shipped sheet says so too.
    let reg = ximod_translator::games::registry();
    assert_eq!(
        reg.spec(Game::Starfield).strings_lang_style,
        LangStyle::Iso2
    );
    assert_eq!(reg.spec(Game::SkyrimSE).strings_lang_style, LangStyle::Name);
}
