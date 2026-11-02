//! End-to-end test of the auto-translation database pipeline: write real pivot
//! JSON bases to two folders, scan them, auto-translate a mod, and consolidate
//! the game folder into a native XIMOD base.

use std::path::Path;

use ximod_translator::basedb::{self, BaseDb};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::{self, TmEntry, TranslationMemory};

fn entry(rec: &str, field: &str, formid: u32, edid: &str, src: &str, tr: &str) -> TmEntry {
    TmEntry {
        rec_sig: rec.into(),
        field_sig: field.into(),
        formid,
        edid: edid.into(),
        source: src.into(),
        trans: tr.into(),
        ..Default::default()
    }
}

fn write_base(path: &Path, entries: Vec<TmEntry>) {
    let tm = TranslationMemory {
        meta: Default::default(),
        entries,
        history: Vec::new(),
    };
    tm::save(&tm, path).expect("save base json");
}

#[test]
fn scan_auto_translate_and_compile() {
    let root = std::env::temp_dir().join("ximod_basedb_test");
    let game = root.join("jeux");
    let mods = root.join("mods");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::create_dir_all(&mods).unwrap();

    // Game base: vanilla weapon + book.
    write_base(
        &game.join("skyrim.json"),
        vec![
            entry(
                "WEAP",
                "FULL",
                0x111,
                "IronSword",
                "Iron Sword",
                "Épée en fer",
            ),
            entry("BOOK", "DESC", 0x222, "Note", "A note.", "Une note."),
        ],
    );
    // Mod base: overrides the weapon translation for the same key.
    write_base(
        &mods.join("mymod.json"),
        vec![entry(
            "WEAP",
            "FULL",
            0x111,
            "IronSword",
            "Iron Sword",
            "Épée de fer (mod)",
        )],
    );

    let dirs = vec![game.clone(), mods.clone()];
    // Raw bases carry no language tag → loaded regardless of the target language.
    let db = BaseDb::scan(&dirs, "fra");
    assert_eq!(db.files(), 2);

    // A fresh mod to translate: the sword (matches, mod wins), the note (matches
    // game), a changed line (same key, different source → not translated), and a
    // brand-new line (no match).
    let mut doc = TranslationMemory {
        meta: Default::default(),
        entries: vec![
            entry("WEAP", "FULL", 0x111, "IronSword", "Iron Sword", ""),
            entry("BOOK", "DESC", 0x222, "Note", "A note.", ""),
            entry("WEAP", "FULL", 0x111, "IronSword", "Steel Sword", ""), // changed text
            entry("MISC", "FULL", 0x999, "Gold", "Gold", ""),             // no match
        ],
        history: Vec::new(),
    };
    let n = db.auto_translate(&mut doc);
    assert_eq!(n, 2, "only the two unchanged, matching lines");
    assert_eq!(doc.entries[0].trans, "Épée de fer (mod)"); // mod overrode game
    assert_eq!(status::derive(&doc.entries[0]), Status::Validated);
    assert_eq!(doc.entries[1].trans, "Une note.");
    assert!(doc.entries[2].trans.is_empty()); // changed source untouched
    assert!(doc.entries[3].trans.is_empty()); // no match

    // Consolidate the game folder into a compact language-tagged XIMOD base.
    let out = game.join("base_jeux_fra.ximdb");
    let (files, entries) = basedb::compile_dir(&game, "fra", &out).expect("compile");
    assert_eq!(files, 1); // one raw game file (the .ximdb output is not re-read)
    assert_eq!(entries, 2);
    assert!(out.is_file());

    // Scanning the game folder now prefers the compiled .ximdb base — and only
    // when its language matches the requested one.
    let db2 = BaseDb::scan(std::slice::from_ref(&game), "fra");
    let mut doc2 = TranslationMemory {
        meta: Default::default(),
        entries: vec![entry("BOOK", "DESC", 0x222, "Note", "A note.", "")],
        history: Vec::new(),
    };
    assert_eq!(db2.auto_translate(&mut doc2), 1);
    assert_eq!(doc2.entries[0].trans, "Une note.");

    // A different target language must NOT pick up the French base.
    let db3 = BaseDb::scan(std::slice::from_ref(&game), "deu");
    let mut doc3 = TranslationMemory {
        meta: Default::default(),
        entries: vec![entry("BOOK", "DESC", 0x222, "Note", "A note.", "")],
        history: Vec::new(),
    };
    assert_eq!(db3.auto_translate(&mut doc3), 0);

    // Per-file compilation: each raw base gets its own sibling `.ximdb`.
    std::fs::remove_file(&out).unwrap(); // drop the merged base first
    let (files, total) = basedb::compile_dir_per_file(&game, "fra").expect("per-file");
    assert_eq!(files, 1);
    assert_eq!(total, 2);
    assert!(game.join("skyrim.ximdb").is_file());

    // `from_file`: apply one specific base — a compiled `.ximdb`…
    let db_ximdb = BaseDb::from_file(&game.join("skyrim.ximdb")).expect("from ximdb");
    let mut d4 = TranslationMemory {
        meta: Default::default(),
        entries: vec![entry("WEAP", "FULL", 0x111, "IronSword", "Iron Sword", "")],
        history: Vec::new(),
    };
    assert_eq!(db_ximdb.auto_translate(&mut d4), 1);
    // …and a raw base (JSON here, same path used for EET/SST).
    let db_raw = BaseDb::from_file(&mods.join("mymod.json")).expect("from raw");
    let mut d5 = TranslationMemory {
        meta: Default::default(),
        entries: vec![entry("WEAP", "FULL", 0x111, "IronSword", "Iron Sword", "")],
        history: Vec::new(),
    };
    assert_eq!(db_raw.auto_translate(&mut d5), 1);

    let _ = std::fs::remove_dir_all(&root);
}
