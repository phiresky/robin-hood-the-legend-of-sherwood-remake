//! Canonical, bounded full-mod packages used by multiplayer distribution.
//!
//! The envelope keeps the exact author-supplied mission and optional shared
//! Spellforge-library ZIP bytes.  Its hash commits to both archives and every
//! launch-critical field, so transfer, cache, consent, mount, reconnect, and
//! replay diagnostics all name the same immutable content.

use robin_engine::sbfile::detect_zip_layout_for_mission;
use robin_engine::spellforge::SpellforgePackage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, btree_map::Entry};
use std::io::{Cursor, Read};

use crate::spellforge_trust::{SpellforgeTrustKey, SpellforgeTrustMetadata};

pub const DISTRIBUTED_MOD_SCHEMA_VERSION: u32 = 1;
pub const DISTRIBUTED_MOD_ARCHIVE_LIMIT: usize = robin_spellforge::ARCHIVE_BYTE_LIMIT;
pub const DISTRIBUTED_MOD_ENCODED_LIMIT: usize = 130 * 1024 * 1024;
pub const DISTRIBUTED_MOD_ENTRY_LIMIT: usize = 4_096;
pub const DISTRIBUTED_MOD_ENTRY_BYTE_LIMIT: u64 = 256 * 1024 * 1024;
pub const DISTRIBUTED_MOD_UNCOMPRESSED_LIMIT: u64 = 512 * 1024 * 1024;
pub const DISTRIBUTED_MOD_PATH_LIMIT: usize = 1_024;
pub const DISTRIBUTED_MOD_TEXT_LIMIT: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct DistributedModManifest {
    pub schema_version: u32,
    pub slug: String,
    pub title: String,
    pub claimed_author: String,
    pub version: String,
    pub source_url: String,
    /// Human-readable licence or a host attestation that redistribution is
    /// authorised. An empty value is rejected before content is offered.
    pub license: String,
    pub mission_basename: String,
    pub mission_rhm_entry: String,
    pub map_filename: String,
    pub requires_spellforge: bool,
    pub mission_archive_bytes: u64,
    pub mission_archive_sha256: [u8; 32],
    pub shared_library_bytes: Option<u64>,
    pub shared_library_sha256: Option<[u8; 32]>,
    pub spellforge_package_sha256: Option<[u8; 32]>,
    pub full_mod_sha256: [u8; 32],
}

/// The wire package owns Vecs; admitted runtime packages share immutable Arcs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct DistributedModPackage<Bytes = Vec<u8>> {
    pub manifest: DistributedModManifest,
    pub mission_archive: Bytes,
    pub shared_library_archive: Option<Bytes>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDistributedMod {
    pub package: DistributedModPackage<std::sync::Arc<[u8]>>,
    pub spellforge_package: Option<SpellforgePackage>,
    pub strip_prefix: String,
    pub prepend_prefix: String,
}

/// Result of applying the same hostile-archive admission used by multiplayer
/// to an exact pair of mission archives. Cold save/replay restoration uses
/// this boundary before mounting bytes recovered from an installed locator.
///
/// This is intentionally crate-private: callers must not treat a successful
/// ZIP parse as authority for the Lua package. The package is exposed only so
/// restoration can compare it with the package embedded in the save/replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedMissionArchives {
    pub spellforge_package: Option<SpellforgePackage>,
    pub strip_prefix: String,
    pub prepend_prefix: String,
}

#[derive(Debug, Clone)]
pub struct PreparedDistributedMod {
    pub validated: ValidatedDistributedMod,
    pub encoded: std::sync::Arc<[u8]>,
    /// Native host's exact installed source. The distributed cache identity is
    /// added separately to every host/client descriptor at launch time.
    pub installed_locator: robin_engine::mission_assets::InstalledArchiveLocator,
}

