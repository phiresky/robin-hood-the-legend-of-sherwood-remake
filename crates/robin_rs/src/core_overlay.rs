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
#[cfg(not(target_arch = "wasm32"))]
use robin_engine::sbfile::{SBFILE_ERROR_PATH_ALREADY_PRESENT, SBFILE_NO_ERROR};
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
    "Data/Interface/UI/allied_portrait_generic.png",
    "Data/Interface/UI/allied_portrait_guisbourne.png",
    "Data/Interface/UI/allied_portrait_longchamp.png",
    "Data/Interface/UI/allied_portrait_prince_john.png",
    "Data/Interface/UI/allied_portrait_scathlock.png",
    "Data/Interface/UI/allied_portrait_sheriff.png",
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

/// Validate the canonical loose core-overlay directory used by packaged
/// desktop builds and official source projection.
///
/// Unlike an archive-backed platform, native startup can inspect the physical
/// tree. It therefore additionally requires `Data/` to contain exactly the
/// compiled inventory as regular, non-symlink files. The validated bytes are
/// returned as a bundle so callers that need a content-addressed snapshot do
/// not have to reopen the source after admission.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_validated_native_directory(root: &Path) -> Result<(CoreOverlayManifest, Bundle)> {
    require_directory(root, "core overlay root")?;

    let manifest_path = root.join(CORE_OVERLAY_MANIFEST_PATH);
    require_regular_file(&manifest_path, "core overlay manifest")?;
    let manifest_bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read core overlay manifest {}", manifest_path.display()))?;

    let actual = collect_native_data_files(root)?;
    let expected = EXPECTED_CORE_OVERLAY_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        let missing = expected.difference(&actual).collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).collect::<Vec<_>>();
        return Err(anyhow!(
            "core overlay physical inventory mismatch (missing: {missing:?}; unexpected: {unexpected:?})"
        ));
    }

    load_validated_bundle(&manifest_bytes, |path| {
        let asset_path = root.join(path);
        require_regular_file(&asset_path, "core overlay asset")?;
        std::fs::read(&asset_path)
            .with_context(|| format!("read core overlay asset {}", asset_path.display()))
    })
}

