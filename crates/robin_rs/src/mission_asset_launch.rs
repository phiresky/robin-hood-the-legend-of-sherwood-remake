//! Exact mission-asset preparation for live custom-mission launches.
//!
//! This is the live counterpart to cold restoration: selection supplies one
//! explicit installed locator or one canonical distributed-mod envelope, and
//! this module turns the exact admitted bytes into the descriptor retained by
//! `MissionRequest`. No engine-facing path is reopened after this boundary.

use std::sync::Arc;

use robin_engine::mission_assets::{DistributedCacheIdentity, InstalledArchiveLocator};
use robin_engine::spellforge::SpellforgePackage;

use crate::distributed_mod::{DISTRIBUTED_MOD_SCHEMA_VERSION, ValidatedDistributedMod};
use crate::mission_asset_restore::{
    LiveArchiveAssets, ResolvedMissionAssets, retain_live_mission_assets,
};

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::read_installed_launch_archives;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{
    locate_installed_mission_source, prepare_cached_distributed_custom_mission,
    prepare_direct_custom_mission, prepare_installed_custom_mission,
};
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::prepare_installed_custom_mission;

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
        LiveArchiveAssets {
            mission_basename: &manifest.mission_basename,
            map_filename: &manifest.map_filename,
            rhm_entry: &manifest.mission_rhm_entry,
            requires_spellforge: manifest.requires_spellforge,
            mission_archive,
            shared_archive,
            installed,
            distributed_cache: Some(DistributedCacheIdentity {
                schema_version: manifest.schema_version,
                full_mod_sha256: manifest.full_mod_sha256,
                encoded_bytes,
            }),
        },
        files,
    )
}

fn prepare_archive_assets(
    live: LiveArchiveAssets<'_>,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<PreparedLiveMissionAssets, String> {
    let (resolved, spellforge_package) = retain_live_mission_assets(live, files)?;
    Ok(PreparedLiveMissionAssets {
        resolved: Arc::new(resolved),
        spellforge_package,
    })
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
    use {super::native::select_direct_rhm_entry, robin_engine::mission_assets::InstalledModsRoot};

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
                LiveArchiveAssets {
                    mission_basename: "H01_Lin",
                    map_filename: "lincoln",
                    rhm_entry: selected,
                    requires_spellforge,
                    mission_archive: bytes.clone(),
                    shared_archive: shared.clone(),
                    installed: Some(InstalledArchiveLocator {
                        root: robin_engine::mission_assets::InstalledModsRoot::ConfiguredMods,
                        mission_relative_path: "mission.zip".into(),
                        shared_relative_path: shared.as_ref().map(|_| "shared.zip".into()),
                    }),
                    distributed_cache: None,
                },
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
                LiveArchiveAssets {
                    mission_basename: "H01_Lin",
                    map_filename: map,
                    rhm_entry: selected,
                    requires_spellforge: false,
                    mission_archive: archive.into(),
                    shared_archive: None,
                    installed: None,
                    distributed_cache: None,
                },
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
