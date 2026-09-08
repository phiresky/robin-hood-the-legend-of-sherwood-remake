//! Durable, host-local consent for exact host-distributed mission content.
//!
//! Trust is deliberately outside deterministic profile/engine state. A grant
//! is scoped to one player profile and one exact `(full mod, Lua package)`
//! identity. Display metadata is retained for audit UI but never participates
//! in admission authority.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SPELLFORGE_TRUST_SCHEMA_VERSION: u32 = 1;
pub const SPELLFORGE_TRUST_GRANT_LIMIT_PER_PROFILE: usize = 1_024;
const DISPLAY_FIELD_BYTE_LIMIT: usize = 4 * 1024;
const NATIVE_STORE_FILE: &str = "spellforge-trust.json";
#[cfg(target_arch = "wasm32")]
const BROWSER_STORE_KEY: &str = "robin-hood-spellforge-trust-v1";

/// Exact executable/content authority. `package_sha256` is `None` only for a
/// distributed vanilla custom mission. A Spellforge offer always supplies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellforgeTrustKey {
    pub full_mod_sha256: [u8; 32],
    pub package_sha256: Option<[u8; 32]>,
}

/// Non-authoritative context shown by the review/revocation UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellforgeTrustMetadata {
    pub mission: String,
    pub title: String,
    pub claimed_author: String,
    pub version: String,
    pub source_url: String,
    pub license: String,
    /// Authenticated iroh endpoint public-key identity which supplied the
    /// content. It proves distributor identity, not authorship.
    pub host_endpoint_id: String,
    pub package_vm_abi: Option<String>,
    pub compressed_bytes: u64,
}