/// Validate and register the loose core overlay before native initialization
/// may consume fonts or engine UI assets. The post-mount probe proves that the
/// runtime search order selects the admitted bytes rather than a colliding
/// retail or host file.
#[cfg(not(target_arch = "wasm32"))]
pub fn mount_validated_native_directory(
    root: &Path,
    mut mount: impl FnMut(&str) -> i32,
    mut read_visible: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<CoreOverlayManifest> {
    let (manifest, _) = load_validated_native_directory(root)?;
    let root_utf8 = root
        .to_str()
        .ok_or_else(|| anyhow!("core overlay path is not UTF-8: {}", root.display()))?;
    match mount(root_utf8) {
        SBFILE_NO_ERROR | SBFILE_ERROR_PATH_ALREADY_PRESENT => {}
        status => {
            return Err(anyhow!(
                "register core overlay directory {}: SBFile error {status}",
                root.display()
            ));
        }
    }

    for entry in &manifest.files {
        let bytes = read_visible(&entry.path)
            .with_context(|| format!("probe mounted core overlay asset {}", entry.path))?;
        validate_file(entry, &bytes).with_context(|| {
            format!(
                "runtime lookup did not select the validated core overlay asset {}",
                entry.path
            )
        })?;
    }
    Ok(manifest)
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

#[cfg(not(target_arch = "wasm32"))]
fn require_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(
            "{label} must be a non-symlink directory: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn require_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!(
            "{label} must be a regular non-symlink file: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_native_data_files(root: &Path) -> Result<BTreeSet<String>> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeSet<String>) -> Result<()> {
        for entry in std::fs::read_dir(directory)
            .with_context(|| format!("enumerate core overlay {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("inspect core overlay node {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(anyhow!("core overlay contains symlink {}", path.display()));
            }
            if metadata.is_dir() {
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("core overlay traversal remains below its root")
                    .to_str()
                    .ok_or_else(|| anyhow!("core overlay path is not UTF-8: {}", path.display()))?
                    .replace('\\', "/");
                if !output.insert(relative.clone()) {
                    return Err(anyhow!("duplicate core overlay path {relative}"));
                }
            } else {
                return Err(anyhow!(
                    "core overlay contains non-regular node {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    let data = root.join("Data");
    require_directory(&data, "core overlay Data root")?;
    let mut files = BTreeSet::new();
    visit(root, &data, &mut files)?;
    Ok(files)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use robin_engine::sbfile::SbFileSystem;

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

    fn materialize_repo_overlay() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(CORE_OVERLAY_MANIFEST_PATH),
            std::fs::read(core_root().join(CORE_OVERLAY_MANIFEST_PATH)).unwrap(),
        )
        .unwrap();
        for path in EXPECTED_CORE_OVERLAY_PATHS {
            let destination = directory.path().join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(core_root().join(path), destination).unwrap();
        }
        directory
    }

    #[test]
    fn repository_inventory_is_complete_valid_and_mounts_ahead_of_shipping() {
        let (manifest_bytes, files) = load_repo_overlay();
        let (manifest, bundle) =
            load_validated_bundle(&manifest_bytes, |path| Ok(files[path].clone())).unwrap();

        let actual = collect_native_data_files(&core_root()).unwrap();
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

    #[test]
    fn native_packaged_startup_validates_then_mounts_ahead_of_primary_data() {
        let overlay = materialize_repo_overlay();
        let primary = tempfile::tempdir().unwrap();
        let colliding_path = "Data/Interface/Fonts/manager.cfg";
        let primary_asset = primary.path().join(colliding_path);
        std::fs::create_dir_all(primary_asset.parent().unwrap()).unwrap();
        std::fs::write(&primary_asset, b"retail collision").unwrap();

        let file_system = SbFileSystem::new(Arc::new(AssetVfs::new()));
        assert_eq!(
            file_system.set_primary_path(primary.path().to_str().unwrap()),
            SBFILE_NO_ERROR
        );
        let manifest = mount_validated_native_directory(
            overlay.path(),
            |path| file_system.add_overlay_path(path),
            |path| {
                file_system
                    .read_all(path)
                    .map_err(|status| anyhow!("SBFile read error {status}"))
            },
        )
        .unwrap();

        assert_eq!(manifest.files.len(), EXPECTED_CORE_OVERLAY_PATHS.len());
        assert_eq!(
            file_system.read_all(colliding_path).unwrap(),
            std::fs::read(overlay.path().join(colliding_path)).unwrap()
        );
    }

    #[test]
    fn native_packaged_startup_fails_before_mount_for_missing_corrupt_or_extra_assets() {
        fn assert_rejected_before_mount(directory: &tempfile::TempDir, expected: &str) {
            let mounted = std::cell::Cell::new(false);
            let error = mount_validated_native_directory(
                directory.path(),
                |_| {
                    mounted.set(true);
                    SBFILE_NO_ERROR
                },
                |_| panic!("visibility probe must not run before a validated mount"),
            )
            .unwrap_err();
            assert!(!mounted.get());
            assert!(
                format!("{error:#}").contains(expected),
                "unexpected error: {error:#}"
            );
        }

        let missing = materialize_repo_overlay();
        std::fs::remove_file(
            missing
                .path()
                .join("Data/Interface/UI/allied_portrait_sheriff.png"),
        )
        .unwrap();
        assert_rejected_before_mount(&missing, "physical inventory mismatch");

        let corrupt = materialize_repo_overlay();
        std::fs::write(
            corrupt.path().join("Data/Interface/Fonts/manager.cfg"),
            b"corrupt",
        )
        .unwrap();
        assert_rejected_before_mount(&corrupt, "manager.cfg");

        let extra = materialize_repo_overlay();
        std::fs::write(extra.path().join("Data/unlisted.bin"), b"unlisted").unwrap();
        assert_rejected_before_mount(&extra, "physical inventory mismatch");
    }

    #[test]
    fn native_packaged_startup_propagates_mount_failure() {
        let overlay = materialize_repo_overlay();
        let error = mount_validated_native_directory(
            overlay.path(),
            |_| -77,
            |_| panic!("visibility probe must not run after a failed mount"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("SBFile error -77"));
    }

    #[cfg(unix)]
    #[test]
    fn native_packaged_startup_rejects_symlinked_assets() {
        use std::os::unix::fs::symlink;

        let overlay = materialize_repo_overlay();
        let asset = overlay.path().join("Data/Interface/Fonts/manager.cfg");
        let replacement = overlay.path().join("replacement.cfg");
        std::fs::rename(&asset, &replacement).unwrap();
        symlink(&replacement, &asset).unwrap();

        let error = load_validated_native_directory(overlay.path()).unwrap_err();
        assert!(error.to_string().contains("symlink"));
    }
}
