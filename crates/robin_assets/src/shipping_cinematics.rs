//! On-demand cinematic assets with small locale-aware catalogs in the boot data.
use crate::shipping_datadir::ShippingDatadir;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const REFERENCE_SUFFIX: &str = ".json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CinematicAsset {
    pub file: String,
    pub byte_length: usize,
    pub sha256: String,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl CinematicAsset {
    pub fn validate(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() == self.byte_length && digest(bytes) == self.sha256,
            "cinematic asset identity mismatch"
        );
        Ok(())
    }

    pub fn data_path(&self) -> Result<String> {
        let leaf = self
            .file
            .strip_prefix("cinematics/assets/")
            .context("invalid cinematic asset path")?;
        ensure!(
            !leaf.is_empty()
                && leaf
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
            "invalid cinematic asset filename"
        );
        Ok(format!("Data/{}", self.file))
    }
}

/// Extract video bytes while preserving shared/locale lookup keys and all other boot data.
/// The caller must publish the returned assets before publishing the updated boot.
pub fn split_cinematics(data: &mut ShippingDatadir) -> Result<BTreeMap<String, Vec<u8>>> {
    let data: &mut crate::shipping_datadir::ShippingDatadirPayload = data;
    let mut assets = BTreeMap::new();
    for raw in std::iter::once(&mut data.raw)
        .chain(data.locales.values_mut().map(|locale| &mut locale.raw))
    {
        let keys: Vec<_> = raw
            .keys()
            .filter(|key| {
                key.starts_with("cinematics/")
                    && [".ogg", ".bik", ".avi", ".mp4", ".webm"]
                        .iter()
                        .any(|extension| key.ends_with(extension))
            })
            .cloned()
            .collect();
        if keys.is_empty() {
            continue;
        }
        for key in keys {
            let bytes = raw
                .remove(&key)
                .context("cinematic disappeared during split")?;
            let sha256 = digest(&bytes);
            let extension = key
                .rsplit_once('.')
                .map(|(_, ext)| ext)
                .context("cinematic has no extension")?;
            ensure!(
                extension.bytes().all(|b| b.is_ascii_alphanumeric()),
                "invalid cinematic extension"
            );
            let file = format!("cinematics/assets/{sha256}.{extension}");
            let reference = format!("{key}{REFERENCE_SUFFIX}");
            ensure!(
                !raw.contains_key(&reference),
                "cinematic reference already exists: {reference}"
            );
            raw.insert(
                reference,
                serde_json::to_vec(&CinematicAsset {
                    file: file.clone(),
                    byte_length: bytes.len(),
                    sha256,
                })?,
            );
            assets.insert(file, bytes);
        }
    }
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shipping_datadir::ShippingLocale;
    #[test]
    fn split_preserves_locale_identity_and_non_video_data() {
        let mut data = ShippingDatadir::default();
        data.raw
            .insert("cinematics/intro.ogg".into(), b"shared".to_vec());
        data.raw
            .insert("interface/test.cfg".into(), b"keep".to_vec());
        let mut locale = ShippingLocale::default();
        locale
            .raw
            .insert("cinematics/intro.ogg".into(), b"translated".to_vec());
        data.locales.insert("de-DE".into(), locale);
        let assets = split_cinematics(&mut data).unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(data.raw["interface/test.cfg"], b"keep");
        assert!(!data.raw.contains_key("cinematics/intro.ogg"));
        for raw in [&data.raw, &data.locales["de-DE"].raw] {
            let entry: CinematicAsset =
                serde_json::from_slice(&raw["cinematics/intro.ogg.json"]).unwrap();
            entry.validate(&assets[&entry.file]).unwrap();
            assert!(entry.validate(b"tampered").is_err());
            entry.data_path().unwrap();
        }
        assert!(split_cinematics(&mut data).unwrap().is_empty());
    }
}
