//! Exact mission-asset preparation for live custom-mission launches.
//!
//! This is the live counterpart to cold restoration: selection supplies one
//! explicit installed locator or one canonical distributed-mod envelope, and
//! this module turns the exact admitted bytes into the descriptor retained by
//! `MissionLaunch`. No engine-facing path is reopened after this boundary.

use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use robin_engine::mission_assets::{
    ArchiveIdentity, ArchiveMissionAssets, InstalledModsRoot, MissionAssetDescriptor,
};
use robin_engine::mission_assets::{DistributedCacheIdentity, InstalledArchiveLocator};
use robin_engine::spellforge::SpellforgePackage;

#[cfg(not(target_arch = "wasm32"))]
use crate::distributed_mod::DISTRIBUTED_MOD_ARCHIVE_LIMIT;
use crate::distributed_mod::{DISTRIBUTED_MOD_SCHEMA_VERSION, ValidatedDistributedMod};
use crate::mission_asset_restore::{ResolvedMissionAssets, retain_live_mission_assets};

/// Runtime-only installed source selected by the picker.
///
/// `root_path` is never persisted. Saves/replays retain only `locator`, whose
/// path is normalized and relative to the stable logical root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledMissionSource {
    pub root_path: std::path::PathBuf,
    pub locator: InstalledArchiveLocator,
}

/// Exact assets and executable package prepared before engine construction.
#[derive(Debug)]
pub struct PreparedLiveMissionAssets {
    pub resolved: Arc<ResolvedMissionAssets>,
    pub spellforge_package: Option<SpellforgePackage>,
}

/// Resolve a selected native archive against the two allowed installed roots.
/// Duplicate filenames remain unambiguous because the selected absolute file
/// must fall under exactly one logical root and its full relative path is kept.
#[cfg(not(target_arch = "wasm32"))]
pub fn locate_installed_mission_source(
    selected_archive: &std::path::Path,
    configured_mods: &std::path::Path,
    bundled_mods: Option<&std::path::Path>,
) -> Result<InstalledMissionSource, String> {
    let selected = std::fs::canonicalize(selected_archive).map_err(|error| {
        format!(
            "resolve selected custom-mission archive {}: {error}",
            selected_archive.display()
        )
    })?;
    if !selected.is_file() {
        return Err(format!(
            "selected custom-mission archive {} is not a regular file",
            selected.display()
        ));
    }

    let mut matches = Vec::new();
    for (kind, root) in [
        (InstalledModsRoot::ConfiguredMods, Some(configured_mods)),
        (InstalledModsRoot::BundledMods, bundled_mods),
    ] {
        let Some(root) = root else { continue };
        let Ok(root) = std::fs::canonicalize(root) else {
            continue;
        };
        if !root.is_dir() {
            continue;
        }
        let Ok(relative) = selected.strip_prefix(&root) else {
            continue;
        };
        matches.push((kind, root, normalized_relative_path(relative)?));
    }
    if matches.len() == 2 && matches[0].1 == matches[1].1 {
        // Both logical roots intentionally point at the same directory. The
        // configured root is the stable first-choice identity.
        matches.truncate(1);
    }
    let [(root, root_path, mission_relative_path)] = matches.as_slice() else {
        return Err(match matches.len() {
            0 => format!(
                "custom-mission archive {} is outside configured root {}{}",
                selected.display(),
                configured_mods.display(),
                bundled_mods
                    .map(|root| format!(" and bundled root {}", root.display()))
                    .unwrap_or_default()
            ),
            _ => format!(
                "custom-mission archive {} is ambiguously contained by both installed roots",
                selected.display()
            ),
        });
    };
    Ok(InstalledMissionSource {
        root_path: root_path.clone(),
        locator: InstalledArchiveLocator {
            root: *root,
            mission_relative_path: mission_relative_path.clone(),
            shared_relative_path: None,
        },
    })
}

/// Read, admit, hash, and mount one native picker launch from the same exact
/// immutable bytes. Spellforge's returned package is the one callers embed in
/// `PendingLuaMission`; gameplay must not derive it again from a path.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_installed_custom_mission(
    launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    let exact = read_installed_launch_archives(launch)?;
    prepare_archive_assets(
        &launch.rhm_basename,
        &launch.map_filename,
        &launch.rhm_zip_entry,
        launch.requires_spellforge,
        exact.mission_archive,
        exact.shared_archive,
        Some(exact.locator),
        None,
        files,
    )
}

