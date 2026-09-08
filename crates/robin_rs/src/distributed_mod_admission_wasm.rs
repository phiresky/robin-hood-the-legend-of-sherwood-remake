//! Browser admission for exact host-distributed full mods.

use crate::distributed_mod_cache::{self, DistributedModCacheLease};
use crate::host::ApplicationContext;
use crate::mod_pack::MountGuard;
use robin_engine::multiplayer::DistributedModOffer;
use std::sync::Arc;

pub struct AdmittedDistributedMod {
    pub mount: DistributedModMount,
    pub cache_lease: DistributedModCacheLease,
}

pub struct DistributedModMount {
    pub mount_guard: MountGuard,
}

pub use crate::distributed_mod_admission_common::{
    DistributedModAdmissionPurpose, admit_trusted_distributed_mod,
};

pub(crate) const OFFER_WAIT: Option<std::time::Duration> = Some(std::time::Duration::from_secs(15));
pub(crate) const TRANSFER_WAIT: Option<std::time::Duration> =
    Some(std::time::Duration::from_secs(15 * 60));
pub(crate) async fn resume_offset(
    _context: &ApplicationContext,
    offer: &DistributedModOffer,
) -> Result<u64, String> {
    distributed_mod_cache::resume_offset(offer.full_mod_sha256, offer.encoded_bytes).await
}
pub(crate) async fn append_chunk(
    _context: &ApplicationContext,
    offer: &DistributedModOffer,
    offset: u64,
    bytes: &[u8],
) -> Result<u64, String> {
    distributed_mod_cache::append_chunk(offer.full_mod_sha256, offer.encoded_bytes, offset, bytes)
        .await
}
pub(crate) async fn finish_transfer(
    _context: &ApplicationContext,
    offer: &DistributedModOffer,
    complete: bool,
) -> Result<DistributedModCacheLease, String> {
    if complete {
        distributed_mod_cache::acquire(offer.full_mod_sha256)
            .await?
            .ok_or_else(|| "cache declared a complete entry but could not acquire it".to_owned())
    } else {
        distributed_mod_cache::finish_partial(offer.full_mod_sha256, offer.encoded_bytes).await
    }
}

pub fn mount_validated_distributed_mod(
    validated: &crate::distributed_mod::ValidatedDistributedMod,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<DistributedModMount, String> {
    mount_validated_distributed_mod_in_scope(validated, "admission", files)
}

pub fn mount_validated_distributed_mod_for_startup(
    validated: &crate::distributed_mod::ValidatedDistributedMod,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<DistributedModMount, String> {
    mount_validated_distributed_mod_in_scope(validated, "startup", files)
}

fn mount_validated_distributed_mod_in_scope(
    validated: &crate::distributed_mod::ValidatedDistributedMod,
    scope: &str,
    files: Arc<robin_engine::sbfile::SbFileSystem>,
) -> Result<DistributedModMount, String> {
    let namespace = format!(
        "{scope}-{}",
        robin_engine::spellforge::hex_hash(&validated.package.manifest.full_mod_sha256)
    );
    let mission = Arc::<[u8]>::from(validated.package.mission_archive.clone());
    let shared = validated
        .package
        .shared_library_archive
        .clone()
        .map(Arc::<[u8]>::from);
    let guard = crate::mod_pack::mount_distributed_archives(
        &namespace,
        mission,
        shared,
        &validated.package.manifest.mission_rhm_entry,
        files,
    )
    .map_err(|error| error.to_string())?;
    Ok(DistributedModMount { mount_guard: guard })
}
