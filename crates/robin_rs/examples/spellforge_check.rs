//! Production Spellforge package/VM checker for mission authors and CI.

use clap::Parser;
use robin_spellforge::{ARCHIVE_BYTE_LIMIT, SpellforgeRuntime51, build_package_from_archives};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(about = "Validate the exact Spellforge package selected from a mission ZIP")]
struct Args {
    /// Downloaded mission/version ZIP.
    mission_zip: PathBuf,

    /// Exact .rhm entry path inside the ZIP (including language directories).
    #[arg(long)]
    rhm_entry: String,

    /// Optional shared lib_*.zip used when the mission has no local lib tree.
    #[arg(long)]
    shared_lib: Option<PathBuf>,

    /// Emit a stable machine-readable report.
    #[arg(long)]
    json: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("spellforge_check: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse();
    let mission = read_bounded(&args.mission_zip)?;
    let shared = args.shared_lib.as_deref().map(read_bounded).transpose()?;
    let basename = args
        .rhm_entry
        .rsplit('/')
        .next()
        .and_then(|leaf| leaf.rsplit_once('.').map(|(basename, _)| basename))
        .filter(|basename| !basename.is_empty())
        .ok_or_else(|| format!("invalid --rhm-entry `{}`", args.rhm_entry))?;
    let layout =
        robin_rs::mod_pack::selected_mission_layout_in_zip(&args.mission_zip, &args.rhm_entry)
            .map_err(|error| format!("mission layout: {error}"))?;
    let expected_mounted_rhm = format!("data/levels/{}.rhm", basename.to_ascii_lowercase());
    if layout.mounted_rhm_path != expected_mounted_rhm {
        return Err(format!(
            "selected mission mounts as `{}`; gameplay requires `{expected_mounted_rhm}`",
            layout.mounted_rhm_path
        ));
    }
    let rhm_header = robin_rs::mod_pack::peek_rhm_header_in_zip(&args.mission_zip, &args.rhm_entry)
        .map_err(|error| format!("selected .rhm header: {error}"))?;
    let package =
        build_package_from_archives(&mission, &args.rhm_entry, basename, shared.as_deref())
            .map_err(|error| format!("package admission ({:?}): {error}", error.kind))?;

    // Construction compiles every loaded module and executes the package
    // bootstrap/entrypoint under the same sandbox used by live gameplay.
    SpellforgeRuntime51::new(package.clone())
        .map_err(|error| format!("guest bootstrap: {error}"))?;
    let source_bytes = package.files.values().map(Vec::len).sum::<usize>();
    let package_hash = robin_engine::spellforge::hex_hash(&package.sha256);
    let mission_archive_digest: [u8; 32] = Sha256::digest(&mission).into();
    let mission_archive_hash = robin_engine::spellforge::hex_hash(&mission_archive_digest);
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "contract_version": package.contract_version,
                "vm_abi": package.vm_abi,
                "script_mode": format!("{:?}", package.script_mode),
                "entrypoint": package.entrypoint,
                "selected_rhm_entry": args.rhm_entry,
                "mounted_rhm_path": layout.mounted_rhm_path,
                "overlay_strip_prefix": layout.strip_prefix,
                "overlay_prepend_prefix": layout.prepend_prefix,
                "map_filename": rhm_header.map_filename,
                "mission_archive_sha256": mission_archive_hash,
                "source_files": package.files.len(),
                "source_bytes": source_bytes,
                "package_sha256": package_hash,
            })
        );
    } else {
        println!("Spellforge package OK");
        println!("  entrypoint: {}", package.entrypoint);
        println!("  selected RHM: {}", args.rhm_entry);
        println!("  mounted RHM: {}", layout.mounted_rhm_path);
        println!("  map: {}", rhm_header.map_filename);
        println!("  mode: {:?}", package.script_mode);
        println!(
            "  sources: {} files, {source_bytes} bytes",
            package.files.len()
        );
        println!("  VM ABI: {}", package.vm_abi);
        println!("  mission archive SHA-256: {mission_archive_hash}");
        println!("  package SHA-256: {package_hash}");
    }
    Ok(())
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let declared = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
        .len();
    if declared > ARCHIVE_BYTE_LIMIT as u64 {
        return Err(format!(
            "{} is {declared} bytes; archive limit is {ARCHIVE_BYTE_LIMIT}",
            path.display()
        ));
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() > ARCHIVE_BYTE_LIMIT {
        return Err(format!(
            "{} grew to {} bytes while reading; archive limit is {ARCHIVE_BYTE_LIMIT}",
            path.display(),
            bytes.len()
        ));
    }
    Ok(bytes)
}
