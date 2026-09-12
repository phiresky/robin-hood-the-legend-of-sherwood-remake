//! activation responsibilities of the admitted release pipeline.
use super::*;

/// Project the publication-lock digest from one inherited, immutable
/// `VpsReleaseManifestV2` descriptor. The caller supplies the manifest digest
/// independently; no release path or sidecar participates in this scalar
/// authority boundary.
pub fn project_vps_publication_lock_v2(
    release_manifest_fd: std::os::fd::RawFd,
    expected_vps_release_manifest_sha256: &str,
) -> Result<Digest32> {
    {
        use std::io::Read as _;

        ensure!(
            release_manifest_fd >= 3,
            "release-manifest descriptor must be at least 3"
        );
        let expected = expected_vps_release_manifest_sha256
            .parse::<Digest32>()
            .context(
                "expected VPS release manifest digest is not canonical lowercase hexadecimal",
            )?;
        ensure!(
            !expected.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        let duplicate = InheritedFd::duplicate(release_manifest_fd)?;
        let initial = fd_policy::stat(duplicate.0)?;
        fd_policy::ensure_private_regular(
            initial.st_mode,
            initial.st_uid,
            initial.st_nlink as u64,
            &format!("release-manifest descriptor has unsafe type, owner, links, mode, or size"),
        )?;
        ensure!(
            initial.st_mode & 0o777 == 0o440
                && initial.st_size > 0
                && u64::try_from(initial.st_size)? <= MAX_DOCUMENT_BYTES,
            "release-manifest descriptor has unsafe type, owner, links, mode, or size"
        );
        let descriptor = PathBuf::from(format!("/proc/self/fd/{}", duplicate.0));
        let mut reader = File::open(&descriptor)?;
        let reader_initial = rustix::fs::fstat(&reader)?;
        ensure!(
            reader_initial.st_dev == initial.st_dev
                && reader_initial.st_ino == initial.st_ino
                && reader_initial.st_mode == initial.st_mode,
            "release-manifest procfs duplicate names another inode"
        );
        let mut bytes = Vec::with_capacity(usize::try_from(initial.st_size)?);
        std::io::Read::by_ref(&mut reader)
            .take(MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == u64::try_from(initial.st_size)?,
            "release-manifest descriptor length changed while it was read"
        );
        let reader_observed = rustix::fs::fstat(&reader)?;
        ensure!(
            reader_observed.st_dev == reader_initial.st_dev
                && reader_observed.st_ino == reader_initial.st_ino
                && reader_observed.st_uid == reader_initial.st_uid
                && reader_observed.st_gid == reader_initial.st_gid
                && reader_observed.st_mode == reader_initial.st_mode
                && reader_observed.st_nlink == reader_initial.st_nlink
                && reader_observed.st_size == reader_initial.st_size
                && reader_observed.st_mtime == reader_initial.st_mtime
                && reader_observed.st_mtime_nsec == reader_initial.st_mtime_nsec
                && reader_observed.st_ctime == reader_initial.st_ctime
                && reader_observed.st_ctime_nsec == reader_initial.st_ctime_nsec,
            "release-manifest procfs duplicate changed while it was read"
        );
        let observed = fd_policy::stat(duplicate.0)?;
        ensure!(
            observed.st_dev == initial.st_dev
                && observed.st_ino == initial.st_ino
                && observed.st_uid == initial.st_uid
                && observed.st_gid == initial.st_gid
                && observed.st_mode == initial.st_mode
                && observed.st_nlink == initial.st_nlink
                && observed.st_size == initial.st_size
                && observed.st_mtime == initial.st_mtime
                && observed.st_mtime_nsec == initial.st_mtime_nsec
                && observed.st_ctime == initial.st_ctime
                && observed.st_ctime_nsec == initial.st_ctime_nsec,
            "release-manifest descriptor changed while it was read"
        );
        ensure!(
            Digest32::digest_bytes(&bytes) == expected,
            "release-manifest descriptor differs from its out-of-band digest"
        );
        let manifest: VpsReleaseManifestV2 = strict_json_from_slice(&bytes)
            .context("parse descriptor-pinned canonical VpsReleaseManifestV2")?;
        manifest.validate()?;
        ensure!(
            canonical_json_bytes(&manifest)? == bytes,
            "descriptor-pinned VpsReleaseManifestV2 is not canonical JSON"
        );
        ensure!(
            fd_policy::stat(duplicate.0)? == observed,
            "release-manifest descriptor changed after canonical validation"
        );
        Ok(manifest.publication_lock_sha256)
    }
}

pub(super) fn load_pinned_vps_plan(
    plan_fd: &Path,
    expected_plan_sha256: &str,
) -> Result<(VpsReleasePlanV2, Vec<u8>, Digest32)> {
    let descriptor = canonical_proc_descriptor(plan_fd, "VPS release plan")?;
    let metadata = fd_policy::stat(descriptor)?;
    fd_policy::ensure_private_regular(
        metadata.st_mode,
        metadata.st_uid,
        metadata.st_nlink as u64,
        &format!("VPS release plan descriptor must be an owner-only regular nlink-1 file"),
    )?;
    ensure!(
        metadata.st_mode & 0o777 == 0o400,
        "VPS release plan descriptor must be an owner-only regular nlink-1 file"
    );
    let expected_plan_sha256 = expected_plan_sha256
        .parse::<Digest32>()
        .context("expected VPS release plan digest is not canonical lowercase hexadecimal")?;
    let plan_bytes =
        read_pinned_descriptor_bounded(descriptor, MAX_DOCUMENT_BYTES, "VPS release plan")?;
    ensure!(
        Digest32::digest_bytes(&plan_bytes) == expected_plan_sha256,
        "VPS release plan descriptor differs from its out-of-band digest"
    );
    let plan = VpsReleasePlanV2::load_pinned_absolute_bytes(&plan_bytes, plan_fd)?;
    Ok((plan, plan_bytes, expected_plan_sha256))
}

/// Validate every inherited deploy authority before acquiring the canonical
/// activation lock, then replace this process with the reviewed deploy script.
/// The same lock open-file-description and plan descriptor remain inherited by
/// the script and every destructive source-consumption child.
pub fn exec_vps_deploy_activation_v2(
    script_fd: &Path,
    bootstrap_manifest_fd: &Path,
    validator_fd: &Path,
    manifest_tool_fd: &Path,
    plan_fd: &Path,
    expected_plan_sha256: &str,
    expected_vps_release_manifest_sha256: &str,
    business_arguments: &[std::ffi::OsString],
) -> Result<()> {
    {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::process::CommandExt as _;

        let (candidate, expected_commit, expected_bootstrap_sha256) =
            parse_deploy_business_arguments(business_arguments)?;
        let authorities = pin_vps_activation_exec_authorities(
            VpsActivationExecOperationV2::Deploy,
            script_fd,
            bootstrap_manifest_fd,
            validator_fd,
            manifest_tool_fd,
            expected_bootstrap_sha256,
            Some(plan_fd),
        )?;
        let (plan, _, _) = load_pinned_vps_plan(plan_fd, expected_plan_sha256)?;
        ensure!(
            plan.source_commit == expected_commit,
            "VPS activation plan source commit differs from the requested deploy"
        );
        validate_deploy_plan_source_paths(&plan, &expected_commit)?;
        let expected_vps_release_manifest_sha256 = expected_vps_release_manifest_sha256
            .parse::<Digest32>()
            .context("expected VPS release manifest is not canonical lowercase hexadecimal")?;
        ensure!(
            !expected_vps_release_manifest_sha256.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        let candidate_fd = pin_vps_activation_candidate(&candidate)?;
        let candidate_path = PathBuf::from(format!("/proc/self/fd/{}/.", candidate_fd.as_raw_fd()));
        let actual_manifest = validate_pinned_current_vps_release_root(&candidate_path)?;
        ensure!(
            actual_manifest == expected_vps_release_manifest_sha256,
            "candidate differs from the out-of-band VPS release manifest digest"
        );
        let manifest: VpsReleaseManifestV2 =
            load_canonical(&candidate_path.join(RELEASE_MANIFEST_FILE))?;
        ensure!(
            manifest.source_commit == expected_commit,
            "candidate VPS release source commit differs from the requested deploy"
        );

        let activation_lock = acquire_vps_activation_lock_v2()?;
        activation_lock.ensure_canonical()?;
        let plan_descriptor = plan_fd_descriptor(plan_fd)?;
        for descriptor in authorities.iter().chain(std::iter::once(&plan_descriptor)) {
            clear_vps_close_on_exec(*descriptor)?;
        }
        clear_vps_close_on_exec(candidate_fd.as_raw_fd())?;
        activation_lock.clear_close_on_exec()?;
        let candidate_fd_path =
            PathBuf::from(format!("/proc/self/fd/{}", candidate_fd.as_raw_fd()));
        let lock_path = PathBuf::from(format!("/proc/self/fd/{}", activation_lock.as_raw_fd()));
        let expected_vps_release_manifest_sha256 = expected_vps_release_manifest_sha256.to_string();
        let mut command = std::process::Command::new(script_fd);
        command.args(business_arguments).args([
            bootstrap_manifest_fd.as_os_str(),
            validator_fd.as_os_str(),
            manifest_tool_fd.as_os_str(),
            plan_fd.as_os_str(),
            candidate_fd_path.as_os_str(),
            lock_path.as_os_str(),
            std::ffi::OsStr::new(expected_plan_sha256),
            std::ffi::OsStr::new(&expected_vps_release_manifest_sha256),
        ]);
        let error = command.exec();
        Err(error).context("exec descriptor-pinned VPS deploy transaction")
    }
}

/// Rollback is intentionally disjoint from uploader source authority: it
/// admits no plan descriptor and can never invoke source consumption.
pub fn exec_vps_rollback_activation_v2(
    script_fd: &Path,
    bootstrap_manifest_fd: &Path,
    validator_fd: &Path,
    manifest_tool_fd: &Path,
    business_arguments: &[std::ffi::OsString],
) -> Result<()> {
    {
        use std::os::unix::process::CommandExt as _;

        let expected_bootstrap_sha256 = parse_rollback_business_arguments(business_arguments)?;
        let authorities = pin_vps_activation_exec_authorities(
            VpsActivationExecOperationV2::Rollback,
            script_fd,
            bootstrap_manifest_fd,
            validator_fd,
            manifest_tool_fd,
            expected_bootstrap_sha256,
            None,
        )?;
        let activation_lock = acquire_vps_activation_lock_v2()?;
        activation_lock.ensure_canonical()?;
        for descriptor in &authorities {
            clear_vps_close_on_exec(*descriptor)?;
        }
        activation_lock.clear_close_on_exec()?;
        let lock_path = PathBuf::from(format!("/proc/self/fd/{}", activation_lock.as_raw_fd()));
        let mut command = std::process::Command::new(script_fd);
        command.args(business_arguments).args([
            bootstrap_manifest_fd.as_os_str(),
            validator_fd.as_os_str(),
            manifest_tool_fd.as_os_str(),
            lock_path.as_os_str(),
        ]);
        let error = command.exec();
        Err(error).context("exec descriptor-pinned VPS rollback transaction")
    }
}

#[derive(Clone, Copy)]
pub(super) enum VpsActivationExecOperationV2 {
    Deploy,
    Rollback,
}

pub(super) fn os_argument(
    arguments: &[std::ffi::OsString],
    index: usize,
    label: &str,
) -> Result<String> {
    arguments
        .get(index)
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .with_context(|| format!("VPS activation {label} is missing or not UTF-8"))
}

pub(super) fn parse_deploy_business_arguments(
    arguments: &[std::ffi::OsString],
) -> Result<(PathBuf, String, &str)> {
    let offset = if arguments.first().and_then(|value| value.to_str()) == Some("--resume-installed")
    {
        ensure!(arguments.len() == 5, "invalid deploy resume argument count");
        1
    } else {
        ensure!(arguments.len() == 4, "invalid deploy argument count");
        0
    };
    let candidate = PathBuf::from(os_argument(arguments, offset, "candidate path")?);
    let commit = os_argument(arguments, offset + 1, "source commit")?;
    ensure!(valid_source_commit(&commit), "invalid deploy source commit");
    let expected_candidate = if offset == 0 {
        Path::new(INSTALL_ROOT)
            .join("releases")
            .join(format!("{commit}.partial"))
    } else {
        Path::new(INSTALL_ROOT).join("releases").join(&commit)
    };
    ensure!(
        candidate == expected_candidate,
        "deploy candidate path is not the exact canonical transaction path"
    );
    let sums = os_argument(arguments, offset + 2, "SHA256SUMS digest")?;
    sums.parse::<Digest32>()
        .context("deploy SHA256SUMS digest is not canonical")?;
    let bootstrap = arguments[offset + 3]
        .to_str()
        .context("deploy bootstrap digest is not UTF-8")?;
    bootstrap
        .parse::<Digest32>()
        .context("deploy bootstrap digest is not canonical")?;
    Ok((candidate, commit, bootstrap))
}

pub(super) fn parse_rollback_business_arguments(arguments: &[std::ffi::OsString]) -> Result<&str> {
    let offset = if arguments.first().and_then(|value| value.to_str()) == Some("--resume-target") {
        ensure!(
            arguments.len() == 4,
            "invalid rollback resume argument count"
        );
        1
    } else {
        ensure!(arguments.len() == 3, "invalid rollback argument count");
        0
    };
    let commit = os_argument(arguments, offset, "rollback source commit")?;
    ensure!(
        valid_source_commit(&commit),
        "invalid rollback source commit"
    );
    os_argument(arguments, offset + 1, "rollback SHA256SUMS digest")?
        .parse::<Digest32>()
        .context("rollback SHA256SUMS digest is not canonical")?;
    let bootstrap = arguments[offset + 2]
        .to_str()
        .context("rollback bootstrap digest is not UTF-8")?;
    bootstrap
        .parse::<Digest32>()
        .context("rollback bootstrap digest is not canonical")?;
    Ok(bootstrap)
}

pub(super) fn plan_fd_descriptor(path: &Path) -> Result<std::os::fd::RawFd> {
    canonical_proc_descriptor(path, "VPS release plan")
}

pub(super) fn canonical_proc_descriptor(path: &Path, label: &str) -> Result<std::os::fd::RawFd> {
    let descriptor = path
        .to_str()
        .and_then(|path| path.strip_prefix("/proc/self/fd/"))
        .with_context(|| format!("{label} must be an explicit /proc/self/fd descriptor"))?;
    ensure!(
        !descriptor.is_empty() && descriptor.bytes().all(|byte| byte.is_ascii_digit()),
        "{label} descriptor is not canonical"
    );
    descriptor
        .parse::<std::os::fd::RawFd>()
        .with_context(|| format!("{label} descriptor is out of range"))
}

pub(super) fn read_pinned_descriptor_bounded(
    descriptor: std::os::fd::RawFd,
    maximum_bytes: u64,
    label: &str,
) -> Result<Vec<u8>> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::FileExt as _;

    ensure!(descriptor >= 3, "{label} descriptor must be at least 3");
    let initial = fd_policy::stat(descriptor)?;
    fd_policy::ensure_private_regular(
        initial.st_mode,
        initial.st_uid,
        initial.st_nlink as u64,
        &format!("{label} descriptor has unsafe type, owner, links, or size"),
    )?;
    ensure!(
        initial.st_size >= 0 && u64::try_from(initial.st_size)? <= maximum_bytes,
        "{label} descriptor has unsafe type, owner, links, or size"
    );
    // Reading can legitimately update atime on relatime/strictatime mounts.
    // Continue checking inode, permissions, size, mtime and ctime for mutation.
    let unchanged = |mut observed: fd_policy::RawStat| {
        observed.st_atime = initial.st_atime;
        observed.st_atime_nsec = initial.st_atime_nsec;
        observed == initial
    };
    let duplicate = InheritedFd::duplicate(descriptor)?;
    ensure!(
        unchanged(fd_policy::stat(duplicate.0)?),
        "{label} descriptor changed before its duplicate was read"
    );
    let duplicate_path = PathBuf::from(format!("/proc/self/fd/{}", duplicate.0));
    let file = File::open(duplicate_path)?;
    ensure!(
        unchanged(fd_policy::stat(file.as_raw_fd())?),
        "{label} procfs duplicate names another inode"
    );
    let expected_length = usize::try_from(initial.st_size)?;
    let mut bytes = vec![0_u8; expected_length];
    let mut offset = 0;
    while offset < expected_length {
        let read = file.read_at(&mut bytes[offset..], u64::try_from(offset)?)?;
        ensure!(read != 0, "{label} descriptor shortened while it was read");
        offset += read;
    }
    let mut extra = [0_u8; 1];
    ensure!(
        file.read_at(&mut extra, u64::try_from(expected_length)?)? == 0,
        "{label} descriptor grew while it was read"
    );
    ensure!(
        unchanged(fd_policy::stat(file.as_raw_fd())?) && unchanged(fd_policy::stat(descriptor)?),
        "{label} descriptor changed while it was read"
    );
    Ok(bytes)
}

pub(super) fn pin_vps_activation_exec_authorities(
    operation: VpsActivationExecOperationV2,
    script_fd: &Path,
    bootstrap_manifest_fd: &Path,
    validator_fd: &Path,
    manifest_tool_fd: &Path,
    expected_bootstrap_sha256: &str,
    plan_fd: Option<&Path>,
) -> Result<Vec<std::os::fd::RawFd>> {
    let descriptors = [
        (script_fd, 0o500, "activation script"),
        (bootstrap_manifest_fd, 0o400, "bootstrap manifest"),
        (validator_fd, 0o500, "release validator"),
        (manifest_tool_fd, 0o550, "manifest tool"),
    ];
    let mut raw = Vec::with_capacity(5);
    for (path, mode, label) in descriptors {
        let descriptor = canonical_proc_descriptor(path, label)?;
        ensure!(descriptor >= 3, "{label} descriptor must be at least 3");
        let metadata = fd_policy::stat(descriptor)?;
        fd_policy::ensure_private_regular(
            metadata.st_mode,
            metadata.st_uid,
            metadata.st_nlink as u64,
            &format!("{label} descriptor has unsafe type, owner, links, or mode"),
        )?;
        ensure!(
            metadata.st_mode & 0o777 == mode,
            "{label} descriptor has unsafe type, owner, links, or mode"
        );
        raw.push(descriptor);
    }
    if let Some(plan_fd) = plan_fd {
        let descriptor = canonical_proc_descriptor(plan_fd, "VPS release plan")?;
        let metadata = fd_policy::stat(descriptor)?;
        fd_policy::ensure_private_regular(
            metadata.st_mode,
            metadata.st_uid,
            metadata.st_nlink as u64,
            &format!("VPS release plan descriptor has unsafe type, owner, links, or mode"),
        )?;
        ensure!(
            metadata.st_mode & 0o777 == 0o400,
            "VPS release plan descriptor has unsafe type, owner, links, or mode"
        );
        raw.push(descriptor);
    }
    ensure!(
        raw.iter().copied().collect::<BTreeSet<_>>().len() == raw.len(),
        "VPS activation authority descriptors must be distinct"
    );

    let executing = fs::metadata("/proc/self/exe")?;
    let manifest_tool = fd_policy::stat(raw[3])?;
    use std::os::unix::fs::MetadataExt as _;
    ensure!(
        executing.dev() == manifest_tool.st_dev && executing.ino() == manifest_tool.st_ino,
        "manifest-tool descriptor is not the currently executing inode"
    );
    let expected_bootstrap = expected_bootstrap_sha256
        .parse::<Digest32>()
        .context("bootstrap manifest digest is not canonical")?;
    let bootstrap_bytes =
        read_pinned_descriptor_bounded(raw[1], MAX_DOCUMENT_BYTES, "bootstrap manifest")?;
    ensure!(
        Digest32::digest_bytes(&bootstrap_bytes) == expected_bootstrap,
        "bootstrap manifest differs from its out-of-band digest"
    );
    let bootstrap_text = std::str::from_utf8(&bootstrap_bytes)?;
    let mut entries = BTreeMap::new();
    for line in bootstrap_text.lines() {
        let (digest, name) = line
            .split_once("  ")
            .context("bootstrap manifest line is not canonical")?;
        let digest = digest
            .parse::<Digest32>()
            .context("bootstrap manifest entry digest is not canonical")?;
        ensure!(
            matches!(
                name,
                "deploy-release.sh" | "rollback-release.sh" | "validate-release-bundle.sh"
            ) && entries.insert(name, digest).is_none(),
            "bootstrap manifest has an unknown or duplicate entry"
        );
    }
    ensure!(
        entries.len() == 3,
        "bootstrap manifest inventory is incomplete"
    );
    let expected_script_name = match operation {
        VpsActivationExecOperationV2::Deploy => "deploy-release.sh",
        VpsActivationExecOperationV2::Rollback => "rollback-release.sh",
    };
    let descriptor_digest = |descriptor, label| -> Result<Digest32> {
        Ok(Digest32::digest_bytes(&read_pinned_descriptor_bounded(
            descriptor,
            MAX_DOCUMENT_BYTES,
            label,
        )?))
    };
    ensure!(
        descriptor_digest(raw[0], "activation script")? == entries[expected_script_name],
        "activation script differs from the pinned bootstrap manifest"
    );
    ensure!(
        descriptor_digest(raw[2], "release validator")? == entries["validate-release-bundle.sh"],
        "release validator differs from the pinned bootstrap manifest"
    );
    Ok(raw)
}

pub(super) fn clear_vps_close_on_exec(descriptor: std::os::fd::RawFd) -> Result<()> {
    fd_policy::clear_close_on_exec(descriptor)
}

pub(super) fn validate_deploy_plan_source_paths(
    plan: &VpsReleasePlanV2,
    commit: &str,
) -> Result<()> {
    let source = Path::new(INSTALL_ROOT)
        .join("incoming")
        .join(format!(".sources-{commit}"));
    ensure!(
        plan.publication_v3 == source.join("publication-v3")
            && plan
                .binaries
                .iter()
                .all(|entry| entry.source == source.join("bin").join(entry.role.output_name()))
            && plan
                .configs
                .iter()
                .all(|entry| entry.source == source.join("config").join(entry.role.output_name()))
            && plan.host_files.iter().all(|entry| {
                entry.source == source.join("host").join(entry.role.output_path())
            }),
        "VPS deploy plan does not bind the exact uploader source closure"
    );
    Ok(())
}

pub(super) fn pin_vps_activation_candidate(path: &Path) -> Result<std::os::fd::OwnedFd> {
    pin_vps_activation_candidate_at(
        path,
        &Path::new(INSTALL_ROOT).join("incoming"),
        &Path::new(INSTALL_ROOT).join("releases"),
    )
}

pub(super) fn pin_vps_activation_candidate_at(
    path: &Path,
    incoming_root: &Path,
    releases_root: &Path,
) -> Result<std::os::fd::OwnedFd> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::os::fd::AsFd as _;

    ensure!(
        normalized_absolute(path) && path.parent() == Some(releases_root),
        "VPS activation candidate is outside the exact releases root"
    );
    let basename = path
        .file_name()
        .context("VPS activation candidate has no basename")?;
    let parents = pin_vps_candidate_parents_at(incoming_root, releases_root)?;
    let path_metadata = statat(
        parents.installed_parent_fd.as_fd(),
        basename,
        AtFlags::SYMLINK_NOFOLLOW,
    )?;
    ensure!(
        FileType::from_raw_mode(path_metadata.st_mode).is_dir()
            && path_metadata.st_uid == rustix::process::geteuid().as_raw()
            && path_metadata.st_mode & 0o777 == 0o550,
        "VPS activation candidate has unsafe type, owner, or mode"
    );
    let fd = openat2(
        parents.installed_parent_fd.as_fd(),
        basename,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let pinned = rustix::fs::fstat(&fd)?;
    ensure!(
        FileType::from_raw_mode(pinned.st_mode).is_dir()
            && pinned.st_dev == path_metadata.st_dev
            && pinned.st_ino == path_metadata.st_ino
            && pinned.st_uid == path_metadata.st_uid
            && pinned.st_mode & 0o777 == 0o550,
        "VPS activation candidate changed while it was pinned"
    );
    Ok(fd)
}
