//! Convert a JSONL replay into the compact URL-safe bitcode format.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use robin_engine::replay::ReplayData;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(input) = args.next() else {
        bail!("usage: replay_to_compact <input.rhrec.jsonl> <output.rhrec>");
    };
    let Some(output) = args.next() else {
        bail!("usage: replay_to_compact <input.rhrec.jsonl> <output.rhrec>");
    };
    if args.next().is_some() {
        bail!("usage: replay_to_compact <input.rhrec.jsonl> <output.rhrec>");
    }

    let input = PathBuf::from(input);
    let output = PathBuf::from(output);
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
