#![cfg(all(not(target_arch = "wasm32"), unix))]

//! Cold custom-mission restart and corpus coverage.
//!
//! The fast test validates the committed 13-row manifest without requiring
//! proprietary game data. The ignored corpus test uses the same production
//! discovery, selected-layout, RHM-header, Spellforge package, and safe VM
//! loaders as the game:
//!
//! ```text
//! ROBINHOOD_MODS_DIR=/absolute/path/to/datadirs/mods \
//!   cargo test -p robin_rs --test authentic_custom_mission_cold_start \
//!   -- --ignored --nocapture
//! ```
//!
//! `ROBINHOOD_MODS_DIR` must contain the exact SHA-pinned archives named in
//! `tests/corpus/installed_custom_missions.json`. The test never downloads or
//! substitutes content: an absent, changed, or newly reordered install is a
//! failure that names the affected archive or picker row.

use robin_engine::mission_assets::{
    ArchiveIdentity, ArchiveMissionAssets, InstalledArchiveLocator, InstalledModsRoot,
    MissionAssetDescriptor, MissionAssetSource,
};
use robin_engine::replay::{ReplayData, ReplayRecorder};
use robin_engine::spellforge::hex_hash;
use robin_engine::{
    campaign::Campaign,
    engine::{Engine, LevelAssets, SimConfig},
};
use robin_rs::game::GamePersistentState;
use robin_rs::mission_asset_restore::{
    MissionAssetRoots, ResolvedMissionAssets, resolve_native_mission_assets,
};
use robin_rs::mod_pack::{MissionStatus, enumerate_missions, scan_mods_dir};
use robin_rs::save_file::{GameSaveFile, SaveHeader, SaveProvenance};
use robin_rs::sound::SoundManager;
use robin_spellforge::{ARCHIVE_BYTE_LIMIT, SpellforgeRuntime51, build_package_from_archives};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const MANIFEST_JSON: &str = include_str!("corpus/installed_custom_missions.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    source_page: String,
    shared_archive: CorpusArchive,
    archives: Vec<CorpusArchive>,
    rows: Vec<CorpusRow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusArchive {
    relative_path: String,
    #[serde(default)]
    page_url: Option<String>,
    url: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusRow {
    name: String,
    archive: String,
    rhm_entry: String,
    mission_basename: String,
    proto_level_filename: String,
    spellforge: bool,
    #[serde(default)]
    spellforge_case: Option<String>,
    strip_prefix: String,
    prepend_prefix: String,
}

const SPELLFORGE_CORPUS_JSON: &str =
    include_str!("../../robin_spellforge/tests/corpus/manifest.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpellforgeCorpus {
    source_page: String,
    archives: Vec<SpellforgeArchive>,
    cases: Vec<SpellforgeCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpellforgeArchive {
    file: String,
    page_url: Option<String>,
    url: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpellforgeCase {
    name: String,
    mission_archive: String,
    rhm_entry: String,
    shared_library_archive: Option<String>,
    expected_package_sha256: String,
    expected_source_files: usize,
    expected_source_bytes: usize,
}

fn spellforge_corpus() -> &'static SpellforgeCorpus {
    static CORPUS: std::sync::OnceLock<SpellforgeCorpus> = std::sync::OnceLock::new();
    CORPUS.get_or_init(|| serde_json::from_str(SPELLFORGE_CORPUS_JSON).expect("Spellforge corpus"))
}

fn authority_archive_hash<'a>(
    authority: &'a SpellforgeCorpus,
    name: &str,
) -> Result<&'a str, String> {
    let archives: Vec<_> = authority
        .archives
        .iter()
        .filter(|archive| archive.file == name)
        .collect();
    let [archive] = archives.as_slice() else {
        return Err(format!("expected exactly one authoritative archive {name}"));
    };
    Ok(&archive.sha256)
}

fn resolve_package_pin<'a>(
    manifest: &CorpusManifest,
    row: &CorpusRow,
    authority: &'a SpellforgeCorpus,
) -> Result<&'a str, String> {
    if !row.spellforge || manifest.source_page != authority.source_page {
        return Err(
            "package authority requires a Spellforge row from the same source corpus".into(),
        );
    }
    let id = row
        .spellforge_case
        .as_deref()
        .ok_or("missing Spellforge case link")?;
    let cases: Vec<_> = authority
        .cases
        .iter()
        .filter(|case| case.name == id)
        .collect();
    let [case] = cases.as_slice() else {
        return Err(format!("expected exactly one authoritative case {id}"));
    };
    if case.rhm_entry != row.rhm_entry {
        return Err(format!("{id} selected RHM differs from authoritative case"));
    }
    let archive = manifest
        .archives
        .iter()
        .find(|archive| archive.relative_path == row.archive)
        .ok_or("missing client archive pin")?;
    if archive.sha256 != authority_archive_hash(authority, &case.mission_archive)? {
        return Err(format!(
            "{id} mission archive SHA differs from authoritative case"
        ));
    }
    if let Some(shared) = &case.shared_library_archive
        && manifest.shared_archive.sha256 != authority_archive_hash(authority, shared)?
    {
        return Err(format!(
            "{id} shared library SHA differs from authoritative case"
        ));
    }
    // Cases without a shared archive carry their own libraries. Production
    // admission ignores the caller's shared fallback for those exact ZIPs.
    if case.expected_package_sha256.len() != 64
        || !case
            .expected_package_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || case.expected_source_files == 0
        || case.expected_source_bytes == 0
    {
        return Err(format!("{id} has invalid authoritative package metadata"));
    }
    Ok(&case.expected_package_sha256)
}

