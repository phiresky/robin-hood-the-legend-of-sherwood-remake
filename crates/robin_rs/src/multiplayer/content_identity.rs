//! Exact native-to-browser content closure identity.
//!
//! Browser artifacts are engine-commit bound, but retail/demo data is owned
//! separately. A browser invitation therefore carries a SHA-256 identity of
//! the host's complete primary `Data/` tree plus the locale trees selected by
//! the native loader. The web converter records that same identity in its
//! canonical Full-package manifest.

use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use sha2::{Digest as _, Sha256};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
const CONTENT_IDENTITY_DOMAIN: &[u8] = b"robinhood/native-content-closure/v1\0";
pub const WEB_CONTENT_MANIFEST_NAME: &str = "robinhood-web-content.json";
pub const WEB_CONTENT_MANIFEST_SCHEMA: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebContentFile {
    pub path: String,
    pub kind: WebContentFileKind,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebContentFileKind {
    Shipping,
    Asset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebContentDatadir {
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebContentManifest {
    pub schema: u32,
    pub edition: String,
    pub engine_version: String,
    pub native_content_sha256: String,
    pub datadir: WebContentDatadir,
    pub files: Vec<WebContentFile>,
}

/// Compute the exact content identity from an original installation's
/// primary `Data/` directory. Locale discovery intentionally mirrors
/// `main_entry::add_language_folder` and the web converter.
#[cfg(not(target_arch = "wasm32"))]
pub fn source_content_identity(data_dir: &Path) -> Result<String, String> {
    let data_dir = fs::canonicalize(data_dir).map_err(|error| {
        format!(
            "resolve content Data directory {}: {error}",
            data_dir.display()
        )
    })?;
    if !data_dir.is_dir() {
        return Err(format!(
            "content Data path is not a directory: {}",
            data_dir.display()
        ));
    }
    let install_root = data_dir
        .parent()
        .ok_or_else(|| {
            format!(
                "content Data directory has no install root: {}",
                data_dir.display()
            )
        })?
        .to_path_buf();
    let mut roots = vec![("primary".to_string(), data_dir)];
    if let Some(locale) = locale_data_dir(&install_root, crate::main_entry::FALLBACK_LOCALE_FOLDER)
    {
        roots.push((
            format!("locale:{}", crate::main_entry::FALLBACK_LOCALE_FOLDER),
            locale,
        ));
    }
    for &folder in crate::main_entry::LANGUAGE_FOLDERS {
        if let Some(locale) = locale_data_dir(&install_root, folder) {
            roots.push((format!("locale:{folder}"), locale));
            break;
        }
    }
    hash_roots(&roots)
}

/// Resolve and hash the content closure used by the active native host.
/// Converted shipping packages are verified against their exact manifest;
/// loose installations are walked directly. Modified overlays fail loudly
/// because they are not part of the selected browser package.
#[cfg(not(target_arch = "wasm32"))]
pub fn active_content_identity() -> Result<String, String> {
    if robin_engine::sbfile::SbFile::has_zip_overlays() {
        return Err(
            "browser invitations require an unmodified content closure; a ZIP overlay is active"
                .to_string(),
        );
    }
    for overlay in robin_engine::sbfile::SbFile::overlay_paths() {
        let normalized = overlay
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        if !normalized.ends_with("/assets/core-datadir") && normalized != "assets/core-datadir" {
            return Err(format!(
                "browser invitations require an unmodified content closure; unsupported overlay is active: {overlay}"
            ));
        }
    }

    if let Some(manifest_path) =
        robin_engine::sbfile::resolve_data_path(&format!("Data/{WEB_CONTENT_MANIFEST_NAME}"))
    {
        return verify_web_content_package(&manifest_path);
    }
    let marker = robin_engine::sbfile::resolve_data_path("Data/robinhood.bks")
        .or_else(|| robin_engine::sbfile::resolve_data_path("Data/datadir.bin"))
        .ok_or_else(|| {
            "cannot identify native content closure: neither Data/robinhood.bks nor Data/datadir.bin is available"
                .to_string()
        })?;
    let data_dir = marker.parent().ok_or_else(|| {
        format!(
            "native content marker has no Data directory: {}",
            marker.display()
        )
    })?;
    if marker.file_name().is_some_and(|name| name == "datadir.bin") {
        return Err(format!(
            "shipping host content at {} has no {WEB_CONTENT_MANIFEST_NAME}; browser invitation publication is unsafe",
            data_dir.display()
        ));
    }
    source_content_identity(data_dir)
}

#[cfg(not(target_arch = "wasm32"))]
fn locale_data_dir(install_root: &Path, locale: &str) -> Option<PathBuf> {
    let locale_root = robin_engine::sbfile::resolve_case_insensitive(&install_root.join(locale))?;
    if !locale_root.is_dir() {
        return None;
    }
    robin_engine::sbfile::resolve_case_insensitive(&locale_root.join("Data"))
        .filter(|candidate| candidate.is_dir())
}

#[cfg(not(target_arch = "wasm32"))]
fn hash_roots(roots: &[(String, PathBuf)]) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(CONTENT_IDENTITY_DOMAIN);
    for (label, root) in roots {
        let files = collect_regular_files(root)?;
        hasher.update((label.len() as u64).to_le_bytes());
        hasher.update(label.as_bytes());
        hasher.update((files.len() as u64).to_le_bytes());
        for (relative, path) in files {
            hasher.update((relative.len() as u64).to_le_bytes());
            hasher.update(relative.as_bytes());
            let mut file = fs::File::open(&path)
                .map_err(|error| format!("open content file {}: {error}", path.display()))?;
            let length = file
                .metadata()
                .map_err(|error| format!("stat content file {}: {error}", path.display()))?
                .len();
            hasher.update(length.to_le_bytes());
            let mut buffer = [0_u8; 128 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| format!("read content file {}: {error}", path.display()))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        }
    }
    Ok(hex_digest(hasher.finalize().into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_regular_files(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!(
                "enumerate content directory {}: {error}",
                directory.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "enumerate content directory {}: {error}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("stat content path {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "content identity refuses symlinked path {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(format!(
                    "content identity refuses non-file path {}",
                    path.display()
                ));
            }
            let relative = path
                .strip_prefix(root)
                .expect("walked path must remain under content root")
                .to_str()
                .ok_or_else(|| format!("content path is not UTF-8: {}", path.display()))?
                .replace('\\', "/")
                .to_ascii_lowercase();
            if relative.is_empty()
                || relative
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(format!("content path is not canonical: {}", path.display()));
            }
            if let Some(previous) = files.insert(relative.clone(), path.clone()) {
                return Err(format!(
                    "content paths collide case-insensitively at {relative}: {} and {}",
                    previous.display(),
                    path.display()
                ));
            }
        }
    }
    Ok(files)
}

#[cfg(not(target_arch = "wasm32"))]
fn verify_web_content_package(manifest_path: &Path) -> Result<String, String> {
    let bytes = fs::read(manifest_path).map_err(|error| {
        format!(
            "read web content manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest: WebContentManifest = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse web content manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let canonical = serde_json::to_vec(&manifest)
        .map_err(|error| format!("serialize web content manifest: {error}"))?;
    if canonical != bytes {
        return Err("web content manifest is not canonical JSON".to_string());
    }
    validate_sha256(&manifest.native_content_sha256, "native content identity")?;
    if manifest.schema != WEB_CONTENT_MANIFEST_SCHEMA || manifest.edition != "full" {
        return Err("web content manifest has unsupported schema or edition".to_string());
    }
    if manifest.engine_version != crate::replay_format::ENGINE_SOURCE_COMMIT {
        return Err("web content manifest was built for a different engine commit".to_string());
    }
    let root = manifest_path
        .parent()
        .ok_or_else(|| "web content manifest has no package root".to_string())?;
    let actual = collect_regular_files(root)?;
    let mut expected = BTreeSet::from([WEB_CONTENT_MANIFEST_NAME.to_string()]);
    verify_manifest_file(
        root,
        &manifest.datadir.path,
        manifest.datadir.byte_length,
        &manifest.datadir.sha256,
    )?;
    expected.insert(manifest.datadir.path.to_ascii_lowercase());
    for file in &manifest.files {
        verify_manifest_file(root, &file.path, file.byte_length, &file.sha256)?;
        if !expected.insert(file.path.to_ascii_lowercase()) {
            return Err(format!("web content manifest repeats {}", file.path));
        }
    }
    let actual_paths: BTreeSet<String> = actual.keys().cloned().collect();
    if actual_paths != expected {
        return Err("web content package has missing or unexpected files".to_string());
    }
    Ok(manifest.native_content_sha256)
}

#[cfg(not(target_arch = "wasm32"))]
fn verify_manifest_file(
    root: &Path,
    relative: &str,
    length: u64,
    digest: &str,
) -> Result<(), String> {
    validate_relative_path(relative)?;
    validate_sha256(digest, "web content file digest")?;
    let path = root.join(relative);
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("stat web content file {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() != length {
        return Err(format!(
            "web content file has wrong type or length: {relative}"
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("read web content file {}: {error}", path.display()))?;
    let actual = hex_digest(Sha256::digest(bytes).into());
    if actual != digest {
        return Err(format!("web content file has wrong digest: {relative}"));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(format!("web content path is not canonical: {path}"));
    }
    Ok(())
}

pub fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(format!("{label} must be one lowercase SHA-256 digest"));
    }
    Ok(())
}

pub fn hex_digest(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{
        WEB_CONTENT_MANIFEST_NAME, WEB_CONTENT_MANIFEST_SCHEMA, WebContentDatadir, WebContentFile,
        WebContentFileKind, WebContentManifest, hex_digest, source_content_identity,
        verify_web_content_package,
    };
    use sha2::{Digest as _, Sha256};

    #[test]
    fn source_identity_is_path_content_and_locale_bound() {
        let root = tempfile::tempdir().expect("temp root");
        let data = root.path().join("Data");
        std::fs::create_dir(&data).expect("Data");
        std::fs::write(data.join("RobinHood.bks"), b"one").expect("primary");
        let first = source_content_identity(&data).expect("first identity");
        std::fs::write(data.join("RobinHood.bks"), b"two").expect("changed primary");
        let second = source_content_identity(&data).expect("second identity");
        assert_ne!(first, second);

        let locale = root.path().join("1033/Data");
        std::fs::create_dir_all(&locale).expect("locale");
        std::fs::write(locale.join("Level.res"), b"localized").expect("localized file");
        let localized = source_content_identity(&data).expect("localized identity");
        assert_ne!(second, localized);
    }

    #[test]
    fn package_manifest_verifies_every_declared_byte_and_no_extras() {
        let root = tempfile::tempdir().expect("package root");
        std::fs::write(root.path().join("datadir.bin"), b"root").expect("datadir");
        std::fs::create_dir(root.path().join("missions")).expect("missions");
        std::fs::write(root.path().join("missions/one.rhmission.zst"), b"mission")
            .expect("mission");
        let digest = |bytes: &[u8]| hex_digest(Sha256::digest(bytes).into());
        let identity = "01".repeat(32);
        let manifest = WebContentManifest {
            schema: WEB_CONTENT_MANIFEST_SCHEMA,
            edition: "full".to_string(),
            engine_version: crate::replay_format::ENGINE_SOURCE_COMMIT.to_string(),
            native_content_sha256: identity.clone(),
            datadir: WebContentDatadir {
                path: "datadir.bin".to_string(),
                byte_length: 4,
                sha256: digest(b"root"),
            },
            files: vec![WebContentFile {
                path: "missions/one.rhmission.zst".to_string(),
                kind: WebContentFileKind::Shipping,
                byte_length: 7,
                sha256: digest(b"mission"),
            }],
        };
        let manifest_path = root.path().join(WEB_CONTENT_MANIFEST_NAME);
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("manifest JSON"),
        )
        .expect("manifest");
        assert_eq!(
            verify_web_content_package(&manifest_path).expect("verified package"),
            identity
        );

        std::fs::write(root.path().join("missions/one.rhmission.zst"), b"changed")
            .expect("tamper mission");
        assert!(
            verify_web_content_package(&manifest_path)
                .unwrap_err()
                .contains("wrong digest")
        );
    }
}
