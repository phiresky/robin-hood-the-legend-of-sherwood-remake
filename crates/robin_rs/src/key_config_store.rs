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
        match fs::read_to_string(&path) {
            Ok(data) => {
                let store: KeyConfigStore = serde_json::from_str(&data)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
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
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn native_and_browser_loads_share_migration_and_validation() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().to_str().unwrap();
        let mut legacy = KeyConfigStore::new("obsolete".into());
        legacy.configs.insert(7, ProfileKeyConfig::default());
        for invalid in [false, true] {
            if invalid {
                legacy.configs.get_mut(&7).unwrap().custom.bindings = vec![
                    crate::key_config::KeyBinding {
                        action: "ZoomIn".into(),
                        primary_key: None,
                        secondary_key: None,
                    }; 2
                ];
            }
            fs::write(
                KeyConfigStore::store_path(directory),
                serde_json::to_vec(&legacy).unwrap(),
            )
            .unwrap();
            let browser = serde_json::to_string(&BrowserKeyConfigEnvelope {
                schema_version: BROWSER_KEY_CONFIG_SCHEMA_VERSION,
                store: &legacy,
            })
            .unwrap();
            let native = KeyConfigStore::load(directory);
            let browser = decode_browser_key_config_archive(&browser, directory);
            if invalid {
                assert_eq!(native.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
                assert_eq!(browser.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
            } else {
                let native = native.unwrap();
                let browser = browser.unwrap();
                assert_eq!(native.save_directory, directory);
                assert_eq!(browser.save_directory, directory);
                assert_eq!(
                    serde_json::to_value(&native).unwrap(),
                    serde_json::to_value(&browser).unwrap()
                );
                let profile = native.get(7).unwrap();
                for config in [&profile.active, &profile.custom] {
                    assert!(config.get_binding("ToggleCloak").is_some());
                }
            }
        }
    }

    #[test]
    fn browser_archive_preserves_wire_format_and_rejects_invalid_documents() {
        let mut store = KeyConfigStore::new("old-directory".into());
        store
            .entry_or_default(7)
            .active
            .set_binding("ZoomIn", Some(KeyCode::Backspace), None);
        let encoded = encode_browser_key_config_archive(&store).unwrap();
        let legacy = serde_json::to_string(&BrowserKeyConfigEnvelope {
            schema_version: BROWSER_KEY_CONFIG_SCHEMA_VERSION,
            store: store.clone(),
        })
        .unwrap();
        assert_eq!(encoded, legacy);
        let decoded = decode_browser_key_config_archive(&encoded, "selected").unwrap();
        assert_eq!(decoded.save_directory, "selected");
        assert_eq!(
            serde_json::to_value(&decoded).unwrap(),
            serde_json::to_value(&store).unwrap()
        );
        let document: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        for (field, value) in [
            ("schema_version", serde_json::json!(999)),
            ("unexpected", serde_json::json!(true)),
            ("store", serde_json::json!({})),
        ] {
            let mut corrupt = document.clone();
            corrupt[field] = value;
            assert_eq!(
                decode_browser_key_config_archive(&corrupt.to_string(), "selected")
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidData
            );
        }
        assert!(decode_browser_key_config_archive("not JSON", "selected").is_err());
        assert!(
            decode_browser_key_config_archive(
                &"x".repeat(BROWSER_KEY_CONFIG_BYTE_LIMIT + 1),
                "selected"
            )
            .is_err()
        );
        for id in 0..=KEY_CONFIG_PROFILE_LIMIT as u32 {
            store.entry_or_default(id);
        }
        assert!(encode_browser_key_config_archive(&store).is_err());
    }

    #[test]
    fn restart_ignores_incomplete_staging_and_preserves_selected_directory() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().to_str().unwrap();
        let mut store = KeyConfigStore::new(directory.into());
        store
            .entry_or_default(7)
            .active
            .set_binding("ZoomIn", Some(KeyCode::Backspace), None);
        store.save().unwrap();
        fs::write(
            dir.path().join(".robin-user-store-staging-abandoned"),
            b"{partial",
        )
        .unwrap();
        let loaded = KeyConfigStore::load(directory).unwrap();
        assert_eq!(loaded.save_directory, directory);
        assert_eq!(
            loaded
                .get(7)
                .unwrap()
                .active
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::Backspace)
        );
        loaded.save().unwrap();
        assert_eq!(KeyConfigStore::load(directory).unwrap().configs.len(), 1);
    }

    #[test]
    fn unreadable_archive_is_not_a_missing_store() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("keyconfigs.json")).unwrap();
        assert!(KeyConfigStore::load(dir.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn fresh_seeds_both_slots_with_default_preset() {
        let entry = ProfileKeyConfig::fresh();
        assert_eq!(entry.active.key_type, KeyConfig::default_preset().key_type);
        assert_eq!(entry.custom.key_type, KeyConfig::default_preset().key_type);
        assert_eq!(
            entry.active.bindings.len(),
            KeyConfig::default_preset().bindings.len()
        );
    }

    #[test]
    fn entry_or_default_inserts_once() {
        let mut store = KeyConfigStore::new("/tmp/test".into());
        store
            .entry_or_default(7)
            .active
            .set_binding("ZoomIn", Some(KeyCode::Backspace), None);

        let again = store.entry_or_default(7);
        assert_eq!(
            again.active.get_binding("ZoomIn").unwrap().primary_key,
            Some(KeyCode::Backspace)
        );
        assert_eq!(store.configs.len(), 1);
    }

    #[test]
    fn invalid_save_preserves_the_last_valid_archive() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().to_str().unwrap();
        let mut valid = KeyConfigStore::new(directory.into());
        valid.entry_or_default(7);
        valid.save().unwrap();
        let path = dir.path().join("keyconfigs.json");
        let original = fs::read(&path).unwrap();
        for custom in [false, true] {
            for invalid_kind in 0..4 {
                let mut invalid = valid.clone();
                let entry = invalid.entry_or_default(7);
                let config = if custom {
                    &mut entry.custom
                } else {
                    &mut entry.active
                };
                match invalid_kind {
                    0 => config.bindings[0].action.clear(),
                    1 => config.bindings[0].action = "x".repeat(KEY_CONFIG_ACTION_BYTE_LIMIT + 1),
                    2 => config.bindings.push(config.bindings[0].clone()),
                    3 => config
                        .bindings
                        .resize(KEY_CONFIG_BINDING_LIMIT + 1, config.bindings[0].clone()),
                    _ => unreachable!(),
                }
                assert_eq!(
                    invalid.save().unwrap_err().kind(),
                    std::io::ErrorKind::InvalidData
                );
                assert_eq!(fs::read(&path).unwrap(), original);
            }
        }
        let mut invalid = valid.clone();
        for id in 0..=KEY_CONFIG_PROFILE_LIMIT as u32 {
            invalid.entry_or_default(id);
        }
        assert_eq!(
            invalid.save().unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            serde_json::to_value(KeyConfigStore::load(directory).unwrap()).unwrap(),
            serde_json::to_value(valid).unwrap()
        );
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap().to_owned();

        {
            let mut store = KeyConfigStore::new(dir_str.clone());
            let entry = store.entry_or_default(3);
            entry.active.set_binding("ZoomIn", Some(KeyCode::F2), None);
            entry.custom.set_binding("ZoomIn", Some(KeyCode::F3), None);
            store.save().unwrap();
        }

        let loaded = KeyConfigStore::load(&dir_str).unwrap();
        let entry = loaded.get(3).expect("profile 3 should round-trip");
        assert_eq!(
            entry.active.get_binding("ZoomIn").unwrap().primary_key,
            Some(KeyCode::F2)
        );
        assert_eq!(
            entry.custom.get_binding("ZoomIn").unwrap().primary_key,
            Some(KeyCode::F3)
        );
    }

    #[test]
    fn loads_keyconfigs_written_before_type_move() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{
            "configs": {
                "41": {
                    "active": {
                        "bindings": [{
                            "action": "Crouch",
                            "primary_key": "ShiftLeft",
                            "secondary_key": "ShiftRight"
                        }],
                        "key_type": 1
                    },
                    "custom": {
                        "bindings": [],
                        "key_type": 2
                    }
                }
            }
        }"#;
        std::fs::write(dir.path().join("keyconfigs.json"), json).unwrap();

        let loaded = KeyConfigStore::load(dir.path().to_str().unwrap()).unwrap();
        let entry = loaded.get(41).expect("legacy profile should load");
        assert_eq!(entry.active.key_type, 1);
        assert_eq!(
            entry.active.get_binding("Crouch").unwrap().primary_key,
            Some(KeyCode::ShiftLeft)
        );
        assert_eq!(
            entry.active.get_binding("Crouch").unwrap().secondary_key,
            Some(KeyCode::ShiftRight)
        );
        assert_eq!(
            entry
                .active
                .get_binding("ToggleCloak")
                .expect("legacy active config gains cloak action")
                .primary_key,
            Some(KeyCode::KeyV)
        );
        assert_eq!(
            entry
                .custom
                .get_binding("ToggleCloak")
                .expect("legacy custom config gains cloak action")
                .primary_key,
            Some(KeyCode::KeyV)
        );
        assert_eq!(entry.custom.key_type, 2);
        assert_eq!(
            entry
                .active
                .get_binding("PlanQuickActions")
                .expect("legacy active config is migrated")
                .primary_key,
            None,
            "an unrelated custom Shift binding must not be duplicated"
        );
        assert_eq!(
            entry
                .custom
                .get_binding("PlanQuickActions")
                .expect("legacy custom config is migrated")
                .primary_key,
            Some(KeyCode::ShiftLeft)
        );
    }

    #[test]
    fn load_missing_file_returns_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyConfigStore::load(dir.path().to_str().unwrap()).unwrap();
        assert!(store.configs.is_empty());
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_persistence_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;
    use winit::keyboard::KeyCode;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn durable_key_config_reload_rejects_unknown_schema() {
        let storage = browser_key_config_storage().unwrap();
        storage.remove_item(BROWSER_KEY_CONFIG_STORE_KEY).unwrap();

        let mut first = KeyConfigStore::load("browser-save").unwrap();
        first
            .entry_or_default(7)
            .active
            .set_binding("ZoomIn", Some(KeyCode::Backspace), None);
        first.save().unwrap();

        let reloaded = KeyConfigStore::load("ignored-after-load").unwrap();
        assert_eq!(
            reloaded
                .get(7)
                .unwrap()
                .active
                .get_binding("ZoomIn")
                .unwrap()
                .primary_key,
            Some(KeyCode::Backspace)
        );
        assert_eq!(reloaded.save_directory, "ignored-after-load");

        storage
            .set_item(
                BROWSER_KEY_CONFIG_STORE_KEY,
                r#"{"schema_version":999,"store":{}}"#,
            )
            .unwrap();
        assert!(KeyConfigStore::load("browser-save").is_err());
        assert!(
            decode_browser_key_config_archive(
                &"x".repeat(BROWSER_KEY_CONFIG_BYTE_LIMIT + 1),
                "browser-save"
            )
            .unwrap_err()
            .to_string()
            .contains("limit")
        );
        storage.remove_item(BROWSER_KEY_CONFIG_STORE_KEY).unwrap();
    }
}
