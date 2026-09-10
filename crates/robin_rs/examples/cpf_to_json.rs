//! Converts a binary `.cpf` profile cache file to JSON.
//!
//! Usage: cpf_to_json [--patch file.json]... <input.cpf> [output.json]
//!
//! If no output path is given, writes to stdout.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use anyhow::Context;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SB_FILE_READ, SbFile};

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
#[command(
    about = "Export CPF profiles to canonical JSON, or validate and patch a profile JSON document"
)]
struct Args {
    /// Apply a JSON Patch file; repeat for patches in load order.
    #[arg(long)]
    patch: Vec<std::path::PathBuf>,
    /// A legacy binary .cpf or canonical .json profile document.
    input: String,
    output: Option<std::path::PathBuf>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args = <Args as clap::Parser>::parse();
    let input_path = &args.input;
    let mut document = if input_path.ends_with(".json") {
        ProfileManager::load_json_document_with_files(
            input_path,
            &SbFile::snapshot_legacy_file_system(),
        )
        .with_context(|| format!("load {input_path}"))?
    } else {
        let mut file = SbFile::open(input_path, SB_FILE_READ)
            .map_err(|status| anyhow::anyhow!("open {input_path}: file error {status}"))?;
        let mut mgr = ProfileManager::new();
        mgr.load_all_legacy_cpf(&mut file)
            .context("decode CPF profiles")?;

        tracing::info!(
            "Loaded: {} hth weapons, {} bows, {} characters, {} soldiers, {} missions, {} civilians",
            mgr.hth_weapons.len(),
            mgr.bows.len(),
            mgr.characters.len(),
            mgr.soldiers.len(),
            mgr.missions.len(),
            mgr.civilians.len(),
        );

        robin_engine::content_patch::profile_document(&mgr).map_err(anyhow::Error::msg)?
    };
    for path in &args.patch {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        document = robin_engine::content_patch::apply_profile_document(&document, &bytes)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("apply {}", path.display()))?;
    }
    let json = serde_json::to_string_pretty(&document)?;

    if let Some(output) = args.output.as_ref() {
        std::fs::write(output, &json).with_context(|| format!("write {}", output.display()))?;
        tracing::info!("Written to {}", output.display());
    } else {
        std::io::Write::write_all(&mut std::io::stdout(), json.as_bytes())
            .context("write to stdout")?;
    }
    Ok(())
}
