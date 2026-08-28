//! Manifested engine-owned core overlay.
//!
//! Native desktop builds mount `assets/core-datadir` as an `SbFile` overlay.
//! Hosts without a native directory (Android, and potentially other packaged
//! targets) use this module to validate the identical inventory before
//! mounting it into the runtime VFS. Missing or modified engine assets are a
//! packaging error; they must never silently fall through to retail data.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use robin_assets::shipping_datadir::SHIPPING_DATADIR_VERSION;
use robin_util::asset_fs::{AssetBytes, AssetVfs, Bundle};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const CORE_OVERLAY_MANIFEST_PATH: &str = "core-overlay-manifest.json";
pub const CORE_OVERLAY_MANIFEST_SCHEMA: u32 = 1;

/// Complete engine-owned inventory. Keeping this independent of the packaged
/// JSON means deleting both a manifest entry and its file remains a hard
/// error, rather than turning an incomplete package into a self-consistent
/// one.
pub const EXPECTED_CORE_OVERLAY_PATHS: &[&str] = &[
    "Data/Interface/Fonts/Debrief.bfn",
    "Data/Interface/Fonts/EditFields.bfn",
    "Data/Interface/Fonts/InfoScroll.bfn",
    "Data/Interface/Fonts/MenuButtonDisabled.bfn",
    "Data/Interface/Fonts/MenuButtonEnabled.bfn",
    "Data/Interface/Fonts/Scroll.bfn",
    "Data/Interface/Fonts/ShortBriefingActive.bfn",
    "Data/Interface/Fonts/ShortBriefingInactive.bfn",
    "Data/Interface/Fonts/Title.bfn",
    "Data/Interface/Fonts/arial.ttf",
    "Data/Interface/Fonts/dialog.fnt",
    "Data/Interface/Fonts/manager.cfg",
    "Data/Interface/Fonts/tooltips.bfn",
    "Data/Interface/UI/allied_formation_box.png",
    "Data/Interface/UI/allied_formation_flank.png",
    "Data/Interface/UI/allied_formation_line.png",
    "Data/Interface/UI/allied_formation_staggered.png",
    "Data/Interface/UI/allied_patrol_off.png",
    "Data/Interface/UI/allied_patrol_on.png",
    "Data/Interface/UI/allied_pin_pinned.png",
    "Data/Interface/UI/allied_pin_unpinned.png",
    "Data/Interface/UI/allied_portrait_background.png",
    "Data/Interface/UI/allied_portrait_foreground.png",
    "Data/Interface/UI/allied_stance_aggressive.png",
    "Data/Interface/UI/allied_stance_defensive.png",
    "Data/Interface/UI/allied_stance_hold.png",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreOverlayManifest {
    pub schema: u32,
    pub shipping_datadir_schema: u32,
    pub files: Vec<CoreOverlayFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CoreOverlayFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Parse the canonical inventory, read every declared asset, and validate its
/// exact length and digest. The returned bundle is complete or no state is
/// changed.
pub fn load_validated_bundle(
    manifest_bytes: &[u8],
    mut read_asset: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<(CoreOverlayManifest, Bundle)> {
    let manifest: CoreOverlayManifest =
        serde_json::from_slice(manifest_bytes).context("parse core overlay manifest")?;
    validate_manifest_inventory(&manifest)?;

    let mut bundle = BTreeMap::new();
    for entry in &manifest.files {
        let bytes = read_asset(&entry.path)
            .with_context(|| format!("read required core overlay asset {}", entry.path))?;
        validate_file(entry, &bytes)?;
        let key = robin_util::asset_fs::bundle_key(Path::new(&entry.path));
        if bundle
            .insert(key.clone(), AssetBytes::from(bytes))
            .is_some()
        {
            return Err(anyhow!(
                "core overlay paths collide after VFS normalization at {key}"
            ));
        }
    }
    Ok((manifest, bundle))
}

/// Install a fully validated core overlay and prove that its required paths
/// are the bytes selected by the runtime lookup order before initialization
/// proceeds to fonts or UI construction.
pub fn install_validated_bundle(
    vfs: &AssetVfs,
    manifest: &CoreOverlayManifest,
    bundle: Bundle,
) -> Result<()> {
    vfs.mount_overlay_bundle(Arc::new(bundle))
        .context("mount core overlay bundle")?;
    for entry in &manifest.files {
        let visible = vfs
            .read_shared(&entry.path)
            .with_context(|| format!("probe mounted core overlay asset {}", entry.path))?;
        validate_file(entry, visible.as_ref()).with_context(|| {
            format!(
                "runtime VFS did not select the packaged core overlay asset {}",
                entry.path
            )
        })?;
    }
    Ok(())
}

fn validate_manifest_inventory(manifest: &CoreOverlayManifest) -> Result<()> {
    if manifest.schema != CORE_OVERLAY_MANIFEST_SCHEMA {
        return Err(anyhow!(
            "unsupported core overlay manifest schema {}; expected {}",
            manifest.schema,
            CORE_OVERLAY_MANIFEST_SCHEMA
        ));
    }
    if manifest.shipping_datadir_schema != SHIPPING_DATADIR_VERSION {
        return Err(anyhow!(
            "core overlay targets shipping datadir schema {}; runtime expects {}",
            manifest.shipping_datadir_schema,
            SHIPPING_DATADIR_VERSION
        ));
    }

    if manifest
        .files
        .windows(2)
        .any(|pair| pair[0].path >= pair[1].path)
    {
        return Err(anyhow!(
            "core overlay manifest paths must be unique and strictly sorted"
        ));
    }

    let declared: BTreeSet<&str> = manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let expected: BTreeSet<&str> = EXPECTED_CORE_OVERLAY_PATHS.iter().copied().collect();
    if declared != expected {
        let missing: Vec<_> = expected.difference(&declared).copied().collect();
        let unexpected: Vec<_> = declared.difference(&expected).copied().collect();
        return Err(anyhow!(
            "core overlay inventory mismatch (missing: {missing:?}; unexpected: {unexpected:?})"
        ));
    }
    Ok(())
}

fn validate_file(entry: &CoreOverlayFile, bytes: &[u8]) -> Result<()> {
    if entry.sha256.len() != 64
        || !entry
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(anyhow!(
            "core overlay asset {} has a non-canonical SHA-256 digest",
            entry.path
        ));
    }
    if bytes.len() as u64 != entry.size {
        return Err(anyhow!(
            "core overlay asset {} has {} bytes; manifest requires {}",
            entry.path,
            bytes.len(),
            entry.size
        ));
    }
    let digest_bytes = Sha256::digest(bytes);
    let mut digest = String::with_capacity(digest_bytes.len() * 2);
    for byte in digest_bytes {
        write!(&mut digest, "{byte:02x}").expect("writing to String cannot fail");
    }
    if digest != entry.sha256 {
        return Err(anyhow!(
            "core overlay asset {} failed SHA-256 validation: expected {}, got {}",
            entry.path,
            entry.sha256,
            digest
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core-datadir")
    }

    fn load_repo_overlay() -> (Vec<u8>, BTreeMap<String, Vec<u8>>) {
        let root = core_root();
        let manifest = std::fs::read(root.join(CORE_OVERLAY_MANIFEST_PATH)).unwrap();
        let files = EXPECTED_CORE_OVERLAY_PATHS
            .iter()
            .map(|path| ((*path).to_owned(), std::fs::read(root.join(path)).unwrap()))
            .collect();
        (manifest, files)
    }

    #[test]
    fn repository_inventory_is_complete_valid_and_mounts_ahead_of_shipping() {
        let (manifest_bytes, files) = load_repo_overlay();
        let (manifest, bundle) =
            load_validated_bundle(&manifest_bytes, |path| Ok(files[path].clone())).unwrap();

        let actual = collect_repo_data_files(&core_root().join("Data"));
        let expected: BTreeSet<String> = EXPECTED_CORE_OVERLAY_PATHS
            .iter()
            .map(|path| (*path).to_owned())
            .collect();
        assert_eq!(actual, expected);

        let vfs = AssetVfs::new();
        let mut shipping = Bundle::new();
        shipping.insert(
            robin_util::asset_fs::bundle_key(Path::new("Data/Interface/Fonts/manager.cfg")),
            AssetBytes::from(b"retail configuration".as_slice()),
        );
        vfs.mount_bundle_first(Arc::new(shipping)).unwrap();
        let mut mission = Bundle::new();
        mission.insert(
            robin_util::asset_fs::bundle_key(Path::new("Data/Interface/Fonts/manager.cfg")),
            AssetBytes::from(b"mission collision".as_slice()),
        );
        vfs.replace_active_bundle(Arc::new(mission)).unwrap();

        install_validated_bundle(&vfs, &manifest, bundle).unwrap();
        assert_eq!(
            vfs.read("Data/Interface/Fonts/manager.cfg").unwrap(),
            files["Data/Interface/Fonts/manager.cfg"]
        );
    }

    #[test]
    fn missing_declared_asset_fails_loudly() {
        let (manifest, files) = load_repo_overlay();
        let error = load_validated_bundle(&manifest, |path| {
            if path.ends_with("allied_pin_pinned.png") {
                return Err(anyhow!("simulated missing APK entry"));
            }
            Ok(files[path].clone())
        })
        .unwrap_err();
        assert!(error.to_string().contains("allied_pin_pinned.png"));
    }

    #[test]
    fn corrupt_asset_and_self_consistent_manifest_omission_are_rejected() {
        let (manifest_bytes, mut files) = load_repo_overlay();
        files
            .get_mut("Data/Interface/Fonts/manager.cfg")
            .unwrap()
            .push(0);
        let error =
            load_validated_bundle(&manifest_bytes, |path| Ok(files[path].clone())).unwrap_err();
        assert!(error.to_string().contains("manager.cfg"));

        let mut manifest: CoreOverlayManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        manifest.files.pop();
        let error = load_validated_bundle(&serde_json::to_vec(&manifest).unwrap(), |path| {
            Ok(files[path].clone())
        })
        .unwrap_err();
        assert!(error.to_string().contains("inventory mismatch"));
    }

    fn collect_repo_data_files(root: &Path) -> BTreeSet<String> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    visit(root, &entry.path(), files);
                } else {
                    files.insert(format!(
                        "Data/{}",
                        entry
                            .path()
                            .strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/")
                    ));
                }
            }
        }

        let mut files = BTreeSet::new();
        visit(root, root, &mut files);
        files
    }
}
