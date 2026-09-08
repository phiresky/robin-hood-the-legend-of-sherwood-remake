//! Trim a benchmark boot manifest using the production proof rule. This writes
//! only datadir.bin; authenticated published packages must use convert_datadir
//! --web-content-manifest --audio-format opus to regenerate their manifest.
use anyhow::{Context, Result, ensure};
use robin_assets::{
    shipping_boot_trim::trim_browser_locale_audio,
    shipping_datadir::{ShippingDatadir, encode_native, zstd_compress_with_window},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn index(root: &Path, path: &Path, files: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            index(root, &entry.path(), files)?;
        } else if entry.file_type()?.is_file() {
            let key = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            files.entry(key).or_insert(entry.path());
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() >= 3,
        "usage: trim_browser_boot <input.bin> <new-output.bin> <source Data root> [fallback Data roots...]"
    );
    ensure!(
        !Path::new(&args[1]).exists(),
        "output must not already exist"
    );
    let mut sources = BTreeMap::new();
    for root in &args[2..] {
        index(Path::new(root), Path::new(root), &mut sources)?;
    }
    let input = std::fs::read(&args[0])?;
    let mut datadir = ShippingDatadir::from_compressed_bytes(&input)?;
    let original_raw = encode_native(&datadir);
    let original_catalog = datadir.audio_assets.clone();
    let data_dir = Path::new(&args[0])
        .parent()
        .context("input has no parent")?;
    for asset in original_catalog.values() {
        let path = data_dir.join(&asset.file);
        let size = std::fs::metadata(&path)
            .with_context(|| format!("catalog file {}", path.display()))?
            .len();
        let end = u64::from(asset.bundle_offset.unwrap_or(0)) + u64::from(asset.encoded_size);
        ensure!(end <= size, "catalog range exceeds {}", path.display());
        if asset.bundle_offset.is_none() {
            ensure!(end == size, "catalog file length mismatch");
        }
    }

    let original_missions = bitcode::encode(&datadir.missions);
    let report = trim_browser_locale_audio(&mut datadir, |key| {
        sources
            .get(key)
            .map(std::fs::read)
            .transpose()
            .with_context(|| format!("read source {key}"))
    })?;
    ensure!(
        datadir.audio_assets == original_catalog,
        "audio playback references changed"
    );
    ensure!(
        bitcode::encode(&datadir.missions) == original_missions,
        "mission dependencies changed"
    );
    let raw = encode_native(&datadir);
    let compressed = zstd_compress_with_window(&raw, 30)?;
    let roundtrip = ShippingDatadir::from_compressed_bytes(&compressed)?;
    // ResourceManager stores HashMaps, so native bitcode byte order changes
    // across decodes. Compare semantic payload values, one field at a time
    // to keep temporary JSON memory bounded.
    macro_rules! same_field { ($($field:ident),*) => { $(
        ensure!(serde_json::to_value(&datadir.$field)? == serde_json::to_value(&roundtrip.$field)?,
            "roundtrip field mismatch: {}", stringify!($field));
    )* }; }
    same_field!(
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
        saved_world_rhs_files
    );
    ensure!(
        datadir.locales.keys().eq(roundtrip.locales.keys()),
        "locale keys changed"
    );
    for (name, locale) in &datadir.locales {
        ensure!(
            serde_json::to_value(locale)? == serde_json::to_value(&roundtrip.locales[name])?,
            "locale roundtrip mismatch: {name}"
        );
    }
    std::fs::write(&args[1], &compressed)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "trim": report, "input_file_bytes": input.len(), "input_bitcode_bytes": original_raw.len(),
            "output_file_bytes": compressed.len(), "output_bitcode_bytes": raw.len(),
            "audio_catalog_identical": true, "mission_references_identical": true,
            "roundtrip_semantically_identical": true, "validated_external_audio_references": original_catalog.len(),
        }))?
    );
    Ok(())
}
