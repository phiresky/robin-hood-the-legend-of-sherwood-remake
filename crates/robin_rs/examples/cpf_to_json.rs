//! Converts a binary `.cpf` profile cache file to JSON.
//!
//! Usage: cpf_to_json [--patch-view] [--patch file.json]... <input.cpf> [output.json]
//!
//! If no output path is given, writes to stdout.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SB_FILE_READ, SbFile};

fn main() {
    tracing_subscriber::fmt::init();
    let args = clap::Command::new("cpf_to_json")
        .arg(
            clap::Arg::new("patch-view")
                .long("patch-view")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            clap::Arg::new("patch")
                .long("patch")
                .action(clap::ArgAction::Append),
        )
        .arg(clap::Arg::new("input").required(true))
        .arg(clap::Arg::new("output"))
        .get_matches();
    let patch_view = args.get_flag("patch-view");
    let input_path = args
        .get_one::<String>("input")
        .expect("required input argument");
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

    if let Some(patches) = args.get_many::<String>("patch") {
        for path in patches {
            let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
            mgr = robin_engine::content_patch::apply_profiles(&mgr, &bytes)
                .unwrap_or_else(|error| panic!("apply {path}: {error}"));
        }
    }
    let document = if patch_view {
        robin_engine::content_patch::profile_document(&mgr).expect("build named profile patch view")
    } else {
        serde_json::to_value(&mgr).expect("serialize profiles")
    };
    let json = serde_json::to_string_pretty(&document).unwrap();

    if let Some(output) = args.get_one::<String>("output") {
        std::fs::write(output, &json).unwrap_or_else(|e| {
            tracing::error!("Failed to write {}: {}", output, e);
            std::process::exit(1);
        });
        tracing::info!("Written to {}", output);
    } else {
        std::io::Write::write_all(&mut std::io::stdout(), json.as_bytes())
            .expect("write to stdout");
    }
}
