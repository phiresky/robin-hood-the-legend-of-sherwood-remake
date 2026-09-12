//! bundle responsibilities of the admitted release pipeline.
use super::*;

/// Assemble one immutable, absent-output-only VPS release bundle.
pub fn assemble_vps_release_v2(plan_path: &Path, output: &Path) -> Result<Digest32> {
    ensure_absent_output(output)?;
    ensure!(output.is_absolute(), "VPS release output must be absolute");
    ensure!(
        normalized_absolute(output),
        "VPS release output is not normalized"
    );
    reject_hardlink(plan_path)?;
    let plan = VpsReleasePlanV2::load(plan_path)?;
    validate_vps_release_assembly_output(output, &plan.source_commit)?;
    let mut publication =
        crate::publication_v3::validate_publication_v3_authority(&plan.publication_v3)?;
    let publication_lock_sha256 = publication.lock_sha256();
    let publication_manifest: PublicationManifestV3 =
        publication.load_document("publication-manifest-v3.json")?;
    let publication_manifest_sha256 = publication_manifest.canonical_digest()?;
    let backend: BackendPublicationV3 = publication.load_document("backend/publication-v3.json")?;
    validate_assembly_inputs(
        &plan,
        output,
        backend.verifier_program.sha256,
        &backend.verifier_operator_config,
        &backend
            .campaign_states
            .iter()
            .map(|state| state.artifact.sha256)
            .collect(),
    )?;
    let build_path = format!(
        "backend/manifests/builds/{}.json",
        backend.build_manifest_sha256
    );
    let build: BuildManifestV2 = publication.load_document(&build_path)?;
    ensure!(
        build.source_commit == plan.source_commit,
        "release source commit differs from the admitted BuildManifestV2"
    );
    let verifier = plan
        .binaries
        .iter()
        .find(|binary| binary.role == VpsBinaryRoleV2::ReplayVerifier)
        .context("release plan omits replay verifier")?;
    ensure!(
        verifier.artifact == backend.verifier_program,
        "release verifier differs from publication-v3"
    );
    let catalog_path = format!(
        "private/verifier/operator-config/{}",
        backend.verifier_operator_config.sha256
    );
    let _: VerifierJobConfigCatalogV1 = publication.load_document(&catalog_path)?;
    ensure!(
        publication.artifact(&catalog_path, &backend.verifier_operator_config.media_type)?
            == backend.verifier_operator_config,
        "publication job catalog differs from its pin"
    );
    publication.ensure_live()?;

    let staging = staging_directory(output)?;
    let assembled = (|| {
        materialize_bundle(staging.path(), &plan, &mut publication)?;
        publication.ensure_live()?;
        let files = payload_inventory(staging.path())?;
        let manifest = VpsReleaseManifestV2 {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source_commit: plan.source_commit.clone(),
            database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
            deployment: canonical_user_deployment(),
            publication_lock_sha256,
            publication_manifest_sha256,
            verifier_sha256: verifier.artifact.sha256,
            files,
        };
        manifest.validate_current_candidate()?;
        write_bytes(
            &staging.path().join(RELEASE_MANIFEST_FILE),
            &canonical_json_bytes(&manifest)?,
        )?;
        write_bytes(
            &staging.path().join(SOURCE_COMMIT_FILE),
            format!("{}\n", plan.source_commit).as_bytes(),
        )?;
        write_mode_inventory(staging.path())?;
        write_sha256sums(staging.path())?;
        make_bundle_read_only(staging.path())?;
        validate_vps_release_root(staging.path(), false)
    })();
    match assembled {
        Ok(manifest_sha256) => match persist_vps_staging(&staging, output) {
            Ok(VpsPersistenceOutcome::Installed) => {
                let _installed_path = staging.keep();
                Ok(manifest_sha256)
            }
            Ok(VpsPersistenceOutcome::InstalledButParentSyncFailed(sync_error)) => {
                let _installed_path = staging.keep();
                Err(vps_installed_durability_error(
                    output,
                    manifest_sha256,
                    &plan.source_commit,
                    sync_error,
                ))
            }
            Err(persist_error) => match discard_failed_vps_staging(staging) {
                Ok(()) => Err(persist_error),
                Err(cleanup_error) => Err(persist_error.context(format!(
                    "VPS persistence also failed to securely remove staging: {cleanup_error:#}"
                ))),
            },
        },
        Err(assembly_error) => match discard_failed_vps_staging(staging) {
            Ok(()) => Err(assembly_error),
            Err(cleanup_error) => Err(assembly_error.context(format!(
                "VPS assembly also failed to securely remove staging: {cleanup_error:#}"
            ))),
        },
    }
}

pub(super) fn validate_vps_release_assembly_output(
    output: &Path,
    source_commit: &str,
) -> Result<()> {
    ensure!(
        output
            == Path::new(INSTALL_ROOT)
                .join("releases")
                .join(format!("{source_commit}.partial")),
        "VPS release assembly output must be exact INSTALL_ROOT/releases/SOURCE_COMMIT.partial"
    );
    Ok(())
}

/// Validate one release solely from its canonical, self-contained evidence.
pub fn validate_vps_release_v2(root: &Path) -> Result<Digest32> {
    validate_vps_release_root(root, true)
}

