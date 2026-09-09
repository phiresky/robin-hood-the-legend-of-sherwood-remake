//! Fail-closed validation for private verifier authority installed on the VPS.
//!
//! Publication artifacts contain no licensed raw game data. A release carries
//! only immutable projection bundles and canonical campaign templates; Demo
//! and Full raw roots are installed independently by the operator. The worker
//! checks this split before leasing any submission.

use crate::ServerConfig;
use robin_run_protocol::{
    CanonicalDocument as _, Digest32, OfficialContentEditionV1, OfficialSourceTreeManifestV2,
    Validate as _, VerifierJobConfigCatalogV1,
};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

pub const RELEASES_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores/releases";
pub const DEMO_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/demo";
pub const FULL_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/full";
const VERIFIER_BUNDLES_RELATIVE: &str = "private/verifier-bundles";
const OPERATOR_CONFIG_RELATIVE: &str = "private/verifier/operator-config";
const CAMPAIGN_STATES_RELATIVE: &str = "private/campaign-states";
const SOURCE_TREE_MANIFESTS_RELATIVE: &str = "private/source-tree-manifests-v2";
const MAX_SOURCE_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RAW_CONTENT_INVENTORY_ENTRIES: usize = 4_000_000;

/// Validate that every runtime authority path resolves to the exact immutable
/// release selected by the catalog and that raw licensed data uses only the
/// two separately admitted operator roots.
pub fn validate_worker_authority_layout(
    catalog_path: &Path,
    catalog: &VerifierJobConfigCatalogV1,
    server: &ServerConfig,
    demo_source_manifest: &Path,
    full_source_manifest: &Path,
) -> anyhow::Result<()> {
    let authority_owner_uid = current_authority_owner_uid()?;
    let catalog_release = release_root_for_operator_catalog(catalog_path)?;
    validate_immutable_file(catalog_path, authority_owner_uid)?;

    let selected_release = catalog_release;
    for template in &catalog.entries {
        let release = release_root_for_projection_catalog(&template.content_catalog_root)?;
        validate_path_components_no_symlinks(&template.content_catalog_root)?;
        anyhow::ensure!(
            std::fs::symlink_metadata(&template.content_catalog_root)?.is_dir(),
            "projection catalog root is not an installed directory"
        );
        anyhow::ensure!(
            selected_release == release,
            "verifier catalog mixes authority from multiple releases"
        );
        let expected_raw = match template.raw_content_edition {
            OfficialContentEditionV1::Demo => Path::new(DEMO_RAW_CONTENT_ROOT),
            OfficialContentEditionV1::Full => Path::new(FULL_RAW_CONTENT_ROOT),
        };
        anyhow::ensure!(
            template.raw_content_root == expected_raw,
            "verifier catalog selects a raw content root outside its exact admitted edition root"
        );
    }
    anyhow::ensure!(
        !catalog.entries.is_empty(),
        "verifier catalog contains no release authority"
    );
    let release = selected_release;

    validate_immutable_directory(Path::new(RELEASES_ROOT), authority_owner_uid, 0o750)?;
    validate_immutable_directory(&release, authority_owner_uid, 0o550)?;
    validate_immutable_directory(&release.join("private"), authority_owner_uid, 0o550)?;

    for profile in &server.admission_profiles {
        let path = profile
            .canonical_campaign_state_path
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "installed profile {} has no canonical campaign-state path",
                    profile.id
                )
            })?;
        validate_campaign_template_path(&release, path)?;
    }

    validate_immutable_tree(
        &release.join(VERIFIER_BUNDLES_RELATIVE),
        authority_owner_uid,
    )?;
    validate_immutable_tree(&release.join(OPERATOR_CONFIG_RELATIVE), authority_owner_uid)?;
    validate_immutable_tree(&release.join(CAMPAIGN_STATES_RELATIVE), authority_owner_uid)?;
    validate_immutable_tree(
        &release.join(SOURCE_TREE_MANIFESTS_RELATIVE),
        authority_owner_uid,
    )?;

    validate_raw_content_authority(
        &release,
        OfficialContentEditionV1::Demo,
        Path::new(DEMO_RAW_CONTENT_ROOT),
        demo_source_manifest,
        authority_owner_uid,
    )?;
    validate_raw_content_authority(
        &release,
        OfficialContentEditionV1::Full,
        Path::new(FULL_RAW_CONTENT_ROOT),
        full_source_manifest,
        authority_owner_uid,
    )?;

    Ok(())
}

