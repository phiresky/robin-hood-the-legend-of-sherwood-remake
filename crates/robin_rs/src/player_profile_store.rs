//! Platform-owned persistence for pure engine profile values.
//!
//! The store selects authority at application initialization. The legacy
//! manager's serialized save_directory remains wire-compatible metadata and
//! cannot redirect an already-created store.
use robin_engine::player_profile::{DifficultyLevel, PlayerProfileManager};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum PlayerProfileStore {
    #[cfg(not(target_arch = "wasm32"))]
    Native {
        directory: std::path::PathBuf,
    },
    #[cfg(target_arch = "wasm32")]
    Browser {
        directory: String,
    },
    Unavailable {
        reason: String,
    },
}

impl Default for PlayerProfileStore {
    fn default() -> Self {
        Self::unavailable("profile persistence authority is unavailable in a deserialized context")
    }
}

impl PlayerProfileStore {
    pub fn for_directory(directory: &str) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::Native {
                directory: directory.into(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self::Browser {
                directory: directory.into(),
            }
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub(crate) fn directory(&self) -> std::io::Result<&str> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Native { directory } => directory.to_str().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "player-profile directory cannot be represented as UTF-8 archive metadata",
                )
            }),
            #[cfg(target_arch = "wasm32")]
            Self::Browser { directory } => Ok(directory),
            Self::Unavailable { reason } => Err(std::io::Error::other(reason.clone())),
        }
    }

    /// Missing storage creates and durably writes the original single Robin
    /// default. Corruption and unavailable storage are errors, not defaults.
    pub fn load(&self) -> std::io::Result<PlayerProfileManager> {
        let directory = self.directory()?;
        let existing = match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Native { directory: root } => {
                match std::fs::read_to_string(root.join("profiles.json")) {
                    Ok(serialized) => Some(decode_native_archive(&serialized, directory)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error),
                }
            }
            #[cfg(target_arch = "wasm32")]
            Self::Browser { directory } => browser_profile_storage()?
                .get_item(BROWSER_PROFILE_STORE_KEY)
                .map_err(|error| browser_profile_io("read browser player profiles", error))?
                .map(|serialized| decode_browser_profile_archive(&serialized, directory))
                .transpose()?,
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        };
        if let Some(manager) = existing {
            return Ok(manager);
        }
        let mut manager = PlayerProfileManager::new(directory.to_owned());
        let index = manager.create_profile("Robin".into(), DifficultyLevel::Medium);
        manager.set_active(index);
        manager.default_profiles = true;
        self.save(&manager)?;
        Ok(manager)
    }

    /// Persist this snapshot without changing the caller's in-memory state.
    /// Invalid snapshots are rejected before touching the existing archive.
    /// Native publication failures include their publication stage in the
    /// error; retain and retry the desired snapshot even when replacement was
    /// visible but its directory synchronization could not be confirmed.
    pub fn save(&self, manager: &PlayerProfileManager) -> std::io::Result<()> {
        if manager.save_directory != self.directory()? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "player-profile directory differs from its initialized persistence authority",
            ));
        }
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Native { directory } => {
                manager
                    .validate_archive()
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
                crate::desktop_persistence::write_json(&directory.join("profiles.json"), manager)
            }
            #[cfg(target_arch = "wasm32")]
            Self::Browser { .. } => {
                let serialized = encode_browser_profile_archive(manager)?;
                browser_profile_storage()?
                    .set_item(BROWSER_PROFILE_STORE_KEY, &serialized)
                    .map_err(|error| browser_profile_io("persist browser player profiles", error))
            }
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }

    /// Rename saves aside before publishing deletion. They remain recoverable:
    /// the profile archive itself decides whether startup must restore them.
    pub(crate) fn quarantine_profile_saves(&self, profile_id: u32) -> std::io::Result<()> {
        self.move_profile_saves(profile_id, false)
    }

    pub(crate) fn restore_profile_saves(&self, profile_id: u32) -> std::io::Result<()> {
        self.move_profile_saves(profile_id, true)
    }

    pub(crate) fn restore_interrupted_deletions(
        &self,
        profiles: &PlayerProfileManager,
    ) -> std::io::Result<()> {
        for profile in &profiles.profiles {
            self.restore_profile_saves(profile.id)?;
        }
        Ok(())
    }

    fn move_profile_saves(&self, profile_id: u32, restore: bool) -> std::io::Result<()> {
        self.directory()?;
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Native { directory } => {
                let name = robin_engine::player_profile::profile_save_subdirectory(profile_id);
                let live = directory.join(&name);
                let deleted = directory.join(format!(".deleted-{name}"));
                let (source, destination) = if restore {
                    (deleted, live)
                } else {
                    (live, deleted)
                };
                match std::fs::symlink_metadata(&source) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => return Err(error),
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(std::io::Error::other(format!(
                            "profile saves are not a directory: {}",
                            source.display()
                        )));
                    }
                    Ok(_) => {}
                }
                match std::fs::symlink_metadata(&destination) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                    Ok(_) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            format!(
                                "refusing to replace profile saves at {}",
                                destination.display()
                            ),
                        ));
                    }
                }
                std::fs::rename(&source, &destination)?;
                #[cfg(unix)]
                std::fs::File::open(directory)?.sync_all()?;
                tracing::info!(
                    "Moved profile saves {} → {}",
                    source.display(),
                    destination.display()
                );
                Ok(())
            }
            #[cfg(target_arch = "wasm32")]
            Self::Browser { .. } => {
                // Browser saves have their own store; the legacy profile
                // archive did not perform filesystem directory deletion.
                let _ = (profile_id, restore);
                Ok(())
            }
            Self::Unavailable { .. } => unreachable!("directory checked authority"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_native_archive(
    serialized: &str,
    directory: &str,
) -> std::io::Result<PlayerProfileManager> {
    let mut manager: PlayerProfileManager = serde_json::from_str(serialized)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    manager
        .validate_archive()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    manager.save_directory = directory.into();
    Ok(manager)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn directory_lookup_borrows_the_selected_storage_path() {
        let store = PlayerProfileStore::for_directory("selected-profile-root");
        let PlayerProfileStore::Native { directory } = &store else {
            panic!("expected native profile storage");
        };
        assert!(std::ptr::eq(
            store.directory().unwrap(),
            directory.to_str().unwrap()
        ));
        assert!(
            PlayerProfileStore::unavailable("no authority")
                .directory()
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_storage_authority_is_rejected_before_filesystem_changes() {
        use std::os::unix::ffi::OsStringExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root
            .path()
            .join(std::ffi::OsString::from_vec(vec![b'p', 0xff]));
        let store = PlayerProfileStore::Native {
            directory: directory.clone(),
        };
        let manager = PlayerProfileManager::new(directory.to_string_lossy().into_owned());
        for result in [
            store.directory().map(|_| ()),
            store.load().map(|_| ()),
            store.save(&manager),
            store.quarantine_profile_saves(0),
            store.restore_profile_saves(0),
        ] {
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn profile_archive_decides_recovery_of_interrupted_save_rename() {
        for committed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let store = PlayerProfileStore::for_directory(root.path().to_str().unwrap());
            let mut profiles = store.load().unwrap();
            profiles.create_profile("Marian".into(), DifficultyLevel::Hard);
            store.save(&profiles).unwrap();
            let saves = root.path().join("Profile_000");
            fs::create_dir(&saves).unwrap();
            fs::write(saves.join("QuickSave.json"), b"precious save").unwrap();
            store.quarantine_profile_saves(0).unwrap();
            if committed {
                profiles.delete_profile(0);
                profiles.set_active(0);
                store.save(&profiles).unwrap();
            }
            // Reopen exactly as startup would after the process died.
            let restarted = PlayerProfileStore::for_directory(root.path().to_str().unwrap());
            let loaded = restarted.load().unwrap();
            restarted.restore_interrupted_deletions(&loaded).unwrap();
            assert_eq!(
                loaded.profiles.iter().any(|profile| profile.id == 0),
                !committed
            );
            let retained = if committed {
                root.path().join(".deleted-Profile_000/QuickSave.json")
            } else {
                saves.join("QuickSave.json")
            };
            assert_eq!(fs::read(retained).unwrap(), b"precious save");
            assert_eq!(saves.exists(), !committed);
            restarted.restore_interrupted_deletions(&loaded).unwrap(); // Recovery is idempotent.
        }
    }

    #[test]
    fn recovery_never_overwrites_a_conflicting_save_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = PlayerProfileStore::for_directory(root.path().to_str().unwrap());
        store.load().unwrap();
        let live = root.path().join("Profile_000");
        fs::create_dir(&live).unwrap();
        fs::write(live.join("old"), b"old").unwrap();
        store.quarantine_profile_saves(0).unwrap();
        fs::create_dir(&live).unwrap();
        fs::write(live.join("new"), b"new").unwrap();
        assert!(
            store
                .restore_interrupted_deletions(&store.load().unwrap())
                .unwrap_err()
                .to_string()
                .contains("refusing to replace")
        );
        assert_eq!(fs::read(live.join("new")).unwrap(), b"new");
        assert_eq!(
            fs::read(root.path().join(".deleted-Profile_000/old")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn restart_ignores_incomplete_staging_without_regenerating_identity() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().to_str().unwrap();
        let store = PlayerProfileStore::for_directory(directory);
        let mut manager = store.load().unwrap();
        manager.profiles[0].name = "Retained Robin".into();
        manager.default_profiles = false;
        let identity = manager.profiles[0].id;
        store.save(&manager).unwrap();
        fs::write(
            dir.path().join(".robin-user-store-staging-abandoned"),
            b"{partial",
        )
        .unwrap();
        let restarted = PlayerProfileStore::for_directory(directory).load().unwrap();
        assert_eq!(restarted.profiles[0].name, "Retained Robin");
        assert_eq!(restarted.profiles[0].id, identity);
        assert!(!restarted.default_profiles);
        store.save(&restarted).unwrap();
    }
    #[test]
    fn load_creates_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = PlayerProfileStore::for_directory(dir.path().to_str().unwrap())
            .load()
            .unwrap();

        assert_eq!(mgr.profile_count(), 1);
        assert_eq!(mgr.profiles[0].name, "Robin");
        assert_eq!(mgr.active_index, Some(0));
        assert!(mgr.default_profiles);

        // File should have been written.
        assert!(dir.path().join("profiles.json").exists());
    }

    #[test]
    fn load_roundtrip_via_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();

        // Create and save.
        {
            let mut mgr = PlayerProfileManager::new(dir_str.into());
            mgr.create_profile("Alice".into(), DifficultyLevel::Hard);
            mgr.set_active(0);
            PlayerProfileStore::for_directory(dir_str)
                .save(&mgr)
                .unwrap();
        }

        // Load back.
        let mgr = PlayerProfileStore::for_directory(dir_str).load().unwrap();
        assert_eq!(mgr.profile_count(), 1);
        assert_eq!(mgr.profiles[0].name, "Alice");
        assert_eq!(mgr.profiles[0].difficulty, DifficultyLevel::Hard);
        assert_eq!(mgr.active_index, Some(0));
        assert!(mgr.profiles[0].campaign_history.attempts().is_empty());
    }

    #[test]
    fn load_rejects_aggregate_only_rust_profile() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let mut mgr = PlayerProfileManager::new(dir_str.into());
        let idx = mgr.create_profile("Legacy Robin".into(), DifficultyLevel::Medium);
        mgr.profiles[idx].score = 42;
        mgr.profiles[idx].play_time = 99;
        let mut document = serde_json::to_value(&mgr).unwrap();
        document["profiles"][idx]
            .as_object_mut()
            .unwrap()
            .remove("campaign_history");
        fs::create_dir_all(dir_str).unwrap();
        fs::write(
            dir.path().join("profiles.json"),
            serde_json::to_vec_pretty(&document).unwrap(),
        )
        .unwrap();

        let error = PlayerProfileStore::for_directory(dir_str)
            .load()
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn archive_directory_cannot_redirect_selected_store() {
        let selected = tempfile::tempdir().unwrap();
        let redirected = tempfile::tempdir().unwrap();
        let store = PlayerProfileStore::for_directory(selected.path().to_str().unwrap());
        let mut manager = store.load().unwrap();
        manager.save_directory = redirected.path().to_str().unwrap().into();
        let serialized = serde_json::to_string(&manager).unwrap();
        fs::write(selected.path().join("profiles.json"), &serialized).unwrap();
        assert_eq!(
            store.save(&manager).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        let mut loaded = store.load().unwrap();
        assert_eq!(loaded.save_directory, selected.path().to_str().unwrap());
        loaded.profiles[0].name = "selected".into();
        store.save(&loaded).unwrap();
        assert!(!redirected.path().join("profiles.json").exists());
        assert_eq!(store.load().unwrap().profiles[0].name, "selected");
    }

    #[test]
    fn corrupt_native_archive_is_not_overwritten_with_defaults() {
        let selected = tempfile::tempdir().unwrap();
        let store = PlayerProfileStore::for_directory(selected.path().to_str().unwrap());
        for corrupt in ["not JSON", "{}"] {
            fs::write(selected.path().join("profiles.json"), corrupt).unwrap();
            assert_eq!(
                store.load().unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
            assert_eq!(
                fs::read_to_string(selected.path().join("profiles.json")).unwrap(),
                corrupt
            );
        }
        let mut invalid = PlayerProfileManager::new(selected.path().to_str().unwrap().into());
        invalid.create_profile("Robin".into(), DifficultyLevel::Medium);
        invalid.active_index = Some(99);
        // Seed corruption directly: the public writer must reject it.
        fs::write(
            selected.path().join("profiles.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert_eq!(
            store.load().unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn invalid_save_preserves_the_last_valid_archive() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlayerProfileStore::for_directory(dir.path().to_str().unwrap());
        let valid = store.load().unwrap();
        let path = dir.path().join("profiles.json");
        let original = fs::read(&path).unwrap();
        for invalid_kind in 0..6 {
            let mut invalid = valid.clone();
            match invalid_kind {
                0 => invalid.profiles.clear(),
                1 => invalid.active_index = None,
                2 => invalid.active_index = Some(99),
                3 => invalid.profiles[0].active = false,
                4 => invalid.profiles[0].name.clear(),
                5 => invalid.profiles.push(invalid.profiles[0].clone()),
                _ => unreachable!(),
            }
            assert_eq!(
                store.save(&invalid).unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        assert_eq!(
            serde_json::to_value(store.load().unwrap()).unwrap(),
            serde_json::to_value(valid).unwrap()
        );
    }

    #[test]
    fn pure_deletion_and_host_save_cleanup_are_separate() {
        let selected = tempfile::tempdir().unwrap();
        let store = PlayerProfileStore::for_directory(selected.path().to_str().unwrap());
        let mut manager = store.load().unwrap();
        let id = manager.profiles[0].id;
        let save = selected
            .path()
            .join(robin_engine::player_profile::profile_save_subdirectory(id));
        fs::create_dir(&save).unwrap();
        fs::write(save.join("save.json"), "fixture").unwrap();
        manager.delete_profile(0);
        assert!(save.exists());
        store.quarantine_profile_saves(id).unwrap();
        assert!(!save.exists());
        assert_eq!(
            fs::read(selected.path().join(".deleted-Profile_000/save.json")).unwrap(),
            b"fixture"
        );
        assert!(selected.path().join("profiles.json").exists());
    }

    #[test]
    fn browser_archive_roundtrip_rejects_corruption_and_wrong_versions() {
        let mut manager = PlayerProfileManager::new("legacy".into());
        manager.create_profile("Robin".into(), DifficultyLevel::Medium);
        manager.set_active(0);
        let encoded = encode_browser_profile_archive(&manager).unwrap();
        let legacy = serde_json::to_string(&BrowserProfileEnvelope {
            schema_version: BROWSER_PROFILE_SCHEMA_VERSION,
            manager: manager.clone(),
        })
        .unwrap();
        assert_eq!(encoded, legacy);
        let decoded = decode_browser_profile_archive(&encoded, "selected").unwrap();
        assert_eq!(decoded.save_directory, "selected");
        assert_eq!(decoded.profiles[0].name, "Robin");
        let mut document: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        document["schema_version"] = 999.into();
        assert!(decode_browser_profile_archive(&document.to_string(), "selected").is_err());
        document["schema_version"] = 1.into();
        document["manager"]["active_index"] = 99.into();
        assert!(decode_browser_profile_archive(&document.to_string(), "selected").is_err());
        document["manager"]["active_index"] = 0.into();
        document["manager"]["profiles"][0]
            .as_object_mut()
            .unwrap()
            .remove("campaign_history");
        assert!(decode_browser_profile_archive(&document.to_string(), "selected").is_err());
        assert!(decode_browser_profile_archive("not JSON", "selected").is_err());
        assert!(
            decode_browser_profile_archive(&"x".repeat(BROWSER_PROFILE_BYTE_LIMIT + 1), "selected")
                .is_err()
        );
    }
}
#[cfg(all(test, target_arch = "wasm32"))]
mod browser_persistence_tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn durable_profile_reload_and_corruption_are_distinguished() {
        let storage = browser_profile_storage().unwrap();
        storage.remove_item(BROWSER_PROFILE_STORE_KEY).unwrap();

        let mut first = PlayerProfileStore::for_directory("browser-save")
            .load()
            .unwrap();
        assert!(first.default_profiles);
        first.default_profiles = false;
        first.profiles[0].name = "Durable Robin".to_owned();
        PlayerProfileStore::for_directory("browser-save")
            .save(&first)
            .unwrap();

        let reloaded = PlayerProfileStore::for_directory("ignored-after-load")
            .load()
            .unwrap();
        assert!(!reloaded.default_profiles);
        assert_eq!(reloaded.get_active().unwrap().name, "Durable Robin");
        assert_eq!(reloaded.save_directory, "ignored-after-load");

        storage
            .set_item(
                BROWSER_PROFILE_STORE_KEY,
                r#"{"schema_version":999,"manager":{}}"#,
            )
            .unwrap();
        assert!(
            PlayerProfileStore::for_directory("browser-save")
                .load()
                .is_err()
        );
        assert!(
            decode_browser_profile_archive(
                &"x".repeat(BROWSER_PROFILE_BYTE_LIMIT + 1),
                "browser-save"
            )
            .unwrap_err()
            .to_string()
            .contains("limit")
        );
        storage.remove_item(BROWSER_PROFILE_STORE_KEY).unwrap();
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
const BROWSER_PROFILE_SCHEMA_VERSION: u32 = 1;
#[cfg(any(test, target_arch = "wasm32"))]
const BROWSER_PROFILE_BYTE_LIMIT: usize = 4 * 1024 * 1024;
#[cfg(target_arch = "wasm32")]
const BROWSER_PROFILE_STORE_KEY: &str = "robin-hood-player-profiles-v1";

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserProfileEnvelope<T = PlayerProfileManager> {
    schema_version: u32,
    manager: T,
}

#[cfg(any(test, target_arch = "wasm32"))]
fn encode_browser_profile_archive(manager: &PlayerProfileManager) -> std::io::Result<String> {
    manager
        .validate_archive()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let serialized = serde_json::to_string(&BrowserProfileEnvelope {
        schema_version: BROWSER_PROFILE_SCHEMA_VERSION,
        manager,
    })
    .map_err(std::io::Error::other)?;
    if serialized.len() > BROWSER_PROFILE_BYTE_LIMIT {
        return Err(std::io::Error::other(format!(
            "browser player-profile archive is {} bytes; limit is {BROWSER_PROFILE_BYTE_LIMIT}",
            serialized.len()
        )));
    }
    Ok(serialized)
}

#[cfg(target_arch = "wasm32")]
fn browser_profile_storage() -> std::io::Result<web_sys::Storage> {
    crate::browser_storage::local_storage().map_err(std::io::Error::other)
}

#[cfg(target_arch = "wasm32")]
fn browser_profile_io(operation: &str, error: impl std::fmt::Debug) -> std::io::Error {
    std::io::Error::other(format!("{operation}: {error:?}"))
}

#[cfg(any(test, target_arch = "wasm32"))]
fn decode_browser_profile_archive(
    serialized: &str,
    directory: &str,
) -> std::io::Result<PlayerProfileManager> {
    if serialized.len() > BROWSER_PROFILE_BYTE_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "browser player-profile archive is {} bytes; limit is {BROWSER_PROFILE_BYTE_LIMIT}",
                serialized.len()
            ),
        ));
    }
    let envelope: BrowserProfileEnvelope = serde_json::from_str(serialized)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if envelope.schema_version != BROWSER_PROFILE_SCHEMA_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "unsupported browser player-profile schema {}; expected {BROWSER_PROFILE_SCHEMA_VERSION}",
                envelope.schema_version
            ),
        ));
    }
    let mut manager = envelope.manager;
    manager
        .validate_archive()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    manager.save_directory = directory.to_owned();
    Ok(manager)
}
