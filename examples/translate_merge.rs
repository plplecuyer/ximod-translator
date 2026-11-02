//! Dev tool: extract a plugin's strings, apply a dictionary, report the match.
//!
//! Usage: `cargo run --example translate_merge -- <plugin.esp> <dict.{sst,eet,xml,json,dsd}>`

use std::path::Path;

use ximod_translator::tm::apply::{MergeOptions, merge};
use ximod_translator::{plugin, tm};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(esp), Some(dict_path)) = (args.next(), args.next()) else {
        eprintln!("usage: translate_merge <plugin.esp> <dict.{{sst,eet,xml,json,dsd}}>");
        std::process::exit(2);
    };

    let mut plug = plugin::read_file(Path::new(&esp)).expect("parse plugin");
    let dict = tm::load(Path::new(&dict_path)).expect("load dictionary");

    println!("plugin  : {} chaînes", plug.entries.len());
    println!("dico    : {} entrées ({} langue(s) {}->{})",
        dict.entries.len(), dict.meta.source_format, dict.meta.lang_source, dict.meta.lang_dest);

    let report = merge(&mut plug, &dict, MergeOptions::default());

    println!("\n== correspondance ==");
    println!("  total       : {}", report.total);
    println!("  appariées   : {} ({:.1}%)", report.matched, report.ratio() * 100.0);
    println!("    par FormID: {}", report.by_formid);
    println!("    par EDID  : {}", report.by_edid);
    println!("    par contexte (rec,champ,source): {}", report.by_context);
    println!("    par source seule: {}", report.by_source);
    println!("  effectives (trad ≠ source): {}", report.effective);
    println!("  non appariées: {}", report.unmatched());

    println!("\nexemples appariés (trad ≠ source):");
    for e in plug.entries.iter().filter(|e| e.is_translated()).take(8) {
        let s: String = e.source.chars().take(32).collect();
        let t: String = e.trans.chars().take(32).collect();
        println!("  [{}:{}] {:?} -> {:?}", e.rec_sig, e.field_sig, s, t);
    }

    let unmatched: Vec<_> = plug.entries.iter().filter(|e| e.trans.is_empty()).collect();
    if !unmatched.is_empty() {
        println!("\nexemples NON appariés:");
        for e in unmatched.iter().take(8) {
            let s: String = e.source.chars().take(50).collect();
            println!("  [{}:{}] {:?}", e.rec_sig, e.field_sig, s);
        }
    }
}