fn validate_raw_content_authority(
    release: &Path,
    edition: OfficialContentEditionV1,
    raw_root: &Path,
    manifest_path: &Path,
    authority_owner_uid: u32,
) -> anyhow::Result<()> {
    validate_source_manifest_path(release, manifest_path)?;
    validate_immutable_file(manifest_path, authority_owner_uid)?;
    let manifest_bytes = read_bounded_regular_file(manifest_path, MAX_SOURCE_MANIFEST_BYTES)?;
    let expected_digest = manifest_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("raw source manifest has no digest filename"))?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(&manifest_bytes)) == expected_digest,
        "raw source manifest bytes differ from their content-addressed filename"
    );
    let manifest: OfficialSourceTreeManifestV2 = serde_json::from_slice(&manifest_bytes)?;
    manifest.validate()?;
    anyhow::ensure!(
        manifest.canonical_bytes()? == manifest_bytes,
        "raw source manifest is not canonical JSON"
    );
    anyhow::ensure!(
        manifest.edition == edition,
        "raw source manifest selects the wrong official edition"
    );

    validate_immutable_directory(
        raw_root
            .parent()
            .expect("fixed raw content roots have a parent"),
        authority_owner_uid,
        0o550,
    )?;
    validate_immutable_tree(raw_root, authority_owner_uid)?;
    validate_raw_content_against_manifest(raw_root, &manifest)
}

fn validate_source_manifest_path(release: &Path, path: &Path) -> anyhow::Result<()> {
    ensure_normalized_absolute(path)?;
    let relative = path
        .strip_prefix(release.join(SOURCE_TREE_MANIFESTS_RELATIVE))
        .map_err(|_| anyhow::anyhow!("raw source manifest is outside the selected release"))?;
    anyhow::ensure!(
        relative.components().count() == 1
            && relative
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
                .is_some_and(is_digest),
        "raw source manifest must use its exact publication digest filename"
    );
    Ok(())
}

