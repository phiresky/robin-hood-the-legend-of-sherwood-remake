//! Inspect independently compressed boot fields and individual resource entries.
use anyhow::{Context, Result};
use robin_assets::shipping_datadir::{ShippingDatadir, encode_native, zstd_max_compress};

fn row(name: &str, bytes: Vec<u8>) -> Result<()> {
    println!(
        "{}\t{}\t{}",
        name,
        bytes.len(),
        zstd_max_compress(&bytes)?.len()
    );
    Ok(())
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: boot_inventory <datadir.bin>")?;
    let dd = ShippingDatadir::load_from_file(std::path::Path::new(&path))?;
    println!("entry\tbitcode_bytes\tzstd_bytes");
    row("TOTAL", encode_native(&dd))?;
    macro_rules! field { ($($name:ident),*) => { $( row(stringify!($name), bitcode::encode(&dd.$name))?; )* }; }
    field!(
        profiles,
        res_files,
        pak_files,
        red_files,
        levels,
        scripts,
        rhs_files,
        sprite_bank,
        raw,
        audio_durations_ms,
        audio_assets,
        missions,
        character_rhs_files,
        character_audio_files,
        character_exclamation_ids,
        mission_exclamation_ids,
        saved_world_rhs_files,
        locales
    );
    for (path, resource) in &dd.res_files {
        row(&format!("res/{path}"), bitcode::encode(resource))?;
    }
    for (path, resource) in &dd.pak_files {
        row(&format!("pak/{path}"), bitcode::encode(resource))?;
    }
    for (path, bytes) in &dd.raw {
        row(&format!("raw/{path}"), bytes.clone())?;
    }
    for (locale, pack) in &dd.locales {
        row(&format!("locale/{locale}"), bitcode::encode(pack))?;
        for (path, resource) in &pack.res_files {
            row(
                &format!("locale/{locale}/res/{path}"),
                bitcode::encode(resource),
            )?;
        }
        for (path, resource) in &pack.pak_files {
            row(
                &format!("locale/{locale}/pak/{path}"),
                bitcode::encode(resource),
            )?;
        }
        for (path, bytes) in &pack.raw {
            row(&format!("locale/{locale}/raw/{path}"), bytes.clone())?;
        }
    }
    Ok(())
}