fn expected_package_hash(manifest: &CorpusManifest, row: &CorpusRow) -> &'static str {
    resolve_package_pin(manifest, row, spellforge_corpus())
        .unwrap_or_else(|error| panic!("{} package authority: {error}", row.name))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ColdRestartArtifact {
    Replay {
        row_name: String,
        compact: String,
    },
    Save {
        row_name: String,
        save_filename: String,
    },
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    Replay,
    Save,
}

fn manifest() -> CorpusManifest {
    serde_json::from_str(MANIFEST_JSON).expect("installed custom-mission manifest must be valid")
}

#[test]
fn installed_spellforge_links_cover_the_single_package_authority_exactly() {
    let manifest = manifest();
    let authority = spellforge_corpus();
    let expected: BTreeSet<_> = authority
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    assert_eq!(
        expected.len(),
        authority.cases.len(),
        "duplicate authority case IDs"
    );
    let mut selected = BTreeSet::new();
    for row in manifest.rows.iter().filter(|row| row.spellforge) {
        assert!(
            selected.insert(row.spellforge_case.as_deref().expect("case link")),
            "duplicate client case link"
        );
        decode_hash(expected_package_hash(&manifest, row));
    }
    assert_eq!(
        selected, expected,
        "client coverage must name every authoritative Spellforge case exactly once"
    );
}

#[test]
fn installed_package_authority_rejects_missing_substituted_and_ambiguous_links() {
    let manifest = manifest();
    let authority = spellforge_corpus();
    let row = row_named(&manifest, "first_lincoln_v1_2");
    let mut changed = row.clone();
    changed.spellforge_case = None;
    assert!(
        resolve_package_pin(&manifest, &changed, authority)
            .unwrap_err()
            .contains("missing")
    );
    changed.spellforge_case = Some("unknown-case".into());
    assert!(
        resolve_package_pin(&manifest, &changed, authority)
            .unwrap_err()
            .contains("exactly one")
    );
    changed.spellforge_case = Some("escape_from_lincoln".into());
    assert!(
        resolve_package_pin(&manifest, &changed, authority)
            .unwrap_err()
            .contains("RHM")
    );
    changed = row.clone();
    changed.rhm_entry = "wrong.rhm".into();
    assert!(
        resolve_package_pin(&manifest, &changed, authority)
            .unwrap_err()
            .contains("RHM")
    );

    let mut changed_manifest = manifest.clone();
    changed_manifest
        .archives
        .iter_mut()
        .find(|archive| archive.relative_path == row.archive)
        .unwrap()
        .sha256 = "0".repeat(64);
    assert!(
        resolve_package_pin(&changed_manifest, row, authority)
            .unwrap_err()
            .contains("mission archive SHA")
    );
    changed_manifest = manifest.clone();
    changed_manifest.shared_archive.sha256 = "0".repeat(64);
    assert!(
        resolve_package_pin(&changed_manifest, row, authority)
            .unwrap_err()
            .contains("shared library SHA")
    );

    let mut changed_authority = authority.clone();
    changed_authority.cases.push(
        authority
            .cases
            .iter()
            .find(|case| Some(case.name.as_str()) == row.spellforge_case.as_deref())
            .unwrap()
            .clone(),
    );
    assert!(
        resolve_package_pin(&manifest, row, &changed_authority)
            .unwrap_err()
            .contains("exactly one")
    );
    changed_authority = authority.clone();
    let archive = authority
        .cases
        .iter()
        .find(|case| Some(case.name.as_str()) == row.spellforge_case.as_deref())
        .unwrap()
        .mission_archive
        .as_str();
    changed_authority.archives.push(
        authority
            .archives
            .iter()
            .find(|entry| entry.file == archive)
            .unwrap()
            .clone(),
    );
    assert!(
        resolve_package_pin(&manifest, row, &changed_authority)
            .unwrap_err()
            .contains("exactly one")
    );
}

#[test]
fn installed_manifest_rejects_reintroduced_duplicate_package_pins() {
    let mut json: serde_json::Value = serde_json::from_str(MANIFEST_JSON).unwrap();
    json["rows"][0]["expected_package_sha256"] = serde_json::Value::String("0".repeat(64));
    let error = serde_json::from_value::<CorpusManifest>(json).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown field `expected_package_sha256`")
    );
}