fn validate_raw_content_against_manifest(
    raw_root: &Path,
    manifest: &OfficialSourceTreeManifestV2,
) -> anyhow::Result<()> {
    manifest.validate()?;
    validate_path_components_no_symlinks(raw_root)?;
    anyhow::ensure!(
        std::fs::symlink_metadata(raw_root)?.is_dir(),
        "raw content root is not a directory"
    );

    let expected_files = manifest
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let mut expected_directories = BTreeSet::new();
    for expected in &manifest.files {
        let mut directory = String::new();
        let component_count = expected.path.split('/').count();
        for component in expected
            .path
            .split('/')
            .take(component_count.saturating_sub(1))
        {
            if !directory.is_empty() {
                directory.push('/');
            }
            directory.push_str(component);
            if expected_directories.insert(directory.clone()) {
                anyhow::ensure!(
                    expected_files
                        .len()
                        .checked_add(expected_directories.len())
                        .is_some_and(|count| count <= MAX_RAW_CONTENT_INVENTORY_ENTRIES),
                    "raw content manifest exceeds the bounded inventory size"
                );
            }
        }
    }

    let expected_entry_count = expected_files
        .len()
        .checked_add(expected_directories.len())
        .ok_or_else(|| anyhow::anyhow!("raw content inventory entry count overflow"))?;
    anyhow::ensure!(
        expected_entry_count <= MAX_RAW_CONTENT_INVENTORY_ENTRIES,
        "raw content manifest exceeds the bounded inventory size"
    );

    let mut pending = vec![raw_root.to_owned()];
    let mut actual_files = BTreeSet::new();
    let mut actual_directories = BTreeSet::new();
    let mut actual_entry_count = 0_usize;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            actual_entry_count = actual_entry_count
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("raw content inventory entry count overflow"))?;
            anyhow::ensure!(
                actual_entry_count <= expected_entry_count
                    && actual_entry_count <= MAX_RAW_CONTENT_INVENTORY_ENTRIES,
                "raw content tree contains entries outside its admitted manifest"
            );

            let path = entry.path();
            let relative = canonical_inventory_relative_path(raw_root, &path)?;
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                anyhow::ensure!(
                    expected_directories.contains(&relative),
                    "raw content tree contains an unexpected directory"
                );
                anyhow::ensure!(
                    actual_directories.insert(relative),
                    "raw content tree contains a duplicate directory"
                );
                pending.push(path);
            } else if metadata.is_file() {
                let expected = expected_files.get(relative.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("raw content tree contains an unexpected regular file")
                })?;
                validate_path_components_no_symlinks(&path)?;
                anyhow::ensure!(
                    digest_regular_file_exact(&path, expected.byte_length)? == expected.sha256,
                    "raw content file digest differs from its admitted manifest"
                );
                anyhow::ensure!(
                    actual_files.insert(relative),
                    "raw content tree contains a duplicate regular file"
                );
            } else {
                anyhow::bail!("raw content tree contains a symlink or special file");
            }
        }
    }

    anyhow::ensure!(
        actual_files.len() == expected_files.len()
            && actual_files
                .iter()
                .map(String::as_str)
                .eq(expected_files.keys().copied()),
        "raw content tree is missing a file from its admitted manifest"
    );
    anyhow::ensure!(
        actual_directories == expected_directories,
        "raw content tree directory inventory differs from its admitted manifest"
    );
    Ok(())
}

fn canonical_inventory_relative_path(raw_root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = path
        .strip_prefix(raw_root)
        .map_err(|_| anyhow::anyhow!("raw content inventory escaped its root"))?;
    let mut components = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            anyhow::bail!("raw content inventory contains a noncanonical relative path");
        };
        components.push(
            component
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("raw content inventory path is not UTF-8"))?,
        );
    }
    anyhow::ensure!(
        !components.is_empty(),
        "raw content inventory contains an empty relative path"
    );
    Ok(components.join("/"))
}

fn digest_regular_file_exact(path: &Path, expected_bytes: u64) -> anyhow::Result<Digest32> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() == expected_bytes,
        "raw content file length differs from its admitted manifest"
    );
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow::anyhow!("raw content byte count overflow"))?;
        anyhow::ensure!(
            total <= expected_bytes,
            "raw content file grew while hashing"
        );
        hasher.update(&buffer[..read]);
    }
    anyhow::ensure!(
        total == expected_bytes,
        "raw content file shrank while hashing"
    );
    Ok(Digest32::from_bytes(hasher.finalize().into()))
}

fn read_bounded_regular_file(path: &Path, maximum: u64) -> anyhow::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "authority file exceeds its exact byte bound"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= maximum,
        "authority file grew while reading"
    );
    Ok(bytes)
}

fn release_root_for_operator_catalog(path: &Path) -> anyhow::Result<PathBuf> {
    let releases = Path::new(RELEASES_ROOT);
    let relative = path
        .strip_prefix(releases)
        .map_err(|_| anyhow::anyhow!("worker job catalog is outside the immutable release root"))?;
    let mut components = relative.components();
    let commit = normal_component(components.next())
        .filter(|value| is_source_commit(value))
        .ok_or_else(|| {
            anyhow::anyhow!("worker job catalog release is not named by a source commit")
        })?;
    let release = releases.join(commit);
    let relative = path
        .strip_prefix(release.join(OPERATOR_CONFIG_RELATIVE))
        .map_err(|_| {
            anyhow::anyhow!("worker job catalog is outside its release operator-config tree")
        })?;
    anyhow::ensure!(
        relative.components().count() == 1
            && relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_digest),
        "worker job catalog must be named by its exact lowercase SHA-256"
    );
    ensure_normalized_absolute(path)?;
    Ok(release)
}

