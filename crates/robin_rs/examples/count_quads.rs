#![deny(clippy::print_stdout, clippy::print_stderr)]
use clap::Parser;
use robin_assets::scb;
use std::collections::BTreeMap;

#[derive(Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    /// Original .scb bytecode file to inspect.
    path: std::path::PathBuf,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    let scb = scb::parse_file(&args.path)?;
    let mut hist: BTreeMap<u8, u32> = BTreeMap::new();
    for c in &scb.classes {
        for q in &c.quads {
            *hist.entry(q.operation).or_insert(0) += 1;
        }
    }
    let total: u32 = hist.values().sum();
    tracing::info!("opcode histogram ({total} total quads):");
    for (op, count) in &hist {
        if *op > 47 {
            tracing::warn!("  {op}: {count}  <- UNKNOWN");
        } else {
            tracing::info!("  {op:2}: {count}", op = *op);
        }
    }
    Ok(())
}
