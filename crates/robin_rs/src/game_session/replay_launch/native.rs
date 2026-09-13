//! Native replay asset restore: installed roots, preferring the distributed-mod cache.

use crate::game_session::MissionError;
use crate::host::ApplicationContext;
use robin_engine::mission_assets::MissionAssetDescriptor;
use robin_engine::spellforge::SpellforgePackage;

/// Resolve a replay descriptor that is not `BuiltIn`.
pub(super) async fn resolve_non_built_in_mission_assets(
    application_context: &ApplicationContext,
    descriptor: &MissionAssetDescriptor,
    package: Option<&SpellforgePackage>,
) -> Result<crate::mission_asset_restore::ResolvedMissionAssets, MissionError> {
    let roots = crate::mission_asset_restore::MissionAssetRoots::discover();
    // The cache attempt crosses `with_distributed_mod_cache_mut`, whose
    // closure reports text; it is only embedded in the fallback's message.
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
            application_context
                .preparation_files()
                .map_err(MissionError::application)?
                .clone(),
        )
        .map_err(|without_cache| {
            // TODO(10/F11): leaf returns String (cache attempt text).
            MissionError::asset(format!(
                "restore replay mission assets without cache: {without_cache}; cache attempt: {cache_error}"
            ))
        }),
    }
}