fn release_root_for_projection_catalog(path: &Path) -> anyhow::Result<PathBuf> {
    let releases = Path::new(RELEASES_ROOT);
    let relative = path
        .strip_prefix(releases)
        .map_err(|_| anyhow::anyhow!("projection catalog is outside the immutable release root"))?;
    let mut components = relative.components();
    let commit = normal_component(components.next())
        .filter(|value| is_source_commit(value))
        .ok_or_else(|| {
            anyhow::anyhow!("projection catalog release is not named by a source commit")
        })?;
    let release = releases.join(commit);
    let authority_root = release.join(VERIFIER_BUNDLES_RELATIVE);
    let bundle_relative = path.strip_prefix(&authority_root).map_err(|_| {
        anyhow::anyhow!("projection catalog is outside the release's verifier-bundle tree")
    })?;
    let mut bundle_components = bundle_relative.components();
    let digest = normal_component(bundle_components.next())
        .filter(|value| is_digest(value))
        .ok_or_else(|| anyhow::anyhow!("projection bundle is not named by its content digest"))?;
    anyhow::ensure!(
        bundle_components.next().is_some(),
        "projection catalog must select a path inside digest-named verifier bundle {digest}"
    );
    ensure_normalized_absolute(path)?;
    Ok(release)
}

fn validate_campaign_template_path(release: &Path, path: &Path) -> anyhow::Result<()> {
    ensure_normalized_absolute(path)?;
    let relative = path
        .strip_prefix(release.join(CAMPAIGN_STATES_RELATIVE))
        .map_err(|_| {
            anyhow::anyhow!("canonical campaign template is outside the selected release")
        })?;
    anyhow::ensure!(
        relative.components().count() == 1
            && relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_digest),
        "canonical campaign template must be named by its exact lowercase SHA-256"
    );
    Ok(())
}

fn normal_component(component: Option<Component<'_>>) -> Option<&str> {
    match component? {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

fn is_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn ensure_normalized_absolute(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
            && path.file_name().is_some(),
        "authority path is not normalized and absolute"
    );
    Ok(())
}

#[cfg(unix)]
fn current_authority_owner_uid() -> anyhow::Result<u32> {
    let owner_uid = rustix::process::geteuid().as_raw();
    anyhow::ensure!(
        owner_uid != 0,
        "production authority layout validation refuses to run as root"
    );
    Ok(owner_uid)
}

#[cfg(not(unix))]
fn current_authority_owner_uid() -> anyhow::Result<u32> {
    anyhow::bail!("production authority layout validation requires Unix credentials")
}

#[cfg(unix)]
fn validate_immutable_file(path: &Path, owner_uid: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    validate_path_components_no_symlinks(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file(),
        "immutable authority is not a regular file"
    );
    anyhow::ensure!(
        metadata.uid() == owner_uid,
        "immutable authority has the wrong owner"
    );
    anyhow::ensure!(metadata.nlink() == 1, "immutable authority is hard-linked");
    anyhow::ensure!(
        metadata.mode() & 0o7777 == 0o440,
        "immutable authority file mode is not 0440"
    );
    Ok(())
}

#[cfg(unix)]
fn validate_immutable_directory(path: &Path, owner_uid: u32, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    validate_path_components_no_symlinks(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir(),
        "immutable authority parent is not a directory"
    );
    anyhow::ensure!(
        metadata.uid() == owner_uid,
        "immutable authority parent has the wrong owner"
    );
    anyhow::ensure!(
        metadata.mode() & 0o7777 == mode,
        "immutable authority parent has the wrong mode"
    );
    Ok(())
}

#[cfg(not(unix))]
fn validate_immutable_directory(_path: &Path, _owner_uid: u32, _mode: u32) -> anyhow::Result<()> {
    anyhow::bail!("production authority layout validation requires Unix metadata")
}

#[cfg(not(unix))]
fn validate_immutable_file(_path: &Path, _owner_uid: u32) -> anyhow::Result<()> {
    anyhow::bail!("production authority layout validation requires Unix metadata")
}

#[cfg(unix)]
fn validate_immutable_tree(root: &Path, owner_uid: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    validate_path_components_no_symlinks(root)?;
    let mut pending = vec![root.to_owned()];
    let mut files = 0_u64;
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            metadata.uid() == owner_uid,
            "immutable authority tree has the wrong owner"
        );
        if metadata.is_dir() {
            anyhow::ensure!(
                metadata.mode() & 0o7777 == 0o550,
                "immutable authority directory mode is not 0550"
            );
            for entry in std::fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            anyhow::ensure!(
                metadata.nlink() == 1,
                "immutable authority tree contains a hard link"
            );
            anyhow::ensure!(
                metadata.mode() & 0o7777 == 0o440,
                "immutable authority tree contains a file whose mode is not 0440"
            );
            files = files
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("immutable authority file count overflow"))?;
        } else {
            anyhow::bail!("immutable authority tree contains a symlink or special file");
        }
    }
    anyhow::ensure!(files > 0, "immutable authority tree is empty");
    Ok(())
}