/// Validate the exact inode named by `partial` through a pinned directory
/// descriptor and atomically promote it to `output` without replacement.
///
/// This is deliberately Linux-only. Deployment must not validate one pathname
/// and later rename whatever a same-UID process substituted at that pathname.
pub fn promote_vps_release_v2(
    partial: &Path,
    output: &Path,
    expected_sha256sums_sha256: &str,
) -> Result<Digest32> {
    {
        ensure!(
            normalized_absolute(partial) && normalized_absolute(output),
            "VPS promotion paths must be normalized absolute paths"
        );
        let release_parent = Path::new(INSTALL_ROOT).join("releases");
        ensure!(
            partial.parent() == Some(release_parent.as_path())
                && output.parent() == Some(release_parent.as_path()),
            "VPS promotion must stay in the exact releases directory"
        );
        let partial_name = partial
            .file_name()
            .context("VPS partial path has no basename")?;
        let output_name = output
            .file_name()
            .context("VPS output path has no basename")?;
        let partial_name = partial_name
            .to_str()
            .context("VPS partial basename is not UTF-8")?;
        let output_name = output_name
            .to_str()
            .context("VPS output basename is not UTF-8")?;
        let source_commit = partial_name
            .strip_suffix(".partial")
            .context("VPS partial basename lacks the exact .partial suffix")?;
        ensure!(
            valid_source_commit(source_commit) && output_name == source_commit,
            "VPS partial and output names do not bind one exact source commit"
        );
        let expected_sha256sums_sha256 = expected_sha256sums_sha256
            .parse::<Digest32>()
            .context("expected SHA256SUMS digest is not canonical lowercase hexadecimal")?;
        promote_pinned_vps_release_with(
            partial,
            output,
            &release_parent,
            source_commit,
            expected_sha256sums_sha256,
            validate_pinned_current_vps_release_root,
        )
    }
}

pub(super) fn promote_pinned_vps_release_with<F>(
    partial: &Path,
    output: &Path,
    release_parent: &Path,
    source_commit: &str,
    expected_sha256sums_sha256: Digest32,
    validate_pinned: F,
) -> Result<Digest32>
where
    F: FnOnce(&Path) -> Result<Digest32>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with};
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    ensure!(
        fs::canonicalize(release_parent)? == release_parent,
        "VPS releases parent is not canonical"
    );
    let parent_fd = openat2(
        rustix::fs::CWD,
        release_parent,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let parent_metadata = fs::metadata(release_parent)?;
    let pinned_parent = rustix::fs::fstat(&parent_fd)?;
    ensure!(
        parent_metadata.dev() == pinned_parent.st_dev
            && parent_metadata.ino() == pinned_parent.st_ino
            && parent_metadata.uid() == rustix::process::geteuid().as_raw()
            && parent_metadata.permissions().mode() & 0o777 == 0o750,
        "VPS releases parent identity, owner, or mode is unsafe"
    );

    let partial_name = partial
        .file_name()
        .context("VPS partial path has no basename")?;
    let output_name = output
        .file_name()
        .context("VPS output path has no basename")?;
    let partial_fd = openat2(
        parent_fd.as_fd(),
        partial_name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_partial = rustix::fs::fstat(&partial_fd)?;
    ensure!(
        pinned_partial.st_uid == rustix::process::geteuid().as_raw()
            && pinned_partial.st_dev == pinned_parent.st_dev
            && pinned_partial.st_mode & 0o777 == 0o550,
        "VPS partial root identity, owner, device, or mode is unsafe"
    );
    // The trailing `/.` makes the final component the pinned directory,
    // rather than the procfs magic-link entry itself.
    let pinned_path = PathBuf::from(format!("/proc/self/fd/{}/.", partial_fd.as_raw_fd()));
    let manifest_sha256 = validate_pinned(&pinned_path)?;
    ensure!(
        fs::read(pinned_path.join(SOURCE_COMMIT_FILE))? == format!("{source_commit}\n").as_bytes(),
        "pinned VPS partial source commit differs from its basename"
    );
    ensure!(
        Digest32::digest_bytes(&fs::read(pinned_path.join(SHA256SUMS_FILE))?)
            == expected_sha256sums_sha256,
        "pinned VPS partial SHA256SUMS differs from the out-of-band digest"
    );

    let observed_parent = fs::metadata(release_parent)?;
    let observed_partial = fs::symlink_metadata(partial)?;
    ensure!(
        observed_parent.dev() == pinned_parent.st_dev
            && observed_parent.ino() == pinned_parent.st_ino
            && observed_partial.dev() == pinned_partial.st_dev
            && observed_partial.ino() == pinned_partial.st_ino,
        "VPS releases parent or validated partial was substituted"
    );
    renameat_with(
        parent_fd.as_fd(),
        partial_name,
        parent_fd.as_fd(),
        output_name,
        RenameFlags::NOREPLACE,
    )
    .context("atomically promote pinned VPS release without replacement")?;
    Ok(manifest_sha256)
}

pub(super) fn validate_vps_release_root(
    root: &Path,
    enforce_directory_name: bool,
) -> Result<Digest32> {
    validate_mount_root(root)?;
    validate_vps_release_contents(root, enforce_directory_name)
}

pub(super) fn validate_pinned_current_vps_release_root(root: &Path) -> Result<Digest32> {
    let metadata = fs::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir(),
        "pinned VPS release root is not a directory"
    );
    let manifest_sha256 = validate_vps_release_contents(root, false)?;
    let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
    manifest.validate_current_candidate()?;
    Ok(manifest_sha256)
}

