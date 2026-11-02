//! Development helper (B-11): writes an autosaved state of a plugin into a
//! rescue folder, as a session killed before a normal close would leave it.
//! Usage: simulate_crash <plugin> <rescue-dir>
use std::path::PathBuf;
use ximod_translator::backup::{Autosave, AutosaveEvent, Manifest};
use ximod_translator::doc_settings::DocSettings;

fn main() {
    let mut args = std::env::args().skip(1);
    let plugin = PathBuf::from(args.next().expect("plugin path"));
    let dir = PathBuf::from(args.next().expect("rescue dir"));
    let data = ximod_translator::config::assets_data_dir();
    ximod_translator::games::spec::init(&data.join("games"), &data.join("fields"), None);
    let mut tm = ximod_translator::plugin::read_file(&plugin).expect("readable plugin");
    let mut n = 0;
    for (i, e) in tm.entries.iter_mut().enumerate() {
        if i % 3 == 0 {
            e.trans = format!("Traduction {i}");
            ximod_translator::status::set(e, ximod_translator::status::Status::Validated);
            n += 1;
        }
    }
    let settings = DocSettings {
        game: tm.meta.game.clone(),
        source: "eng".into(),
        target: "fra".into(),
        output: Some(ximod_translator::config::OutputMode::Strings),
        use_personal: false,
    };
    let mut auto = Autosave::new(dir, 5);
    auto.spawn(tm, Manifest::new(&plugin, &settings, n), 1);
    loop {
        if auto
            .poll()
            .iter()
            .any(|e| matches!(e, AutosaveEvent::Done { .. }))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    println!("{n} lignes autosauvegardées");
}
