//! Durable identity for the immutable assets used to construct one mission.
//!
//! Native saves and replays persist this descriptor alongside their simulation
//! state.  It is intentionally an identity-and-locator record, not a bag of
//! guessed fallback paths: cold loading must either reproduce these exact
//! bytes or fail before constructing an [`Engine`](crate::engine::Engine).

use serde::{Deserialize, Serialize};

/// Maximum UTF-8 byte length of every persisted mission identifier or path.
pub const MISSION_ASSET_TEXT_BYTE_LIMIT: usize = 1_024;
/// Maximum accepted byte length of one mission/shared archive.
pub const MISSION_ARCHIVE_BYTE_LIMIT: u64 = 64 * 1024 * 1024;
/// Maximum accepted byte length of one encoded distributed-mod envelope.
pub const DISTRIBUTED_MOD_ENVELOPE_BYTE_LIMIT: u64 = 130 * 1024 * 1024;

/// Complete immutable mission-asset identity required for a cold restart.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
#[serde(deny_unknown_fields)]
pub struct MissionAssetDescriptor {
    /// Selected `.rhm` leaf without its extension.
    pub mission_basename: String,
    /// Proto-level selected by the campaign profile (`.rhp` leaf, extension
    /// omitted).
    pub proto_level_filename: String,
    /// Map/proto identity declared by the selected RHM header.  Keeping this
    /// separately prevents two archives with the same RHM basename from being
    /// mistaken for each other.
    pub map_filename: String,
    /// Whether assets come from the shipping datadir or exact ZIP bytes.
    pub source: MissionAssetSource,
}

/// Physical source of a mission's immutable assets.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
pub enum MissionAssetSource {
    /// Ordinary mission resolved from the active shipping datadir.
    BuiltIn,
    /// Custom mission resolved from an exact archive set.
    Archive(ArchiveMissionAssets),
}

/// Exact custom-mission archives plus bounded ways to find those bytes again.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
#[serde(deny_unknown_fields)]
pub struct ArchiveMissionAssets {
    pub mission_archive: ArchiveIdentity,
    /// Exact normalized archive entry selected for the mission.
    pub selected_rhm_entry: String,
    /// Optional shared Spellforge-library archive used by this launch.
    pub shared_archive: Option<ArchiveIdentity>,
    /// Logical installed location.  Paths are relative to the selected root;
    /// absolute machine-specific paths are never persisted.
    pub installed: Option<InstalledArchiveLocator>,
    /// Identity of a bounded canonical distributed-mod envelope in the cache.
    pub distributed_cache: Option<DistributedCacheIdentity>,
}

/// Immutable SHA-256 and byte length of one archive.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
#[serde(deny_unknown_fields)]
pub struct ArchiveIdentity {
    pub sha256: [u8; 32],
    pub bytes: u64,
}

/// Stable logical installed-mod roots.  The process resolves these roots from
/// current configuration, then verifies the exact bytes before mounting them.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
pub enum InstalledModsRoot {
    ConfiguredMods,
    BundledMods,
}

/// Safe paths relative to one logical installed-mod root.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
#[serde(deny_unknown_fields)]
pub struct InstalledArchiveLocator {
    pub root: InstalledModsRoot,
    pub mission_relative_path: String,
    pub shared_relative_path: Option<String>,
}