#[cfg(target_arch = "wasm32")]
pub fn prepare_installed_custom_mission(
    _launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
    _files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    Err("browser custom missions must arrive as canonical in-memory distributed content".to_owned())
}

/// Prepare direct `--custom-mission` launch arguments. When an archive has
/// multiple same-basename language entries, the launcher requires the exact
/// `--custom-mission-entry`; it never guesses one from central-directory
/// order.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_direct_custom_mission(
    application_context: &crate::host::ApplicationContext,
    archive_path: &std::path::Path,
    mission_basename: &str,
    map_filename: &str,
    selected_entry: Option<&str>,
) -> Result<PreparedLiveMissionAssets, String> {
    let installed = locate_installed_mission_source(
        archive_path,
        &crate::mod_pack::default_mods_root(),
        crate::main_entry::overlay_mods_dir().as_deref(),
    );
    let mission_archive = match installed.as_ref() {
        Ok(source) => read_source_archive(source, archive_path, "mission")?,
        Err(_) => read_external_archive(archive_path, "mission")?,
    };
    let rhm_entry = select_direct_rhm_entry(&mission_archive, mission_basename, selected_entry)?;
    match installed {
        Ok(source) => prepare_archive_assets(
            mission_basename,
            map_filename,
            &rhm_entry,
            false,
            mission_archive,
            None,
            Some(source.locator),
            None,
            application_context.preparation_files()?.clone(),
        ),
        Err(locator_error) => prepare_external_direct_in_cache(
            application_context,
            archive_path,
            mission_basename,
            map_filename,
            &rhm_entry,
            mission_archive,
        )
        .map_err(|cache_error| {
            format!(
                "archive has no unambiguous installed locator ({locator_error}); durable cache preparation failed: {cache_error}"
            )
        }),
    }
}

/// Prepare exact canonical multiplayer/browser bytes. Every such descriptor
/// keeps the distributed-cache identity, including a native host descriptor
/// which also has an installed locator.
pub fn prepare_distributed_custom_mission(
    validated: &ValidatedDistributedMod,
    encoded_bytes: u64,
    installed: Option<InstalledArchiveLocator>,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    let manifest = &validated.package.manifest;
    if manifest.schema_version != DISTRIBUTED_MOD_SCHEMA_VERSION {
        return Err(format!(
            "distributed mod schema is {}, expected {DISTRIBUTED_MOD_SCHEMA_VERSION}",
            manifest.schema_version
        ));
    }
    let mission_archive: Arc<[u8]> = Arc::clone(&validated.package.mission_archive);
    let shared_archive = validated
        .package
        .shared_library_archive
        .as_ref()
        .map(Arc::clone);
    prepare_archive_assets(
        &manifest.mission_basename,
        &manifest.map_filename,
        &manifest.mission_rhm_entry,
        manifest.requires_spellforge,
        mission_archive,
        shared_archive,
        installed,
        Some(DistributedCacheIdentity {
            schema_version: manifest.schema_version,
            full_mod_sha256: manifest.full_mod_sha256,
            encoded_bytes,
        }),
        files,
    )
}

