//! Browser replay asset restore: only the exact distributed-mod cache object.

use crate::host::ApplicationContext;
use robin_engine::mission_assets::{MissionAssetDescriptor, MissionAssetSource};
use robin_engine::spellforge::SpellforgePackage;

/// Resolve a replay descriptor that is not `BuiltIn`.
pub(super) async fn resolve_non_built_in_mission_assets(
    application_context: &ApplicationContext,
    descriptor: &MissionAssetDescriptor,
    package: Option<&SpellforgePackage>,
) -> Result<crate::mission_asset_restore::ResolvedMissionAssets, String> {
    match &descriptor.source {
        // TODO: unreachable — the caller resolves `BuiltIn` before dispatching
        // here; kept so the moved browser body stays behaviour-identical.
        MissionAssetSource::BuiltIn => {
            crate::mission_asset_restore::resolve_built_in_mission_assets(descriptor, package)
                .map_err(|error| error.to_string())
        }
        MissionAssetSource::Archive(archive) => {
            let cache_identity = archive.distributed_cache.as_ref().ok_or_else(|| {
                format!(
                    "browser cold replay `{}` has no exact distributed-cache identity",
                    descriptor.mission_basename
                )
            })?;
            let lease = crate::distributed_mod_cache::acquire(cache_identity.full_mod_sha256)
                .await
                .map_err(|error| format!("acquire browser replay mission cache: {error}"))?
                .ok_or_else(|| {
                    format!(
                        "browser replay mission cache has no exact object {}",
                        robin_engine::spellforge::hex_hash(&cache_identity.full_mod_sha256)
                    )
                })?;
            crate::mission_asset_restore::resolve_cached_mission_assets(
                descriptor,
                package,
                lease,
                application_context.preparation_files()?.clone(),
            )
            .map_err(|error| error.to_string())
        }
    }
}