pub(super) fn validate_vps_release_contents(
    root: &Path,
    enforce_directory_name: bool,
) -> Result<Digest32> {
    reject_mounts_strictly_below(root)?;

    reject_links_and_special_nodes(root, true)?;
    validate_single_owner_tree(root)?;
    validate_single_device_tree(root)?;
    let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
    if enforce_directory_name {
        ensure!(
            root.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| valid_release_directory_name(name, &manifest.source_commit)),
            "VPS release directory is neither SOURCE_COMMIT nor its exact .partial candidate"
        );
        if root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| candidate_release_directory_name(name, &manifest.source_commit))
        {
            manifest.validate_current_candidate()?;
        }
    }
    ensure!(
        fs::read(root.join(SOURCE_COMMIT_FILE))?
            == format!("{}\n", manifest.source_commit).as_bytes(),
        "SOURCE_COMMIT differs from release manifest"
    );
    ensure!(
        payload_inventory(root)? == manifest.files,
        "VPS payload inventory differs from its canonical manifest"
    );
    ensure!(
        fs::read(root.join(MODE_INVENTORY_FILE))? == expected_mode_inventory(root)?,
        "MODE_INVENTORY is missing, reordered, or substituted"
    );
    ensure!(
        fs::read(root.join(SHA256SUMS_FILE))? == expected_sha256sums(root)?,
        "SHA256SUMS is missing, reordered, or substituted"
    );
    validate_bundle_shape(root, &manifest)?;
    validate_embedded_publication(root, &manifest)?;
    Ok(manifest.canonical_digest()?)
}

pub(super) fn materialize_bundle(
    root: &Path,
    plan: &VpsReleasePlanV2,
    publication: &mut ValidatedPublicationV3,
) -> Result<()> {
    for binary in &plan.binaries {
        copy_exact(
            &binary.source,
            &root.join("bin").join(binary.role.output_name()),
            &binary.artifact,
        )?;
    }
    for config in &plan.configs {
        copy_exact(
            &config.source,
            &root.join("config").join(config.role.output_name()),
            &config.artifact,
        )?;
    }
    for file in &plan.host_files {
        copy_exact(
            &file.source,
            &root.join(file.role.output_path()),
            &file.artifact,
        )?;
    }
    write_bytes(
        &root.join(ROOT_ONCE_SHA256SUMS_FILE),
        &expected_root_once_sha256sums(root)?,
    )?;
    write_bytes(
        &root.join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
        &expected_deploy_bootstrap_sha256sums(root)?,
    )?;

    copy_publication_tree_exact(
        publication,
        "backend/manifests",
        &root.join("config/manifests"),
    )?;
    for (source, destination) in [
        (
            "private/official-content-authority/verifier-bundles",
            "private/verifier-bundles",
        ),
        ("private/campaign-states", "private/campaign-states"),
        (
            "private/verifier/operator-config",
            "private/verifier/operator-config",
        ),
        (
            "private/official-content-authority/private/source-tree-manifests-v2",
            "private/source-tree-manifests-v2",
        ),
    ] {
        copy_publication_tree_exact(publication, source, &root.join(destination))?;
    }
    for (source, destination) in [
        (
            "backend/publication-v3.json",
            "publication/backend-publication-v3.json",
        ),
        (
            "publication-manifest-v3.json",
            "publication/publication-manifest-v3.json",
        ),
        (
            "publication-manifest-v3.sha256",
            "publication/publication-manifest-v3.sha256",
        ),
        (
            "publication-lock-v3.json",
            "publication/publication-lock-v3.json",
        ),
        (
            "publication-lock-v3.sha256",
            "publication/publication-lock-v3.sha256",
        ),
    ] {
        copy_publication_file_exact(publication, source, &root.join(destination))?;
    }
    publication.ensure_live()?;
    let declarations = PrivateRawRootDeclarationsV2 {
        schema_version: RAW_ROOTS_SCHEMA_VERSION,
        roots: plan.private_raw_roots.clone(),
    };
    declarations.validate()?;
    write_bytes(
        &root.join(RAW_ROOT_DECLARATIONS_FILE),
        &canonical_json_bytes(&declarations)?,
    )?;
    Ok(())
}

pub(super) fn copy_publication_tree_exact(
    publication: &mut ValidatedPublicationV3,
    source_prefix: &str,
    destination: &Path,
) -> Result<()> {
    let directories = publication.relative_directories(source_prefix);
    ensure!(
        directories.first().is_some_and(|path| path == "."),
        "validated PublicationV3 omits source directory {source_prefix}"
    );
    ensure!(!destination.exists(), "bundle destination already exists");
    fs::create_dir_all(destination)?;
    for relative in directories.into_iter().filter(|path| path != ".") {
        fs::create_dir(destination.join(relative))?;
    }
    for relative in publication.relative_files(source_prefix) {
        copy_publication_file_exact(
            publication,
            &format!("{source_prefix}/{relative}"),
            &destination.join(relative),
        )?;
    }
    publication.ensure_live()
}

pub(super) fn copy_publication_file_exact(
    publication: &mut ValidatedPublicationV3,
    source: &str,
    destination: &Path,
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = publication.copy_file_to(source, &mut output)?;
    ensure!(
        artifact_from_file(destination, &copied.media_type)? == copied,
        "VPS bundle copy changed retained PublicationV3 source {source}"
    );
    Ok(())
}

pub(super) fn copy_exact(
    source: &Path,
    destination: &Path,
    expected: &ArtifactRefV1,
) -> Result<()> {
    validate_pinned_file(source, expected)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut reader = BufReader::new(File::open(source)?);
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?,
    );
    std::io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    ensure!(
        artifact_from_file(destination, &expected.media_type)? == *expected,
        "bundle copy changed {}",
        destination.display()
    );
    Ok(())
}

