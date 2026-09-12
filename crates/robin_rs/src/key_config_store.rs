//! Per-profile key-binding persistence.
//!
//! Each profile gets its own active and custom slots. The original game stores
//! both configurations on the player profile and copies the active one into its
//! input translator. The Rust port keeps the same per-profile ownership on the
//! host side because physical [`winit::keyboard::KeyCode`] values are not
//! deterministic engine state.
//!
//! Stored as `<save_directory>/keyconfigs.json` next to `profiles.json`.

use crate::key_config::KeyConfig;
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const KEY_CONFIG_PROFILE_LIMIT: usize = 10;
const KEY_CONFIG_BINDING_LIMIT: usize = 64;
const KEY_CONFIG_ACTION_BYTE_LIMIT: usize = 256;
#[cfg(any(test, target_arch = "wasm32"))]
const BROWSER_KEY_CONFIG_SCHEMA_VERSION: u32 = 1;
#[cfg(any(test, target_arch = "wasm32"))]
const BROWSER_KEY_CONFIG_BYTE_LIMIT: usize = 512 * 1024;
#[cfg(target_arch = "wasm32")]
const BROWSER_KEY_CONFIG_STORE_KEY: &str = "robin-hood-key-configs-v1";

/// One profile's two key-config slots.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileKeyConfig {
    /// Currently-applied bindings.
    pub active: KeyConfig,
    /// User's saved custom bindings. The User Defined button restores
    /// from this slot.
    pub custom: KeyConfig,
}

impl ProfileKeyConfig {
    /// New entry seeded with `default_preset` for both slots, so a
    /// freshly-created profile has sensible bindings before the user
    /// edits them.
    pub fn fresh() -> Self {
        let preset = KeyConfig::default_preset();
        Self {
            active: preset.clone(),
            custom: preset,
        }
    }

    fn ensure_current_bindings(&mut self) {
        self.active.migrate_post_port_bindings();
        self.custom.migrate_post_port_bindings();
    }
}

/// Per-profile key-binding store, keyed by `PlayerProfile::id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyConfigStore {
    pub configs: BTreeMap<u32, ProfileKeyConfig>,
    #[serde(skip)]
    pub save_directory: String,
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserKeyConfigEnvelope<T = KeyConfigStore> {
    schema_version: u32,
    store: T,
}

impl KeyConfigStore {
    pub fn new(save_directory: String) -> Self {
        Self {
            configs: BTreeMap::new(),
            save_directory,
        }
    }

