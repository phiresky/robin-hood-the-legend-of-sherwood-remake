//! Required-content profile loading for the confined verifier.
//!
//! Ranked verification never creates player profiles and never consults
//! presentation localization. This loader admits only the official profile
//! catalog inside the irreversibly confined raw-content root. Ranked official
//! content currently permits no overlays.

use robin_engine::engine::GlobalOptions;
use robin_engine::profiles::ProfileManager;
use robin_engine::sbfile::{SB_FILE_READ, SbFileSystem};

#[derive(Debug, thiserror::Error)]
pub enum ProfileLoadError {
    #[error("invalid JSON profile catalog {path}: {message}")]
    Json { path: &'static str, message: String },
    #[error("cannot open legacy profile catalog {path}: file status {status}")]
    Open { path: &'static str, status: i32 },
    #[error("cannot decode legacy profile catalog {path}: {message}")]
    Decode { path: &'static str, message: String },
}

pub fn load_profiles(
    options: &GlobalOptions,
    files: &SbFileSystem,
) -> Result<ProfileManager, ProfileLoadError> {
    let profiles = {
        let json_path = "Data/Configuration/profile.cpf.json";
        if files
            .try_exists(json_path)
            .map_err(|status| ProfileLoadError::Open {
                path: json_path,
                status,
            })?
        {
            tracing::info!(path = json_path, "verifier loading JSON profile catalog");
            let mut profiles =
                ProfileManager::load_json_with_files(json_path, files).map_err(|message| {
                    ProfileLoadError::Json {
                        path: json_path,
                        message,
                    }
                })?;
            profiles.import_beam_mes_with_files(&options.level_directory, files);
            profiles
        } else {
            let cpf_path = "Data/Configuration/profile.cpf";
            tracing::info!(path = cpf_path, "verifier loading legacy profile catalog");
            let mut file =
                files
                    .open(cpf_path, SB_FILE_READ)
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
            profiles.import_beam_mes_with_files(&options.level_directory, files);
            profiles
        }
    };

    Ok(profiles)
}
