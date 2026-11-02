//! End-to-end DSD output: extract a Skyrim SE plugin's strings, translate them,
//! and emit a Dynamic String Distributor JSON. Proves the `form_id` owner
//! resolution — the mod's own records point at the mod, base-game overrides point
//! at the master — which is what makes a DSD load without redistributing the mod.

use serde_json::Value;
use ximod_translator::plugin;
use ximod_translator::tm::dsd;

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
fn rec24(sig: &str, formid: u32, subs: &[u8]) -> Vec<u8> {
    let mut v = sig.as_bytes().to_vec();
    v.extend_from_slice(&(subs.len() as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&formid.to_le_bytes());
    v.extend_from_slice(&[0u8; 4]);
    v.extend_from_slice(&44u16.to_le_bytes()); // Skyrim SE
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

#[test]
fn dsd_resolves_owner_plugin_per_record() {
    // Non-localized SSE plugin, master Skyrim.esm. One own record (high byte 1)
    // and one base-game override (high byte 0).
    let tes4 = {
        let mut s = sub_u16("HEDR", &[0u8; 12]);
        s.extend(sub_u16("MAST", &zstr("Skyrim.esm")));
        rec24("TES4", 0, &s)
    };
    let own = {
        let mut s = sub_u16("EDID", &zstr("MyBlade"));
        s.extend(sub_u16("FULL", &zstr("My Blade")));
        rec24("WEAP", 0x0100_0801, &s)
    };
    let over = {
        let mut s = sub_u16("EDID", &zstr("IronSword"));
        s.extend(sub_u16("FULL", &zstr("Iron Sword")));
        rec24("WEAP", 0x0001_3986, &s)
    };
    let mut body = own;
    body.extend(over);
    let mut src = tes4;
    src.extend(top_grup("WEAP", &body));

    let parsed = plugin::parse(&src).unwrap();
    let mut tm = plugin::extract_strings(&parsed, "MyMod.esp");
    assert_eq!(tm.meta.plugin_name, "MyMod.esp");
    for e in &mut tm.entries {
        e.trans = format!("FR:{}", e.source);
    }

    let (bytes, n) = dsd::to_json_bytes(&tm);
    assert_eq!(n, 2, "both translated entries emitted");
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().unwrap();

    let find = |fid: &str| {
        arr.iter()
            .find(|o| o["form_id"].as_str().unwrap().starts_with(fid))
            .unwrap_or_else(|| panic!("no entry for {fid}"))
    };
    // Own record → owned by the mod itself.
    let own_e = find("01000801");
    assert_eq!(own_e["form_id"], "01000801|MyMod.esp");
    assert_eq!(own_e["type"], "WEAP FULL");
    assert_eq!(own_e["original"], "My Blade");
    assert_eq!(own_e["string"], "FR:My Blade");
    // Base-game override → owned by the master.
    let base_e = find("00013986");
    assert_eq!(base_e["form_id"], "00013986|Skyrim.esm");
}
