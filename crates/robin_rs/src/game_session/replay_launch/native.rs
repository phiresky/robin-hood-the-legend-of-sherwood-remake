//! Native replay asset restore: installed roots, preferring the distributed-mod cache.

use crate::host::ApplicationContext;
use robin_engine::mission_assets::MissionAssetDescriptor;
use robin_engine::spellforge::SpellforgePackage;

/// Resolve a replay descriptor that is not `BuiltIn`.
pub(super) async fn resolve_non_built_in_mission_assets(
    application_context: &ApplicationContext,
    descriptor: &MissionAssetDescriptor,
    package: Option<&SpellforgePackage>,
) -> Result<crate::mission_asset_restore::ResolvedMissionAssets, String> {
    let roots = crate::mission_asset_restore::MissionAssetRoots::discover();
    let with_cache = application_context.with_distributed_mod_cache_mut(|cache| {
        crate::mission_asset_restore::resolve_native_mission_assets(
            descriptor,
            package,
            &roots,
            Some(cache),
            application_context.preparation_files()?.clone(),
        )
        .map_err(|error| error.to_string())
    });
    match with_cache {
        Ok(resolved) => Ok(resolved),
        Err(cache_error) => crate::mission_asset_restore::resolve_native_mission_assets(
            descriptor,
            package,
            &roots,
            None,
            application_context.preparation_files()?.clone(),
        )
        .map_err(|without_cache| {
            format!(
                "restore replay mission assets without cache: {without_cache}; cache attempt: {cache_error}"
            )
        }),
    }
}