/// Build the exact full-mod envelope used by a native custom-mission host.
/// Metadata comes from the catalog row and the bytes come only from the
/// selected archive plus the selected shared Spellforge library. Missing
/// licence metadata requires an explicit host attestation supplied by the
/// lobby; silently inventing permission is forbidden.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_local_distributed_mod(
    launch: &crate::main_menu::custom_missions::CustomMissionLaunch,
    host_attestation: Option<String>,
) -> Result<PreparedDistributedMod, String> {
    let exact = crate::mission_asset_launch::read_installed_launch_archives(launch)?;
    let mission_archive = exact.mission_archive.to_vec();
    let shared_library_archive = exact.shared_archive.as_ref().map(|bytes| bytes.to_vec());
    let license = if launch.license.trim().is_empty() {
        host_attestation.ok_or_else(|| {
            "this mod has no licence metadata; the host must explicitly attest redistribution permission"
                .to_owned()
        })?
    } else {
        launch.license.clone()
    };
    let validated = DistributedModPackage::build(
        launch.slug.clone(),
        launch.mod_title.clone(),
        launch.claimed_author.clone(),
        launch.version.clone(),
        launch.source_url.clone(),
        license,
        launch.rhm_basename.clone(),
        launch.rhm_zip_entry.clone(),
        launch.map_filename.clone(),
        launch.requires_spellforge,
        mission_archive,
        shared_library_archive,
    )
    .map_err(|error| format!("build canonical distributed mod: {error}"))?;
    let encoded = validated
        .package
        .encode()
        .map_err(|error| format!("encode canonical distributed mod: {error}"))?;
    Ok(PreparedDistributedMod {
        validated,
        encoded: std::sync::Arc::from(encoded),
        installed_locator: exact.locator,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DistributedModError {
    #[error("invalid distributed-mod manifest: {0}")]
    Manifest(String),
    #[error("invalid {archive} archive: {message}")]
    Archive {
        archive: &'static str,
        message: String,
    },
    #[error("Spellforge package admission failed: {0}")]
    Spellforge(String),
    #[error("distributed-mod hash mismatch: declared {declared}, computed {computed}")]
    HashMismatch { declared: String, computed: String },
}

impl DistributedModPackage {
    pub fn build(
        slug: String,
        title: String,
        claimed_author: String,
        version: String,
        source_url: String,
        license: String,
        mission_basename: String,
        mission_rhm_entry: String,
        map_filename: String,
        requires_spellforge: bool,
        mission_archive: Vec<u8>,
        shared_library_archive: Option<Vec<u8>>,
    ) -> Result<ValidatedDistributedMod, DistributedModError> {
        let mission_archive_sha256 = Sha256::digest(&mission_archive).into();
        let shared_library_sha256 = shared_library_archive
            .as_deref()
            .map(|bytes| Sha256::digest(bytes).into());
        let mut package = Self {
            manifest: DistributedModManifest {
                schema_version: DISTRIBUTED_MOD_SCHEMA_VERSION,
                slug,
                title,
                claimed_author,
                version,
                source_url,
                license,
                mission_basename,
                mission_rhm_entry,
                map_filename,
                requires_spellforge,
                mission_archive_bytes: mission_archive.len() as u64,
                mission_archive_sha256,
                shared_library_bytes: shared_library_archive
                    .as_ref()
                    .map(|bytes| bytes.len() as u64),
                shared_library_sha256,
                spellforge_package_sha256: None,
                full_mod_sha256: [0; 32],
            },
            mission_archive,
            shared_library_archive,
        };
        if package.manifest.requires_spellforge {
            let spellforge = package.build_spellforge_package()?;
            package.manifest.spellforge_package_sha256 = Some(spellforge.sha256);
        }
        package.manifest.full_mod_sha256 = package.compute_full_mod_sha256();
        package.validate()
    }
}

impl<Bytes: std::ops::Deref<Target = [u8]> + bitcode::Encode> DistributedModPackage<Bytes> {
    pub fn encode(&self) -> Result<Vec<u8>, DistributedModError> {
        let bytes = bitcode::encode(self);
        if bytes.len() > DISTRIBUTED_MOD_ENCODED_LIMIT {
            return Err(DistributedModError::Manifest(format!(
                "encoded full mod is {} bytes; limit is {DISTRIBUTED_MOD_ENCODED_LIMIT}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }
}

impl DistributedModPackage {
    pub fn decode(bytes: &[u8]) -> Result<ValidatedDistributedMod, DistributedModError> {
        if bytes.len() > DISTRIBUTED_MOD_ENCODED_LIMIT {
            return Err(DistributedModError::Manifest(format!(
                "encoded full mod is {} bytes; limit is {DISTRIBUTED_MOD_ENCODED_LIMIT}",
                bytes.len()
            )));
        }
        let package: Self = bitcode::decode(bytes).map_err(|error| {
            DistributedModError::Manifest(format!("decode canonical package: {error}"))
        })?;
        package.validate()
    }

    pub fn validate(self) -> Result<ValidatedDistributedMod, DistributedModError> {
        self.manifest.validate_fields()?;
        validate_declared_archive(
            "mission",
            &self.mission_archive,
            self.manifest.mission_archive_bytes,
            self.manifest.mission_archive_sha256,
        )?;
        match (
            self.shared_library_archive.as_deref(),
            self.manifest.shared_library_bytes,
            self.manifest.shared_library_sha256,
        ) {
            (None, None, None) => {}
            (Some(bytes), Some(len), Some(hash)) => {
                validate_declared_archive("shared library", bytes, len, hash)?;
            }
            _ => {
                return Err(DistributedModError::Manifest(
                    "shared-library bytes, length, and hash must either all be present or all be absent"
                        .to_owned(),
                ));
            }
        }

        let admitted = validate_mission_archives(
            &self.mission_archive,
            self.shared_library_archive.as_deref(),
            &self.manifest.mission_basename,
            &self.manifest.mission_rhm_entry,
            &self.manifest.map_filename,
            self.manifest.requires_spellforge,
        )?;
        let spellforge_package = admitted.spellforge_package;
        if self.manifest.requires_spellforge {
            let package = spellforge_package
                .as_ref()
                .expect("archive admission must derive a package when Spellforge is required");
            if self.manifest.spellforge_package_sha256 != Some(package.sha256) {
                return Err(DistributedModError::Manifest(format!(
                    "Spellforge package hash is {}, not declared {}",
                    robin_engine::spellforge::hex_hash(&package.sha256),
                    self.manifest
                        .spellforge_package_sha256
                        .map(|hash| robin_engine::spellforge::hex_hash(&hash))
                        .unwrap_or_else(|| "<missing>".to_owned())
                )));
            }
        } else if self.manifest.spellforge_package_sha256.is_some() {
            return Err(DistributedModError::Manifest(
                "vanilla distributed mod declares a Spellforge package hash".to_owned(),
            ));
        }

        let computed = self.compute_full_mod_sha256();
        if computed != self.manifest.full_mod_sha256 {
            return Err(DistributedModError::HashMismatch {
                declared: robin_engine::spellforge::hex_hash(&self.manifest.full_mod_sha256),
                computed: robin_engine::spellforge::hex_hash(&computed),
            });
        }
        Ok(ValidatedDistributedMod {
            package: DistributedModPackage {
                manifest: self.manifest,
                mission_archive: self.mission_archive.into(),
                shared_library_archive: self.shared_library_archive.map(Into::into),
            },
            spellforge_package,
            strip_prefix: admitted.strip_prefix,
            prepend_prefix: admitted.prepend_prefix,
        })
    }

    fn build_spellforge_package(&self) -> Result<SpellforgePackage, DistributedModError> {
        robin_spellforge::build_package_from_archives(
            &self.mission_archive,
            &self.manifest.mission_rhm_entry,
            &self.manifest.mission_basename,
            self.shared_library_archive.as_deref(),
        )
        .map_err(|error| DistributedModError::Spellforge(format!("{:?}: {error}", error.kind)))
    }
}

impl<Bytes: std::ops::Deref<Target = [u8]>> DistributedModPackage<Bytes> {
    pub fn compute_full_mod_sha256(&self) -> [u8; 32] {
        let manifest = &self.manifest;
        let mut hasher = Sha256::new();
        hasher.update(b"robin-distributed-mod-v1\0");
        hash_u32(&mut hasher, manifest.schema_version);
        for value in [
            &manifest.slug,
            &manifest.title,
            &manifest.claimed_author,
            &manifest.version,
            &manifest.source_url,
            &manifest.license,
            &manifest.mission_basename,
            &manifest.mission_rhm_entry,
            &manifest.map_filename,
        ] {
            hash_bytes(&mut hasher, value.as_bytes());
        }
        hasher.update([u8::from(manifest.requires_spellforge)]);
        hash_bytes(&mut hasher, &self.mission_archive);
        match &self.shared_library_archive {
            Some(bytes) => {
                hasher.update([1]);
                hash_bytes(&mut hasher, bytes);
            }
            None => hasher.update([0]),
        }
        match manifest.spellforge_package_sha256 {
            Some(hash) => {
                hasher.update([1]);
                hasher.update(hash);
            }
            None => hasher.update([0]),
        }
        hasher.finalize().into()
    }
}

/// Validate exact mission/shared archive bytes without inventing mutable
/// catalog metadata. This is the single hostile ZIP/RHM/Spellforge admission
/// boundary shared by multiplayer packages and cold save/replay restoration.
pub(crate) fn validate_mission_archives(
    mission_archive: &[u8],
    shared_library_archive: Option<&[u8]>,
    mission_basename: &str,
    mission_rhm_entry: &str,
    map_filename: &str,
    requires_spellforge: bool,
) -> Result<ValidatedMissionArchives, DistributedModError> {
    #[cfg(test)]
    MISSION_ARCHIVE_ADMISSIONS.with(|count| count.set(count.get() + 1));
    let mission_entries = validate_zip("mission", mission_archive)?;
    if let Some(bytes) = shared_library_archive {
        let _ = validate_zip("shared library", bytes)?;
    }
    let (strip_prefix, prepend_prefix) =
        detect_zip_layout_for_mission(mission_entries.keys(), mission_rhm_entry).map_err(
            |message| DistributedModError::Archive {
                archive: "mission",
                message,
            },
        )?;
    let selected = canonical_archive_path(mission_rhm_entry)?;
    let relative =
        selected
            .strip_prefix(&strip_prefix)
            .ok_or_else(|| DistributedModError::Archive {
                archive: "mission",
                message: format!(
                    "selected mission `{selected}` is outside detected root `{strip_prefix}`"
                ),
            })?;
    let mounted = format!("{prepend_prefix}{relative}");
    let expected_mounted = format!("data/levels/{}.rhm", mission_basename.to_ascii_lowercase());
    if mounted != expected_mounted {
        return Err(DistributedModError::Archive {
            archive: "mission",
            message: format!(
                "selected mission mounts as `{mounted}`; expected `{expected_mounted}`"
            ),
        });
    }
    let rhm = mission_entries
        .get(&selected)
        .ok_or_else(|| DistributedModError::Archive {
            archive: "mission",
            message: format!("selected mission `{selected}` is missing"),
        })?;
    let declared_map =
        parse_rhm_map_filename(rhm).map_err(|message| DistributedModError::Archive {
            archive: "mission",
            message,
        })?;
    if !declared_map.eq_ignore_ascii_case(map_filename) {
        return Err(DistributedModError::Manifest(format!(
            "selected mission declares map `{declared_map}`, not `{map_filename}`"
        )));
    }

    let spellforge_package = if requires_spellforge {
        Some(
            robin_spellforge::build_package_from_archives(
                mission_archive,
                mission_rhm_entry,
                mission_basename,
                shared_library_archive,
            )
            .map_err(|error| {
                DistributedModError::Spellforge(format!("{:?}: {error}", error.kind))
            })?,
        )
    } else {
        if shared_library_archive.is_some() {
            return Err(DistributedModError::Manifest(
                "vanilla mission unexpectedly supplied a shared-library archive".to_owned(),
            ));
        }
        None
    };

    Ok(ValidatedMissionArchives {
        spellforge_package,
        strip_prefix,
        prepend_prefix,
    })
}

#[cfg(test)]
std::thread_local! {
    // Per-thread so unrelated parallel tests cannot perturb admission assertions.
    pub(crate) static MISSION_ARCHIVE_ADMISSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub fn make_distributed_mod_offer(
    validated: &ValidatedDistributedMod,
    encoded_bytes: u64,
    host_endpoint_id: String,
) -> Result<robin_engine::multiplayer::DistributedModOffer, DistributedModError> {
    let manifest = &validated.package.manifest;
    let offer = robin_engine::multiplayer::DistributedModOffer {
        schema_version: manifest.schema_version,
        full_mod_sha256: manifest.full_mod_sha256,
        spellforge_package_sha256: manifest.spellforge_package_sha256,
        spellforge_vm_abi: validated
            .spellforge_package
            .as_ref()
            .map(|package| package.vm_abi.clone()),
        encoded_bytes,
        mission_basename: manifest.mission_basename.clone(),
        mission_rhm_entry: manifest.mission_rhm_entry.clone(),
        map_filename: manifest.map_filename.clone(),
        title: manifest.title.clone(),
        claimed_author: manifest.claimed_author.clone(),
        version: manifest.version.clone(),
        source_url: manifest.source_url.clone(),
        license: manifest.license.clone(),
        host_endpoint_id,
    };
    offer.validate().map_err(DistributedModError::Manifest)?;
    if offer.encoded_bytes == 0 || offer.encoded_bytes > DISTRIBUTED_MOD_ENCODED_LIMIT as u64 {
        return Err(DistributedModError::Manifest(format!(
            "offered canonical package is {} bytes; expected 1..={DISTRIBUTED_MOD_ENCODED_LIMIT}",
            offer.encoded_bytes
        )));
    }
    Ok(offer)
}

pub fn offer_trust_identity(
    offer: &robin_engine::multiplayer::DistributedModOffer,
) -> Result<(SpellforgeTrustKey, SpellforgeTrustMetadata), String> {
    offer.validate()?;
    Ok((
        SpellforgeTrustKey {
            full_mod_sha256: offer.full_mod_sha256,
            package_sha256: offer.spellforge_package_sha256,
        },
        SpellforgeTrustMetadata {
            mission: offer.mission_basename.clone(),
            title: offer.title.clone(),
            claimed_author: offer.claimed_author.clone(),
            version: offer.version.clone(),
            source_url: offer.source_url.clone(),
            license: offer.license.clone(),
            host_endpoint_id: offer.host_endpoint_id.clone(),
            package_vm_abi: offer.spellforge_vm_abi.clone(),
            compressed_bytes: offer.encoded_bytes,
        },
    ))
}

impl DistributedModManifest {
    fn validate_fields(&self) -> Result<(), DistributedModError> {
        if self.schema_version != DISTRIBUTED_MOD_SCHEMA_VERSION {
            return Err(DistributedModError::Manifest(format!(
                "schema is {}; expected {DISTRIBUTED_MOD_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        for (label, value) in [
            ("slug", &self.slug),
            ("title", &self.title),
            ("claimed author", &self.claimed_author),
            ("version", &self.version),
            ("source URL", &self.source_url),
            ("license", &self.license),
            ("mission basename", &self.mission_basename),
            ("mission RHM entry", &self.mission_rhm_entry),
            ("map filename", &self.map_filename),
        ] {
            robin_engine::multiplayer::validate_safe_display_text(
                &format!("distributed-mod manifest {label}"),
                value,
                DISTRIBUTED_MOD_TEXT_LIMIT,
            )
            .map_err(DistributedModError::Manifest)?;
        }
        if self.mission_basename.contains(['/', '\\']) {
            return Err(DistributedModError::Manifest(
                "mission basename must be one filename component".to_owned(),
            ));
        }
        if self.mission_archive_bytes > DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 {
            return Err(DistributedModError::Manifest(format!(
                "mission archive declares {} bytes; limit is {DISTRIBUTED_MOD_ARCHIVE_LIMIT}",
                self.mission_archive_bytes
            )));
        }
        if self
            .shared_library_bytes
            .is_some_and(|len| len > DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64)
        {
            return Err(DistributedModError::Manifest(format!(
                "shared-library archive exceeds {DISTRIBUTED_MOD_ARCHIVE_LIMIT} bytes"
            )));
        }
        Ok(())
    }
}

fn validate_declared_archive(
    archive: &'static str,
    bytes: &[u8],
    declared_len: u64,
    declared_hash: [u8; 32],
) -> Result<(), DistributedModError> {
    if bytes.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT {
        return Err(DistributedModError::Archive {
            archive,
            message: format!(
                "{} compressed bytes exceeds {DISTRIBUTED_MOD_ARCHIVE_LIMIT}",
                bytes.len()
            ),
        });
    }
    if declared_len != bytes.len() as u64 {
        return Err(DistributedModError::Archive {
            archive,
            message: format!(
                "declared length {declared_len} does not match {} bytes",
                bytes.len()
            ),
        });
    }
    let computed: [u8; 32] = Sha256::digest(bytes).into();
    if computed != declared_hash {
        return Err(DistributedModError::Archive {
            archive,
            message: format!(
                "SHA-256 mismatch: declared {}, computed {}",
                robin_engine::spellforge::hex_hash(&declared_hash),
                robin_engine::spellforge::hex_hash(&computed)
            ),
        });
    }
    Ok(())
}

/// Validate and fully inflate an archive once before it can enter the cache.
/// This makes CRC failures and understated ZIP sizes admission failures, not
/// surprises while the game is loading an asset.
fn validate_zip(
    archive_label: &'static str,
    bytes: &[u8],
) -> Result<BTreeMap<String, Vec<u8>>, DistributedModError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| DistributedModError::Archive {
            archive: archive_label,
            message: format!("open ZIP: {error}"),
        })?;
    if archive.len() > DISTRIBUTED_MOD_ENTRY_LIMIT {
        return Err(DistributedModError::Archive {
            archive: archive_label,
            message: format!(
                "{} entries exceeds {DISTRIBUTED_MOD_ENTRY_LIMIT}",
                archive.len()
            ),
        });
    }
    let mut entries = BTreeMap::new();
    let mut declared_total = 0u64;
    let mut actual_total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| DistributedModError::Archive {
                archive: archive_label,
                message: format!("open entry {index}: {error}"),
            })?;
        if entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(DistributedModError::Archive {
                archive: archive_label,
                message: format!("entry `{}` is a symbolic link", entry.name()),
            });
        }
        let path = canonical_archive_path(entry.name())?;
        if entry.size() > DISTRIBUTED_MOD_ENTRY_BYTE_LIMIT {
            return Err(DistributedModError::Archive {
                archive: archive_label,
                message: format!(
                    "entry `{path}` declares {} bytes; per-entry limit is {DISTRIBUTED_MOD_ENTRY_BYTE_LIMIT}",
                    entry.size()
                ),
            });
        }
        declared_total = declared_total.checked_add(entry.size()).ok_or_else(|| {
            DistributedModError::Archive {
                archive: archive_label,
                message: "declared uncompressed size overflow".to_owned(),
            }
        })?;
        if declared_total > DISTRIBUTED_MOD_UNCOMPRESSED_LIMIT {
            return Err(DistributedModError::Archive {
                archive: archive_label,
                message: format!(
                    "declared uncompressed total exceeds {DISTRIBUTED_MOD_UNCOMPRESSED_LIMIT} bytes"
                ),
            });
        }
        let mut content = Vec::with_capacity((entry.size() as usize).min(1024 * 1024));
        entry
            .by_ref()
            .take(DISTRIBUTED_MOD_ENTRY_BYTE_LIMIT + 1)
            .read_to_end(&mut content)
            .map_err(|error| DistributedModError::Archive {
                archive: archive_label,
                message: format!("inflate `{path}`: {error}"),
            })?;
        if content.len() as u64 > DISTRIBUTED_MOD_ENTRY_BYTE_LIMIT {
            return Err(DistributedModError::Archive {
                archive: archive_label,
                message: format!("entry `{path}` inflated past the per-entry limit"),
            });
        }
        actual_total = actual_total
            .checked_add(content.len() as u64)
            .ok_or_else(|| DistributedModError::Archive {
                archive: archive_label,
                message: "actual uncompressed size overflow".to_owned(),
            })?;
        if actual_total > DISTRIBUTED_MOD_UNCOMPRESSED_LIMIT {
            return Err(DistributedModError::Archive {
                archive: archive_label,
                message: format!(
                    "actual uncompressed total exceeds {DISTRIBUTED_MOD_UNCOMPRESSED_LIMIT} bytes"
                ),
            });
        }
        match entries.entry(path) {
            Entry::Vacant(entry) => {
                entry.insert(content);
            }
            Entry::Occupied(entry) => {
                return Err(DistributedModError::Archive {
                    archive: archive_label,
                    message: format!("duplicate case-insensitive path `{}`", entry.key()),
                });
            }
        }
    }
    Ok(entries)
}

fn canonical_archive_path(raw: &str) -> Result<String, DistributedModError> {
    if raw.is_empty()
        || raw.len() > DISTRIBUTED_MOD_PATH_LIMIT
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.contains('\\')
        || raw.contains('\0')
        || raw
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        || raw
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
    {
        return Err(DistributedModError::Archive {
            archive: "package",
            message: format!("unsafe archive path `{raw}`"),
        });
    }
    Ok(raw.to_ascii_lowercase())
}

fn parse_rhm_map_filename(bytes: &[u8]) -> Result<&str, String> {
    if bytes.len() < 34 {
        return Err(format!("selected RHM is too short ({} bytes)", bytes.len()));
    }
    if !matches!(&bytes[..4], b"RHMI" | b"DUTY") {
        return Err(format!(
            "selected RHM has unknown tag {:?}",
            String::from_utf8_lossy(&bytes[..4])
        ));
    }
    let length = u16::from_le_bytes([bytes[32], bytes[33]]) as usize;
    let end = 34usize
        .checked_add(length)
        .ok_or_else(|| "selected RHM map-name length overflow".to_owned())?;
    if end > bytes.len() {
        return Err("selected RHM truncates its map filename".to_owned());
    }
    let map = std::str::from_utf8(&bytes[34..end])
        .map_err(|error| format!("selected RHM map filename is not UTF-8: {error}"))?;
    let map = map.trim_end_matches('\0');
    if map.is_empty() || map.contains(['/', '\\']) {
        return Err(format!("selected RHM has invalid map filename `{map}`"));
    }
    Ok(map)
}

fn hash_u32(hasher: &mut Sha256, value: u32) {
    hasher.update(value.to_le_bytes());
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rhm(map: &str) -> Vec<u8> {
        let mut bytes = vec![0; 34];
        bytes[..4].copy_from_slice(b"RHMI");
        bytes[32..34].copy_from_slice(&(map.len() as u16).to_le_bytes());
        bytes.extend_from_slice(map.as_bytes());
        bytes
    }

    #[test]
    fn map_filename_borrows_validated_archive_bytes() {
        for tag in [b"RHMI", b"DUTY"] {
            for name in ["Map", "Map\0\0", "Mäp\0"] {
                let mut bytes = rhm(name);
                bytes[..4].copy_from_slice(tag);
                bytes.extend_from_slice(b"ignored payload");
                let map = parse_rhm_map_filename(&bytes).unwrap();
                assert_eq!(map, name.trim_end_matches('\0'));
                assert_eq!(map.as_ptr(), bytes[34..].as_ptr());
            }
        }
    }

    #[test]
    fn map_filename_rejects_malformed_archive_fields() {
        assert!(
            parse_rhm_map_filename(&[0; 33])
                .unwrap_err()
                .contains("too short")
        );
        let mut bytes = rhm("Map");
        bytes[..4].copy_from_slice(b"NOPE");
        assert!(
            parse_rhm_map_filename(&bytes)
                .unwrap_err()
                .contains("unknown tag")
        );
        bytes[..4].copy_from_slice(b"RHMI");
        bytes.pop();
        assert!(
            parse_rhm_map_filename(&bytes)
                .unwrap_err()
                .contains("truncates")
        );
        let mut bytes = rhm("Map");
        bytes[34] = 0xff;
        assert!(
            parse_rhm_map_filename(&bytes)
                .unwrap_err()
                .contains("not UTF-8")
        );
        for name in ["", "\0\0", "a/b", "a\\b"] {
            assert!(
                parse_rhm_map_filename(&rhm(name))
                    .unwrap_err()
                    .contains("invalid map filename")
            );
        }
    }

    fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, bytes) in entries {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn vanilla() -> ValidatedDistributedMod {
        DistributedModPackage::build(
            "test".into(),
            "Test Mission".into(),
            "Author".into(),
            "1.0".into(),
            "https://example.invalid/test".into(),
            "CC-BY-4.0".into(),
            "Mission".into(),
            "English/Data/Levels/Mission.rhm".into(),
            "Map".into(),
            false,
            archive(&[
                ("English/Data/Levels/Mission.rhm", rhm("Map")),
                ("English/Data/Text/level.res", b"text".to_vec()),
                ("English/Data/Sounds/voice.wav", b"audio".to_vec()),
                ("English/Data/Textures/art.tga", b"art".to_vec()),
            ]),
            None,
        )
        .unwrap()
    }

    #[test]
    fn full_mod_roundtrip_preserves_exact_archive_and_hash() {
        let validated = vanilla();
        let encoded = validated.package.encode().unwrap();
        let decoded = DistributedModPackage::decode(&encoded).unwrap();
        assert_eq!(decoded.package, validated.package);
        assert_eq!(
            decoded.package.manifest.full_mod_sha256,
            validated.package.manifest.full_mod_sha256
        );
        assert_eq!(decoded.strip_prefix, "english/");
        assert_eq!(decoded.prepend_prefix, "");
    }

    #[test]
    fn admitted_archives_share_storage_and_preserve_wire_encoding() {
        for shared in [
            None,
            Some(archive(&[("Data/Text/shared.res", b"shared".to_vec())])),
        ] {
            let mut wire: DistributedModPackage =
                bitcode::decode(&vanilla().package.encode().unwrap()).unwrap();
            wire.shared_library_archive = shared;
            wire.manifest.shared_library_bytes = wire
                .shared_library_archive
                .as_ref()
                .map(|bytes| bytes.len() as u64);
            wire.manifest.shared_library_sha256 = wire
                .shared_library_archive
                .as_ref()
                .map(|bytes| Sha256::digest(bytes).into());
            if wire.shared_library_archive.is_some() {
                wire.manifest.requires_spellforge = true;
                wire.mission_archive = archive(&[
                    ("English/Data/Levels/Mission.rhm", rhm("Map")),
                    (
                        "English/Data/Levels/Mission.lua",
                        b"function StartUp() return 1 end".to_vec(),
                    ),
                ]);
                wire.manifest.mission_archive_bytes = wire.mission_archive.len() as u64;
                wire.manifest.mission_archive_sha256 = Sha256::digest(&wire.mission_archive).into();
                wire.manifest.spellforge_package_sha256 =
                    Some(wire.build_spellforge_package().unwrap().sha256);
            }
            wire.manifest.full_mod_sha256 = wire.compute_full_mod_sha256();
            let encoded = wire.encode().unwrap();
            let json = serde_json::to_value(&wire).unwrap();
            let admitted = wire.validate().unwrap();
            assert_eq!(admitted.package.encode().unwrap(), encoded);
            assert_eq!(serde_json::to_value(&admitted.package).unwrap(), json);
            assert_eq!(
                admitted.package.compute_full_mod_sha256(),
                admitted.package.manifest.full_mod_sha256
            );
            let cloned = admitted.clone();
            assert!(std::sync::Arc::ptr_eq(
                &admitted.package.mission_archive,
                &cloned.package.mission_archive
            ));
            match (
                &admitted.package.shared_library_archive,
                &cloned.package.shared_library_archive,
            ) {
                (Some(original), Some(clone)) => assert!(std::sync::Arc::ptr_eq(original, clone)),
                (None, None) => {}
                _ => panic!("cloning changed shared archive presence"),
            }
            assert_eq!(DistributedModPackage::decode(&encoded).unwrap(), admitted);
        }
    }

    #[test]
    fn archive_or_manifest_tampering_is_rejected() {
        let mut archive_tampered =
            bitcode::decode::<DistributedModPackage>(&vanilla().package.encode().unwrap()).unwrap();
        archive_tampered.mission_archive.push(0);
        assert!(matches!(
            archive_tampered.validate(),
            Err(DistributedModError::Archive { .. })
        ));

        let mut manifest_tampered =
            bitcode::decode::<DistributedModPackage>(&vanilla().package.encode().unwrap()).unwrap();
        manifest_tampered.manifest.title = "Impostor".into();
        assert!(matches!(
            manifest_tampered.validate(),
            Err(DistributedModError::HashMismatch { .. })
        ));

        let mut prompt_spoof =
            bitcode::decode::<DistributedModPackage>(&vanilla().package.encode().unwrap()).unwrap();
        prompt_spoof.manifest.title = "Trusted title\nFull-mod SHA-256: fake".into();
        prompt_spoof.manifest.full_mod_sha256 = prompt_spoof.compute_full_mod_sha256();
        assert!(matches!(
            prompt_spoof.validate(),
            Err(DistributedModError::Manifest(_))
        ));

        for unsafe_title in [" padded", "padded ", "Trusted\u{202e}fake"] {
            let mut prompt_spoof =
                bitcode::decode::<DistributedModPackage>(&vanilla().package.encode().unwrap())
                    .unwrap();
            prompt_spoof.manifest.title = unsafe_title.into();
            prompt_spoof.manifest.full_mod_sha256 = prompt_spoof.compute_full_mod_sha256();
            assert!(matches!(
                prompt_spoof.validate(),
                Err(DistributedModError::Manifest(_))
            ));
        }
    }

    #[test]
    fn exact_language_and_map_are_admission_contracts() {
        let mut wrong_entry =
            bitcode::decode::<DistributedModPackage>(&vanilla().package.encode().unwrap()).unwrap();
        wrong_entry.manifest.mission_rhm_entry = "German/Data/Levels/Mission.rhm".to_owned();
        wrong_entry.manifest.full_mod_sha256 = wrong_entry.compute_full_mod_sha256();
        assert!(matches!(
            wrong_entry.validate(),
            Err(DistributedModError::Archive { .. })
        ));

        let mut wrong_map =
            bitcode::decode::<DistributedModPackage>(&vanilla().package.encode().unwrap()).unwrap();
        wrong_map.manifest.map_filename = "Different".to_owned();
        wrong_map.manifest.full_mod_sha256 = wrong_map.compute_full_mod_sha256();
        assert!(matches!(
            wrong_map.validate(),
            Err(DistributedModError::Manifest(_))
        ));
    }

    #[test]
    fn unsafe_and_duplicate_case_folded_paths_are_rejected() {
        let bad = archive(&[("../Mission.rhm", rhm("Map"))]);
        assert!(validate_zip("test", &bad).is_err());

        let duplicate = archive(&[
            ("Data/Levels/Mission.rhm", rhm("Map")),
            ("data/levels/mission.rhm", rhm("Map")),
        ]);
        assert!(validate_zip("test", &duplicate).is_err());
    }

    #[test]
    fn spellforge_hash_commits_to_exact_executable_package() {
        let mission = archive(&[
            ("Data/Levels/Mission.rhm", rhm("Map")),
            (
                "Data/Levels/Mission.lua",
                b"function StartUp() return 1 end".to_vec(),
            ),
        ]);
        let validated = DistributedModPackage::build(
            "spell".into(),
            "Spell".into(),
            "Author".into(),
            "1".into(),
            "https://example.invalid/spell".into(),
            "Author permits host redistribution".into(),
            "Mission".into(),
            "Data/Levels/Mission.rhm".into(),
            "Map".into(),
            true,
            mission,
            None,
        )
        .unwrap();
        assert_eq!(
            validated.package.manifest.spellforge_package_sha256,
            validated.spellforge_package.map(|package| package.sha256)
        );
    }
}

pub mod admission;
pub mod cache;
pub mod policy;