impl SpellforgeTrustMetadata {
    fn validate(&self) -> Result<(), String> {
        for (label, value) in [
            ("mission", &self.mission),
            ("title", &self.title),
            ("claimed author", &self.claimed_author),
            ("version", &self.version),
            ("source URL", &self.source_url),
            ("license", &self.license),
        ] {
            robin_engine::multiplayer::validate_safe_display_text(
                &format!("Spellforge trust {label}"),
                value,
                DISPLAY_FIELD_BYTE_LIMIT,
            )?;
        }
        robin_engine::multiplayer::validate_safe_display_text(
            "Spellforge trust host endpoint",
            &self.host_endpoint_id,
            robin_engine::multiplayer::DistributedModOffer::AUTHENTICATED_HOST_ID_BYTE_LIMIT,
        )?;
        if let Some(value) = self.package_vm_abi.as_ref() {
            robin_engine::multiplayer::validate_safe_display_text(
                "Spellforge trust VM ABI",
                value,
                DISPLAY_FIELD_BYTE_LIMIT,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellforgeTrustGrant {
    pub key: SpellforgeTrustKey,
    pub metadata: SpellforgeTrustMetadata,
    pub first_approved_unix_seconds: u64,
    pub last_approved_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpellforgeTrustStore {
    pub schema_version: u32,
    /// Sorted by exact key for deterministic UI and persistence output.
    grants: BTreeMap<u32, Vec<SpellforgeTrustGrant>>,
    #[serde(skip)]
    save_directory: String,
    /// A corrupt/unavailable store must never silently become an empty,
    /// writable approval store. UI may explicitly reset it.
    #[serde(skip)]
    persistence_error: Option<String>,
}

impl SpellforgeTrustStore {
    pub fn new(save_directory: String) -> Self {
        Self {
            schema_version: SPELLFORGE_TRUST_SCHEMA_VERSION,
            grants: BTreeMap::new(),
            save_directory,
            persistence_error: None,
        }
    }

    pub fn unavailable(save_directory: String, error: impl Into<String>) -> Self {
        let mut store = Self::new(save_directory);
        store.persistence_error = Some(error.into());
        store
    }

    pub fn load(directory: &str) -> Result<Self, String> {
        let serialized = load_serialized(directory)?;
        let Some(serialized) = serialized else {
            return Ok(Self::new(directory.to_owned()));
        };
        let mut store: Self = serde_json::from_str(&serialized)
            .map_err(|error| format!("parse Spellforge trust store: {error}"))?;
        store.save_directory = directory.to_owned();
        store.persistence_error = None;
        store.validate()?;
        Ok(store)
    }

    pub fn persistence_error(&self) -> Option<&str> {
        self.persistence_error.as_deref()
    }

    /// Make every grant unusable without attempting another write. Used when
    /// a profile-domain replacement requires revocation but the durable store
    /// cannot be rewritten. A later explicit reset may recover persistence.
    pub(crate) fn fail_closed(&mut self, error: impl Into<String>) {
        self.grants.clear();
        self.persistence_error = Some(error.into());
    }

    pub fn require_available(&self) -> Result<(), String> {
        match &self.persistence_error {
            Some(error) => Err(format!(
                "Spellforge trust persistence is unavailable: {error}; reset it in Spellforge Content settings before accepting remote code"
            )),
            None => Ok(()),
        }
    }

    pub fn grants_for_profile(&self, profile_id: u32) -> &[SpellforgeTrustGrant] {
        self.grants.get(&profile_id).map_or(&[], Vec::as_slice)
    }

    pub fn is_trusted(&self, profile_id: u32, key: SpellforgeTrustKey) -> Result<bool, String> {
        self.require_available()?;
        Ok(self
            .grants_for_profile(profile_id)
            .binary_search_by_key(&key, |grant| grant.key)
            .is_ok())
    }

    /// Persist approval transactionally. A failed durable write restores the
    /// previous in-memory grants and returns an error; callers must cancel the
    /// join rather than treating it as session-only consent.
    pub fn grant(
        &mut self,
        profile_id: u32,
        key: SpellforgeTrustKey,
        metadata: SpellforgeTrustMetadata,
        approved_unix_seconds: u64,
    ) -> Result<(), String> {
        self.require_available()?;
        metadata.validate()?;
        let before = self.grants.clone();
        let grants = self.grants.entry(profile_id).or_default();
        match grants.binary_search_by_key(&key, |grant| grant.key) {
            Ok(index) => {
                grants[index].metadata = metadata;
                grants[index].last_approved_unix_seconds = approved_unix_seconds;
            }
            Err(index) => {
                if grants.len() >= SPELLFORGE_TRUST_GRANT_LIMIT_PER_PROFILE {
                    return Err(format!(
                        "profile {profile_id} already has the maximum {SPELLFORGE_TRUST_GRANT_LIMIT_PER_PROFILE} Spellforge trust grants; revoke an old grant first"
                    ));
                }
                grants.insert(
                    index,
                    SpellforgeTrustGrant {
                        key,
                        metadata,
                        first_approved_unix_seconds: approved_unix_seconds,
                        last_approved_unix_seconds: approved_unix_seconds,
                    },
                );
            }
        }
        if let Err(error) = self.save() {
            self.grants = before;
            return Err(error);
        }
        Ok(())
    }

    pub fn revoke(&mut self, profile_id: u32, key: SpellforgeTrustKey) -> Result<bool, String> {
        self.require_available()?;
        let before = self.grants.clone();
        let removed = self.grants.get_mut(&profile_id).is_some_and(|grants| {
            grants
                .binary_search_by_key(&key, |grant| grant.key)
                .map(|index| {
                    grants.remove(index);
                })
                .is_ok()
        });
        if self.grants.get(&profile_id).is_some_and(Vec::is_empty) {
            self.grants.remove(&profile_id);
        }
        if removed && let Err(error) = self.save() {
            self.grants = before;
            return Err(error);
        }
        Ok(removed)
    }

    pub fn revoke_all(&mut self, profile_id: u32) -> Result<usize, String> {
        self.require_available()?;
        let before = self.grants.clone();
        let removed = self
            .grants
            .remove(&profile_id)
            .map_or(0, |grants| grants.len());
        if removed != 0
            && let Err(error) = self.save()
        {
            self.grants = before;
            return Err(error);
        }
        Ok(removed)
    }

    pub fn remove_profile(&mut self, profile_id: u32) -> Result<(), String> {
        self.revoke_all(profile_id).map(|_| ())
    }

    /// Explicit recovery used by settings UI after warning the user. This is
    /// the only operation allowed to replace a corrupt store.
    pub fn reset(&mut self) -> Result<(), String> {
        let before = self.clone();
        self.schema_version = SPELLFORGE_TRUST_SCHEMA_VERSION;
        self.grants.clear();
        self.persistence_error = None;
        if let Err(error) = self.save() {
            *self = before;
            return Err(error);
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != SPELLFORGE_TRUST_SCHEMA_VERSION {
            return Err(format!(
                "unsupported Spellforge trust schema {}; expected {SPELLFORGE_TRUST_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        for (&profile_id, grants) in &self.grants {
            if grants.len() > SPELLFORGE_TRUST_GRANT_LIMIT_PER_PROFILE {
                return Err(format!(
                    "profile {profile_id} has {} Spellforge trust grants; limit is {SPELLFORGE_TRUST_GRANT_LIMIT_PER_PROFILE}",
                    grants.len()
                ));
            }
            let mut prior = None;
            for grant in grants {
                grant.metadata.validate()?;
                if grant.last_approved_unix_seconds < grant.first_approved_unix_seconds {
                    return Err(format!(
                        "profile {profile_id} Spellforge trust grant has last approval {} before first approval {}",
                        grant.last_approved_unix_seconds, grant.first_approved_unix_seconds
                    ));
                }
                if prior.is_some_and(|prior| prior >= grant.key) {
                    return Err(format!(
                        "profile {profile_id} Spellforge trust keys are duplicate or unsorted"
                    ));
                }
                prior = Some(grant.key);
            }
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        self.require_available()?;
        self.validate()?;
        let serialized = serde_json::to_string_pretty(self)
            .map_err(|error| format!("encode Spellforge trust store: {error}"))?;
        save_serialized(&self.save_directory, &serialized)
    }

    fn store_path(directory: &str) -> PathBuf {
        Path::new(directory).join(NATIVE_STORE_FILE)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_serialized(directory: &str) -> Result<Option<String>, String> {
    let path = SpellforgeTrustStore::store_path(directory);
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|error| format!("read {}: {error}", path.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_serialized(directory: &str, serialized: &str) -> Result<(), String> {
    use std::io::Write as _;

    std::fs::create_dir_all(directory)
        .map_err(|error| format!("create Spellforge trust directory {directory}: {error}"))?;
    let destination = SpellforgeTrustStore::store_path(directory);
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("create temporary Spellforge trust store: {error}"))?;
    temporary
        .write_all(serialized.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("write temporary Spellforge trust store: {error}"))?;
    temporary.persist(&destination).map_err(|error| {
        format!(
            "atomically replace Spellforge trust store {}: {}",
            destination.display(),
            error.error
        )
    })?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_owned())?
        .local_storage()
        .map_err(|error| format!("open browser localStorage: {error:?}"))?
        .ok_or_else(|| "browser localStorage is unavailable".to_owned())
}

#[cfg(target_arch = "wasm32")]
fn load_serialized(_directory: &str) -> Result<Option<String>, String> {
    browser_storage()?
        .get_item(BROWSER_STORE_KEY)
        .map_err(|error| format!("read browser Spellforge trust store: {error:?}"))
}

#[cfg(target_arch = "wasm32")]
fn save_serialized(_directory: &str, serialized: &str) -> Result<(), String> {
    browser_storage()?
        .set_item(BROWSER_STORE_KEY, serialized)
        .map_err(|error| format!("persist browser Spellforge trust store: {error:?}"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn key(value: u8) -> SpellforgeTrustKey {
        SpellforgeTrustKey {
            full_mod_sha256: [value; 32],
            package_sha256: Some([value.wrapping_add(1); 32]),
        }
    }

    fn metadata(title: &str) -> SpellforgeTrustMetadata {
        SpellforgeTrustMetadata {
            mission: "H01_Lin_VL".to_owned(),
            title: title.to_owned(),
            claimed_author: "Author".to_owned(),
            version: "1.0".to_owned(),
            source_url: "https://example.invalid/mod".to_owned(),
            license: "Host attests redistribution permission".to_owned(),
            host_endpoint_id: "endpoint-public-key".to_owned(),
            package_vm_abi: Some("spellforge-v1-sha256:00".to_owned()),
            compressed_bytes: 123,
        }
    }

    #[test]
    fn grant_roundtrip_is_exact_and_profile_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        let mut store = SpellforgeTrustStore::new(root.clone());
        store.grant(7, key(1), metadata("First"), 10).unwrap();

        assert!(store.is_trusted(7, key(1)).unwrap());
        assert!(!store.is_trusted(8, key(1)).unwrap());
        assert!(!store.is_trusted(7, key(2)).unwrap());
        let loaded = SpellforgeTrustStore::load(&root).unwrap();
        assert!(loaded.is_trusted(7, key(1)).unwrap());
        assert_eq!(loaded.grants_for_profile(7)[0].metadata.title, "First");
    }

    #[test]
    fn metadata_never_changes_the_authoritative_key() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        let mut store = SpellforgeTrustStore::new(root);
        store.grant(7, key(1), metadata("First"), 10).unwrap();
        store.grant(7, key(1), metadata("Renamed"), 20).unwrap();

        let grants = store.grants_for_profile(7);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].metadata.title, "Renamed");
        assert_eq!(grants[0].first_approved_unix_seconds, 10);
        assert_eq!(grants[0].last_approved_unix_seconds, 20);
    }

    #[test]
    fn revoke_and_profile_removal_are_durable() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        let mut store = SpellforgeTrustStore::new(root.clone());
        store.grant(1, key(1), metadata("One"), 1).unwrap();
        store.grant(1, key(2), metadata("Two"), 2).unwrap();
        store.grant(2, key(1), metadata("Other"), 3).unwrap();
        assert!(store.revoke(1, key(1)).unwrap());
        store.remove_profile(1).unwrap();

        let loaded = SpellforgeTrustStore::load(&root).unwrap();
        assert!(loaded.grants_for_profile(1).is_empty());
        assert!(loaded.is_trusted(2, key(1)).unwrap());
    }

    #[test]
    fn corrupt_store_is_explicitly_unavailable_until_reset() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        std::fs::write(directory.path().join(NATIVE_STORE_FILE), "not json").unwrap();
        let error = SpellforgeTrustStore::load(&root).unwrap_err();
        let mut unavailable = SpellforgeTrustStore::unavailable(root.clone(), error);
        assert!(unavailable.is_trusted(1, key(1)).is_err());
        assert!(unavailable.grant(1, key(1), metadata("No"), 1).is_err());
        unavailable.reset().unwrap();
        assert!(!unavailable.is_trusted(1, key(1)).unwrap());
        assert!(SpellforgeTrustStore::load(&root).is_ok());
    }

    #[test]
    fn invalid_or_unsorted_serialized_state_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        let invalid = SpellforgeTrustStore {
            schema_version: SPELLFORGE_TRUST_SCHEMA_VERSION,
            grants: BTreeMap::from([(
                1,
                vec![
                    SpellforgeTrustGrant {
                        key: key(2),
                        metadata: metadata("Two"),
                        first_approved_unix_seconds: 1,
                        last_approved_unix_seconds: 1,
                    },
                    SpellforgeTrustGrant {
                        key: key(1),
                        metadata: metadata("One"),
                        first_approved_unix_seconds: 1,
                        last_approved_unix_seconds: 1,
                    },
                ],
            )]),
            save_directory: String::new(),
            persistence_error: None,
        };
        std::fs::write(
            directory.path().join(NATIVE_STORE_FILE),
            serde_json::to_string(&invalid).unwrap(),
        )
        .unwrap();
        assert!(SpellforgeTrustStore::load(&root).is_err());
    }

    #[test]
    fn prompt_metadata_rejects_padding_and_formatting_spoofing() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        let mut store = SpellforgeTrustStore::new(root);
        for unsafe_license in [
            "CC0\nDistributor key: fake",
            " padded",
            "padded ",
            "CC0\u{202e}fake",
        ] {
            let mut spoofed = metadata("Title");
            spoofed.license = unsafe_license.into();
            assert!(store.grant(1, key(1), spoofed, 1).is_err());
        }
        assert!(store.grants_for_profile(1).is_empty());
    }

    #[test]
    fn unknown_persisted_authority_fields_fail_closed() {
        for nested in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().to_string_lossy().into_owned();
            let mut store = SpellforgeTrustStore::new(root.clone());
            store.grant(1, key(1), metadata("One"), 1).unwrap();
            let mut serialized = serde_json::to_value(&store).unwrap();
            let target = if nested {
                serialized["grants"]["1"][0].as_object_mut().unwrap()
            } else {
                serialized.as_object_mut().unwrap()
            };
            target.insert("unknown_authority".into(), serde_json::json!(true));
            std::fs::write(
                directory.path().join(NATIVE_STORE_FILE),
                serde_json::to_vec(&serialized).unwrap(),
            )
            .unwrap();

            assert!(SpellforgeTrustStore::load(&root).is_err());
        }
    }

    #[test]
    fn reversed_approval_timestamps_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().into_owned();
        let mut store = SpellforgeTrustStore::new(root.clone());
        store.grant(1, key(1), metadata("One"), 10).unwrap();
        let mut serialized = serde_json::to_value(&store).unwrap();
        serialized["grants"]["1"][0]["last_approved_unix_seconds"] = serde_json::json!(9);
        std::fs::write(
            directory.path().join(NATIVE_STORE_FILE),
            serde_json::to_vec(&serialized).unwrap(),
        )
        .unwrap();

        assert!(SpellforgeTrustStore::load(&root).is_err());
    }
}