pub(super) fn payload_inventory(root: &Path) -> Result<Vec<VpsReleaseFileV2>> {
    let mut files = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, absolute)| Ok((path_to_manifest(&relative)?, absolute)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|(path, _)| {
            !matches!(
                path.as_str(),
                SOURCE_COMMIT_FILE | SHA256SUMS_FILE | MODE_INVENTORY_FILE | RELEASE_MANIFEST_FILE
            )
        })
        .map(|(path, absolute)| {
            Ok(VpsReleaseFileV2 {
                unix_mode: canonical_file_mode(&path),
                path,
                artifact: artifact_from_file(&absolute, "application/octet-stream")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // `PathBuf::Ord` compares path components, while the canonical manifest
    // contract compares the serialized UTF-8 paths. Those orders differ for
    // prefix siblings such as `private/verifier/` and
    // `private/verifier-bundles/` (`'-' < '/'` in the manifest strings).
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

pub(super) fn write_mode_inventory(root: &Path) -> Result<()> {
    let bytes = expected_mode_inventory_with_future_metadata(root)?;
    write_bytes(&root.join(MODE_INVENTORY_FILE), &bytes)
}

pub(super) fn expected_mode_inventory_with_future_metadata(root: &Path) -> Result<Vec<u8>> {
    let mut entries = tree_modes(root, true)?;
    entries.insert(MODE_INVENTORY_FILE.into(), ('f', 0o440));
    entries.insert(SHA256SUMS_FILE.into(), ('f', 0o440));
    mode_inventory_bytes(&entries)
}

pub(super) fn expected_mode_inventory(root: &Path) -> Result<Vec<u8>> {
    let entries = tree_modes(root, false)?;
    mode_inventory_bytes(&entries)
}

pub(super) fn tree_modes(root: &Path, expected: bool) -> Result<BTreeMap<String, (char, u32)>> {
    let root_mode = if expected { 0o550 } else { actual_mode(root)? };
    let mut entries = BTreeMap::from([(".".to_owned(), ('d', root_mode))]);
    let mut pending = vec![(PathBuf::new(), root.to_path_buf())];
    while let Some((relative_root, absolute_root)) = pending.pop() {
        let mut children = fs::read_dir(&absolute_root)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let metadata = fs::symlink_metadata(child.path())?;
            let relative = relative_root.join(child.file_name());
            let path = path_to_manifest(&relative)?;
            if metadata.is_dir() {
                let mode = if expected {
                    0o550
                } else {
                    actual_mode(&child.path())?
                };
                entries.insert(format!("{path}/"), ('d', mode));
                pending.push((relative, child.path()));
            } else {
                ensure!(metadata.is_file(), "mode inventory found a special node");
                let mode = if expected {
                    canonical_file_mode(&path)
                } else {
                    actual_mode(&child.path())?
                };
                entries.insert(path, ('f', mode));
            }
        }
    }
    Ok(entries)
}

pub(super) fn mode_inventory_bytes(entries: &BTreeMap<String, (char, u32)>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for (path, (kind, mode)) in entries {
        ensure!(
            matches!((*kind, *mode), ('f', 0o440) | ('f', 0o550) | ('d', 0o550)),
            "bundle contains mutable mode"
        );
        writeln!(&mut bytes, "{kind} {mode:04o}  {path}")?;
    }
    Ok(bytes)
}

pub(super) fn write_sha256sums(root: &Path) -> Result<()> {
    write_bytes(&root.join(SHA256SUMS_FILE), &expected_sha256sums(root)?)
}

pub(super) fn expected_sha256sums(root: &Path) -> Result<Vec<u8>> {
    let mut files = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, absolute)| Ok((path_to_manifest(&relative)?, absolute)))
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut bytes = Vec::new();
    for (path, absolute) in files {
        if path == SHA256SUMS_FILE {
            continue;
        }
        let artifact = artifact_from_file(&absolute, "application/octet-stream")?;
        writeln!(&mut bytes, "{}  {path}", artifact.sha256)?;
    }
    Ok(bytes)
}

pub(super) fn expected_root_once_sha256sums(root: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for name in ROOT_ONCE_KIT_FILES {
        let artifact =
            artifact_from_file(&root.join("deploy").join(name), "application/octet-stream")?;
        writeln!(&mut bytes, "{}  {name}", artifact.sha256)?;
    }
    Ok(bytes)
}

pub(super) fn validate_root_once_sha256sums(root: &Path) -> Result<()> {
    ensure!(
        fs::read(root.join(ROOT_ONCE_SHA256SUMS_FILE))? == expected_root_once_sha256sums(root)?,
        "ROOT_ONCE_SHA256SUMS is missing, reordered, substituted, or has extra entries"
    );
    Ok(())
}

pub(super) fn expected_deploy_bootstrap_sha256sums(root: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for name in DEPLOY_BOOTSTRAP_FILES {
        let artifact =
            artifact_from_file(&root.join("deploy").join(name), "application/octet-stream")?;
        writeln!(&mut bytes, "{}  {name}", artifact.sha256)?;
    }
    Ok(bytes)
}

pub(super) fn validate_deploy_bootstrap_sha256sums(root: &Path) -> Result<()> {
    ensure!(
        fs::read(root.join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE))?
            == expected_deploy_bootstrap_sha256sums(root)?,
        "DEPLOY_BOOTSTRAP_SHA256SUMS is missing, reordered, substituted, or has extra entries"
    );
    Ok(())
}

