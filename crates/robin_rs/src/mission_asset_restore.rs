//! Fail-closed cold restoration of custom-mission archives.
//!
//! A save/replay descriptor names one exact installed archive locator and/or
//! one exact durable distributed-cache object. Restoration never scans by mod
//! name, mission basename, or archive filename and never fetches the network.
//! Filesystem archives are read, hashed, admitted, and mounted from the same
//! immutable in-memory bytes, closing the path-reopen TOCTOU seam.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(target_arch = "wasm32"))]
use std::{fs::File, io::Read, path::Path};

use robin_engine::mission_assets::{
    ArchiveIdentity, ArchiveMissionAssets, DistributedCacheIdentity, InstalledArchiveLocator,
    MissionAssetDescriptor, MissionAssetSource,
};
use robin_engine::spellforge::{SpellforgePackage, hex_hash};
use sha2::{Digest, Sha256};

#[cfg(not(target_arch = "wasm32"))]
use robin_engine::mission_assets::InstalledModsRoot;

use crate::distributed_mod::{DISTRIBUTED_MOD_SCHEMA_VERSION, validate_mission_archives};
use crate::distributed_mod_cache::DistributedModCacheLease;
use crate::mod_pack::{MountGuard, mount_distributed_archives};

#[cfg(not(target_arch = "wasm32"))]
use crate::distributed_mod::DISTRIBUTED_MOD_ARCHIVE_LIMIT;
#[cfg(not(target_arch = "wasm32"))]
use crate::distributed_mod_cache::DistributedModCache;

static MOUNT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Exact roots which an [`InstalledArchiveLocator`] is allowed to address.
///
/// `configured_mods` is the user/development custom-mission root (normally
/// `datadirs/mods`). `bundled_mods` is the install-owned `mods/` overlay root.
/// Callers may override both explicitly in tests and portable installations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionAssetRoots {
    pub configured_mods: PathBuf,
    pub bundled_mods: Option<PathBuf>,
}

impl MissionAssetRoots {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn discover() -> Self {
        Self {
            configured_mods: crate::mod_pack::default_mods_root(),
            bundled_mods: crate::main_entry::overlay_mods_dir(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn selected(&self, root: &InstalledModsRoot) -> Option<&Path> {
        match root {
            InstalledModsRoot::ConfiguredMods => Some(&self.configured_mods),
            InstalledModsRoot::BundledMods => self.bundled_mods.as_deref(),
        }
    }
}

/// Mounted exact archive bytes plus, for cache restoration, the eviction pin.
///
/// This value must live through engine construction and the complete mission
/// session. Its explicit `Drop` ordering unmounts the overlays before releasing
/// the durable-cache lease.
#[must_use = "keep resolved mission assets alive for the complete mission"]
pub struct ResolvedMissionAssets {
    descriptor: MissionAssetDescriptor,
    mount: Option<MountGuard>,
    cache_lease: Option<DistributedModCacheLease>,
    mission_archive: Option<Arc<[u8]>>,
    shared_archive: Option<Arc<[u8]>>,
    selected_rhm_entry: Option<String>,
}

impl std::fmt::Debug for ResolvedMissionAssets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedMissionAssets")
            .field("descriptor", &self.descriptor)
            .field("mounted", &self.mount.is_some())
            .field("cache_backed", &self.cache_lease.is_some())
            .field("selected_rhm_entry", &self.selected_rhm_entry)
            .finish_non_exhaustive()
    }
}

impl ResolvedMissionAssets {
    fn built_in(descriptor: &MissionAssetDescriptor) -> Self {
        Self {
            descriptor: descriptor.clone(),
            mount: None,
            cache_lease: None,
            mission_archive: None,
            shared_archive: None,
            selected_rhm_entry: None,
        }
    }

    /// The exact validated descriptor which produced this mounted lifetime.
    /// Startup stores this same value on `Game` for all replay/save capture.
    pub fn descriptor(&self) -> &MissionAssetDescriptor {
        &self.descriptor
    }

    /// Whether restoration installed an in-memory custom-mission overlay.
    pub fn is_archive(&self) -> bool {
        self.mount.is_some()
    }

    /// Whether a durable distributed-cache object is pinned by this lifetime.
    pub fn is_cache_backed(&self) -> bool {
        self.cache_lease.is_some()
    }

    /// Exact selected archive entry retained for diagnostics and load setup.
    pub fn selected_rhm_entry(&self) -> Option<&str> {
        self.selected_rhm_entry.as_deref()
    }

    /// Exact admitted mission bytes. These are the same immutable bytes held
    /// by the active in-memory overlay, never a reopened filesystem path.
    pub fn mission_archive(&self) -> Option<&Arc<[u8]>> {
        self.mission_archive.as_ref()
    }

