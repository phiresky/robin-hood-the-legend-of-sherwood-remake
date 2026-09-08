//! Converts a binary `.cpf` profile cache file to JSON.
//!
//! Usage: cpf_to_json <input.cpf> [output.json]
//!
//! If no output path is given, writes to stdout.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SB_FILE_READ, SbFile};

fn main() {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        tracing::error!("Usage: cpf_to_json <input.cpf> [output.json]");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let mut file = SbFile::open(input_path, SB_FILE_READ).unwrap_or_else(|e| {
        tracing::error!("Failed to open {}: error {}", input_path, e);
        std::process::exit(1);
    });

    let mut mgr = ProfileManager::new();

    mgr.load_all_legacy_cpf(&mut file).unwrap_or_else(|e| {
        tracing::error!("Failed to read profiles: error {}", e);
        std::process::exit(1);
    });

    tracing::info!(
        "Loaded: {} hth weapons, {} bows, {} characters, {} soldiers, {} missions, {} civilians",
        mgr.hth_weapons.len(),
        mgr.bows.len(),
        mgr.characters.len(),
        mgr.soldiers.len(),
        mgr.missions.len(),
        mgr.civilians.len(),
    );

    let json = serde_json::to_string_pretty(&mgr).unwrap();

    if args.len() >= 3 {
        std::fs::write(&args[2], &json).unwrap_or_else(|e| {
            tracing::error!("Failed to write {}: {}", args[2], e);
            std::process::exit(1);
        });
        tracing::info!("Written to {}", args[2]);
    } else {
        std::io::Write::write_all(&mut std::io::stdout(), json.as_bytes())
            .expect("write to stdout");
    }
}
