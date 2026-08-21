//! Print the localized character names + VIP map resolved from a
//! `Level.res` text resource file — for checking name-table layouts
//! across game/demo builds.
//!
//!   cargo run --example dump_character_names -- path/to/Level.res
#![deny(clippy::print_stdout, clippy::print_stderr)]
fn main() {
    tracing_subscriber::fmt::init();
    let path = std::env::args().nth(1).expect("usage: <path-to-Level.res>");
    let mut res = robin_assets::resource_manager::ResourceManager::new();
    res.attach_resource_file(&path).expect("attach res file");
    let names = robin_rs::ui_panel::load_localized_character_names(&mut res);
    tracing::info!("names: {names:?}");
    let vip = robin_rs::game_session::load_fixed_vip_name_map(&mut res);
    tracing::info!("vip map: {vip:?}");
    let (first, sur) = robin_rs::game_session::load_peasant_name_pool(&mut res);
    tracing::info!("peasant firstnames: {first:?}");
    tracing::info!("peasant surnames: {sur:?}");
}
