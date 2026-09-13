//! Native installed-mod, cached-distributed and direct-archive launches. Every
//! path here reads the exact admitted bytes from the local filesystem.

use std::sync::Arc;

use robin_engine::mission_assets::{
    ArchiveIdentity, ArchiveMissionAssets, DistributedCacheIdentity, InstalledArchiveLocator,
    InstalledModsRoot, MissionAssetDescriptor,
};

use super::{
    InstalledMissionSource, LiveArchiveAssets, PreparedLiveMissionAssets, prepare_archive_assets,
};
use crate::distributed_mod::{DISTRIBUTED_MOD_ARCHIVE_LIMIT, ValidatedDistributedMod};

/// Resolve a selected native archive against the two allowed installed roots.
/// Duplicate filenames remain unambiguous because the selected absolute file
/// must fall under exactly one logical root and its full relative path is kept.
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
pub fn prepare_installed_custom_mission(
    launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    let exact = read_installed_launch_archives(launch)?;
    prepare_archive_assets(
        LiveArchiveAssets {
            mission_basename: &launch.rhm_basename,
            map_filename: &launch.map_filename,
            rhm_entry: &launch.rhm_zip_entry,
            requires_spellforge: launch.requires_spellforge,
            mission_archive: exact.mission_archive,
            shared_archive: exact.shared_archive,
            installed: Some(exact.locator),
            distributed_cache: None,
        },
        files,
    )
}

/// Prepare direct `--custom-mission` launch arguments. When an archive has
/// multiple same-basename language entries, the launcher requires the exact
/// `--custom-mission-entry`; it never guesses one from central-directory
/// order.
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
            LiveArchiveAssets {
                mission_basename,
                map_filename,
                rhm_entry: &rhm_entry,
                requires_spellforge: false,
                mission_archive,
                shared_archive: None,
                installed: Some(source.locator),
                distributed_cache: None,
            },
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

/// Native canonical-content launch with a durable local cache pin. Hosts use
/// this too, so the cache identity written to their replay/save is an honest
/// recovery source rather than merely an advertised network identity.
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

pub(crate) struct ExactInstalledLaunchArchives {
    pub mission_archive: Arc<[u8]>,
    pub shared_archive: Option<Arc<[u8]>>,
    pub locator: InstalledArchiveLocator,
}

/// Read the launch's installed archive set exactly once. Multiplayer package
/// construction and single-player preparation share this boundary.
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

pub(super) fn select_direct_rhm_entry(
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
