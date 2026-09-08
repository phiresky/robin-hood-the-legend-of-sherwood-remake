//! First-use and reconnect admission for host-distributed full mods.
//!
//! Transport authentication, player trust, durable byte staging, canonical
//! package validation, and VFS mounting are separate checks. This module is
//! the native coordinator which performs every check before acknowledging
//! `ContentReady`; no cache hit or trust record substitutes for validating
//! and mounting the exact offered bytes.

#![cfg(not(target_arch = "wasm32"))]

use crate::distributed_mod_cache::DistributedModCacheLease;
use crate::host::ApplicationContext;
use crate::mod_pack::MountGuard;
use robin_engine::multiplayer::DistributedModOffer;

/// Keeps both the verified cache object and its mounted in-memory archives
/// alive for one mission. Dropping this unmounts the archives before releasing
/// the cache pin, so eviction can never race a live session.
pub struct AdmittedDistributedMod {
    pub mount: DistributedModMount,
    pub cache_lease: DistributedModCacheLease,
}

/// Exact native archive overlays retained for one mission.
pub struct DistributedModMount {
    pub mount_guard: MountGuard,
    /// Native `SbFile` ZIP overlays keep their files open. Retain the exact
    /// verified archive materialization until the overlay is removed.
    _archive_directory: tempfile::TempDir,
}

pub use crate::distributed_mod_admission_common::{
    DistributedModAdmissionPurpose, admit_trusted_distributed_mod,
};

pub(crate) const OFFER_WAIT: Option<std::time::Duration> = None;
pub(crate) const TRANSFER_WAIT: Option<std::time::Duration> = None;
pub(crate) async fn resume_offset(
    context: &ApplicationContext,
    offer: &DistributedModOffer,
) -> Result<u64, String> {
    context.with_distributed_mod_cache_mut(|cache| {
        cache.resume_offset(offer.full_mod_sha256, offer.encoded_bytes)
    })
}
pub(crate) async fn append_chunk(
    context: &ApplicationContext,
    offer: &DistributedModOffer,
    offset: u64,
    bytes: &[u8],
) -> Result<u64, String> {
    context.with_distributed_mod_cache_mut(|cache| {
        cache.append_chunk(offer.full_mod_sha256, offer.encoded_bytes, offset, bytes)
    })
}
pub(crate) async fn finish_transfer(
    context: &ApplicationContext,
    offer: &DistributedModOffer,
    complete: bool,
) -> Result<DistributedModCacheLease, String> {
    context.with_distributed_mod_cache_mut(|cache| {
        if complete {
            cache.acquire(offer.full_mod_sha256)?.ok_or_else(|| {
                "cache declared a complete entry but could not acquire it".to_owned()
            })
        } else {
            cache.finish_partial(offer.full_mod_sha256, offer.encoded_bytes)
        }
    })
}

pub fn mount_validated_distributed_mod(
    validated: &crate::distributed_mod::ValidatedDistributedMod,
    files: std::sync::Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<DistributedModMount, String> {
    let directory = tempfile::Builder::new()
        .prefix("robin-distributed-mod-")
        .tempdir()
        .map_err(|error| format!("create verified mod mount directory: {error}"))?;
    let mission_path = directory.path().join("mission.zip");
    std::fs::write(&mission_path, &validated.package.mission_archive)
        .map_err(|error| format!("materialize verified mission archive: {error}"))?;
    if let Some(shared) = validated.package.shared_library_archive.as_deref() {
        let lib_directory = directory.path().join("lib");
        std::fs::create_dir(&lib_directory)
            .map_err(|error| format!("create verified Spellforge library directory: {error}"))?;
        std::fs::write(lib_directory.join("shared.zip"), shared)
            .map_err(|error| format!("materialize verified Spellforge library: {error}"))?;
    }
    let guard = crate::mod_pack::mount_for_selected_mission(
        &mission_path,
        validated.package.manifest.requires_spellforge,
        directory.path(),
        &validated.package.manifest.mission_rhm_entry,
        files,
    )
    .map_err(|error| error.to_string())?;
    Ok(DistributedModMount {
        mount_guard: guard,
        _archive_directory: directory,
    })
}
