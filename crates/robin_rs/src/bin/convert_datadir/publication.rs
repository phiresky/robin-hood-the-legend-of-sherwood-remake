//! Publish the final boot index after all dependency payloads are prepared.
use super::*;

pub(super) fn publish_shipping(
    dd: &ShippingDatadir,
    data_out: &Path,
    opts: &ShippingOpts,
) -> Result<()> {
    // Serialize + compress with the configured window log.
    let out_file = data_out.join("datadir.bin");
    let blob = robin_assets::shipping_datadir::encode_native(&dd);
    let compressed =
        robin_assets::shipping_datadir::zstd_compress_with_window(&blob, opts.zstd_window_log)?;
    fs::write(&out_file, compressed).with_context(|| format!("write {}", out_file.display()))?;
    tracing::info!(
        "wrote {} (windowLog={}, map={:?}, audio={:?})",
        out_file.display(),
        opts.zstd_window_log,
        opts.map_format,
        opts.audio_format
    );
    Ok(())
}
