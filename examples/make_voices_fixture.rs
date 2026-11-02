//! Voice-map fixture (lot 4, C-13): `tests/fixtures/voices/Data/` — the
//! dialogue plugin as MonMod.esp, loose .fuz files under
//! Sound/Voice/MonMod.esp/<voice type>/ and a `MonMod - Voices.bsa`.
//!
//! Replies of skyrim_dialogue.esp: 0x01000301 (2 responses), 302, 303,
//! 304 (prompt + 1), 305, 306, 307. Voiced here: 301/1 (loose, MaleNord
//! and FemaleNord), 301/2 (BSA), 302/1 (loose, load index 05), 303/1 (lip
//! only), 305/1 (BSA). Unvoiced: 303 (lips alone), 304, 306, 307.
use ba2::prelude::*;
use ba2::tes4::{Archive, ArchiveKey, ArchiveOptions, Directory, DirectoryKey, File, Version};
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let data = root.join("voices").join("Data");
    let _ = std::fs::remove_dir_all(&data);
    let voice = data.join("Sound").join("Voice").join("MonMod.esp");
    let male = voice.join("MaleNord");
    let female = voice.join("FemaleNord");
    std::fs::create_dir_all(&male).unwrap();
    std::fs::create_dir_all(&female).unwrap();
    std::fs::copy(root.join("skyrim_dialogue.esp"), data.join("MonMod.esp")).unwrap();
    let fuz = b"FUZE\0\0\0\0";
    std::fs::write(male.join("MQ101_MQ101RalofTopic01_01000301_1.fuz"), fuz).unwrap();
    std::fs::write(female.join("MQ101_MQ101RalofTopic01_01000301_1.fuz"), fuz).unwrap();
    std::fs::write(male.join("MQ101_MQ101RalofTopic01_05000302_1.fuz"), fuz).unwrap();
    std::fs::write(male.join("MQ101_MQ101RalofTopic01_01000303_1.lip"), b"LIP").unwrap();
    std::fs::write(male.join("readme.txt"), b"not a voice").unwrap();
    let f1 = File::from_decompressed(b"FUZE 301/2");
    let f2 = File::from_decompressed(b"FUZE 305/1");
    let dir: Directory = [
        (
            DirectoryKey::from(b"MQ101_MQ101RalofTopic01_01000301_2.fuz".as_slice()),
            f1,
        ),
        (
            DirectoryKey::from(b"MQ101_MQ101IntroScene_01000305_1.fuz".as_slice()),
            f2,
        ),
    ]
    .into_iter()
    .collect();
    let archive: Archive = [(
        ArchiveKey::from(b"sound\\voice\\monmod.esp\\malenord".as_slice()),
        dir,
    )]
    .into_iter()
    .collect();
    let opts = ArchiveOptions::builder().version(Version::SSE).build();
    let mut bytes = Vec::new();
    archive.write(&mut bytes, &opts).unwrap();
    std::fs::write(data.join("MonMod - Voices.bsa"), bytes).unwrap();
    println!("{}", data.display());
}
