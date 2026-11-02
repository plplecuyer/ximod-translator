//! Dev tool: build a translation override patch (mode 3) from a plugin.
//!
//! Usage: `cargo run --example write_patch -- <plugin> <original_name> <out.esp>`
//! Every extractable string is translated to `FR:<source>` (a visible marker) so
//! the result can be loaded in xEdit / xTranslator to check structural validity.

use ximod_translator::plugin;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: write_patch <plugin> <original_name> <out.esp>");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1]).expect("read plugin");
    let parsed = plugin::parse(&data).expect("parse");
    let mut tm = plugin::extract_strings(&parsed, &args[2]);
    for e in &mut tm.entries {
        e.trans = format!("FR:{}", e.source);
    }
    let patch = plugin::write_override_esp(&data, &tm, &args[2]).expect("override");
    std::fs::write(&args[3], &patch).expect("write out");

    let re = plugin::parse(&patch).expect("re-parse patch");
    println!("source  : {} bytes, {} records", data.len(), parsed.records.len());
    println!("patch   : {} bytes, {} records", patch.len(), re.records.len());
    println!("masters : {:?}", re.masters);
    println!("game    : {}", plugin::extract_strings(&re, &args[2]).meta.game);
}
