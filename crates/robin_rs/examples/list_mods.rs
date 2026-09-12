//! Probe utility: scan a `datadirs/mods/` tree, peek every `.rhm`'s
//! header, and print one row per launchable mission entry.
//!
//! Run from the repo root:
//!
//! ```text
//! cargo run --example list_mods -- datadirs/mods
//! ```

use anyhow::Context as _;
use robin_rs::mod_pack::{MissionStatus, enumerate_missions, scan_mods_dir};
use std::path::Path;

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    #[arg(default_value = "datadirs/mods")]
    mods_root: String,
}

fn main() -> anyhow::Result<()> {
    let mods_root = <Args as clap::Parser>::parse().mods_root;
    let mods = scan_mods_dir(Path::new(&mods_root));
    println!("Found {} mods under {mods_root}", mods.len());
    for m in &mods {
        println!(
            "  {:40} tags={:?} versions={}",
            m.details.title,
            m.details.tags,
            m.details.versions.len()
        );
    }
    println!();
    let files = robin_engine::sbfile::SbFileSystem::new(std::sync::Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    ));
    for m in &mods {
        let path = m
            .mod_dir
            .to_str()
            .with_context(|| format!("mod directory is not UTF-8: {}", m.mod_dir.display()))?;
        files
            .add_overlay_path(path)
            .with_context(|| format!("failed to mount mod directory {}", m.mod_dir.display()))?;
    }
    let entries = enumerate_missions(&mods, &files);
    println!("{} mission entries:", entries.len());
    for e in &entries {
        match &e.status {
            MissionStatus::Ok { map_filename } => {
                println!(
                    "  OK   {:35} v={:25} rhm={:20} map={}",
                    e.mod_title, e.version_label, e.rhm_basename, map_filename,
                );
            }
            MissionStatus::Broken { reason } => {
                println!(
                    "  BAD  {:35} v={:25} -- {reason}",
                    e.mod_title, e.version_label,
                );
            }
        }
    }
    Ok(())
}