pub(super) fn make_bundle_read_only(root: &Path) -> Result<()> {
    {
        use std::os::unix::fs::PermissionsExt as _;
        for (relative, path) in walk_regular_files(root)? {
            let relative = path_to_manifest(&relative)?;
            fs::set_permissions(
                path,
                fs::Permissions::from_mode(canonical_file_mode(&relative)),
            )?;
        }
        let mut directories = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            directories.push(directory.clone());
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o550))?;
        }
        Ok(())
    }
}

pub(super) fn reject_mounts_at_or_below(root: &Path) -> Result<()> {
    reject_mounts(root, true)
}

pub(super) fn reject_mounts_strictly_below(root: &Path) -> Result<()> {
    reject_mounts(root, false)
}

pub(super) fn reject_mounts(root: &Path, reject_root_itself: bool) -> Result<()> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let canonical_root = fs::canonicalize(root)?;
    let mountinfo = fs::read("/proc/self/mountinfo").context("read mount inventory")?;
    for line in mountinfo
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let mount_field = line
            .split(|byte| *byte == b' ')
            .nth(4)
            .context("malformed /proc/self/mountinfo line")?;
        let mut decoded = Vec::with_capacity(mount_field.len());
        let mut index = 0;
        while index < mount_field.len() {
            if mount_field[index] == b'\\'
                && index + 3 < mount_field.len()
                && mount_field[index + 1..index + 4]
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'7'))
            {
                decoded.push(
                    (mount_field[index + 1] - b'0') * 64
                        + (mount_field[index + 2] - b'0') * 8
                        + (mount_field[index + 3] - b'0'),
                );
                index += 4;
            } else {
                decoded.push(mount_field[index]);
                index += 1;
            }
        }
        let mount_path = PathBuf::from(OsString::from_vec(decoded));
        ensure!(
            !mount_path.starts_with(&canonical_root)
                || (!reject_root_itself && mount_path == canonical_root),
            "failed VPS staging contains a mount at {}",
            mount_path.display()
        );
    }
    Ok(())
}

