//! Native shipping payload reads: loose datadir files on desktop, bundled
//! assets on Android.

#[cfg(not(target_os = "android"))]
use anyhow::Context as _;
use anyhow::Result;
use robin_assets::shipping_datadir::ShippingDatadir;

use super::CompressedPayload;

#[cfg(not(target_os = "android"))]
pub(super) async fn fetch(datadir: &ShippingDatadir, relative: &str) -> Result<CompressedPayload> {
    let path = datadir.source_file_path(relative)?;
    std::fs::read(&path)
        .map(CompressedPayload::Owned)
        .with_context(|| format!("read {}", path.display()))
}

#[cfg(target_os = "android")]
pub(super) async fn fetch(_datadir: &ShippingDatadir, relative: &str) -> Result<CompressedPayload> {
    crate::android::read_bundled_asset(&format!("Data/{relative}")).map(CompressedPayload::Owned)
}