/// Native canonical-content launch with a durable local cache pin. Hosts use
/// this too, so the cache identity written to their replay/save is an honest
/// recovery source rather than merely an advertised network identity.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_cached_distributed_custom_mission(
    application_context: &crate::host::ApplicationContext,
    validated: &ValidatedDistributedMod,
    encoded: Arc<[u8]>,
    installed: Option<InstalledArchiveLocator>,
) -> Result<PreparedLiveMissionAssets, String> {
    let manifest = &validated.package.manifest;
    let expected_hash = manifest.full_mod_sha256;
    let encoded_bytes = encoded.len() as u64;
    let lease = application_context
        .with_distributed_mod_cache_mut(|cache| cache.install(encoded.to_vec(), expected_hash))?;
    if &lease.validated != validated {
        return Err(
            "durable cache decoded content differently from the already-validated launch envelope"
                .to_owned(),
        );
    }
    let descriptor = distributed_descriptor(validated, encoded_bytes, installed)?;
    let spellforge_package = validated.spellforge_package.clone();
    let resolved = crate::mission_asset_restore::resolve_cached_mission_assets(
        &descriptor,
        spellforge_package.as_ref(),
        lease,
        application_context.preparation_files()?.clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok(PreparedLiveMissionAssets {
        resolved: Arc::new(resolved),
        spellforge_package,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct ExactInstalledLaunchArchives {
    pub mission_archive: Arc<[u8]>,
    pub shared_archive: Option<Arc<[u8]>>,
    pub locator: InstalledArchiveLocator,
}

/// Read the launch's installed archive set exactly once. Multiplayer package
/// construction and single-player preparation share this boundary.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_installed_launch_archives(
    launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
) -> Result<ExactInstalledLaunchArchives, String> {
    if launch.version_zip_bytes.is_some() {
        return Err(
            "installed custom-mission launch unexpectedly supplied separate in-memory bytes"
                .to_owned(),
        );
    }
    let source = launch.installed_source.as_ref().ok_or_else(|| {
        "custom-mission launch has no explicit configured/bundled source locator".to_owned()
    })?;
    let mission_archive = read_source_archive(source, &launch.version_zip, "mission")?;
    let (shared_archive, shared_relative_path) = if launch.requires_spellforge {
        let shared_path =
            crate::mod_pack::find_lib_zip(&source.root_path.join("lib")).ok_or_else(|| {
                format!(
                    "Spellforge shared library archive is missing under {}",
                    source.root_path.join("lib").display()
                )
            })?;
        let relative = shared_path
            .canonicalize()
            .map_err(|error| format!("resolve shared library {}: {error}", shared_path.display()))?
            .strip_prefix(&source.root_path)
            .map_err(|_| {
                format!(
                    "Spellforge shared library {} escapes installed root {}",
                    shared_path.display(),
                    source.root_path.display()
                )
            })
            .and_then(normalized_relative_path)?;
        (
            Some(read_relative_archive(
                &source.root_path,
                &relative,
                "shared library",
            )?),
            Some(relative),
        )
    } else {
        (None, None)
    };
    let mut locator = source.locator.clone();
    locator.shared_relative_path = shared_relative_path;
    Ok(ExactInstalledLaunchArchives {
        mission_archive,
        shared_archive,
        locator,
    })
}

fn prepare_archive_assets(
    mission_basename: &str,
    map_filename: &str,
    rhm_entry: &str,
    requires_spellforge: bool,
    mission_archive: Arc<[u8]>,
    shared_archive: Option<Arc<[u8]>>,
    installed: Option<InstalledArchiveLocator>,
    distributed_cache: Option<DistributedCacheIdentity>,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    let (resolved, spellforge_package) = retain_live_mission_assets(
        mission_basename,
        map_filename,
        rhm_entry,
        requires_spellforge,
        mission_archive,
        shared_archive,
        installed,
        distributed_cache,
        files,
    )?;
    Ok(PreparedLiveMissionAssets {
        resolved: Arc::new(resolved),
        spellforge_package,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn distributed_descriptor(
    validated: &ValidatedDistributedMod,
    encoded_bytes: u64,
    installed: Option<InstalledArchiveLocator>,
) -> Result<MissionAssetDescriptor, String> {
    let manifest = &validated.package.manifest;
    let descriptor = MissionAssetDescriptor::archive(
        &manifest.mission_basename,
        &manifest.map_filename,
        &manifest.map_filename,
        ArchiveMissionAssets {
            mission_archive: ArchiveIdentity {
                sha256: manifest.mission_archive_sha256,
                bytes: manifest.mission_archive_bytes,
            },
            selected_rhm_entry: manifest.mission_rhm_entry.clone(),
            shared_archive: match (
                manifest.shared_library_sha256,
                manifest.shared_library_bytes,
            ) {
                (Some(sha256), Some(bytes)) => Some(ArchiveIdentity { sha256, bytes }),
                (None, None) => None,
                _ => {
                    return Err(
                        "validated distributed manifest has partial shared-archive identity"
                            .to_owned(),
                    );
                }
            },
            installed,
            distributed_cache: Some(DistributedCacheIdentity {
                schema_version: manifest.schema_version,
                full_mod_sha256: manifest.full_mod_sha256,
                encoded_bytes,
            }),
        },
    )
    .map_err(|error| error.to_string())?;
    descriptor
        .validate_spellforge_package(validated.spellforge_package.as_ref())
        .map_err(|error| error.to_string())?;
    Ok(descriptor)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_source_archive(
    source: &InstalledMissionSource,
    selected_path: &std::path::Path,
    label: &'static str,
) -> Result<Arc<[u8]>, String> {
    let selected = selected_path
        .canonicalize()
        .map_err(|error| format!("resolve selected {label} archive: {error}"))?;
    let expected = source
        .root_path
        .join(&source.locator.mission_relative_path)
        .canonicalize()
        .map_err(|error| format!("resolve installed {label} archive: {error}"))?;
    if selected != expected {
        return Err(format!(
            "selected {label} archive {} differs from explicit installed locator {}",
            selected.display(),
            expected.display()
        ));
    }
    read_relative_archive(
        &source.root_path,
        &source.locator.mission_relative_path,
        label,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn read_relative_archive(
    root: &std::path::Path,
    relative: &str,
    label: &'static str,
) -> Result<Arc<[u8]>, String> {
    use std::io::Read;

    let root = root
        .canonicalize()
        .map_err(|error| format!("resolve installed mods root {}: {error}", root.display()))?;
    let path = root.join(relative);
    let path = path.canonicalize().map_err(|error| {
        format!(
            "resolve installed {label} archive {}: {error}",
            path.display()
        )
    })?;
    if !path.starts_with(&root) {
        return Err(format!(
            "installed {label} archive {} escapes root {}",
            path.display(),
            root.display()
        ));
    }
    let mut file = std::fs::File::open(&path)
        .map_err(|error| format!("open installed {label} archive {}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "inspect installed {label} archive {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "installed {label} archive {} is not a non-empty regular file",
            path.display()
        ));
    }
    if metadata.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 {
        return Err(format!(
            "installed {label} archive {} is {} bytes; limit is {DISTRIBUTED_MOD_ARCHIVE_LIMIT}",
            path.display(),
            metadata.len()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read installed {label} archive {}: {error}", path.display()))?;
    if bytes.is_empty() || bytes.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT {
        return Err(format!(
            "installed {label} archive {} changed to {} bytes while reading",
            path.display(),
            bytes.len()
        ));
    }
    Ok(Arc::from(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_external_archive(path: &std::path::Path, label: &'static str) -> Result<Arc<[u8]>, String> {
    use std::io::Read;

    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve {label} archive {}: {error}", path.display()))?;
    let mut file = std::fs::File::open(&path)
        .map_err(|error| format!("open {label} archive {}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect {label} archive {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "{label} archive {} is not a non-empty regular file",
            path.display()
        ));
    }
    if metadata.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 {
        return Err(format!(
            "{label} archive {} is {} bytes; limit is {DISTRIBUTED_MOD_ARCHIVE_LIMIT}",
            path.display(),
            metadata.len()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label} archive {}: {error}", path.display()))?;
    if bytes.is_empty() || bytes.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT {
        return Err(format!(
            "{label} archive {} changed to {} bytes while reading",
            path.display(),
            bytes.len()
        ));
    }
    Ok(Arc::from(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare_external_direct_in_cache(
    application_context: &crate::host::ApplicationContext,
    archive_path: &std::path::Path,
    mission_basename: &str,
    map_filename: &str,
    rhm_entry: &str,
    mission_archive: Arc<[u8]>,
) -> Result<PreparedLiveMissionAssets, String> {
    let file_label = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("custom-mission.zip");
    let validated = crate::distributed_mod::DistributedModPackage::build(
        format!("direct-{mission_basename}"),
        format!("Direct custom mission {mission_basename}"),
        "local player".to_owned(),
        file_label.to_owned(),
        "local://direct-custom-mission".to_owned(),
        "local launch only; redistribution not authorised".to_owned(),
        mission_basename.to_owned(),
        rhm_entry.to_owned(),
        map_filename.to_owned(),
        false,
        mission_archive.to_vec(),
        None,
    )
    .map_err(|error| format!("build canonical local cache envelope: {error}"))?;
    let encoded = validated
        .package
        .encode()
        .map_err(|error| format!("encode canonical local cache envelope: {error}"))?;
    let full_mod_sha256 = validated.package.manifest.full_mod_sha256;
    let encoded_bytes = encoded.len() as u64;
    let lease = application_context
        .with_distributed_mod_cache_mut(|cache| cache.install(encoded, full_mod_sha256))?;
    let manifest = &lease.validated.package.manifest;
    let descriptor = MissionAssetDescriptor::archive(
        mission_basename,
        map_filename,
        map_filename,
        ArchiveMissionAssets {
            mission_archive: ArchiveIdentity {
                sha256: manifest.mission_archive_sha256,
                bytes: manifest.mission_archive_bytes,
            },
            selected_rhm_entry: rhm_entry.to_owned(),
            shared_archive: None,
            installed: None,
            distributed_cache: Some(DistributedCacheIdentity {
                schema_version: manifest.schema_version,
                full_mod_sha256,
                encoded_bytes,
            }),
        },
    )
    .map_err(|error| error.to_string())?;
    let resolved = crate::mission_asset_restore::resolve_cached_mission_assets(
        &descriptor,
        None,
        lease,
        application_context.preparation_files()?.clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok(PreparedLiveMissionAssets {
        resolved: Arc::new(resolved),
        spellforge_package: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn normalized_relative_path(path: &std::path::Path) -> Result<String, String> {
    use std::path::Component;

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| "installed mod path is not valid UTF-8".to_owned())?,
            ),
            _ => return Err("installed mod path is not a normalized relative path".to_owned()),
        }
    }
    if parts.is_empty() {
        return Err("installed archive path is empty".to_owned());
    }
    Ok(parts.join("/"))
}

#[cfg(not(target_arch = "wasm32"))]
fn select_direct_rhm_entry(
    archive_bytes: &[u8],
    mission_basename: &str,
    selected_entry: Option<&str>,
) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes))
        .map_err(|error| format!("open direct custom-mission archive: {error}"))?;
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| format!("read direct custom-mission entry {index}: {error}"))?;
        if !entry.is_dir() {
            entries.push(entry.name().replace('\\', "/"));
        }
    }
    if let Some(selected) = selected_entry {
        if entries.iter().any(|entry| entry == selected) {
            return Ok(selected.to_owned());
        }
        return Err(format!(
            "--custom-mission-entry `{selected}` is not present in the selected archive"
        ));
    }
    let expected_leaf = format!("{mission_basename}.rhm");
    let matches = entries
        .into_iter()
        .filter(|entry| {
            entry
                .rsplit('/')
                .next()
                .is_some_and(|leaf| leaf.eq_ignore_ascii_case(&expected_leaf))
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [entry] => Ok(entry.clone()),
        [] => Err(format!(
            "selected archive has no `{expected_leaf}` entry; pass the exact mission and archive"
        )),
        _ => Err(format!(
            "selected archive has {} `{expected_leaf}` entries; pass --custom-mission-entry with the exact language/path",
            matches.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::mission_assets::MissionAssetSource;
    fn independent_files() -> Arc<robin_engine::sbfile::SbFileSystem> {
        Arc::new(robin_engine::sbfile::SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )))
    }
    use std::io::Write;

    #[cfg(not(target_arch = "wasm32"))]
    fn write_zip(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn zip_bytes(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn rhm(map: &str, marker: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 34 + map.len() + 2];
        bytes[..4].copy_from_slice(b"RHMI");
        bytes[12..16].copy_from_slice(b"HEAD");
        bytes[32..34].copy_from_slice(&((map.len() + 1) as u16).to_le_bytes());
        bytes[34..34 + map.len()].copy_from_slice(map.as_bytes());
        *bytes.last_mut().unwrap() = marker;
        bytes
    }

    #[test]
    fn live_archives_are_admitted_once_and_retain_the_exact_arc() {
        use crate::distributed_mod::MISSION_ARCHIVE_ADMISSIONS;

        let files = independent_files();
        let selected = "German/Data/Levels/H01_Lin.rhm";
        let level = rhm("lincoln", 0x5a);
        let script = b"function StartUp() return 1 end".to_vec();
        let bytes: Arc<[u8]> = zip_bytes(&[
            (selected, level.clone()),
            ("German/Data/Levels/H01_Lin.lua", script.clone()),
        ])
        .into();
        for requires_spellforge in [false, true] {
            let shared: Option<Arc<[u8]>> = requires_spellforge
                .then(|| zip_bytes(&[("lib/common.lua", b"return 7".to_vec())]).into());
            let before = MISSION_ARCHIVE_ADMISSIONS.get();
            let prepared = prepare_archive_assets(
                "H01_Lin",
                "lincoln",
                selected,
                requires_spellforge,
                bytes.clone(),
                shared.clone(),
                Some(InstalledArchiveLocator {
                    root: robin_engine::mission_assets::InstalledModsRoot::ConfiguredMods,
                    mission_relative_path: "mission.zip".into(),
                    shared_relative_path: shared.as_ref().map(|_| "shared.zip".into()),
                }),
                None,
                files.clone(),
            )
            .unwrap();
            assert_eq!(MISSION_ARCHIVE_ADMISSIONS.get() - before, 1);
            assert!(Arc::ptr_eq(
                prepared.resolved.mission_archive().unwrap(),
                &bytes
            ));
            if requires_spellforge {
                let package = prepared.spellforge_package.as_ref().unwrap();
                assert_eq!(package.entrypoint, "h01_lin.lua");
                assert_eq!(package.files["h01_lin.lua"], script);
                assert_eq!(package.files["lib/common.lua"], b"return 7");
                assert!(Arc::ptr_eq(
                    prepared.resolved.shared_archive().unwrap(),
                    shared.as_ref().unwrap(),
                ));
                package.validate_wire().unwrap();
            } else {
                assert!(prepared.spellforge_package.is_none());
            }
            assert_eq!(files.read_all("Data/Levels/H01_Lin.rhm").unwrap(), level);
            drop(prepared);
            assert!(files.read_all("Data/Levels/H01_Lin.rhm").is_err());
        }
    }

    #[test]
    fn live_archive_admission_rejects_corruption_before_descriptor_errors() {
        let files = independent_files();
        let selected = "Data/Levels/H01_Lin.rhm";
        let bytes = zip_bytes(&[(selected, rhm("lincoln", 0x5a))]);
        let mut corrupt = bytes.clone();
        let payload = corrupt.windows(4).position(|part| part == b"RHMI").unwrap();
        corrupt[payload] ^= 1; // Stored entry now disagrees with its ZIP CRC.
        for (archive, map) in [(corrupt, "lincoln"), (bytes, "wrong_map")] {
            let error = prepare_archive_assets(
                "H01_Lin",
                map,
                selected,
                false,
                archive.into(),
                None,
                None,
                None,
                files.clone(),
            )
            .unwrap_err(); // Missing locator would also invalidate a descriptor.
            assert!(
                error.starts_with("admit exact custom-mission archives:"),
                "{error}"
            );
            assert!(files.read_all("Data/Levels/H01_Lin.rhm").is_err());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn configured_and_bundled_duplicate_names_keep_distinct_locators() {
        let temporary = tempfile::tempdir().unwrap();
        let configured = temporary.path().join("configured");
        let bundled = temporary.path().join("bundled");
        std::fs::create_dir_all(configured.join("same")).unwrap();
        std::fs::create_dir_all(bundled.join("same")).unwrap();
        let configured_zip = configured.join("same/mission.zip");
        let bundled_zip = bundled.join("same/mission.zip");
        write_zip(&configured_zip, &[("Data/Levels/test.rhm", b"configured")]);
        write_zip(&bundled_zip, &[("Data/Levels/test.rhm", b"bundled")]);

        let first =
            locate_installed_mission_source(&configured_zip, &configured, Some(&bundled)).unwrap();
        let second =
            locate_installed_mission_source(&bundled_zip, &configured, Some(&bundled)).unwrap();
        assert_eq!(first.locator.root, InstalledModsRoot::ConfiguredMods);
        assert_eq!(second.locator.root, InstalledModsRoot::BundledMods);
        assert_eq!(first.locator.mission_relative_path, "same/mission.zip");
        assert_eq!(second.locator.mission_relative_path, "same/mission.zip");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn direct_multilingual_archive_requires_exact_entry() {
        let temporary = tempfile::tempdir().unwrap();
        let archive_path = temporary.path().join("mission.zip");
        write_zip(
            &archive_path,
            &[
                ("English/Data/Levels/H01_Lin.rhm", b"english"),
                ("German/Data/Levels/H01_Lin.rhm", b"german"),
            ],
        );
        let bytes = std::fs::read(archive_path).unwrap();
        let error = select_direct_rhm_entry(&bytes, "H01_Lin", None).unwrap_err();
        assert!(error.contains("--custom-mission-entry"));
        assert_eq!(
            select_direct_rhm_entry(&bytes, "H01_Lin", Some("German/Data/Levels/H01_Lin.rhm"))
                .unwrap(),
            "German/Data/Levels/H01_Lin.rhm"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn installed_launch_mounts_the_same_bytes_it_hashes() {
        let files = independent_files();
        let temporary = tempfile::tempdir().unwrap();
        let configured = temporary.path().join("configured");
        let archive_path = configured.join("nested/mission.zip");
        std::fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
        let selected = "German/Data/Levels/H01_Lin.rhm";
        let original_rhm = rhm("lincoln", 0x5a);
        let original = zip_bytes(&[(selected, original_rhm.clone())]);
        std::fs::write(&archive_path, &original).unwrap();
        let source = locate_installed_mission_source(&archive_path, &configured, None).unwrap();
        let launch = crate::main_menu::custom_missions::CustomMissionLaunch {
            slug: "test".to_owned(),
            mod_title: "Test".to_owned(),
            claimed_author: "Author".to_owned(),
            version: "1".to_owned(),
            source_url: String::new(),
            license: "CC0-1.0".to_owned(),
            version_zip: archive_path.clone(),
            installed_source: Some(source),
            version_zip_bytes: None,
            rhm_zip_entry: selected.to_owned(),
            rhm_basename: "H01_Lin".to_owned(),
            map_filename: "lincoln".to_owned(),
            requires_spellforge: false,
        };

        let prepared = prepare_installed_custom_mission(&launch, files.clone()).unwrap();
        std::fs::write(
            &archive_path,
            zip_bytes(&[(selected, rhm("lincoln", 0x33))]),
        )
        .unwrap();
        assert_eq!(
            files.read_all("Data/Levels/H01_Lin.rhm").unwrap(),
            original_rhm
        );
        let MissionAssetSource::Archive(assets) = &prepared.resolved.descriptor().source else {
            panic!("live custom mission must have archive descriptor")
        };
        use sha2::{Digest, Sha256};
        assert_eq!(
            assets.mission_archive.sha256,
            <[u8; 32]>::from(Sha256::digest(&original))
        );
        assert_eq!(assets.mission_archive.bytes, original.len() as u64);
        assert_eq!(
            assets.installed.as_ref().unwrap().mission_relative_path,
            "nested/mission.zip"
        );
        drop(prepared);
    }

    #[test]
    fn canonical_distributed_launch_records_cache_identity_and_exact_bytes() {
        let files = independent_files();
        let selected = "Data/Levels/H01_Lin.rhm";
        let mission_archive = zip_bytes(&[(selected, rhm("lincoln", 0x77))]);
        let validated = crate::distributed_mod::DistributedModPackage::build(
            "test".to_owned(),
            "Test".to_owned(),
            "Author".to_owned(),
            "1".to_owned(),
            "https://example.invalid".to_owned(),
            "CC0-1.0".to_owned(),
            "H01_Lin".to_owned(),
            selected.to_owned(),
            "lincoln".to_owned(),
            false,
            mission_archive.clone(),
            None,
        )
        .unwrap();
        let encoded = validated.package.encode().unwrap();
        let expected_hash = validated.package.manifest.full_mod_sha256;
        let prepared =
            prepare_distributed_custom_mission(&validated, encoded.len() as u64, None, files)
                .unwrap();
        let MissionAssetSource::Archive(assets) = &prepared.resolved.descriptor().source else {
            panic!("distributed mission must have archive descriptor")
        };
        assert!(assets.installed.is_none());
        assert!(Arc::ptr_eq(
            prepared.resolved.mission_archive().unwrap(),
            &validated.package.mission_archive
        ));
        let cache = assets.distributed_cache.unwrap();
        assert_eq!(cache.schema_version, DISTRIBUTED_MOD_SCHEMA_VERSION);
        assert_eq!(cache.full_mod_sha256, expected_hash);
        assert_eq!(cache.encoded_bytes, encoded.len() as u64);
        assert_eq!(
            prepared.resolved.mission_archive().unwrap().as_ref(),
            mission_archive
        );
        drop(prepared);
    }
}
