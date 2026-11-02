//! Write the built-in game sheets to a folder (default: `assets/data/games`
//! of the workspace). Run once after changing `GameSpec::builtin`:
//!
//! ```text
//! cargo run -p ximod-games --example gen_game_specs -- assets/data/games
//! ```
fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/data/games".to_string());
    ximod_games::GameRegistry::write_builtin(std::path::Path::new(&dir))
        .expect("write game sheets");
    println!("sheets written to {dir}");
}