pub(super) fn validate_bundle_shape(root: &Path, manifest: &VpsReleaseManifestV2) -> Result<()> {
    let paths = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    for required in [
        "bin/robin-highscores-admin",
        "bin/robin-highscores-manifestctl",
        "bin/robin-highscores-server",
        "bin/robin-highscores-worker",
        "bin/robin-replay-verifier",
        "config/highscores-server.toml",
        "config/highscores-worker.toml",
        "config/api.env",
        "config/worker.env",
        "systemd/user/robin-highscores.target",
        "systemd/user/robin-highscores-api.service",
        "systemd/user/robin-highscores-worker.service",
        "systemd/user/robin-highscores-backup.service",
        "systemd/user/robin-highscores-backup.timer",
        "deploy/deploy-release.sh",
        "deploy/rollback-release.sh",
        "deploy/validate-release-bundle.sh",
        "deploy/tests/real-runtime-fence-release-gate.sh",
        "deploy/tests/real-runtime-fence-e2e.py",
        "deploy/tests/real-runtime-fence-e2e-selftest.py",
        DEPLOY_BOOTSTRAP_SHA256SUMS_FILE,
        ROOT_ONCE_SHA256SUMS_FILE,
        "deploy/root-once.sh",
        "deploy/nginx-robinhood-api.challenge.conf",
        "deploy/nginx-robinhood-cloudflare-only.conf",
        "deploy/nginx-robinhood-api.locations.conf",
        "deploy/nginx-robinhood-api.vhost.conf",
        "deploy/README.md",
        "deploy/VPS_RELEASE_INSTALL.md",
        "deploy/BACKUP_RESTORE.md",
        RAW_ROOT_DECLARATIONS_FILE,
        "publication/backend-publication-v3.json",
        "publication/publication-manifest-v3.json",
        "publication/publication-manifest-v3.sha256",
        "publication/publication-lock-v3.json",
        "publication/publication-lock-v3.sha256",
    ] {
        ensure!(paths.contains(required), "VPS bundle omits {required}");
    }
    ensure!(
        paths.iter().all(|path| !forbidden_release_path(path)),
        "release retained broker, polkit, socket, or root system-service authority"
    );
    ensure!(
        paths.iter().all(|path| {
            !path.starts_with("cloudflare-public/")
                && !path.starts_with("cloudflare-identity-signer/")
                && !path.starts_with("static/")
                && !path.starts_with("datadirs/")
                && !path.starts_with("raw/")
        }),
        "VPS release contains public/private static or copyrighted raw files"
    );
    let declarations: PrivateRawRootDeclarationsV2 =
        load_canonical(&root.join(RAW_ROOT_DECLARATIONS_FILE))?;
    declarations.validate()?;
    for declaration in &declarations.roots {
        validate_immutable_raw_root(&declaration.root).with_context(|| {
            format!(
                "validate separately installed {:?} raw root",
                declaration.edition
            )
        })?;
    }
    let mut canonical_roots = declarations
        .roots
        .iter()
        .map(|declaration| fs::canonicalize(&declaration.root))
        .collect::<std::io::Result<Vec<_>>>()?;
    canonical_roots.push(fs::canonicalize(root)?);
    for left in 0..canonical_roots.len() {
        for right in left + 1..canonical_roots.len() {
            ensure!(
                !paths_overlap(&canonical_roots[left], &canonical_roots[right]),
                "installed raw roots overlap each other or the release"
            );
        }
    }
    for binary in [
        "robin-highscores-admin",
        "robin-highscores-manifestctl",
        "robin-highscores-server",
        "robin-highscores-worker",
        "robin-replay-verifier",
    ] {
        validate_linux_elf(&root.join("bin").join(binary))?;
    }
    let backend: BackendPublicationV3 =
        load_canonical(&root.join("publication/backend-publication-v3.json"))?;
    for (role, relative) in [
        (VpsConfigRoleV2::Server, "config/highscores-server.toml"),
        (VpsConfigRoleV2::Worker, "config/highscores-worker.toml"),
        (VpsConfigRoleV2::ApiEnvironment, "config/api.env"),
        (VpsConfigRoleV2::WorkerEnvironment, "config/worker.env"),
    ] {
        validate_final_config(
            role,
            &root.join(relative),
            manifest.verifier_sha256,
            &backend.verifier_operator_config,
            &manifest.source_commit,
            &backend
                .campaign_states
                .iter()
                .map(|state| state.artifact.sha256)
                .collect(),
        )?;
    }
    validate_worker_raw_authority(
        &root.join("config/highscores-worker.toml"),
        &manifest.source_commit,
        &root.join("private/source-tree-manifests-v2"),
        &declarations.roots,
    )?;
    validate_root_once_sha256sums(root)?;
    validate_deploy_bootstrap_sha256sums(root)?;
    for (role, relative) in [
        (
            VpsHostFileRoleV2::UserTarget,
            "systemd/user/robin-highscores.target",
        ),
        (
            VpsHostFileRoleV2::ApiService,
            "systemd/user/robin-highscores-api.service",
        ),
        (
            VpsHostFileRoleV2::WorkerService,
            "systemd/user/robin-highscores-worker.service",
        ),
        (
            VpsHostFileRoleV2::BackupService,
            "systemd/user/robin-highscores-backup.service",
        ),
        (
            VpsHostFileRoleV2::BackupTimer,
            "systemd/user/robin-highscores-backup.timer",
        ),
        (
            VpsHostFileRoleV2::DeployReleaseScript,
            "deploy/deploy-release.sh",
        ),
        (
            VpsHostFileRoleV2::RollbackReleaseScript,
            "deploy/rollback-release.sh",
        ),
        (
            VpsHostFileRoleV2::ValidateReleaseScript,
            "deploy/validate-release-bundle.sh",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            "deploy/tests/real-runtime-fence-release-gate.sh",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            "deploy/tests/real-runtime-fence-e2e.py",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            "deploy/tests/real-runtime-fence-e2e-selftest.py",
        ),
        (VpsHostFileRoleV2::RootOnceScript, "deploy/root-once.sh"),
        (
            VpsHostFileRoleV2::NginxChallenge,
            "deploy/nginx-robinhood-api.challenge.conf",
        ),
        (
            VpsHostFileRoleV2::NginxCloudflareOnly,
            "deploy/nginx-robinhood-cloudflare-only.conf",
        ),
        (
            VpsHostFileRoleV2::NginxApiLocations,
            "deploy/nginx-robinhood-api.locations.conf",
        ),
        (
            VpsHostFileRoleV2::NginxVhost,
            "deploy/nginx-robinhood-api.vhost.conf",
        ),
        (VpsHostFileRoleV2::DeploymentReadme, "deploy/README.md"),
        (
            VpsHostFileRoleV2::OperatorRunbook,
            "deploy/VPS_RELEASE_INSTALL.md",
        ),
        (VpsHostFileRoleV2::BackupRunbook, "deploy/BACKUP_RESTORE.md"),
    ] {
        validate_final_host_file(role, &root.join(relative), &manifest.source_commit)?;
    }
    validate_backup_sandbox_contract(
        &root.join("config/highscores-server.toml"),
        &root.join("systemd/user/robin-highscores-api.service"),
        &root.join("systemd/user/robin-highscores-worker.service"),
        &root.join("systemd/user/robin-highscores-backup.service"),
        &root.join("systemd/user/robin-highscores-backup.timer"),
        &manifest.source_commit,
    )?;
    Ok(())
}

