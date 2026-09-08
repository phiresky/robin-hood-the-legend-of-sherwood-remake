//! Browser-publication cleanup; source/native publications retain their bytes.
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::shipping_datadir::ShippingDatadir;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BrowserAudioTrim {
    pub candidates: usize,
    pub removed_files: usize,
    pub removed_bytes: usize,
    pub retained_without_catalog: usize,
    pub retained_distinct_source: usize,
}

/// Remove only proven copies of source audio already represented by the
/// browser's on-demand Opus catalog. The source reader must resolve paths with
/// exactly the same precedence used when constructing that catalog. Translated
/// or otherwise distinct locale audio remains embedded; a matching filename is
/// insufficient proof. Call only for a browser Opus publication.
pub fn trim_browser_locale_audio(
    datadir: &mut ShippingDatadir,
    mut read_catalog_source: impl FnMut(&str) -> Result<Option<Vec<u8>>>,
) -> Result<BrowserAudioTrim> {
    let catalog: std::collections::BTreeSet<_> = datadir.audio_assets.keys().cloned().collect();
    let mut report = BrowserAudioTrim::default();
    // Decide first, mutate only after every source read succeeds.
    let mut remove = Vec::new();
    for (locale_name, locale) in &datadir.locales {
        for (key, bytes) in &locale.raw {
            let canonical = crate::shipping_datadir::canonical_shipping_asset_key(key);
            if !matches!(
                Path::new(&canonical)
                    .extension()
                    .and_then(|ext| ext.to_str()),
                Some("wav" | "ogg")
            ) {
                continue;
            }
            report.candidates += 1;
            let opus_key = Path::new(&canonical)
                .with_extension("opus")
                .to_string_lossy()
                .into_owned();
            if !catalog.contains(&opus_key) {
                report.retained_without_catalog += 1;
                continue;
            }
            if read_catalog_source(&canonical)?.as_deref() != Some(bytes.as_slice()) {
                report.retained_distinct_source += 1;
                continue;
            }
            report.removed_files += 1;
            report.removed_bytes += bytes.len();
            remove.push((locale_name.clone(), key.clone()));
        }
    }
    for (locale, key) in remove {
        datadir
            .locales
            .get_mut(&locale)
            .expect("inspected locale disappeared")
            .raw
            .remove(&key)
            .expect("inspected source audio disappeared");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shipping_datadir::{ShippingAudioAsset, ShippingLocale};

    fn fixture() -> ShippingDatadir {
        let mut datadir = ShippingDatadir::default();
        datadir.audio_assets.insert(
            "sounds/voice.opus".into(),
            ShippingAudioAsset {
                file: "audio/voice.bundle".into(),
                encoded_size: 3,
                duration_ms: 10,
                bundle_offset: Some(7),
            },
        );
        for (locale, bytes) in [("en-US", vec![1, 2]), ("de-DE", vec![3, 4])] {
            datadir.locales.insert(
                locale.into(),
                ShippingLocale {
                    raw: [
                        ("Sounds\\Voice.WAV".into(), bytes),
                        ("sounds/unmapped.wav".into(), vec![5]),
                        ("interface/start.sxt".into(), vec![6]),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                },
            );
        }
        datadir
    }

    #[test]
    fn removes_only_catalog_covered_identical_source_and_preserves_playback_reference() {
        let mut datadir = fixture();
        let before = datadir.audio_assets.clone();
        let report = trim_browser_locale_audio(&mut datadir, |_| Ok(Some(vec![1, 2]))).unwrap();
        assert_eq!(
            (
                report.candidates,
                report.removed_files,
                report.removed_bytes
            ),
            (4, 1, 2)
        );
        assert_eq!(report.retained_without_catalog, 2);
        assert_eq!(report.retained_distinct_source, 1);
        assert!(
            !datadir.locales["en-US"]
                .raw
                .contains_key("Sounds\\Voice.WAV")
        );
        assert_eq!(
            datadir.locales["de-DE"].raw["Sounds\\Voice.WAV"],
            vec![3, 4]
        );
        assert!(
            datadir.locales["en-US"]
                .raw
                .contains_key("interface/start.sxt")
        );
        assert_eq!(datadir.audio_assets, before);
    }

    #[test]
    fn unavailable_source_is_retained_and_read_failure_is_atomic() {
        let mut datadir = fixture();
        let before = crate::shipping_datadir::encode_native(&datadir);
        let report = trim_browser_locale_audio(&mut datadir, |_| Ok(None)).unwrap();
        assert_eq!(report.removed_files, 0);
        let mut calls = 0;
        assert!(
            trim_browser_locale_audio(&mut datadir, |_| {
                calls += 1;
                if calls == 1 {
                    Ok(Some(vec![3, 4]))
                } else {
                    anyhow::bail!("source read failed")
                }
            })
            .is_err()
        );
        assert_eq!(crate::shipping_datadir::encode_native(&datadir), before);
    }
}