    /// Load from `<directory>/keyconfigs.json`.  Returns an empty store
    /// if the file does not yet exist (first-run case).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(directory: &str) -> std::io::Result<Self> {
        let path = Self::store_path(directory);
        match fs::File::open(&path) {
            Ok(file) => {
                let store: KeyConfigStore = serde_json::from_reader(std::io::BufReader::new(file))
                    .map_err(|error| {
                        std::io::Error::new(
                            error
                                .io_error_kind()
                                .unwrap_or(std::io::ErrorKind::InvalidData),
                            error,
                        )
                    })?;
                store.finish_loading(directory)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::new(directory.to_owned()))
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn load(directory: &str) -> std::io::Result<Self> {
        let storage = browser_key_config_storage()?;
        let Some(serialized) = storage
            .get_item(BROWSER_KEY_CONFIG_STORE_KEY)
            .map_err(|error| browser_key_config_io("read browser key configs", error))?
        else {
            return Ok(Self::new(directory.to_owned()));
        };
        decode_browser_key_config_archive(&serialized, directory)
    }

    /// Atomically persist to `<save_directory>/keyconfigs.json`.
    /// Invalid snapshots are rejected before touching the existing archive.
    /// On error retain this desired snapshot for retry. Native errors expose
    /// [`crate::desktop_persistence::PublicationFailure`] to distinguish a
    /// published archive whose directory synchronization failed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self) -> std::io::Result<()> {
        self.validate_archive()?;
        let path = Self::store_path(&self.save_directory);
        crate::desktop_persistence::write_json(&path, self)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn save(&self) -> std::io::Result<()> {
        let serialized = encode_browser_key_config_archive(self)?;
        browser_key_config_storage()?
            .set_item(BROWSER_KEY_CONFIG_STORE_KEY, &serialized)
            .map_err(|error| browser_key_config_io("persist browser key configs", error))
    }

    /// Apply the same migrations and admission policy to every persisted format.
    fn finish_loading(mut self, directory: &str) -> std::io::Result<Self> {
        for config in self.configs.values_mut() {
            config.ensure_current_bindings();
        }
        self.validate_archive()?;
        self.save_directory = directory.to_owned();
        Ok(self)
    }

    fn validate_archive(&self) -> std::io::Result<()> {
        if self.configs.len() > KEY_CONFIG_PROFILE_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "key-config archive contains {} profiles; limit is {KEY_CONFIG_PROFILE_LIMIT}",
                    self.configs.len()
                ),
            ));
        }
        for (&profile_id, profile) in &self.configs {
            for (label, config) in [("active", &profile.active), ("custom", &profile.custom)] {
                if config.bindings.len() > KEY_CONFIG_BINDING_LIMIT {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "profile {profile_id} {label} key config has {} bindings; limit is {KEY_CONFIG_BINDING_LIMIT}",
                            config.bindings.len()
                        ),
                    ));
                }
                let mut actions = std::collections::BTreeSet::new();
                for binding in &config.bindings {
                    if binding.action.is_empty()
                        || binding.action.len() > KEY_CONFIG_ACTION_BYTE_LIMIT
                    {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "profile {profile_id} {label} key action is {} bytes; expected 1..={KEY_CONFIG_ACTION_BYTE_LIMIT}",
                                binding.action.len()
                            ),
                        ));
                    }
                    if !actions.insert(binding.action.as_str()) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "profile {profile_id} {label} key config repeats action `{}`",
                                binding.action
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Look up — or insert a default — the entry for `profile_id`.
    pub fn entry_or_default(&mut self, profile_id: u32) -> &mut ProfileKeyConfig {
        let config = self
            .configs
            .entry(profile_id)
            .or_insert_with(ProfileKeyConfig::fresh);
        config.ensure_current_bindings();
        config
    }

    /// Read-only lookup; returns `None` if the profile has no entry.
    pub fn get(&self, profile_id: u32) -> Option<&ProfileKeyConfig> {
        self.configs.get(&profile_id)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn store_path(directory: &str) -> PathBuf {
        Path::new(directory).join("keyconfigs.json")
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_key_config_storage() -> std::io::Result<web_sys::Storage> {
    crate::browser_storage::local_storage().map_err(std::io::Error::other)
}

#[cfg(target_arch = "wasm32")]
fn browser_key_config_io(operation: &str, error: impl std::fmt::Debug) -> std::io::Error {
    std::io::Error::other(format!("{operation}: {error:?}"))
}

#[cfg(any(test, target_arch = "wasm32"))]
fn encode_browser_key_config_archive(store: &KeyConfigStore) -> std::io::Result<String> {
    store.validate_archive()?;
    let serialized = serde_json::to_string(&BrowserKeyConfigEnvelope {
        schema_version: BROWSER_KEY_CONFIG_SCHEMA_VERSION,
        store,
    })
    .map_err(std::io::Error::other)?;
    if serialized.len() > BROWSER_KEY_CONFIG_BYTE_LIMIT {
        return Err(std::io::Error::other(format!(
            "browser key-config archive is {} bytes; limit is {BROWSER_KEY_CONFIG_BYTE_LIMIT}",
            serialized.len()
        )));
    }
    Ok(serialized)
}

#[cfg(any(test, target_arch = "wasm32"))]
fn decode_browser_key_config_archive(
    serialized: &str,
    directory: &str,
) -> std::io::Result<KeyConfigStore> {
    if serialized.len() > BROWSER_KEY_CONFIG_BYTE_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "browser key-config archive is {} bytes; limit is {BROWSER_KEY_CONFIG_BYTE_LIMIT}",
                serialized.len()
            ),
        ));
    }
    let envelope: BrowserKeyConfigEnvelope = serde_json::from_str(serialized)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if envelope.schema_version != BROWSER_KEY_CONFIG_SCHEMA_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported browser key-config schema {}; expected {BROWSER_KEY_CONFIG_SCHEMA_VERSION}",
                envelope.schema_version
            ),
        ));
    }
    envelope.store.finish_loading(directory)
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_persistence_tests;
