//! Convert a JSONL replay into the compact URL-safe bitcode format.

use std::path::PathBuf;

use anyhow::{Context, Result};
use robin_engine::replay::ReplayData;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    input: PathBuf,
    output: PathBuf,
}

fn main() -> Result<()> {
    let Args { input, output } = <Args as clap::Parser>::parse();
    let input_str = input
        .to_str()
        .with_context(|| format!("input path is not UTF-8: {}", input.display()))?;
    let replay = ReplayData::from_file(input_str).map_err(anyhow::Error::msg)?;
    let compact = robin_rs::replay_format::encode_compact(
        &replay,
        robin_rs::replay_format::ENGINE_VERSION_HASH,
    )
    .context("encode compact replay")?;
    std::fs::write(&output, compact.as_bytes())
        .with_context(|| format!("write {}", output.display()))?;
    println!(
        "{} -> {} ({} bytes)",
        input.display(),
        output.display(),
        compact.len()
    );
    Ok(())
}
