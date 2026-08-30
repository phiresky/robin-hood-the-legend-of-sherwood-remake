//! Per-profile key-binding persistence.
//!
//! Each profile gets its own active and custom slots. The original game stores
//! both configurations on `RHPlayerProfile` and copies the active one into its
//! input translator. The Rust port keeps the same per-profile ownership on the
//! host side because physical [`winit::keyboard::KeyCode`] values are not
//! deterministic engine state.
//!
//! Stored as `<save_directory>/keyconfigs.json` next to `profiles.json`.

use crate::key_config::KeyConfig;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

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
        self.active.ensure_reusable_cloak_binding();
        self.custom.ensure_reusable_cloak_binding();
    }
}

/// Per-profile key-binding store, keyed by `PlayerProfile::id`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyConfigStore {
    pub configs: BTreeMap<u32, ProfileKeyConfig>,
    #[serde(skip)]
    pub save_directory: String,
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
    pub fn load(directory: &str) -> std::io::Result<Self> {
        let path = Self::store_path(directory);
        if path.exists() {
            let data = fs::read_to_string(&path)?;
            let mut store: KeyConfigStore = serde_json::from_str(&data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            for config in store.configs.values_mut() {
                config.ensure_current_bindings();
            }
            store.save_directory = directory.to_owned();
            Ok(store)
        } else {
            Ok(Self::new(directory.to_owned()))
        }
    }

    /// Persist to `<save_directory>/keyconfigs.json`.
    pub fn save(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.save_directory)?;
        let path = Self::store_path(&self.save_directory);
        let data = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, data)
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

    fn store_path(directory: &str) -> PathBuf {
        Path::new(directory).join("keyconfigs.json")
    }
}

// ─── Global singleton ───────────────────────────────────────────────

static GLOBAL_STORE: Mutex<Option<KeyConfigStore>> = Mutex::new(None);

impl KeyConfigStore {
    /// Acquire the global key-config store.  Initialized in
    /// `main_entry::init_global_key_config_store`.
    pub fn global() -> std::sync::MutexGuard<'static, Option<KeyConfigStore>> {
        GLOBAL_STORE.lock().unwrap()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

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
    }

    #[test]
    fn load_missing_file_returns_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyConfigStore::load(dir.path().to_str().unwrap()).unwrap();
        assert!(store.configs.is_empty());
    }
}