pub(super) fn validate_embedded_publication(
    root: &Path,
    manifest: &VpsReleaseManifestV2,
) -> Result<()> {
    let publication_root = root.join("publication");
    let publication_manifest: PublicationManifestV3 =
        load_canonical(&publication_root.join("publication-manifest-v3.json"))?;
    ensure!(
        publication_manifest.canonical_digest()? == manifest.publication_manifest_sha256,
        "embedded publication manifest differs from release identity"
    );
    ensure!(
        fs::read(publication_root.join("publication-manifest-v3.sha256"))?
            == manifest.publication_manifest_sha256.to_string().as_bytes(),
        "embedded publication manifest sidecar mismatch"
    );
    let lock: PublicationLockV3 =
        load_canonical(&publication_root.join("publication-lock-v3.json"))?;
    ensure!(
        lock.publication_manifest_sha256 == manifest.publication_manifest_sha256
            && lock.canonical_digest()? == manifest.publication_lock_sha256,
        "embedded publication lock does not bind the release publication"
    );
    ensure!(
        fs::read(publication_root.join("publication-lock-v3.sha256"))?
            == manifest.publication_lock_sha256.to_string().as_bytes(),
        "embedded publication lock sidecar mismatch"
    );
    let locked_files = lock
        .files
        .iter()
        .map(|file| (file.path.as_str(), &file.artifact))
        .collect::<BTreeMap<_, _>>();
    for (source, bundled) in [
        (
            "backend/publication-v3.json",
            "publication/backend-publication-v3.json",
        ),
        (
            "publication-manifest-v3.json",
            "publication/publication-manifest-v3.json",
        ),
        (
            "publication-manifest-v3.sha256",
            "publication/publication-manifest-v3.sha256",
        ),
    ] {
        let expected = locked_files
            .get(source)
            .with_context(|| format!("publication lock omits {source}"))?;
        let actual = artifact_from_file(&root.join(bundled), "application/octet-stream")?;
        ensure!(
            actual.sha256 == expected.sha256 && actual.byte_length == expected.byte_length,
            "embedded publication evidence differs from lock at {source}"
        );
    }
    let backend: BackendPublicationV3 =
        load_canonical(&publication_root.join("backend-publication-v3.json"))?;
    ensure!(
        backend.build_manifest_sha256 == publication_manifest.build_manifest_sha256
            && backend.verifier_program.sha256 == manifest.verifier_sha256,
        "embedded backend publication is substituted"
    );
    let catalog_path = root
        .join("private/verifier/operator-config")
        .join(backend.verifier_operator_config.sha256.to_string());
    let _: VerifierJobConfigCatalogV1 = load_canonical(&catalog_path)?;
    ensure!(
        artifact_from_file(&catalog_path, &backend.verifier_operator_config.media_type)?
            == backend.verifier_operator_config,
        "bundled verifier job catalog differs from publication"
    );
    let build: BuildManifestV2 = load_canonical(
        &root
            .join("config/manifests/builds")
            .join(format!("{}.json", backend.build_manifest_sha256)),
    )?;
    ensure!(
        build.source_commit == manifest.source_commit,
        "bundle source commit differs from its BuildManifestV2"
    );
    validate_publication_subset(root, &lock, &backend, manifest)?;
    Ok(())
}

pub(super) fn validate_publication_subset(
    root: &Path,
    lock: &PublicationLockV3,
    backend: &BackendPublicationV3,
    manifest: &VpsReleaseManifestV2,
) -> Result<()> {
    let source_files = lock
        .files
        .iter()
        .map(|file| (file.path.as_str(), &file.artifact))
        .collect::<BTreeMap<_, _>>();
    let mut expected = BTreeMap::<String, &ArtifactRefV1>::new();
    for (source, artifact) in &source_files {
        let destination = publication_file_to_vps_path(source);
        if let Some(destination) = destination {
            ensure!(
                expected.insert(destination, artifact).is_none(),
                "publication subset destination collision"
            );
        }
    }
    ensure!(
        expected
            .keys()
            .any(|path| path.starts_with("private/verifier-bundles/"))
            && expected
                .keys()
                .any(|path| path.starts_with("private/campaign-states/"))
            && expected
                .keys()
                .any(|path| path.starts_with("private/source-tree-manifests-v2/")),
        "publication lock omits verifier bundles, campaign templates, or source manifests"
    );
    for (path, artifact) in expected {
        let actual = artifact_from_file(&root.join(&path), "application/octet-stream")?;
        ensure!(
            actual.sha256 == artifact.sha256 && actual.byte_length == artifact.byte_length,
            "publication file omitted or substituted at {path}"
        );
    }
    let mut exact_paths = source_files
        .keys()
        .filter_map(|source| publication_file_to_vps_path(source))
        .collect::<BTreeSet<_>>();
    exact_paths.extend(
        [
            "bin/robin-highscores-admin",
            "bin/robin-highscores-manifestctl",
            "bin/robin-highscores-server",
            "bin/robin-highscores-worker",
            "bin/robin-replay-verifier",
            "config/highscores-server.toml",
            "config/highscores-worker.toml",
            "config/api.env",
            "config/worker.env",
            "systemd/user/robin-highscores.target",
            "systemd/user/robin-highscores-api.service",
            "systemd/user/robin-highscores-worker.service",
            "systemd/user/robin-highscores-backup.service",
            "systemd/user/robin-highscores-backup.timer",
            "deploy/deploy-release.sh",
            "deploy/rollback-release.sh",
            "deploy/validate-release-bundle.sh",
            DEPLOY_BOOTSTRAP_SHA256SUMS_FILE,
            ROOT_ONCE_SHA256SUMS_FILE,
            "deploy/root-once.sh",
            "deploy/nginx-robinhood-api.challenge.conf",
            "deploy/nginx-robinhood-cloudflare-only.conf",
            "deploy/nginx-robinhood-api.locations.conf",
            "deploy/nginx-robinhood-api.vhost.conf",
            "deploy/README.md",
            "deploy/VPS_RELEASE_INSTALL.md",
            "deploy/BACKUP_RESTORE.md",
            RAW_ROOT_DECLARATIONS_FILE,
            "publication/backend-publication-v3.json",
            "publication/publication-manifest-v3.json",
            "publication/publication-manifest-v3.sha256",
            "publication/publication-lock-v3.json",
            "publication/publication-lock-v3.sha256",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    extend_real_runtime_fence_payload_paths(&mut exact_paths);
    let actual_paths = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    ensure!(
        actual_paths == exact_paths,
        "VPS release contains an omitted or extra payload path"
    );
    let verifier_path = root.join("bin/robin-replay-verifier");
    let verifier = artifact_from_file(&verifier_path, &backend.verifier_program.media_type)?;
    ensure!(
        verifier == backend.verifier_program,
        "named verifier binary differs from publication"
    );
    let expected_campaigns = backend
        .campaign_states
        .iter()
        .map(|state| state.artifact.sha256.to_string())
        .collect::<BTreeSet<_>>();
    let actual_campaigns = fs::read_dir(root.join("private/campaign-states"))?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("campaign filename is not UTF-8"))
        })
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    ensure!(
        actual_campaigns == expected_campaigns,
        "campaign template set differs from publication"
    );
    Ok(())
}