#[cfg(not(unix))]
fn validate_immutable_tree(_root: &Path, _owner_uid: u32) -> anyhow::Result<()> {
    anyhow::bail!("production authority layout validation requires Unix metadata")
}

fn validate_path_components_no_symlinks(path: &Path) -> anyhow::Result<()> {
    ensure_normalized_absolute(path)?;
    let mut current = PathBuf::from("/");
    for component in path.components() {
        if let Component::Normal(component) = component {
            current.push(component);
            let metadata = std::fs::symlink_metadata(&current)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "immutable authority path contains a symlink"
            );
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use robin_run_protocol::{
        OfficialProjectionSourceFormatV1, OfficialSourceClosureKindV2, OfficialSourceFileV1,
    };
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

    fn make_tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("authority");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("artifact");
        std::fs::write(&file, b"authority").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o440)).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o550)).unwrap();
        (temporary, root, file)
    }

    #[test]
    fn immutable_tree_rejects_mode_symlink_and_hardlink_substitution() {
        let (_temporary, root, file) = make_tree();
        let owner = std::fs::metadata(&root).unwrap().uid();
        validate_immutable_tree(&root, owner).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(validate_immutable_tree(&root, owner).is_err());

        let (_temporary, root, file) = make_tree();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::hard_link(&file, root.join("alias")).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o550)).unwrap();
        assert!(validate_immutable_tree(&root, owner).is_err());

        let (_temporary, root, file) = make_tree();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o750)).unwrap();
        symlink(&file, root.join("alias")).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o550)).unwrap();
        assert!(validate_immutable_tree(&root, owner).is_err());
    }

    #[test]
    fn release_paths_bind_one_commit_projection_bundle_and_campaign_tree() {
        let commit = "a".repeat(40);
        let digest = "b".repeat(64);
        let release = Path::new(RELEASES_ROOT).join(&commit);
        let catalog = release
            .join(VERIFIER_BUNDLES_RELATIVE)
            .join(&digest)
            .join("catalog/mission");
        assert_eq!(
            release_root_for_projection_catalog(&catalog).unwrap(),
            release
        );
        assert!(
            release_root_for_projection_catalog(
                &Path::new(RELEASES_ROOT)
                    .join(&commit)
                    .join("private/elsewhere/catalog")
            )
            .is_err()
        );
        assert!(
            release_root_for_projection_catalog(
                &Path::new(RELEASES_ROOT)
                    .join("current")
                    .join(VERIFIER_BUNDLES_RELATIVE)
                    .join(&digest)
                    .join("catalog")
            )
            .is_err()
        );

        let campaign = release.join(CAMPAIGN_STATES_RELATIVE).join(&digest);
        validate_campaign_template_path(&release, &campaign).unwrap();
        assert!(
            validate_campaign_template_path(
                &release,
                &Path::new("/home/robinhood/.local/share/robin-highscores/campaign-states")
                    .join(&digest)
            )
            .is_err()
        );

        let operator_catalog = release.join(OPERATOR_CONFIG_RELATIVE).join(&digest);
        assert_eq!(
            release_root_for_operator_catalog(&operator_catalog).unwrap(),
            release
        );
        assert!(
            release_root_for_operator_catalog(
                &Path::new("/etc/robin-highscores/verifier/job-config-catalog").join(&digest)
            )
            .is_err()
        );

        let source_manifest = release
            .join(SOURCE_TREE_MANIFESTS_RELATIVE)
            .join(format!("{digest}.json"));
        validate_source_manifest_path(&release, &source_manifest).unwrap();
        assert!(
            validate_source_manifest_path(
                &release,
                &Path::new("/etc/robin-highscores").join(format!("{digest}.json"))
            )
            .is_err()
        );
    }

    #[test]
    fn raw_roots_are_exact_edition_specific_operator_paths() {
        assert_eq!(
            Path::new(DEMO_RAW_CONTENT_ROOT),
            Path::new("/home/robinhood/.local/share/robin-highscores/raw-content/demo")
        );
        assert_ne!(
            Path::new(DEMO_RAW_CONTENT_ROOT),
            Path::new(FULL_RAW_CONTENT_ROOT)
        );
    }

    #[test]
    fn raw_root_bytes_must_match_the_selected_source_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let raw_root = temporary.path().join("demo");
        let file = raw_root.join("Data/mission.bin");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"exact raw input").unwrap();
        let manifest = raw_manifest();
        manifest.validate().unwrap();
        validate_raw_content_against_manifest(&raw_root, &manifest).unwrap();

        std::fs::write(&file, b"wrong raw input").unwrap();
        assert!(validate_raw_content_against_manifest(&raw_root, &manifest).is_err());
    }

    #[test]
    fn raw_root_rejects_a_regular_file_outside_the_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let raw_root = create_exact_raw_root(&temporary);
        std::fs::write(raw_root.join("Data/extra.bin"), b"not admitted").unwrap();

        assert!(validate_raw_content_against_manifest(&raw_root, &raw_manifest()).is_err());
    }

    #[test]
    fn raw_root_rejects_an_extra_empty_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let raw_root = create_exact_raw_root(&temporary);
        std::fs::create_dir(raw_root.join("Data/empty")).unwrap();

        assert!(validate_raw_content_against_manifest(&raw_root, &raw_manifest()).is_err());
    }

    #[test]
    fn raw_root_rejects_a_manifest_file_that_is_missing() {
        let temporary = tempfile::tempdir().unwrap();
        let raw_root = temporary.path().join("demo");
        std::fs::create_dir(&raw_root).unwrap();

        assert!(validate_raw_content_against_manifest(&raw_root, &raw_manifest()).is_err());
    }

    fn create_exact_raw_root(temporary: &tempfile::TempDir) -> PathBuf {
        let raw_root = temporary.path().join("demo");
        let file = raw_root.join("Data/mission.bin");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"exact raw input").unwrap();
        raw_root
    }

    fn raw_manifest() -> OfficialSourceTreeManifestV2 {
        OfficialSourceTreeManifestV2 {
            schema_version: 2,
            edition: OfficialContentEditionV1::Demo,
            source_format: OfficialProjectionSourceFormatV1::LooseNativeV1,
            closure_kind: OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1,
            files: vec![OfficialSourceFileV1 {
                path: "Data/mission.bin".into(),
                sha256: Digest32::digest_bytes(b"exact raw input"),
                byte_length: 15,
            }],
        }
    }
}