#[test]
fn installed_custom_mission_manifest_pins_all_thirteen_selectable_rows() {
    let manifest = manifest();
    assert_eq!(manifest.source_page, "https://rhmods.com/missions/");
    assert_eq!(manifest.rows.len(), 13, "picker corpus row count changed");
    assert_eq!(
        manifest.rows.iter().filter(|row| row.spellforge).count(),
        10
    );
    assert_eq!(
        manifest.rows.iter().filter(|row| !row.spellforge).count(),
        3
    );

    let mut archive_paths = BTreeSet::new();
    validate_archive(&manifest.shared_archive);
    assert_eq!(
        manifest.shared_archive.relative_path,
        "lib/lib_2026_01_25_.zip"
    );
    assert_eq!(manifest.shared_archive.bytes, 17_946);
    for archive in &manifest.archives {
        validate_archive(archive);
        assert!(
            archive_paths.insert(archive.relative_path.as_str()),
            "duplicate archive path {}",
            archive.relative_path
        );
    }
    assert_eq!(archive_paths.len(), 9);

    let mut names = BTreeSet::new();
    let mut exact_rows = BTreeSet::new();
    for row in &manifest.rows {
        assert!(
            names.insert(row.name.as_str()),
            "duplicate row {}",
            row.name
        );
        assert!(
            archive_paths.contains(row.archive.as_str()),
            "{} names unpinned archive {}",
            row.name,
            row.archive
        );
        assert!(
            exact_rows.insert((row.archive.as_str(), row.rhm_entry.as_str())),
            "duplicate source/RHM pair for {}",
            row.name
        );
        assert_eq!(
            row.rhm_entry
                .rsplit('/')
                .next()
                .expect("RHM leaf")
                .trim_end_matches(".rhm"),
            row.mission_basename,
            "{} basename must come from its exact selected RHM",
            row.name
        );
        assert!(!row.proto_level_filename.is_empty());
        assert_eq!(
            row.spellforge_case.is_some(),
            row.spellforge,
            "{} package pin must agree with its Spellforge tag",
            row.name
        );
        if row.spellforge {
            decode_hash(expected_package_hash(&manifest, row));
        }
        assert!(
            matches!(row.prepend_prefix.as_str(), "" | "data/levels/"),
            "unexpected mounted prefix in {}",
            row.name
        );

        let descriptor = descriptor_for_row(&manifest, row);
        descriptor
            .validate()
            .unwrap_or_else(|error| panic!("{} descriptor is invalid: {error}", row.name));
        assert_eq!(descriptor.mission_basename, row.mission_basename);
        assert_eq!(descriptor.proto_level_filename, row.proto_level_filename);
        assert_eq!(descriptor.map_filename, row.proto_level_filename);
        let MissionAssetSource::Archive(assets) = &descriptor.source else {
            panic!("{} custom row became built-in", row.name)
        };
        assert_eq!(assets.selected_rhm_entry, row.rhm_entry);
        let installed = assets.installed.as_ref().expect("installed corpus locator");
        assert_eq!(installed.mission_relative_path, row.archive);
        assert_eq!(
            installed.shared_relative_path.as_deref(),
            row.spellforge
                .then_some(manifest.shared_archive.relative_path.as_str())
        );
    }

    assert!(
        manifest
            .rows
            .iter()
            .any(|row| row.rhm_entry == "H01_Lin_VL/H01_Lin_VL.rhm"),
        "nested archive layout lost"
    );
    for language in ["English", "German", "Polish"] {
        assert!(
            manifest
                .rows
                .iter()
                .any(|row| row.rhm_entry.starts_with(&format!("{language}/"))),
            "{language} selectable layout lost"
        );
    }

    // These five rows intentionally resolve the same in-game basename from
    // three byte-distinct archives and three language roots. A basename-only
    // restore would silently choose the wrong mission source.
    let rescue_sources = manifest
        .rows
        .iter()
        .filter(|row| row.mission_basename == "H06_Lin_VL")
        .map(|row| (row.archive.as_str(), row.rhm_entry.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(rescue_sources.len(), 5);
    assert_eq!(
        rescue_sources
            .iter()
            .map(|(archive, _)| *archive)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
}

#[test]
#[ignore = "requires the exact SHA-pinned datadirs/mods corpus; see module documentation"]
fn installed_corpus_matches_picker_layout_and_passes_production_loader_and_vm() {
    let mods_root = std::env::var_os("ROBINHOOD_MODS_DIR")
        .map(PathBuf::from)
        .expect("set ROBINHOOD_MODS_DIR to the current datadirs/mods directory");
    let manifest = manifest();
    let archive_pins = manifest
        .archives
        .iter()
        .map(|archive| (archive.relative_path.as_str(), archive))
        .collect::<BTreeMap<_, _>>();

    let shared_bytes = read_pinned_archive(&mods_root, &manifest.shared_archive);
    let mut mission_bytes = BTreeMap::new();
    for archive in &manifest.archives {
        mission_bytes.insert(
            archive.relative_path.as_str(),
            read_pinned_archive(&mods_root, archive),
        );
    }

    let discovered = scan_mods_dir(&mods_root);
    let actual_rows = enumerate_missions(&discovered)
        .into_iter()
        .filter(|entry| !entry.hackable)
        .map(|entry| {
            let relative_archive = relative_slash_path(&mods_root, &entry.version_zip);
            let map_filename = match entry.status {
                MissionStatus::Ok { map_filename } => map_filename,
                MissionStatus::Broken { reason } => {
                    panic!(
                        "picker row {}/{} is broken: {reason}",
                        relative_archive, entry.rhm_zip_entry
                    )
                }
            };
            (
                (relative_archive, entry.rhm_zip_entry),
                (entry.rhm_basename, map_filename, entry.requires_spellforge),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual_rows.len(),
        13,
        "the installed picker corpus must expose exactly the pinned rows"
    );

    for row in &manifest.rows {
        let actual = actual_rows
            .get(&(row.archive.clone(), row.rhm_entry.clone()))
            .unwrap_or_else(|| {
                panic!(
                    "missing exact picker row {}/{} ({})",
                    row.archive, row.rhm_entry, row.name
                )
            });
        assert_eq!(actual.0, row.mission_basename, "{} basename", row.name);
        assert_eq!(
            actual.1, row.proto_level_filename,
            "{} proto/map filename",
            row.name
        );
        assert_eq!(
            actual.2, row.spellforge,
            "{} Spellforge classification",
            row.name
        );

        let archive = archive_pins
            .get(row.archive.as_str())
            .expect("row archive was checked above");
        let archive_path = mods_root.join(&archive.relative_path);
        let layout =
            robin_rs::mod_pack::selected_mission_layout_in_zip(&archive_path, &row.rhm_entry)
                .unwrap_or_else(|error| panic!("{} layout failed: {error}", row.name));
        assert_eq!(layout.strip_prefix, row.strip_prefix, "{} strip", row.name);
        assert_eq!(
            layout.prepend_prefix, row.prepend_prefix,
            "{} prepend",
            row.name
        );
        assert_eq!(
            layout.mounted_rhm_path,
            format!(
                "data/levels/{}.rhm",
                row.mission_basename.to_ascii_lowercase()
            ),
            "{} mounted RHM",
            row.name
        );

        let header = robin_rs::mod_pack::peek_rhm_header_in_zip(&archive_path, &row.rhm_entry)
            .unwrap_or_else(|error| panic!("{} RHM header failed: {error}", row.name));
        assert_eq!(
            header.map_filename, row.proto_level_filename,
            "{} RHM proto/map filename",
            row.name
        );

        if row.spellforge {
            let package = build_package_from_archives(
                mission_bytes
                    .get(row.archive.as_str())
                    .expect("pinned mission bytes"),
                &row.rhm_entry,
                &row.mission_basename,
                Some(&shared_bytes),
            )
            .unwrap_or_else(|error| panic!("{} package admission failed: {error}", row.name));
            assert_eq!(
                hex_hash(&package.sha256),
                expected_package_hash(&manifest, row),
                "{} canonical package identity",
                row.name
            );
            SpellforgeRuntime51::new(package)
                .unwrap_or_else(|error| panic!("{} safe VM bootstrap failed: {error}", row.name));
        }
    }
}

#[test]
#[ignore = "requires the exact SHA-pinned datadirs/mods corpus; see module documentation"]
fn all_pinned_spellforge_rows_pass_the_current_production_loader_and_vm() {
    let mods_root = std::env::var_os("ROBINHOOD_MODS_DIR")
        .map(PathBuf::from)
        .expect("set ROBINHOOD_MODS_DIR to the current datadirs/mods directory");
    let manifest = manifest();
    let shared_bytes = read_pinned_archive(&mods_root, &manifest.shared_archive);
    let mut mismatches = Vec::new();
    for row in manifest.rows.iter().filter(|row| row.spellforge) {
        let mission = read_pinned_archive(&mods_root, archive_named(&manifest, &row.archive));
        let package = build_package_from_archives(
            &mission,
            &row.rhm_entry,
            &row.mission_basename,
            Some(&shared_bytes),
        )
        .unwrap_or_else(|error| panic!("{} package admission failed: {error}", row.name));
        let actual = hex_hash(&package.sha256);
        let expected = expected_package_hash(&manifest, row);
        if actual != expected {
            mismatches.push(format!(
                "{}: expected {expected}, actual {actual}",
                row.name
            ));
        }
        SpellforgeRuntime51::new(package)
            .unwrap_or_else(|error| panic!("{} safe VM bootstrap failed: {error}", row.name));
    }
    assert!(
        mismatches.is_empty(),
        "Spellforge package pins changed:\n{}",
        mismatches.join("\n")
    );
}

#[test]
#[ignore = "requires the exact SHA-pinned datadirs/mods corpus; see module documentation"]
fn nested_and_multilingual_assets_survive_a_fresh_process() {
    let files = Arc::new(robin_engine::sbfile::SbFileSystem::new(Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    let source_root = std::env::var_os("ROBINHOOD_MODS_DIR")
        .map(PathBuf::from)
        .expect("set ROBINHOOD_MODS_DIR to the current datadirs/mods directory");
    let manifest = manifest();
    for (row_name, artifact_kind) in [
        ("first_lincoln_v1_2", ArtifactKind::Replay),
        ("rescue_allan_german", ArtifactKind::Save),
    ] {
        let row = row_named(&manifest, row_name);
        let install = stage_exact_row(&source_root, &manifest, row);
        let package = package_for_row(install.path(), &manifest, row);
        let descriptor = descriptor_for_row(&manifest, row);
        let roots = isolated_roots(install.path());

        assert!(
            !files.has_zip_overlays(),
            "parent must start without a stale custom-mission overlay"
        );
        let original_assets =
            resolve_native_mission_assets(&descriptor, Some(&package), &roots, None, files.clone())
                .unwrap_or_else(|error| panic!("{row_name} original resolve failed: {error}"));
        assert_resolved_assets(&original_assets, &descriptor, &manifest, row);
        assert!(files.has_zip_overlays());
        assert!(
            files
                .try_exists(&format!("Data/Levels/{}.rhm", row.mission_basename))
                .unwrap()
        );

        // Capture the ordinary persisted artifact while the exact custom
        // mission is live. The child receives only this replay/save and the
        // logical mods root; no mount handle or machine-absolute path crosses
        // the restart boundary.
        let artifact = match artifact_kind {
            ArtifactKind::Replay => ColdRestartArtifact::Replay {
                row_name: row_name.to_owned(),
                compact: record_compact_replay(install.path(), &descriptor, package.clone()),
            },
            ArtifactKind::Save => {
                let save_filename = "cold-custom-mission.rhsave.json".to_owned();
                write_custom_mission_save(
                    &install.path().join(&save_filename),
                    &descriptor,
                    package.clone(),
                );
                ColdRestartArtifact::Save {
                    row_name: row_name.to_owned(),
                    save_filename,
                }
            }
        };
        let artifact_path = write_restart_artifact(install.path(), &artifact);

        drop(original_assets);
        assert!(
            !files.has_zip_overlays(),
            "dropping the original launch must unmount it before restart"
        );

        let output = spawn_cold_worker(&artifact_path, install.path(), true);
        assert!(
            output.status.success(),
            "{row_name} cold worker failed: status={}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "requires the exact SHA-pinned datadirs/mods corpus; see module documentation"]
fn cold_restore_fails_closed_for_missing_changed_and_disabled_spellforge_assets() {
    let source_root = std::env::var_os("ROBINHOOD_MODS_DIR")
        .map(PathBuf::from)
        .expect("set ROBINHOOD_MODS_DIR to the current datadirs/mods directory");
    let manifest = manifest();
    let row = row_named(&manifest, "first_lincoln_v1_2");

    let missing_install = stage_exact_row(&source_root, &manifest, row);
    let missing_artifact = replay_artifact_for_row(missing_install.path(), &manifest, row);
    let missing_artifact_path = write_restart_artifact(missing_install.path(), &missing_artifact);
    std::fs::remove_file(missing_install.path().join(&row.archive))
        .expect("remove the exact mission archive after replay capture");
    let missing = spawn_cold_worker(&missing_artifact_path, missing_install.path(), true);
    assert_worker_rejected(&missing, "is missing at");

    let changed_install = stage_exact_row(&source_root, &manifest, row);
    let changed_artifact = save_artifact_for_row(changed_install.path(), &manifest, row);
    let changed_artifact_path = write_restart_artifact(changed_install.path(), &changed_artifact);
    let changed_path = changed_install.path().join(&row.archive);
    let mut changed_bytes = std::fs::read(&changed_path).expect("read archive before corruption");
    changed_bytes.push(0);
    std::fs::write(&changed_path, changed_bytes).expect("change exact mission archive bytes");
    let changed = spawn_cold_worker(&changed_artifact_path, changed_install.path(), true);
    assert_worker_rejected(&changed, "archive identity mismatch");

    let disabled_install = stage_exact_row(&source_root, &manifest, row);
    let disabled_artifact = replay_artifact_for_row(disabled_install.path(), &manifest, row);
    let disabled_artifact_path =
        write_restart_artifact(disabled_install.path(), &disabled_artifact);
    let disabled = spawn_cold_worker(&disabled_artifact_path, disabled_install.path(), false);
    assert_worker_rejected(
        &disabled,
        "Spellforge is disabled before asset restoration or script execution",
    );
}

/// Sentinel invoked by `nested_and_multilingual_assets_survive_a_fresh_process`.
///
/// Keeping the worker inside the integration-test binary gives it a genuinely
/// new process and fresh global VFS while avoiding a test-only game CLI or any
/// mutation of production startup code.
#[test]
fn cold_process_worker() {
    let files = Arc::new(robin_engine::sbfile::SbFileSystem::new(Arc::new(
        robin_util::asset_fs::AssetVfs::new(),
    )));
    let Some(mode) = std::env::var_os("FEATURE40_COLD_WORKER") else {
        return;
    };
    assert_eq!(mode, "persisted-mission");
    assert!(
        !files.has_zip_overlays(),
        "cold worker inherited a process-local overlay"
    );

    let artifact_path = std::env::var_os("FEATURE40_COLD_ARTIFACT")
        .map(PathBuf::from)
        .expect("cold worker artifact path");
    let artifact: ColdRestartArtifact =
        serde_json::from_slice(&std::fs::read(&artifact_path).expect("read cold restart artifact"))
            .expect("decode cold restart artifact");
    let (row_name, descriptor, package) = match artifact {
        ColdRestartArtifact::Replay { row_name, compact } => {
            let (_, replay) = robin_rs::replay_format::decode_compact_for_local_playback(&compact)
                .expect("decode persisted compact custom-mission replay");
            (
                row_name,
                replay.header().mission_assets.clone(),
                replay
                    .header()
                    .spellforge_package
                    .clone()
                    .expect("Spellforge replay must embed its sole executable package"),
            )
        }
        ColdRestartArtifact::Save {
            row_name,
            save_filename,
        } => {
            assert_safe_relative_path(&save_filename);
            let save_path = artifact_path
                .parent()
                .expect("cold artifact parent")
                .join(save_filename);
            let save = GameSaveFile::read_from(&save_path).expect("load persisted custom save");
            let package = save
                .engine
                .spellforge_package()
                .expect("Spellforge save must embed its sole executable package")
                .as_ref()
                .clone();
            (row_name, save.header.mission_assets, package)
        }
    };
    let manifest = manifest();
    let row = row_named(&manifest, &row_name);
    descriptor
        .validate()
        .unwrap_or_else(|error| panic!("{} persisted descriptor failed: {error}", row.name));
    assert_eq!(descriptor, descriptor_for_row(&manifest, row));
    assert_eq!(
        hex_hash(&package.sha256),
        expected_package_hash(&manifest, row),
        "the persisted package is the sole executable authority"
    );

    if std::env::var_os("FEATURE40_SPELLFORGE_ALLOWED").as_deref()
        == Some(std::ffi::OsStr::new("0"))
    {
        assert!(
            !files.has_zip_overlays(),
            "disabled Spellforge must reject before mounting archives"
        );
        panic!("Spellforge is disabled before asset restoration or script execution");
    }

    let roots = MissionAssetRoots::discover();
    let resolved =
        resolve_native_mission_assets(&descriptor, Some(&package), &roots, None, files.clone())
            .unwrap_or_else(|error| panic!("{} cold resolve failed: {error}", row.name));
    assert_resolved_assets(&resolved, &descriptor, &manifest, row);
    assert!(files.has_zip_overlays());
    assert!(
        files
            .try_exists(&format!("Data/Levels/{}.rhm", row.mission_basename))
            .unwrap()
    );

    let session = robin_rs::lua_session::LuaSession::start_from_package(
        row.mission_basename.clone(),
        package,
    )
    .unwrap_or_else(|error| panic!("{} embedded runtime failed: {error}", row.name));
    assert_eq!(
        hex_hash(&session.runtime().package().sha256),
        expected_package_hash(&manifest, row),
        "the embedded package, not a package rebuilt by cold startup, is runtime authority"
    );
    drop(session);
    drop(resolved);
    assert!(
        !files.has_zip_overlays(),
        "cold mission owner must unmount before process exit"
    );
}

fn package_for_row(
    root: &Path,
    manifest: &CorpusManifest,
    row: &CorpusRow,
) -> robin_engine::spellforge::SpellforgePackage {
    assert!(
        row.spellforge,
        "cold package helper requires a Spellforge corpus row"
    );
    let package = build_package_from_archives(
        &read_pinned_archive(root, archive_named(manifest, &row.archive)),
        &row.rhm_entry,
        &row.mission_basename,
        Some(&read_pinned_archive(root, &manifest.shared_archive)),
    )
    .unwrap_or_else(|error| panic!("{} embedded package failed: {error}", row.name));
    assert_eq!(
        hex_hash(&package.sha256),
        expected_package_hash(manifest, row),
        "{} canonical package identity",
        row.name
    );
    package
}

fn isolated_roots(mods_root: &Path) -> MissionAssetRoots {
    MissionAssetRoots {
        configured_mods: mods_root.to_path_buf(),
        bundled_mods: None,
    }
}

fn assert_resolved_assets(
    resolved: &ResolvedMissionAssets,
    descriptor: &MissionAssetDescriptor,
    manifest: &CorpusManifest,
    row: &CorpusRow,
) {
    assert_eq!(resolved.descriptor(), descriptor);
    assert!(resolved.is_archive());
    assert!(!resolved.is_cache_backed());
    assert_eq!(resolved.selected_rhm_entry(), Some(row.rhm_entry.as_str()));

    let archive = archive_named(manifest, &row.archive);
    let mission_bytes = resolved
        .mission_archive()
        .expect("installed custom mission retains its exact admitted bytes");
    assert_eq!(mission_bytes.len() as u64, archive.bytes);
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(mission_bytes.as_ref())),
        decode_hash(&archive.sha256)
    );

    let shared_bytes = resolved
        .shared_archive()
        .expect("Spellforge row retains the exact admitted shared archive");
    assert_eq!(shared_bytes.len() as u64, manifest.shared_archive.bytes);
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(shared_bytes.as_ref())),
        decode_hash(&manifest.shared_archive.sha256)
    );
}

fn record_compact_replay(
    root: &Path,
    descriptor: &MissionAssetDescriptor,
    package: robin_engine::spellforge::SpellforgePackage,
) -> String {
    let jsonl_path = root.join("cold-custom-mission.rhrec.jsonl");
    let writer = File::create(&jsonl_path).expect("create custom replay JSONL");
    let recorder = ReplayRecorder::with_writer_and_spellforge_package(
        Box::new(writer),
        descriptor.mission_basename.clone(),
        descriptor.clone(),
        0x40_c01d_5eed,
        SimConfig::default(),
        &Campaign::default(),
        Some(package),
    )
    .expect("record custom-mission replay header");
    drop(recorder);

    let replay = ReplayData::from_reader(BufReader::new(
        File::open(&jsonl_path).expect("reopen recorded custom replay"),
    ))
    .expect("load recorded custom replay JSONL");
    robin_rs::replay_format::encode_compact(&replay, robin_rs::replay_format::ENGINE_VERSION_HASH)
        .expect("encode canonical compact custom replay")
}

fn write_custom_mission_save(
    path: &Path,
    descriptor: &MissionAssetDescriptor,
    package: robin_engine::spellforge::SpellforgePackage,
) {
    let runtime = Arc::new(
        SpellforgeRuntime51::new(package.clone()).expect("bootstrap save fixture Spellforge VM"),
    );
    let mut assets = LevelAssets {
        attachments: robin_engine::engine::LevelRuntimeAttachments {
            spellforge_runtime: Some(runtime),
            ..Default::default()
        },
        ..LevelAssets::default()
    };
    let engine = Engine::new_for_test(1024.0, 768.0, Campaign::default(), &mut assets)
        .expect("construct authoritative custom-mission save engine");
    assert_eq!(
        engine.spellforge_package().as_deref(),
        Some(&package),
        "save engine must own the exact executable package"
    );
    let save = GameSaveFile {
        header: SaveHeader::new(
            40,
            descriptor.clone(),
            "Feature 40 cold custom mission".to_owned(),
            SaveProvenance::new("Custom mission".to_owned(), 1, "Cold Tester".to_owned())
                .expect("valid custom save provenance"),
        )
        .expect("valid custom save header"),
        engine,
        sound: SoundManager::default(),
        game_persistent: GamePersistentState::default(),
    };
    save.write_to(path).expect("write custom mission save");
}

fn replay_artifact_for_row(
    root: &Path,
    manifest: &CorpusManifest,
    row: &CorpusRow,
) -> ColdRestartArtifact {
    let descriptor = descriptor_for_row(manifest, row);
    let package = package_for_row(root, manifest, row);
    ColdRestartArtifact::Replay {
        row_name: row.name.clone(),
        compact: record_compact_replay(root, &descriptor, package),
    }
}

fn save_artifact_for_row(
    root: &Path,
    manifest: &CorpusManifest,
    row: &CorpusRow,
) -> ColdRestartArtifact {
    let descriptor = descriptor_for_row(manifest, row);
    let package = package_for_row(root, manifest, row);
    let save_filename = "cold-custom-mission.rhsave.json".to_owned();
    write_custom_mission_save(&root.join(&save_filename), &descriptor, package);
    ColdRestartArtifact::Save {
        row_name: row.name.clone(),
        save_filename,
    }
}

fn write_restart_artifact(root: &Path, artifact: &ColdRestartArtifact) -> PathBuf {
    let path = root.join("cold-restart-artifact.json");
    std::fs::write(
        &path,
        serde_json::to_vec(artifact).expect("serialize cold restart artifact"),
    )
    .expect("write cold restart artifact");
    path
}

fn spawn_cold_worker(
    artifact_path: &Path,
    mods_root: &Path,
    spellforge_allowed: bool,
) -> std::process::Output {
    Command::new(std::env::current_exe().expect("current integration test binary"))
        .arg("--exact")
        .arg("cold_process_worker")
        .arg("--nocapture")
        .env("FEATURE40_COLD_WORKER", "persisted-mission")
        .env("FEATURE40_COLD_ARTIFACT", artifact_path)
        .env(
            "FEATURE40_SPELLFORGE_ALLOWED",
            if spellforge_allowed { "1" } else { "0" },
        )
        .env("ROBINHOOD_MODS_DIR", mods_root)
        .output()
        .expect("spawn isolated cold custom-mission worker")
}

fn assert_worker_rejected(output: &std::process::Output, expected: &str) {
    assert!(
        !output.status.success(),
        "cold worker unexpectedly accepted a rejected restoration"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "cold worker rejection did not contain {expected:?}: status={}\nstdout={}\nstderr={stderr}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
    );
}

fn validate_archive(archive: &CorpusArchive) {
    assert!(archive.bytes > 0);
    assert!(archive.bytes <= ARCHIVE_BYTE_LIMIT as u64);
    assert!(
        archive
            .url
            .starts_with("https://rhmods.com/content/uploads/")
    );
    if let Some(page_url) = &archive.page_url {
        assert!(page_url.starts_with("https://rhmods.com/missions/"));
    }
    assert_safe_relative_path(&archive.relative_path);
    decode_hash(&archive.sha256);
}

fn row_named<'a>(manifest: &'a CorpusManifest, name: &str) -> &'a CorpusRow {
    manifest
        .rows
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("manifest row {name} is missing"))
}

fn archive_named<'a>(manifest: &'a CorpusManifest, relative_path: &str) -> &'a CorpusArchive {
    manifest
        .archives
        .iter()
        .find(|archive| archive.relative_path == relative_path)
        .unwrap_or_else(|| panic!("manifest archive {relative_path} is missing"))
}

fn descriptor_for_row(manifest: &CorpusManifest, row: &CorpusRow) -> MissionAssetDescriptor {
    let mission_archive = archive_named(manifest, &row.archive);
    MissionAssetDescriptor::archive(
        row.mission_basename.clone(),
        row.proto_level_filename.clone(),
        row.proto_level_filename.clone(),
        ArchiveMissionAssets {
            mission_archive: archive_identity(mission_archive),
            selected_rhm_entry: row.rhm_entry.clone(),
            shared_archive: row
                .spellforge
                .then(|| archive_identity(&manifest.shared_archive)),
            installed: Some(InstalledArchiveLocator {
                root: InstalledModsRoot::ConfiguredMods,
                mission_relative_path: row.archive.clone(),
                shared_relative_path: row
                    .spellforge
                    .then(|| manifest.shared_archive.relative_path.clone()),
            }),
            distributed_cache: None,
        },
    )
    .unwrap_or_else(|error| panic!("{} descriptor fixture failed: {error}", row.name))
}

fn archive_identity(archive: &CorpusArchive) -> ArchiveIdentity {
    ArchiveIdentity {
        sha256: decode_hash(&archive.sha256),
        bytes: archive.bytes,
    }
}

fn stage_exact_row(
    source_root: &Path,
    manifest: &CorpusManifest,
    row: &CorpusRow,
) -> tempfile::TempDir {
    let install = tempfile::tempdir().expect("create isolated cold install");
    for archive in [
        archive_named(manifest, &row.archive),
        &manifest.shared_archive,
    ] {
        let source = source_root.join(&archive.relative_path);
        let destination = install.path().join(&archive.relative_path);
        std::fs::create_dir_all(destination.parent().expect("archive parent"))
            .expect("create isolated archive parent");
        std::fs::copy(&source, &destination).unwrap_or_else(|error| {
            panic!(
                "copy pinned archive {} to {}: {error}",
                source.display(),
                destination.display()
            )
        });
    }
    install
}

fn assert_safe_relative_path(path: &str) {
    assert!(!path.is_empty());
    assert!(!path.starts_with('/'));
    assert!(!path.contains('\\'));
    assert!(
        path.split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
    );
}

fn decode_hash(encoded: &str) -> [u8; 32] {
    let bytes = hex::decode(encoded).expect("pinned SHA-256 must be hexadecimal");
    bytes
        .try_into()
        .expect("pinned SHA-256 must contain exactly 32 bytes")
}

fn read_pinned_archive(root: &Path, archive: &CorpusArchive) -> Vec<u8> {
    let path = root.join(&archive.relative_path);
    let declared = std::fs::metadata(&path)
        .unwrap_or_else(|error| panic!("cannot inspect pinned archive {}: {error}", path.display()))
        .len();
    assert_eq!(
        declared,
        archive.bytes,
        "pinned archive byte length changed: {}",
        path.display()
    );
    assert!(declared <= ARCHIVE_BYTE_LIMIT as u64);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("cannot read pinned archive {}: {error}", path.display()));
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(
        digest,
        decode_hash(&archive.sha256),
        "pinned archive digest changed: {}",
        path.display()
    );
    bytes
}

fn relative_slash_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or_else(|_| {
            panic!(
                "picker archive {} is outside configured mods root {}",
                path.display(),
                root.display()
            )
        })
        .to_string_lossy()
        .replace('\\', "/")
}
