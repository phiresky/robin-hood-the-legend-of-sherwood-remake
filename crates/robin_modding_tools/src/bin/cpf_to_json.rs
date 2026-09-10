//! Converts a binary `.cpf` profile cache file to JSON.
//!
//! Usage: cpf_to_json [--patch-view] [--patch file.json]... <input.cpf> [output.json]
//!
//! If no output path is given, writes to stdout.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SB_FILE_READ, SbFile};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
#[command(about = "Export a CPF profile cache as JSON, optionally applying JSON Patches")]
struct Args {
    /// Export named profile maps suitable for JSON Patch authoring.
    #[arg(long)]
    patch_view: bool,
    /// Apply a JSON Patch file; repeat to apply multiple patches in order.
    #[arg(long)]
    patch: Vec<std::path::PathBuf>,
    input: String,
    output: Option<std::path::PathBuf>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = <Args as clap::Parser>::parse();
    let input_path = &args.input;
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

    for path in &args.patch {
        let bytes =
            std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        mgr = robin_engine::content_patch::apply_profiles(&mgr, &bytes)
            .unwrap_or_else(|error| panic!("apply {}: {error}", path.display()));
    }
    let document = if args.patch_view {
        robin_engine::content_patch::profile_document(&mgr).expect("build named profile patch view")
    } else {
        serde_json::to_value(&mgr).expect("serialize profiles")
    };
    let json = serde_json::to_string_pretty(&document).unwrap();

    if let Some(output) = args.output.as_ref() {
        std::fs::write(output, &json).unwrap_or_else(|e| {
            tracing::error!("Failed to write {}: {}", output.display(), e);
            std::process::exit(1);
        });
        tracing::info!("Written to {}", output.display());
    } else {
        std::io::Write::write_all(&mut std::io::stdout(), json.as_bytes())
            .expect("write to stdout");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn patches_preserve_order_and_output_is_optional() {
        use clap::Parser;
        let args = super::Args::try_parse_from([
            "cpf_to_json",
            "--patch-view",
            "--patch",
            "first.json",
            "--patch",
            "second.json",
            "profile.cpf",
            "output.json",
        ])
        .unwrap();
        assert!(args.patch_view);
        assert_eq!(
            args.patch,
            vec![std::path::PathBuf::from("first.json"), "second.json".into()]
        );
        assert_eq!(args.input, "profile.cpf");
        assert_eq!(args.output.unwrap(), std::path::Path::new("output.json"));
        assert!(
            super::Args::try_parse_from(["cpf_to_json", "profile.cpf"])
                .unwrap()
                .output
                .is_none()
        );
    }
}
