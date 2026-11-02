//! Dev tool: parse a Bethesda plugin and print extraction statistics.
//!
//! Usage: `cargo run --example dump_plugin -- <path.esp|.esm|.esl>`

use std::collections::BTreeMap;

use ximod_translator::plugin;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: dump_plugin <path.esp|.esm|.esl>");
        std::process::exit(2);
    };
    let data = std::fs::read(&path).expect("read plugin");
    let parsed = plugin::parse(&data).expect("parse plugin");

    println!("== {path} ==");
    println!("records          : {}", parsed.records.len());
    println!("masters          : {:?}", parsed.masters);
    println!("localized        : {}", parsed.localized);
    println!("author           : {:?}", parsed.author);

    let tm = plugin::extract_strings(&parsed, "CheatTerminal.esm");
    println!("game (inferred)  : {}", tm.meta.game);
    println!("translatable str : {}", tm.entries.len());

    // Breakdown by (record, field).
    let mut by_field: BTreeMap<String, usize> = BTreeMap::new();
    for e in &tm.entries {
        *by_field
            .entry(format!("{}:{}", e.rec_sig, e.field_sig))
            .or_default() += 1;
    }
    println!("\nby (record:field):");
    for (k, n) in &by_field {
        println!("  {k:10} {n}");
    }

    println!("\nsamples:");
    for e in tm.entries.iter().filter(|e| !e.source.is_empty()).take(12) {
        let src: String = e.source.chars().take(60).collect();
        println!("  [{}:{}] {} = {:?}", e.rec_sig, e.field_sig, e.edid, src);
    }

    // Optional: dump the extracted strings as a pivot JSON (plugin -> JSON).
    if let Some(out) = std::env::args().nth(2) {
        ximod_translator::tm::save(&tm, std::path::Path::new(&out)).expect("save json");
        println!("\nwrote pivot JSON: {out}");
    }
}