pub(super) fn extend_real_runtime_fence_payload_paths(paths: &mut BTreeSet<String>) {
    paths.extend(
        [
            "deploy/tests/real-runtime-fence-release-gate.sh",
            "deploy/tests/real-runtime-fence-e2e.py",
            "deploy/tests/real-runtime-fence-e2e-selftest.py",
        ]
        .into_iter()
        .map(str::to_owned),
    );
}

pub(super) fn publication_file_to_vps_path(source: &str) -> Option<String> {
    if let Some(relative) = source.strip_prefix("backend/manifests/") {
        Some(format!("config/manifests/{relative}"))
    } else if let Some(relative) =
        source.strip_prefix("private/official-content-authority/verifier-bundles/")
    {
        Some(format!("private/verifier-bundles/{relative}"))
    } else if let Some(relative) =
        source.strip_prefix("private/official-content-authority/private/source-tree-manifests-v2/")
    {
        Some(format!("private/source-tree-manifests-v2/{relative}"))
    } else if source.starts_with("private/campaign-states/")
        || source.starts_with("private/verifier/operator-config/")
    {
        Some(source.to_owned())
    } else {
        None
    }
}

pub(super) fn load_canonical<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical document {}", path.display()))?;
    document.validate()?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "{} is not canonical JSON",
        path.display()
    );
    Ok(document)
}

pub(super) fn validate_immutable_raw_root(root: &Path) -> Result<()> {
    validate_mount_root(root)?;
    reject_links_and_special_nodes(root, true)?;
    validate_single_owner_tree(root)?;
    for (_, file) in walk_regular_files(root)? {
        ensure!(
            actual_mode(&file)? == 0o440,
            "private raw root files must use mode 0440"
        );
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        ensure!(
            actual_mode(&directory)? == 0o550,
            "private raw root directories must use mode 0550"
        );
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

pub(super) fn reject_links_and_special_nodes(root: &Path, reject_writable: bool) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let metadata = fs::symlink_metadata(&directory)?;
        ensure!(metadata.is_dir(), "tree contains a non-directory root");
        if reject_writable {
            ensure!(
                actual_mode(&directory)? & 0o222 == 0,
                "tree contains a mutable directory"
            );
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "tree contains forbidden symlink {}",
                entry.path().display()
            );
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                ensure!(metadata.is_file(), "tree contains a special node");
                reject_hardlink(&entry.path())?;
                if reject_writable {
                    ensure!(
                        actual_mode(&entry.path())? & 0o222 == 0,
                        "tree contains a mutable file"
                    );
                }
            }
        }
    }
    Ok(())
}

pub(super) fn reject_hardlink(path: &Path) -> Result<()> {
    validate_regular_file(path)?;
    {
        use std::os::unix::fs::MetadataExt as _;
        ensure!(
            fs::symlink_metadata(path)?.nlink() == 1,
            "hard-linked release input is forbidden: {}",
            path.display()
        );
    }
    Ok(())
}

pub(super) fn validate_single_owner_tree(root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let expected_uid = fs::symlink_metadata(root)?.uid();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.uid() == expected_uid,
            "immutable tree contains mixed ownership at {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

pub(super) fn validate_single_device_tree(root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let expected_device = fs::symlink_metadata(root)?.dev();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.dev() == expected_device,
            "immutable tree crosses devices at {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

pub(super) fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

pub(super) fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

pub(super) fn valid_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn valid_release_directory_name(name: &str, source_commit: &str) -> bool {
    name == source_commit || candidate_release_directory_name(name, source_commit)
}

pub(super) fn candidate_release_directory_name(name: &str, source_commit: &str) -> bool {
    name == format!("{source_commit}.partial")
}

pub(super) fn forbidden_release_path(path: &str) -> bool {
    path.starts_with("polkit/")
        || path.starts_with("systemd/system/")
        || (path.starts_with("systemd/") && !path.starts_with("systemd/user/"))
        || path.contains("verifier-broker")
        || path.ends_with(".socket")
}

pub(super) fn valid_relative_manifest_path(value: &str) -> bool {
    crate::fs_util::valid_relative_path(value) && !value.chars().any(char::is_control)
}

pub(super) fn actual_mode(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
}

pub(super) fn canonical_user_deployment() -> VpsUserDeploymentV2 {
    VpsUserDeploymentV2 {
        user: DEPLOYMENT_USER.to_owned(),
        home: DEPLOYMENT_HOME.into(),
        install_root: INSTALL_ROOT.into(),
        persistent_state_root: STATE_ROOT.into(),
        current_link: format!("{INSTALL_ROOT}/current").into(),
    }
}

pub(super) fn canonical_file_mode(path: &str) -> u32 {
    if matches!(
        path,
        "bin/robin-highscores-admin"
            | "bin/robin-highscores-manifestctl"
            | "bin/robin-highscores-server"
            | "bin/robin-highscores-worker"
            | "bin/robin-replay-verifier"
            | "deploy/deploy-release.sh"
            | "deploy/rollback-release.sh"
            | "deploy/validate-release-bundle.sh"
            | "deploy/tests/real-runtime-fence-release-gate.sh"
            | "deploy/tests/real-runtime-fence-e2e.py"
            | "deploy/tests/real-runtime-fence-e2e-selftest.py"
            | "deploy/root-once.sh"
    ) {
        0o550
    } else {
        0o440
    }
}
