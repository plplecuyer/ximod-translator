//! C-07 on the recette plugins (not in the repository, skipped when
//! absent): the messages of the Oblivion and New Vegas scripts, found in
//! the compiled form, translated and written in the source and the
//! compiled form together, the compiled scripts still well formed.

use std::path::{Path, PathBuf};

use ximod_translator::plugin::{self, sctx};
use ximod_translator::status::{self, Status};

fn recette(rel: &str) -> Option<PathBuf> {
    let p = Path::new("/root/ximod_work/recette_lot3").join(rel);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("skipped: {} absent", p.display());
        None
    }
}

/// Every compiled script of a plugin is well formed.
fn compiled_ok(bytes: &[u8]) -> usize {
    let p = plugin::parse(bytes).unwrap();
    let mut n = 0;
    for r in &p.records {
        for s in r.subrecords.iter().filter(|s| s.sig == "SCDA") {
            assert!(sctx::well_formed(&s.data), "{} {:08X}", r.sig, r.formid);
            n += 1;
        }
    }
    n
}

#[test]
fn messages_translated_in_source_and_compiled() {
    let cases = [
        (
            "Lot 3-3/x_Tutorial_-_MessageBox-47148-1-0.7z/Data/kvTutMessages01.esp",
            0,
        ),
        (
            "Lot 3-3/x_FTSC_Fast_Travel_Stops_Combat_MenuQue_XO/FTSC - Fast Travel Stops Combat - MenuQue - xOBSE.esm",
            0,
        ),
        ("Lot 3-3/x_fonv/_FONV_Portableinterfaces.esp", 0),
        (
            "Lot 3-4/x_MOO493revision4-2-47698-rev4-2-155827992/Maskar's Oblivion Overhaul.esp",
            5,
        ),
    ];
    for (rel, max_missing) in cases {
        let Some(path) = recette(rel) else {
            continue;
        };
        let src = std::fs::read(&path).unwrap();
        let n_scda = compiled_ok(&src);
        let mut tm = plugin::read_file(&path).unwrap();
        let lines: Vec<usize> = tm
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.field_sig == "SCTX")
            .map(|(i, _)| i)
            .collect();
        assert!(!lines.is_empty(), "{rel}");
        let missing = lines
            .iter()
            .filter(|&&i| tm.entries[i].prov == "lock:compiled")
            .count();
        assert!(missing <= max_missing, "{rel}: {missing} not found");
        // Without change: identical.
        assert_eq!(plugin::write_full_esp(&src, &tm).unwrap(), src, "{rel}");
        // Every line translated (longer and shorter).
        for (k, &i) in lines.iter().enumerate() {
            let e = &mut tm.entries[i];
            e.trans = if k % 2 == 0 {
                format!("{} [traduit]", e.source)
            } else {
                "T".to_string()
            };
            status::set(e, Status::Validated);
        }
        let out = plugin::write_full_esp(&src, &tm).unwrap();
        assert_eq!(compiled_ok(&out), n_scda, "{rel}");
        let back = plugin::extract_strings(&plugin::parse(&out).unwrap(), "x");
        let got: Vec<(&str, &str)> = back
            .entries
            .iter()
            .filter(|e| e.field_sig == "SCTX")
            .map(|e| (e.source.as_str(), e.prov.as_str()))
            .collect();
        assert_eq!(got.len(), lines.len(), "{rel}");
        for (k, (s, prov)) in got.iter().enumerate() {
            let want = &tm.entries[lines[k]].trans;
            assert_eq!(s, want, "{rel} line {k}");
            if tm.entries[lines[k]].prov != "lock:compiled" {
                assert_ne!(
                    *prov, "lock:compiled",
                    "{rel}: {s} lost in the compiled form"
                );
            }
        }
        eprintln!(
            "{rel}: {} lines ({missing} not compiled), {n_scda} compiled scripts",
            lines.len()
        );
    }
}
