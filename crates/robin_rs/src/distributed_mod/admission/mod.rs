//! Shared admission workflow with compile-time storage adapters.
//!
//! [`workflow`] owns the platform-independent admission sequence. Every
//! platform-specific step (durable staging, final acquisition, VFS mounting
//! and the offer/transfer waiting policy) goes through the explicit
//! [`DistributedModStore`] interface, implemented once per backend.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
pub mod workflow;

#[cfg(target_arch = "wasm32")]
pub use browser::DistributedModMount;
#[cfg(target_arch = "wasm32")]
pub use browser::{BrowserDistributedModStore, mount_validated_distributed_mod_for_startup};
#[cfg(not(target_arch = "wasm32"))]
pub use native::DistributedModMount;
#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeDistributedModStore;

pub use workflow::{DistributedModAdmissionPurpose, admit_trusted_distributed_mod};

use crate::distributed_mod_cache::DistributedModCacheLease;
use crate::host::ApplicationContext;
use robin_engine::multiplayer::DistributedModOffer;

/// The storage backend selected for this target.
#[cfg(target_arch = "wasm32")]
pub type PlatformDistributedModStore = BrowserDistributedModStore;
/// The storage backend selected for this target.
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformDistributedModStore = NativeDistributedModStore;

/// Keeps both the verified cache object and its mounted in-memory archives
/// alive for one mission. Dropping this unmounts the archives before releasing
/// the cache pin, so eviction can never race a live session.
pub struct AdmittedDistributedMod {
    pub mount: DistributedModMount,
    pub cache_lease: DistributedModCacheLease,
}

/// Platform storage adapter used by the shared admission workflow.
///
/// The lease and mount types are the target's own cache/mount types; the
/// interface exists so the workflow names every backend operation explicitly
/// instead of importing whichever cfg-selected free functions exist.
pub(crate) trait DistributedModStore {
    /// How long the workflow waits for the transport to expose the
    /// authenticated offer. `None` means "must already be available" (no wait).
    const OFFER_WAIT: Option<std::time::Duration>;
    /// Total budget for the chunked transfer. `None` means unbounded.
    const TRANSFER_WAIT: Option<std::time::Duration>;

    /// Durable byte offset from which the transfer of `offer` resumes.
    async fn resume_offset(
        context: &ApplicationContext,
        offer: &DistributedModOffer,
    ) -> Result<u64, String>;

    /// Durably append one sequential chunk; returns the new durable offset.
    async fn append_chunk(
        context: &ApplicationContext,
        offer: &DistributedModOffer,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, String>;

    /// Acquire the validated complete entry (`complete`) or finish the staged
    /// partial into one.
    async fn finish_transfer(
        context: &ApplicationContext,
        offer: &DistributedModOffer,
        complete: bool,
    ) -> Result<DistributedModCacheLease, String>;

    /// Mount the exact validated archives for one admission.
    fn mount(
        validated: &crate::distributed_mod::ValidatedDistributedMod,
        files: std::sync::Arc<robin_engine::sbfile::SbFileSystem>,
    ) -> Result<DistributedModMount, String>;
}
