//! Probe utility: scan a `datadirs/mods/` tree, peek every `.rhm`'s
//! header, and print one row per launchable mission entry.
//!
//! Run from the repo root:
//!
//! ```text
//! cargo run --example list_mods -- datadirs/mods
//! ```

use robin_rs::mod_pack::{MissionStatus, enumerate_missions, scan_mods_dir};
use std::path::Path;

fn main() {
    let mods_root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "datadirs/mods".to_string());
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
    let entries = enumerate_missions(&mods);
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
}