    /// Exact admitted shared-library bytes, when the descriptor has one.
    pub fn shared_archive(&self) -> Option<&Arc<[u8]>> {
        self.shared_archive.as_ref()
    }
}

/// Validate and retain a built-in descriptor without consulting any ambient
/// filesystem or cache state. This gives cold save/replay startup one common
/// lifetime type on native and browser builds.
pub fn resolve_built_in_mission_assets(
    descriptor: &MissionAssetDescriptor,
    embedded_spellforge_package: Option<&SpellforgePackage>,
) -> Result<ResolvedMissionAssets, MissionAssetRestoreError> {
    descriptor
        .validate()
        .map_err(|error| MissionAssetRestoreError::InvalidDescriptor(error.to_string()))?;
    if !matches!(descriptor.source, MissionAssetSource::BuiltIn) {
        return Err(MissionAssetRestoreError::CacheIdentityMismatch(
            "archive descriptor requires an exact installed or durable-cache source".to_owned(),
        ));
    }
    if embedded_spellforge_package.is_some() {
        return Err(MissionAssetRestoreError::BuiltInSpellforgePackage);
    }
    Ok(ResolvedMissionAssets::built_in(descriptor))
}

/// Admit and retain exact archive bytes selected by a live launcher.
///
/// Unlike cold restoration this function never resolves a path or cache key:
/// callers must first read or receive the exact immutable bytes they intend to
/// launch. Admission, descriptor construction, and mounting stay in this one
/// boundary: no caller can substitute bytes or metadata after admission. The
/// returned package is derived from those same bytes, not supplied by callers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn retain_live_mission_assets(
    mission_basename: &str,
    map_filename: &str,
    rhm_entry: &str,
    requires_spellforge: bool,
    mission_archive: Arc<[u8]>,
    shared_archive: Option<Arc<[u8]>>,
    installed: Option<InstalledArchiveLocator>,
    distributed_cache: Option<DistributedCacheIdentity>,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<(ResolvedMissionAssets, Option<SpellforgePackage>), String> {
    let admitted = validate_mission_archives(
        &mission_archive,
        shared_archive.as_deref(),
        mission_basename,
        rhm_entry,
        map_filename,
        requires_spellforge,
    )
    .map_err(|error| format!("admit exact custom-mission archives: {error}"))?;
    let spellforge_package = admitted.spellforge_package;
    let identity = |bytes: &[u8]| ArchiveIdentity {
        sha256: Sha256::digest(bytes).into(),
        bytes: bytes.len() as u64,
    };
    let archive = ArchiveMissionAssets {
        mission_archive: identity(&mission_archive),
        selected_rhm_entry: rhm_entry.to_owned(),
        shared_archive: shared_archive.as_deref().map(identity),
        installed,
        distributed_cache,
    };
    let descriptor =
        MissionAssetDescriptor::archive(mission_basename, map_filename, map_filename, archive)
            .map_err(|error| error.to_string())?;
    descriptor
        .validate_spellforge_package(spellforge_package.as_ref())
        .map_err(|error| error.to_string())?;
    if let Some(package) = &spellforge_package {
        package.validate_wire().map_err(|error| {
            MissionAssetRestoreError::SpellforgePackageMismatch(format!(
                "embedded package is invalid: {error}"
            ))
            .to_string()
        })?;
    }
    let resolved = mount_resolved(
        &descriptor,
        descriptor
            .archive_assets()
            .expect("constructed archive descriptor"),
        mission_archive,
        shared_archive,
        None,
        files,
    )
    .map_err(|error| error.to_string())?;
    Ok((resolved, spellforge_package))
}

impl Drop for ResolvedMissionAssets {
    fn drop(&mut self) {
        // This order is part of the cache-eviction contract. SbFile first
        // releases the Arc-backed overlays, then the cache object may unpin.
        drop(self.mount.take());
        drop(self.cache_lease.take());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MissionAssetRestoreError {
    #[error("invalid mission asset descriptor: {0}")]
    InvalidDescriptor(String),
    #[error("built-in mission unexpectedly carries an embedded Spellforge package")]
    BuiltInSpellforgePackage,
    #[error("installed {root} mods root is unavailable")]
    MissingInstalledRoot { root: &'static str },
    #[error("installed {label} archive is missing at {path}")]
    MissingInstalledArchive { label: &'static str, path: PathBuf },
    #[error("installed {label} archive path escapes its declared {root} mods root: {path}")]
    InstalledPathEscapesRoot {
        label: &'static str,
        root: &'static str,
        path: PathBuf,
    },
    #[error("cannot inspect installed {label} archive {path}: {message}")]
    InstalledArchiveIo {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    #[error("installed {label} archive {path} is not a regular file")]
    InstalledArchiveNotFile { label: &'static str, path: PathBuf },
    #[error("installed {label} archive {path} is {bytes} bytes; expected 1..={limit} bytes")]
    InstalledArchiveSize {
        label: &'static str,
        path: PathBuf,
        bytes: u64,
        limit: usize,
    },
    #[error(
        "{label} archive identity mismatch: descriptor says {declared_bytes} bytes/{declared_hash}, recovered {actual_bytes} bytes/{actual_hash}"
    )]
    ArchiveIdentityMismatch {
        label: &'static str,
        declared_bytes: u64,
        declared_hash: String,
        actual_bytes: u64,
        actual_hash: String,
    },
    #[error("custom-mission archive admission failed: {0}")]
    ArchiveAdmission(String),
    #[error("installed locator shared-archive shape does not match the descriptor")]
    InstalledSharedArchiveShape,
    #[error("descriptor requires durable cache content but no cache was opened")]
    CacheUnavailable,
    #[error("durable distributed cache has no exact object {hash}")]
    CacheMiss { hash: String },
    #[error("durable distributed cache failed: {0}")]
    Cache(String),
    #[error("durable cache identity mismatch: {0}")]
    CacheIdentityMismatch(String),
    #[error(
        "all declared cold asset sources are unavailable (installed: {installed}; cache: {cache})"
    )]
    AllSourcesUnavailable { installed: String, cache: String },
    #[error(
        "embedded Spellforge package does not match the package derived from exact archives: {0}"
    )]
    SpellforgePackageMismatch(String),
    #[error("mount exact custom-mission archives: {0}")]
    Mount(String),
}

/// Resolve a built-in mission or an exact custom mission on native platforms.
///
/// Installed content is authoritative when present. A genuinely missing
/// installed path may fall back to the descriptor's exact durable-cache key;
/// an installed hash/content mismatch is fatal and cannot be hidden by cache.
/// No network operation or catalog/name scan occurs here.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_native_mission_assets(
    descriptor: &MissionAssetDescriptor,
    embedded_spellforge_package: Option<&SpellforgePackage>,
    roots: &MissionAssetRoots,
    cache: Option<&mut DistributedModCache>,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<ResolvedMissionAssets, MissionAssetRestoreError> {
    descriptor
        .validate()
        .map_err(|error| MissionAssetRestoreError::InvalidDescriptor(error.to_string()))?;
    let MissionAssetSource::Archive(archive) = &descriptor.source else {
        return resolve_built_in_mission_assets(descriptor, embedded_spellforge_package);
    };
    validate_proto_and_map(descriptor)?;

    let mut missing_installed = None;
    if let Some(locator) = archive.installed.as_ref() {
        match resolve_installed(
            descriptor,
            archive,
            locator,
            embedded_spellforge_package,
            roots,
            files.clone(),
        ) {
            Ok(resolved) => return Ok(resolved),
            Err(InstalledAttempt::Missing(error)) => missing_installed = Some(error),
            Err(InstalledAttempt::Fatal(error)) => return Err(error),
        }
    }

    let Some(cache_identity) = archive.distributed_cache.as_ref() else {
        return Err(missing_installed.unwrap_or(MissionAssetRestoreError::CacheUnavailable));
    };
    let cache_result = (|| {
        let cache = cache.ok_or(MissionAssetRestoreError::CacheUnavailable)?;
        let lease = cache
            .acquire(cache_identity.full_mod_sha256)
            .map_err(MissionAssetRestoreError::Cache)?
            .ok_or_else(|| MissionAssetRestoreError::CacheMiss {
                hash: hex_hash(&cache_identity.full_mod_sha256),
            })?;
        resolve_cached_mission_assets(descriptor, embedded_spellforge_package, lease, files)
    })();
    match (missing_installed, cache_result) {
        (_, Ok(resolved)) => Ok(resolved),
        (Some(installed), Err(cache)) => Err(MissionAssetRestoreError::AllSourcesUnavailable {
            installed: installed.to_string(),
            cache: cache.to_string(),
        }),
        (None, Err(cache)) => Err(cache),
    }
}

/// Finish restoration from an already-acquired durable cache lease.
///
/// Browser startup uses this after its asynchronous IndexedDB acquire; native
/// startup uses it through [`resolve_native_mission_assets`]. The lease's full
/// validated manifest must agree with every overlapping descriptor field.
pub fn resolve_cached_mission_assets(
    descriptor: &MissionAssetDescriptor,
    embedded_spellforge_package: Option<&SpellforgePackage>,
    lease: DistributedModCacheLease,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<ResolvedMissionAssets, MissionAssetRestoreError> {
    descriptor
        .validate()
        .map_err(|error| MissionAssetRestoreError::InvalidDescriptor(error.to_string()))?;
    let MissionAssetSource::Archive(archive) = &descriptor.source else {
        return Err(MissionAssetRestoreError::CacheIdentityMismatch(
            "built-in descriptor cannot consume a distributed custom-mod cache lease".to_owned(),
        ));
    };
    validate_proto_and_map(descriptor)?;
    validate_cache_identity(descriptor, archive, &lease)?;
    // Share the lease's immutable archive allocations with the mount while
    // retaining its cache pin. Re-hash and re-admit these exact bytes, never
    // a cache/file reopen.
    let mission_archive: Arc<[u8]> = Arc::clone(&lease.validated.package.mission_archive);
    let shared_archive: Option<Arc<[u8]>> = lease
        .validated
        .package
        .shared_library_archive
        .as_ref()
        .map(Arc::clone);
    verify_archive_arc_identity("mission", &mission_archive, &archive.mission_archive)?;
    if let (Some(bytes), Some(identity)) = (&shared_archive, &archive.shared_archive) {
        verify_archive_arc_identity("shared library", bytes, identity)?;
    }
    let admitted = validate_mission_archives(
        &mission_archive,
        shared_archive.as_deref(),
        &descriptor.mission_basename,
        &archive.selected_rhm_entry,
        &descriptor.map_filename,
        lease.validated.package.manifest.requires_spellforge,
    )
    .map_err(|error| MissionAssetRestoreError::ArchiveAdmission(error.to_string()))?;
    verify_spellforge_authority(
        admitted.spellforge_package.as_ref(),
        embedded_spellforge_package,
    )?;
    mount_resolved(
        descriptor,
        archive,
        mission_archive,
        shared_archive,
        Some(lease),
        files,
    )
}

#[cfg(not(target_arch = "wasm32"))]
enum InstalledAttempt {
    Missing(MissionAssetRestoreError),
    Fatal(MissionAssetRestoreError),
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_installed(
    descriptor: &MissionAssetDescriptor,
    archive: &ArchiveMissionAssets,
    locator: &InstalledArchiveLocator,
    embedded_spellforge_package: Option<&SpellforgePackage>,
    roots: &MissionAssetRoots,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<ResolvedMissionAssets, InstalledAttempt> {
    if archive.shared_archive.is_some() != locator.shared_relative_path.is_some() {
        return Err(InstalledAttempt::Fatal(
            MissionAssetRestoreError::InstalledSharedArchiveShape,
        ));
    }
    let root_name = installed_root_name(&locator.root);
    let root = roots.selected(&locator.root).ok_or({
        InstalledAttempt::Missing(MissionAssetRestoreError::MissingInstalledRoot {
            root: root_name,
        })
    })?;
    let mission_archive = read_exact_installed_archive(
        root,
        root_name,
        &locator.mission_relative_path,
        "mission",
        &archive.mission_archive,
    )?;
    let shared_archive = match (
        locator.shared_relative_path.as_deref(),
        archive.shared_archive.as_ref(),
    ) {
        (Some(relative), Some(identity)) => Some(read_exact_installed_archive(
            root,
            root_name,
            relative,
            "shared library",
            identity,
        )?),
        (None, None) => None,
        _ => {
            return Err(InstalledAttempt::Fatal(
                MissionAssetRestoreError::InstalledSharedArchiveShape,
            ));
        }
    };

    let admitted = validate_mission_archives(
        &mission_archive,
        shared_archive.as_deref(),
        &descriptor.mission_basename,
        &archive.selected_rhm_entry,
        &descriptor.map_filename,
        embedded_spellforge_package.is_some(),
    )
    .map_err(|error| {
        InstalledAttempt::Fatal(MissionAssetRestoreError::ArchiveAdmission(
            error.to_string(),
        ))
    })?;
    verify_spellforge_authority(
        admitted.spellforge_package.as_ref(),
        embedded_spellforge_package,
    )
    .map_err(InstalledAttempt::Fatal)?;
    mount_resolved(
        descriptor,
        archive,
        mission_archive,
        shared_archive,
        None,
        files,
    )
    .map_err(InstalledAttempt::Fatal)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_exact_installed_archive(
    root: &Path,
    root_name: &'static str,
    relative: &str,
    label: &'static str,
    identity: &ArchiveIdentity,
) -> Result<Arc<[u8]>, InstalledAttempt> {
    // Descriptor validation already rejects traversal. Keep this local check
    // because this function is the ambient-filesystem authority boundary.
    if !is_safe_logical_relative_path(relative) {
        return Err(InstalledAttempt::Fatal(
            MissionAssetRestoreError::InstalledPathEscapesRoot {
                label,
                root: root_name,
                path: PathBuf::from(relative),
            },
        ));
    }
    let canonical_root = match std::fs::canonicalize(root) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            return Err(InstalledAttempt::Missing(
                MissionAssetRestoreError::MissingInstalledRoot { root: root_name },
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(InstalledAttempt::Missing(
                MissionAssetRestoreError::MissingInstalledRoot { root: root_name },
            ));
        }
        Err(error) => {
            return Err(InstalledAttempt::Fatal(
                MissionAssetRestoreError::InstalledArchiveIo {
                    label,
                    path: root.to_path_buf(),
                    message: error.to_string(),
                },
            ));
        }
    };
    let logical_path = relative
        .split('/')
        .fold(canonical_root.clone(), |path, component| {
            path.join(component)
        });
    let canonical_path = match std::fs::canonicalize(&logical_path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(InstalledAttempt::Missing(
                MissionAssetRestoreError::MissingInstalledArchive {
                    label,
                    path: logical_path,
                },
            ));
        }
        Err(error) => {
            return Err(InstalledAttempt::Fatal(
                MissionAssetRestoreError::InstalledArchiveIo {
                    label,
                    path: logical_path,
                    message: error.to_string(),
                },
            ));
        }
    };
    if !canonical_path.starts_with(&canonical_root) {
        return Err(InstalledAttempt::Fatal(
            MissionAssetRestoreError::InstalledPathEscapesRoot {
                label,
                root: root_name,
                path: canonical_path,
            },
        ));
    }
    let mut file = File::open(&canonical_path).map_err(|error| {
        let failure = MissionAssetRestoreError::InstalledArchiveIo {
            label,
            path: canonical_path.clone(),
            message: error.to_string(),
        };
        if error.kind() == std::io::ErrorKind::NotFound {
            InstalledAttempt::Missing(failure)
        } else {
            InstalledAttempt::Fatal(failure)
        }
    })?;
    let metadata = file.metadata().map_err(|error| {
        InstalledAttempt::Fatal(MissionAssetRestoreError::InstalledArchiveIo {
            label,
            path: canonical_path.clone(),
            message: error.to_string(),
        })
    })?;
    if !metadata.is_file() {
        return Err(InstalledAttempt::Fatal(
            MissionAssetRestoreError::InstalledArchiveNotFile {
                label,
                path: canonical_path,
            },
        ));
    }
    if metadata.len() == 0 || metadata.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 {
        return Err(InstalledAttempt::Fatal(
            MissionAssetRestoreError::InstalledArchiveSize {
                label,
                path: canonical_path,
                bytes: metadata.len(),
                limit: DISTRIBUTED_MOD_ARCHIVE_LIMIT,
            },
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(DISTRIBUTED_MOD_ARCHIVE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            InstalledAttempt::Fatal(MissionAssetRestoreError::InstalledArchiveIo {
                label,
                path: canonical_path.clone(),
                message: error.to_string(),
            })
        })?;
    if bytes.is_empty() || bytes.len() > DISTRIBUTED_MOD_ARCHIVE_LIMIT {
        return Err(InstalledAttempt::Fatal(
            MissionAssetRestoreError::InstalledArchiveSize {
                label,
                path: canonical_path,
                bytes: bytes.len() as u64,
                limit: DISTRIBUTED_MOD_ARCHIVE_LIMIT,
            },
        ));
    }
    let bytes: Arc<[u8]> = Arc::from(bytes);
    verify_archive_arc_identity(label, &bytes, identity).map_err(InstalledAttempt::Fatal)?;
    Ok(bytes)
}

fn validate_cache_identity(
    descriptor: &MissionAssetDescriptor,
    archive: &ArchiveMissionAssets,
    lease: &DistributedModCacheLease,
) -> Result<(), MissionAssetRestoreError> {
    let cache = archive.distributed_cache.as_ref().ok_or_else(|| {
        MissionAssetRestoreError::CacheIdentityMismatch(
            "descriptor has no durable-cache identity".to_owned(),
        )
    })?;
    if cache.schema_version != DISTRIBUTED_MOD_SCHEMA_VERSION {
        return Err(MissionAssetRestoreError::CacheIdentityMismatch(format!(
            "descriptor cache schema is {}; expected {DISTRIBUTED_MOD_SCHEMA_VERSION}",
            cache.schema_version
        )));
    }
    if lease.encoded().len() as u64 != cache.encoded_bytes {
        return Err(MissionAssetRestoreError::CacheIdentityMismatch(format!(
            "descriptor declares {} encoded bytes; cache lease has {}",
            cache.encoded_bytes,
            lease.encoded().len()
        )));
    }
    let validated = &lease.validated;
    let manifest = &validated.package.manifest;
    if manifest.schema_version != cache.schema_version
        || manifest.full_mod_sha256 != cache.full_mod_sha256
    {
        return Err(MissionAssetRestoreError::CacheIdentityMismatch(
            "cache lease full-mod identity does not match the descriptor".to_owned(),
        ));
    }
    require_manifest_field(
        "mission basename",
        &descriptor.mission_basename,
        &manifest.mission_basename,
    )?;
    require_manifest_field(
        "selected RHM entry",
        &archive.selected_rhm_entry,
        &manifest.mission_rhm_entry,
    )?;
    require_manifest_field(
        "map filename",
        &descriptor.map_filename,
        &manifest.map_filename,
    )?;
    require_manifest_field(
        "proto-level filename",
        &descriptor.proto_level_filename,
        &manifest.map_filename,
    )?;
    require_archive_identity(
        "mission",
        &archive.mission_archive,
        manifest.mission_archive_bytes,
        manifest.mission_archive_sha256,
    )?;
    match (
        archive.shared_archive.as_ref(),
        manifest.shared_library_bytes,
        manifest.shared_library_sha256,
        validated.package.shared_library_archive.as_ref(),
    ) {
        (None, None, None, None) => {}
        (Some(expected), Some(bytes), Some(hash), Some(_)) if manifest.requires_spellforge => {
            require_archive_identity("shared library", expected, bytes, hash)?;
        }
        _ => {
            return Err(MissionAssetRestoreError::CacheIdentityMismatch(
                "descriptor and cached manifest disagree about shared/Spellforge content"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn require_manifest_field(
    label: &'static str,
    expected: &str,
    actual: &str,
) -> Result<(), MissionAssetRestoreError> {
    if actual != expected {
        return Err(MissionAssetRestoreError::CacheIdentityMismatch(format!(
            "{label} is `{actual}`; descriptor requires `{expected}`"
        )));
    }
    Ok(())
}

fn require_archive_identity(
    label: &'static str,
    expected: &ArchiveIdentity,
    actual_bytes: u64,
    actual_hash: [u8; 32],
) -> Result<(), MissionAssetRestoreError> {
    if expected.bytes != actual_bytes || expected.sha256 != actual_hash {
        return Err(MissionAssetRestoreError::ArchiveIdentityMismatch {
            label,
            declared_bytes: expected.bytes,
            declared_hash: hex_hash(&expected.sha256),
            actual_bytes,
            actual_hash: hex_hash(&actual_hash),
        });
    }
    Ok(())
}

fn verify_archive_arc_identity(
    label: &'static str,
    bytes: &Arc<[u8]>,
    expected: &ArchiveIdentity,
) -> Result<(), MissionAssetRestoreError> {
    let actual_hash: [u8; 32] = Sha256::digest(bytes).into();
    require_archive_identity(label, expected, bytes.len() as u64, actual_hash)
}

fn validate_proto_and_map(
    descriptor: &MissionAssetDescriptor,
) -> Result<(), MissionAssetRestoreError> {
    if descriptor.proto_level_filename != descriptor.map_filename {
        return Err(MissionAssetRestoreError::ArchiveAdmission(format!(
            "custom mission proto-level `{}` does not exactly match selected RHM map `{}`",
            descriptor.proto_level_filename, descriptor.map_filename
        )));
    }
    Ok(())
}

fn verify_spellforge_authority(
    derived: Option<&SpellforgePackage>,
    embedded: Option<&SpellforgePackage>,
) -> Result<(), MissionAssetRestoreError> {
    if let Some(package) = embedded {
        package.validate_wire().map_err(|error| {
            MissionAssetRestoreError::SpellforgePackageMismatch(format!(
                "embedded package is invalid: {error}"
            ))
        })?;
    }
    match (derived, embedded) {
        (None, None) => Ok(()),
        (Some(derived), Some(embedded)) if derived == embedded => Ok(()),
        (Some(derived), Some(embedded)) => Err(
            MissionAssetRestoreError::SpellforgePackageMismatch(format!(
                "embedded hash {} differs from archive-derived hash {}",
                hex_hash(&embedded.sha256),
                hex_hash(&derived.sha256)
            )),
        ),
        (Some(derived), None) => Err(MissionAssetRestoreError::SpellforgePackageMismatch(
            format!(
                "archives derive Spellforge package {}, but the save/replay embeds none",
                hex_hash(&derived.sha256)
            ),
        )),
        (None, Some(embedded)) => Err(MissionAssetRestoreError::SpellforgePackageMismatch(
            format!(
                "save/replay embeds Spellforge package {}, but the descriptor has no shared archive",
                hex_hash(&embedded.sha256)
            ),
        )),
    }
}

fn mount_resolved(
    descriptor: &MissionAssetDescriptor,
    archive: &ArchiveMissionAssets,
    mission_archive: Arc<[u8]>,
    shared_archive: Option<Arc<[u8]>>,
    cache_lease: Option<DistributedModCacheLease>,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<ResolvedMissionAssets, MissionAssetRestoreError> {
    let sequence = MOUNT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let namespace = format!(
        "cold-{}-{sequence}",
        &hex_hash(&archive.mission_archive.sha256)[..16]
    );
    let mount = mount_distributed_archives(
        &namespace,
        Arc::clone(&mission_archive),
        shared_archive.as_ref().map(Arc::clone),
        &archive.selected_rhm_entry,
        files,
    )
    .map_err(|error| MissionAssetRestoreError::Mount(error.to_string()))?;
    Ok(ResolvedMissionAssets {
        descriptor: descriptor.clone(),
        mount: Some(mount),
        cache_lease,
        mission_archive: Some(mission_archive),
        shared_archive,
        selected_rhm_entry: Some(archive.selected_rhm_entry.clone()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn installed_root_name(root: &InstalledModsRoot) -> &'static str {
    match root {
        InstalledModsRoot::ConfiguredMods => "configured",
        InstalledModsRoot::BundledMods => "bundled",
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn is_safe_logical_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1_024
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        && !path
            .split('/')
            .next()
            .is_some_and(|component| component.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use robin_engine::mission_assets::{
        ArchiveMissionAssets, DistributedCacheIdentity, InstalledArchiveLocator, InstalledModsRoot,
    };
    #[cfg(not(target_arch = "wasm32"))]
    fn independent_files() -> Arc<robin_engine::sbfile::SbFileSystem> {
        Arc::new(robin_engine::sbfile::SbFileSystem::new(Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        )))
    }
    #[cfg(not(target_arch = "wasm32"))]
    use std::io::Write;

    #[cfg(not(target_arch = "wasm32"))]
    fn rhm(map: &str, marker: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 34 + map.len() + 2];
        bytes[..4].copy_from_slice(b"RHMI");
        bytes[12..16].copy_from_slice(b"HEAD");
        bytes[32..34].copy_from_slice(&((map.len() + 1) as u16).to_le_bytes());
        bytes[34..34 + map.len()].copy_from_slice(map.as_bytes());
        *bytes.last_mut().unwrap() = marker;
        bytes
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            for (path, bytes) in entries {
                writer
                    .start_file(*path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn identity(bytes: &[u8]) -> ArchiveIdentity {
        ArchiveIdentity {
            sha256: Sha256::digest(bytes).into(),
            bytes: bytes.len() as u64,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn installed_descriptor(
        mission: &[u8],
        relative: &str,
        selected: &str,
    ) -> MissionAssetDescriptor {
        MissionAssetDescriptor::archive(
            "ColdMission".to_owned(),
            "ColdMap".to_owned(),
            "ColdMap".to_owned(),
            ArchiveMissionAssets {
                mission_archive: identity(mission),
                selected_rhm_entry: selected.to_owned(),
                shared_archive: None,
                installed: Some(InstalledArchiveLocator {
                    root: InstalledModsRoot::ConfiguredMods,
                    mission_relative_path: relative.to_owned(),
                    shared_relative_path: None,
                }),
                distributed_cache: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn safe_relative_paths_reject_escape_and_platform_ambiguity() {
        assert!(is_safe_logical_relative_path("nested/v1.0-DE.zip"));
        for unsafe_path in [
            "",
            "/absolute.zip",
            "../escape.zip",
            "nested/../../escape.zip",
            "nested\\escape.zip",
            "C:/escape.zip",
            "nested//archive.zip",
            "nested/./archive.zip",
        ] {
            assert!(!is_safe_logical_relative_path(unsafe_path), "{unsafe_path}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn installed_multilingual_archive_is_mounted_from_admitted_memory_until_drop() {
        let files = independent_files();
        let temp = tempfile::tempdir().unwrap();
        let relative = "rescue-allan/versions/v1.0-DE.zip";
        let path = temp.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let selected = "German/DATA/Levels/ColdMission.rhm";
        let selected_bytes = rhm("ColdMap", 0x5a);
        let archive = zip(&[
            (selected, selected_bytes.clone()),
            ("German/DATA/Levels/ColdMap.rhp", b"proto".to_vec()),
            ("English/DATA/Levels/ColdMission.rhm", rhm("ColdMap", 0x33)),
        ]);
        std::fs::write(&path, &archive).unwrap();
        let descriptor = installed_descriptor(&archive, relative, selected);
        let roots = MissionAssetRoots {
            configured_mods: temp.path().to_owned(),
            bundled_mods: None,
        };

        let resolved =
            resolve_native_mission_assets(&descriptor, None, &roots, None, files.clone()).unwrap();
        assert!(resolved.is_archive());
        assert!(!resolved.is_cache_backed());
        assert_eq!(resolved.selected_rhm_entry(), Some(selected));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            files.read_all("Data/Levels/ColdMission.rhm").unwrap(),
            selected_bytes
        );
        drop(resolved);
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn installed_hash_mismatch_is_fatal_and_never_mounts() {
        let files = independent_files();
        let temp = tempfile::tempdir().unwrap();
        let relative = "mod/v1.zip";
        let path = temp.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let selected = "ColdMission.rhm";
        let archive = zip(&[(selected, rhm("ColdMap", 1))]);
        let mut descriptor = installed_descriptor(&archive, relative, selected);
        let MissionAssetSource::Archive(content) = &mut descriptor.source else {
            unreachable!()
        };
        content.mission_archive.sha256[0] ^= 0xff;
        std::fs::write(path, archive).unwrap();
        let roots = MissionAssetRoots {
            configured_mods: temp.path().to_owned(),
            bundled_mods: None,
        };
        let error = resolve_native_mission_assets(&descriptor, None, &roots, None, files.clone())
            .expect_err("tampered installed archive must fail");
        assert!(matches!(
            error,
            MissionAssetRestoreError::ArchiveIdentityMismatch { .. }
        ));
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cold_restore_still_admits_archives_with_matching_identity() {
        use crate::distributed_mod::MISSION_ARCHIVE_ADMISSIONS;

        let files = independent_files();
        let temp = tempfile::tempdir().unwrap();
        let selected = "ColdMission.rhm";
        // Correct ZIP and descriptor hashes cannot authorize a different RHM map.
        let archive = zip(&[(selected, rhm("WrongMap", 1))]);
        let descriptor = installed_descriptor(&archive, "mission.zip", selected);
        std::fs::write(temp.path().join("mission.zip"), archive).unwrap();
        let roots = MissionAssetRoots {
            configured_mods: temp.path().to_owned(),
            bundled_mods: None,
        };
        let before = MISSION_ARCHIVE_ADMISSIONS.get();
        let error = resolve_native_mission_assets(&descriptor, None, &roots, None, files.clone())
            .expect_err("matching archive identity does not bypass cold admission");
        assert_eq!(MISSION_ARCHIVE_ADMISSIONS.get() - before, 1);
        assert!(
            matches!(error, MissionAssetRestoreError::ArchiveAdmission(_)),
            "{error}"
        );
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn exact_durable_cache_recovers_a_missing_installed_archive_and_stays_pinned() {
        let files = independent_files();
        let temp = tempfile::tempdir().unwrap();
        let selected = "Nested/German/DATA/Levels/ColdMission.rhm";
        let archive = zip(&[(selected, rhm("ColdMap", 7))]);
        let validated = crate::distributed_mod::DistributedModPackage::build(
            "cold-mod".into(),
            "Cold Mod".into(),
            "Author".into(),
            "1".into(),
            "https://example.invalid".into(),
            "test-only".into(),
            "ColdMission".into(),
            selected.into(),
            "ColdMap".into(),
            false,
            archive.clone(),
            None,
        )
        .unwrap();
        let encoded = validated.package.encode().unwrap();
        let full_hash = validated.package.manifest.full_mod_sha256;
        let mut cache = DistributedModCache::open(temp.path().to_str().unwrap()).unwrap();
        drop(cache.install(encoded.clone(), full_hash).unwrap());
        let descriptor = MissionAssetDescriptor::archive(
            "ColdMission",
            "ColdMap",
            "ColdMap",
            ArchiveMissionAssets {
                mission_archive: identity(&archive),
                selected_rhm_entry: selected.into(),
                shared_archive: None,
                installed: Some(InstalledArchiveLocator {
                    root: InstalledModsRoot::ConfiguredMods,
                    mission_relative_path: "not-installed/v1.zip".into(),
                    shared_relative_path: None,
                }),
                distributed_cache: Some(DistributedCacheIdentity {
                    schema_version: DISTRIBUTED_MOD_SCHEMA_VERSION,
                    full_mod_sha256: full_hash,
                    encoded_bytes: encoded.len() as u64,
                }),
            },
        )
        .unwrap();
        let roots = MissionAssetRoots {
            configured_mods: temp.path().join("mods"),
            bundled_mods: None,
        };
        let resolved = resolve_native_mission_assets(
            &descriptor,
            None,
            &roots,
            Some(&mut cache),
            files.clone(),
        )
        .unwrap();
        assert!(resolved.is_cache_backed());
        let retained = &resolved.cache_lease.as_ref().unwrap().validated.package;
        assert!(Arc::ptr_eq(
            resolved.mission_archive().unwrap(),
            &retained.mission_archive
        ));
        assert!(
            cache.clear().is_err(),
            "mounted mission must retain the cache pin"
        );
        assert_eq!(
            files.read_all("Data/Levels/ColdMission.rhm").unwrap(),
            rhm("ColdMap", 7)
        );
        drop(resolved);
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());
        assert_eq!(
            cache.clear().unwrap(),
            1,
            "dropping the mount must release its pin"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cache_manifest_must_match_every_descriptor_identity() {
        let files = independent_files();
        let temp = tempfile::tempdir().unwrap();
        let selected = "ColdMission.rhm";
        let archive = zip(&[(selected, rhm("ColdMap", 4))]);
        let validated = crate::distributed_mod::DistributedModPackage::build(
            "cold-mod".into(),
            "Cold Mod".into(),
            "Author".into(),
            "1".into(),
            "https://example.invalid".into(),
            "test-only".into(),
            "ColdMission".into(),
            selected.into(),
            "ColdMap".into(),
            false,
            archive.clone(),
            None,
        )
        .unwrap();
        let encoded = validated.package.encode().unwrap();
        let full_hash = validated.package.manifest.full_mod_sha256;
        let mut cache = DistributedModCache::open(temp.path().to_str().unwrap()).unwrap();
        let lease = cache.install(encoded.clone(), full_hash).unwrap();
        let mut descriptor = MissionAssetDescriptor::archive(
            "ColdMission",
            "ColdMap",
            "ColdMap",
            ArchiveMissionAssets {
                mission_archive: identity(&archive),
                selected_rhm_entry: selected.into(),
                shared_archive: None,
                installed: None,
                distributed_cache: Some(DistributedCacheIdentity {
                    schema_version: DISTRIBUTED_MOD_SCHEMA_VERSION,
                    full_mod_sha256: full_hash,
                    encoded_bytes: encoded.len() as u64,
                }),
            },
        )
        .unwrap();
        descriptor.proto_level_filename = "DifferentMap".into();
        descriptor.map_filename = "DifferentMap".into();
        let error = resolve_cached_mission_assets(&descriptor, None, lease, files.clone())
            .expect_err("descriptor/cache manifest mismatch must fail");
        assert!(matches!(
            error,
            MissionAssetRestoreError::CacheIdentityMismatch(_)
        ));
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cache_spellforge_archives_require_the_exact_embedded_package_authority() {
        let files = independent_files();
        let temp = tempfile::tempdir().unwrap();
        let selected = "Data/Levels/ColdMission.rhm";
        let shared = zip(&[("Data/Text/shared.res", b"shared".to_vec())]);
        let archive = zip(&[
            (selected, rhm("ColdMap", 8)),
            (
                "Data/Levels/ColdMission.lua",
                b"function StartUp() return 8 end".to_vec(),
            ),
        ]);
        let validated = crate::distributed_mod::DistributedModPackage::build(
            "cold-spell".into(),
            "Cold Spell".into(),
            "Author".into(),
            "1".into(),
            "https://example.invalid".into(),
            "test-only".into(),
            "ColdMission".into(),
            selected.into(),
            "ColdMap".into(),
            true,
            archive.clone(),
            Some(shared.clone()),
        )
        .unwrap();
        let authoritative_package = validated.spellforge_package.clone().unwrap();
        let encoded = validated.package.encode().unwrap();
        let full_hash = validated.package.manifest.full_mod_sha256;
        let mut cache = DistributedModCache::open(temp.path().to_str().unwrap()).unwrap();
        drop(cache.install(encoded.clone(), full_hash).unwrap());
        let descriptor = MissionAssetDescriptor::archive(
            "ColdMission",
            "ColdMap",
            "ColdMap",
            ArchiveMissionAssets {
                mission_archive: identity(&archive),
                selected_rhm_entry: selected.into(),
                shared_archive: Some(identity(&shared)),
                installed: None,
                distributed_cache: Some(DistributedCacheIdentity {
                    schema_version: DISTRIBUTED_MOD_SCHEMA_VERSION,
                    full_mod_sha256: full_hash,
                    encoded_bytes: encoded.len() as u64,
                }),
            },
        )
        .unwrap();

        let missing_authority = resolve_cached_mission_assets(
            &descriptor,
            None,
            cache.acquire(full_hash).unwrap().unwrap(),
            files.clone(),
        )
        .expect_err("derived guest code without embedded authority must fail");
        assert!(matches!(
            missing_authority,
            MissionAssetRestoreError::SpellforgePackageMismatch(_)
        ));
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());

        let resolved = resolve_cached_mission_assets(
            &descriptor,
            Some(&authoritative_package),
            cache.acquire(full_hash).unwrap().unwrap(),
            files.clone(),
        )
        .unwrap();
        assert!(resolved.is_archive());
        let retained = &resolved.cache_lease.as_ref().unwrap().validated.package;
        assert!(Arc::ptr_eq(
            resolved.shared_archive().unwrap(),
            retained.shared_library_archive.as_ref().unwrap()
        ));
        drop(resolved);
        assert!(files.read_all("Data/Levels/ColdMission.rhm").is_err());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn built_in_descriptor_rejects_embedded_guest_code() {
        let descriptor = MissionAssetDescriptor::built_in("H01_Lin", "lincoln", "lincoln").unwrap();
        let package = SpellforgePackage {
            contract_version: robin_engine::spellforge::SPELLFORGE_CONTRACT_VERSION,
            vm_abi: format!(
                "{}{}",
                robin_engine::spellforge::SPELLFORGE_VM_ABI_SCHEME,
                "5a".repeat(32)
            ),
            script_mode: robin_engine::spellforge::SpellforgeScriptMode::Replace,
            entrypoint: "mission.lua".into(),
            files: [(
                "mission.lua".to_owned(),
                b"function OnInit() return 0 end".to_vec(),
            )]
            .into(),
            sha256: [0; 32],
        };
        assert!(matches!(
            resolve_built_in_mission_assets(&descriptor, Some(&package)),
            Err(MissionAssetRestoreError::BuiltInSpellforgePackage)
        ));
        #[cfg(not(target_arch = "wasm32"))]
        assert!(matches!(
            resolve_native_mission_assets(
                &descriptor,
                Some(&package),
                &MissionAssetRoots {
                    configured_mods: PathBuf::from("unused"),
                    bundled_mods: None,
                },
                None,
                independent_files(),
            ),
            Err(MissionAssetRestoreError::BuiltInSpellforgePackage)
        ));
    }
}
