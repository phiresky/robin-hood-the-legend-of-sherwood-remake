//! Required-content profile loading for the confined verifier.
//!
//! Ranked verification never creates player profiles and never consults
//! presentation localization. This loader admits only the official profile
//! catalog inside the irreversibly confined raw-content root. Ranked official
//! content currently permits no overlays.

use robin_engine::engine::GlobalOptions;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::SbFileSystem;

#[derive(Debug, thiserror::Error)]
pub enum ProfileLoadError {
    #[error("invalid JSON profile catalog {path}: {source}")]
    Json {
        path: &'static str,
        #[source]
        source: robin_engine::profiles::ProfileJsonLoadError,
    },
    #[error("cannot open legacy profile catalog {path}: file status {status}")]
    Open { path: &'static str, status: i32 },
    #[error("cannot decode legacy profile catalog {path}: {message}")]
    Decode { path: &'static str, message: String },
}

pub fn load_profiles(
    options: &GlobalOptions,
    files: &SbFileSystem,
) -> Result<ProfileManager, ProfileLoadError> {
    let document = {
        let json_path = "Data/Configuration/profile.cpf.json";
        if files
            .try_exists(json_path)
            .map_err(|status| ProfileLoadError::Open {
                path: json_path,
                status,
            })?
        {
            tracing::info!(path = json_path, "verifier loading JSON profile catalog");
            ProfileManager::load_json_document_with_files(json_path, files).map_err(|source| {
                ProfileLoadError::Json {
                    path: json_path,
                    source,
                }
            })?
        } else {
            let cpf_path = "Data/Configuration/profile.cpf";
            tracing::info!(path = cpf_path, "verifier loading legacy profile catalog");
            let mut file = files
                .open(cpf_path)
                .map_err(|status| ProfileLoadError::Open {
                    path: cpf_path,
                    status,
                })?;
            let mut profiles = ProfileManager::new();
            profiles
                .load_all_legacy_cpf(&mut file)
                .map_err(|error| ProfileLoadError::Decode {
                    path: cpf_path,
                    message: error.to_string(),
                })?;
            robin_engine::content_patch::profile_document(&profiles).map_err(|message| {
                ProfileLoadError::Decode {
                    path: cpf_path,
                    message,
                }
            })?
        }
    };

    let mut document = document;
    let path = robin_engine::content_patch::PROFILE_PATCH_PATH;
    robin_engine::content_patch::reject_legacy(
        files,
        "Data/Configuration/soldier-profiles.patch.json",
        path,
    )
    .map_err(|message| ProfileLoadError::Decode { path, message })?;
    let layers = robin_engine::content_patch::read_layers(files, path)
        .map_err(|message| ProfileLoadError::Decode { path, message })?;
    for (index, bytes) in layers.iter().enumerate() {
        document = robin_engine::content_patch::apply_profile_document(&document, bytes).map_err(
            |message| ProfileLoadError::Decode {
                path,
                message: format!("layer {index}: {message}"),
            },
        )?;
    }
    let mut profiles = robin_engine::content_patch::profiles_from_document(document)
        .map_err(|message| ProfileLoadError::Decode { path, message })?;
    profiles.import_beam_mes_with_files(&options.level_directory, files);
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn verifier_applies_generic_profile_patch_to_actual_catalog() {
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        let base = ProfileManager {
            soldiers: vec![robin_engine::profiles::SoldierProfile {
                filename: "Guard".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut document = robin_engine::content_patch::profile_document(&base).unwrap();
        let guard = document["soldiers"]
            .as_object_mut()
            .unwrap()
            .remove("Guard")
            .unwrap();
        document["soldiers"]["guard-template"] = guard;
        document["soldier_order"][0] = serde_json::json!("guard-template");
        vfs.install_preloaded_asset(
            "Data/Configuration/profile.cpf.json",
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        vfs.install_preloaded_asset(
            robin_engine::content_patch::PROFILE_PATCH_PATH,
            br#"[
            {"op":"replace","path":"/soldiers/guard-template/life_point","value":150}
        ]"#
            .to_vec(),
        )
        .unwrap();
        let result = load_profiles(&GlobalOptions::default(), &SbFileSystem::new(vfs)).unwrap();
        assert_eq!(result.soldiers[0].life_point, 150);
    }

    #[test]
    fn verifier_keeps_profile_json_source_chain() {
        let vfs = std::sync::Arc::new(robin_util::asset_fs::AssetVfs::new());
        vfs.install_preloaded_asset("Data/Configuration/profile.cpf.json", b"{ broken".to_vec())
            .unwrap();
        let files = SbFileSystem::new(vfs);
        let error = load_profiles(&GlobalOptions::default(), &files).unwrap_err();
        let error = crate::ranked_verifier::RankedVerifierLoadError::Profiles(error);
        let catalog = error.source().unwrap();
        assert!(catalog.is::<ProfileLoadError>());
        let json = catalog.source().unwrap();
        assert!(json.is::<robin_engine::profiles::ProfileJsonLoadError>());
        assert!(json.source().unwrap().is::<serde_json::Error>());
    }
}
