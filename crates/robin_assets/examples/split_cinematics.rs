//! Migrate a trusted shipping boot file to separate cinematic assets without changing its wire version.
//! Regenerate any web content manifest after this offline operation.
use anyhow::{Context, Result, ensure};
use robin_assets::{
    shipping_cinematics::split_cinematics,
    shipping_datadir::{ShippingDatadir, encode_native, zstd_compress_with_window},
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(clap::Parser, Serialize, Deserialize)]
struct Args {
    input: PathBuf,
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = <Args as clap::Parser>::parse();
    ensure!(!args.output.exists(), "output already exists");
    let input = fs::read(&args.input)?;
    let mut data = ShippingDatadir::from_compressed_bytes(&input)?;
    let root = args.output.parent().context("output has no parent")?;
    fs::create_dir_all(root)?;
    for (file, bytes) in split_cinematics(&mut data)? {
        let destination = root.join(&file);
        fs::create_dir_all(destination.parent().context("asset has no parent")?)?;
        fs::write(destination, &bytes)?;
        println!("{file}: {} bytes", bytes.len());
    }
    let boot = zstd_compress_with_window(&encode_native(&data), 30)?;
    fs::write(args.output, &boot)?;
    println!("boot: {} -> {} compressed bytes", input.len(), boot.len());
    Ok(())
}
