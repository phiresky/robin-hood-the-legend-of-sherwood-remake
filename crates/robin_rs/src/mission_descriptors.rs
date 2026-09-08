//! Presentation policy for optional, application-owned mission metadata.

use robin_assets::{res_descr, shipping_datadir::ShippingDatadir};

use crate::host::ApplicationContext;

/// Presentation may continue with generic/empty text when optional metadata
/// is missing. Invalid metadata or missing preparation authority is separately
/// diagnosed, never silently reinterpreted as absence or read from global state.
pub(crate) fn for_presentation(
    application: &ApplicationContext,
    shipping: Option<&ShippingDatadir>,
    mission_id: u32,
) -> Option<res_descr::LevelDescriptors> {
    let result = application
        .preparation_files()
        .map_err(anyhow::Error::msg)
        .and_then(|files| res_descr::resolve(mission_id, files, shipping));
    match result {
        Ok(Some(descriptor)) => Some(descriptor),
        Ok(None) => {
            tracing::debug!(
                mission_id,
                "Optional mission descriptor absent; using presentation fallback"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                mission_id,
                "Mission descriptor unavailable: {error:#}; using presentation fallback"
            );
            None
        }
    }
}