/// Exact identity of a canonical distributed-mod envelope in the content
/// cache.  The resolver must decode and compare its entire validated manifest;
/// this record alone never authorizes unverified bytes.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
    robin_state_hash_derive::StateHash,
)]
#[serde(deny_unknown_fields)]
pub struct DistributedCacheIdentity {
    pub schema_version: u32,
    pub full_mod_sha256: [u8; 32],
    pub encoded_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid mission asset descriptor: {0}")]
pub struct MissionAssetDescriptorError(String);

impl MissionAssetDescriptor {
    pub fn built_in(
        mission_basename: impl Into<String>,
        proto_level_filename: impl Into<String>,
        map_filename: impl Into<String>,
    ) -> Result<Self, MissionAssetDescriptorError> {
        let descriptor = Self {
            mission_basename: mission_basename.into(),
            proto_level_filename: proto_level_filename.into(),
            map_filename: map_filename.into(),
            source: MissionAssetSource::BuiltIn,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn archive(
        mission_basename: impl Into<String>,
        proto_level_filename: impl Into<String>,
        map_filename: impl Into<String>,
        archive: ArchiveMissionAssets,
    ) -> Result<Self, MissionAssetDescriptorError> {
        let descriptor = Self {
            mission_basename: mission_basename.into(),
            proto_level_filename: proto_level_filename.into(),
            map_filename: map_filename.into(),
            source: MissionAssetSource::Archive(archive),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Validate all allocation and path invariants before any asset lookup.
    pub fn validate(&self) -> Result<(), MissionAssetDescriptorError> {
        validate_leaf("mission basename", &self.mission_basename)?;
        validate_leaf("proto-level filename", &self.proto_level_filename)?;
        validate_leaf("map filename", &self.map_filename)?;

        let MissionAssetSource::Archive(archive) = &self.source else {
            return Ok(());
        };
        archive.mission_archive.validate("mission archive")?;
        validate_relative_path("selected RHM entry", &archive.selected_rhm_entry)?;
        let (selected_stem, selected_extension) = split_leaf_extension(&archive.selected_rhm_entry)
            .ok_or_else(|| invalid("selected RHM entry must end in `.rhm`"))?;
        if !selected_extension.eq_ignore_ascii_case("rhm") {
            return Err(invalid("selected RHM entry must end in `.rhm`"));
        }
        if !selected_stem.eq_ignore_ascii_case(&self.mission_basename) {
            return Err(invalid(format!(
                "selected RHM basename `{selected_stem}` does not match `{}`",
                self.mission_basename
            )));
        }
        if let Some(shared) = &archive.shared_archive {
            shared.validate("shared archive")?;
        }
        if archive.installed.is_none() && archive.distributed_cache.is_none() {
            return Err(invalid(
                "archive source requires an installed locator or distributed-cache identity",
            ));
        }
        if let Some(installed) = &archive.installed {
            installed.validate(archive.shared_archive.is_some())?;
        }
        if let Some(cache) = &archive.distributed_cache {
            cache.validate()?;
        }
        Ok(())
    }

    pub fn archive_assets(&self) -> Option<&ArchiveMissionAssets> {
        match &self.source {
            MissionAssetSource::BuiltIn => None,
            MissionAssetSource::Archive(archive) => Some(archive),
        }
    }

    /// Cross-check the executable package embedded by a save/replay against
    /// the selected mission identity. The package bytes remain authoritative;
    /// the descriptor may only prove that they belong to this mission.
    pub fn validate_spellforge_package(
        &self,
        package: Option<&crate::spellforge::SpellforgePackage>,
    ) -> Result<(), MissionAssetDescriptorError> {
        self.validate()?;
        match (&self.source, package) {
            (MissionAssetSource::BuiltIn, Some(_)) => Err(invalid(
                "built-in mission descriptor cannot carry a Spellforge package",
            )),
            (MissionAssetSource::BuiltIn, None) => Ok(()),
            (MissionAssetSource::Archive(archive), None) if archive.shared_archive.is_some() => {
                Err(invalid(
                    "shared Spellforge archive requires an embedded canonical package",
                ))
            }
            (MissionAssetSource::Archive(_), None) => Ok(()),
            (MissionAssetSource::Archive(_), Some(package)) => {
                let expected = format!("{}.lua", self.mission_basename.to_ascii_lowercase());
                if !package.entrypoint.eq_ignore_ascii_case(&expected) {
                    return Err(invalid(format!(
                        "Spellforge entrypoint `{}` does not match mission `{}`",
                        package.entrypoint, self.mission_basename
                    )));
                }
                Ok(())
            }
        }
    }
}

impl ArchiveIdentity {
    pub fn validate(&self, label: &str) -> Result<(), MissionAssetDescriptorError> {
        if self.bytes == 0 || self.bytes > MISSION_ARCHIVE_BYTE_LIMIT {
            return Err(invalid(format!(
                "{label} byte length {} is outside 1..={MISSION_ARCHIVE_BYTE_LIMIT}",
                self.bytes
            )));
        }
        if self.sha256 == [0; 32] {
            return Err(invalid(format!("{label} SHA-256 must not be all zero")));
        }
        Ok(())
    }
}

impl InstalledArchiveLocator {
    fn validate(&self, has_shared_archive: bool) -> Result<(), MissionAssetDescriptorError> {
        validate_relative_path("installed mission archive", &self.mission_relative_path)?;
        if let Some(path) = &self.shared_relative_path {
            validate_relative_path("installed shared archive", path)?;
        }
        if self.shared_relative_path.is_some() != has_shared_archive {
            return Err(invalid(
                "installed shared path and shared archive identity must either both be present or both be absent",
            ));
        }
        Ok(())
    }
}

impl DistributedCacheIdentity {
    fn validate(&self) -> Result<(), MissionAssetDescriptorError> {
        if self.schema_version == 0 {
            return Err(invalid("distributed-cache schema version must be non-zero"));
        }
        if self.full_mod_sha256 == [0; 32] {
            return Err(invalid(
                "distributed-cache full-mod SHA-256 must not be all zero",
            ));
        }
        if self.encoded_bytes == 0 || self.encoded_bytes > DISTRIBUTED_MOD_ENVELOPE_BYTE_LIMIT {
            return Err(invalid(format!(
                "distributed-cache envelope byte length {} is outside 1..={DISTRIBUTED_MOD_ENVELOPE_BYTE_LIMIT}",
                self.encoded_bytes
            )));
        }
        Ok(())
    }
}

fn validate_leaf(label: &str, value: &str) -> Result<(), MissionAssetDescriptorError> {
    validate_text(label, value)?;
    if value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
    {
        return Err(invalid(format!("{label} is not a safe filename leaf")));
    }
    Ok(())
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), MissionAssetDescriptorError> {
    validate_text(label, value)?;
    robin_util::asset_fs::validate_canonical_relative_path(value)
        .map_err(|error| invalid(format!("{label}: {error}")))
}

fn validate_text(label: &str, value: &str) -> Result<(), MissionAssetDescriptorError> {
    if value.is_empty() {
        return Err(invalid(format!("{label} is empty")));
    }
    if value.len() > MISSION_ASSET_TEXT_BYTE_LIMIT {
        return Err(invalid(format!(
            "{label} is {} bytes; limit is {MISSION_ASSET_TEXT_BYTE_LIMIT}",
            value.len()
        )));
    }
    if value
        .chars()
        .any(robin_util::display_text::is_unsafe_display_character)
    {
        return Err(invalid(format!(
            "{label} contains a control, invisible, or bidirectional formatting character"
        )));
    }
    Ok(())
}

fn split_leaf_extension(path: &str) -> Option<(&str, &str)> {
    path.rsplit('/').next()?.rsplit_once('.')
}

fn invalid(message: impl Into<String>) -> MissionAssetDescriptorError {
    MissionAssetDescriptorError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_descriptor() -> MissionAssetDescriptor {
        MissionAssetDescriptor::archive(
            "H06_Lin_VL",
            "lincoln",
            "lincoln",
            ArchiveMissionAssets {
                mission_archive: ArchiveIdentity {
                    sha256: [1; 32],
                    bytes: 123,
                },
                selected_rhm_entry: "German/DATA/Levels/H06_Lin_VL.rhm".into(),
                shared_archive: Some(ArchiveIdentity {
                    sha256: [2; 32],
                    bytes: 456,
                }),
                installed: Some(InstalledArchiveLocator {
                    root: InstalledModsRoot::ConfiguredMods,
                    mission_relative_path: "rescue-allan/v1.0.zip".into(),
                    shared_relative_path: Some("lib/spellforge.zip".into()),
                }),
                distributed_cache: Some(DistributedCacheIdentity {
                    schema_version: 1,
                    full_mod_sha256: [3; 32],
                    encoded_bytes: 789,
                }),
            },
        )
        .unwrap()
    }

    #[test]
    fn multilingual_nested_archive_descriptor_is_valid() {
        archive_descriptor().validate().unwrap();
    }

    #[test]
    fn selected_archive_entry_must_name_the_mission() {
        let mut descriptor = archive_descriptor();
        let MissionAssetSource::Archive(archive) = &mut descriptor.source else {
            unreachable!()
        };
        archive.selected_rhm_entry = "German/DATA/Levels/Other.rhm".into();
        assert!(descriptor.validate().is_err());
    }

    #[test]
    fn paths_reject_absolute_parent_backslash_and_controls() {
        for path in [
            "/mission.zip",
            "../mission.zip",
            "mods/../mission.zip",
            "mods\\mission.zip",
            "C:/escape.zip",
            "mods/stream:mission.zip",
            "mods/mission\0.zip",
        ] {
            let mut descriptor = archive_descriptor();
            let MissionAssetSource::Archive(archive) = &mut descriptor.source else {
                unreachable!()
            };
            archive.installed.as_mut().unwrap().mission_relative_path = path.into();
            assert!(
                descriptor.validate().is_err(),
                "accepted unsafe path {path:?}"
            );
        }
    }

    #[test]
    fn archive_requires_a_bounded_recovery_source() {
        let mut descriptor = archive_descriptor();
        let MissionAssetSource::Archive(archive) = &mut descriptor.source else {
            unreachable!()
        };
        archive.installed = None;
        archive.distributed_cache = None;
        assert!(descriptor.validate().is_err());
    }

    #[test]
    fn cryptographic_identities_reject_zero_sentinels() {
        let mut descriptor = archive_descriptor();
        let MissionAssetSource::Archive(archive) = &mut descriptor.source else {
            unreachable!()
        };
        archive.mission_archive.sha256 = [0; 32];
        assert!(descriptor.validate().is_err());

        let mut descriptor = archive_descriptor();
        let MissionAssetSource::Archive(archive) = &mut descriptor.source else {
            unreachable!()
        };
        archive.distributed_cache.as_mut().unwrap().full_mod_sha256 = [0; 32];
        assert!(descriptor.validate().is_err());
    }

    #[test]
    fn identifiers_reject_invisible_and_bidi_spoofing() {
        for mission in ["mission\u{200b}hidden", "mission\u{202e}zip"] {
            assert!(MissionAssetDescriptor::built_in(mission, "map", "map").is_err());
        }
    }

    #[test]
    fn spellforge_entrypoint_is_bound_to_selected_mission() {
        let descriptor = archive_descriptor();
        let package = crate::spellforge::SpellforgePackage {
            contract_version: crate::spellforge::SPELLFORGE_CONTRACT_VERSION,
            vm_abi: format!(
                "{}{}",
                crate::spellforge::SPELLFORGE_VM_ABI_SCHEME,
                "1".repeat(64)
            ),
            script_mode: crate::spellforge::SpellforgeScriptMode::Replace,
            entrypoint: "other.lua".into(),
            files: [("other.lua".into(), b"return 0".to_vec())]
                .into_iter()
                .collect(),
            sha256: [1; 32],
        };
        assert!(
            descriptor
                .validate_spellforge_package(Some(&package))
                .is_err()
        );
    }

    #[test]
    fn serde_rejects_unknown_fields() {
        let mut value = serde_json::to_value(archive_descriptor()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("legacy_path".into(), serde_json::json!("ignored.zip"));
        assert!(serde_json::from_value::<MissionAssetDescriptor>(value).is_err());
    }
}
