//! Mesure du coût de la détection de langue (Lingua) : premier appel
//! (chargement des modèles) puis appels suivants, sur le texte d'une fixture.
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/fixtures/skyrimle_riverwood.esp".into());
    let tm = ximod_translator::plugin::read_file(std::path::Path::new(&path)).expect("lecture");
    let texts: Vec<&str> = tm.entries.iter().map(|e| e.source.as_str()).collect();
    let t = Instant::now();
    let first = ximod_translator::langdetect::detect_texts(texts.iter().copied());
    println!("premier appel : {:?} → {:?}", t.elapsed(), first);
    let t = Instant::now();
    let again = ximod_translator::langdetect::detect_texts(texts.iter().copied());
    println!("second appel  : {:?} → {:?}", t.elapsed(), again);
}
