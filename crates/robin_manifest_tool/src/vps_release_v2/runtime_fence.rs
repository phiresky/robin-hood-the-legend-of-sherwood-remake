//! runtime fence responsibilities of the admitted release pipeline.
use super::*;

pub(super) const RUNTIME_FENCE_INTENT_NAME: &str = ".runtime-fence-init-v1.json";
pub(super) const RUNTIME_FENCE_INTENT_TEMPORARY_NAME: &str = ".runtime-fence-init-v1.json.new";
pub(super) const RUNTIME_FENCE_INTENT_WRITING_NAME: &str = ".runtime-fence-init-v1.json.writing";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RuntimeFenceInitPhaseV1 {
    Authorized,
    StagingBound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuntimeFenceInitIntentV1 {
    pub(super) schema_version: u32,
    pub(super) phase: RuntimeFenceInitPhaseV1,
    pub(super) source_commit: String,
    pub(super) staging_name: String,
    pub(super) staging_device: Option<u64>,
    pub(super) staging_inode: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeFenceInitBoundaryV1 {
    IntentWritingCreated,
    IntentWritingSynced,
    IntentNewPublished,
    AuthorizedIntentPublished,
    StagingCreated,
    BoundIntentWritingCreated,
    BoundIntentWritingSynced,
    BoundIntentNewPublished,
    BoundIntentExchanged,
    StagingBoundIntentPublished,
    AdmissionLeafSynced,
    QuiescenceLeafSynced,
    StagingSealed,
    FinalPublished,
    IntentRemoved,
}

pub(super) struct PinnedRuntimeFenceIntentV1 {
    pub(super) file: File,
    pub(super) metadata: rustix::fs::Stat,
    pub(super) document: RuntimeFenceInitIntentV1,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn pin_runtime_fence_intent(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
    allowed_modes: &[u32],
) -> Result<PinnedRuntimeFenceIntentV1> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};

    use std::os::fd::AsFd as _;

    let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    let state_metadata = rustix::fs::fstat(state)?;
    fd_policy::ensure_private_regular(
        named.st_mode,
        named.st_uid,
        named.st_nlink as u64,
        &format!("runtime-fence initializer intent has unsafe metadata"),
    )?;
    ensure!(
        named.st_dev == state_metadata.st_dev
            && allowed_modes.contains(&(named.st_mode & 0o777))
            && named.st_size > 0
            && named.st_size as u64 <= MAX_DOCUMENT_BYTES,
        "runtime-fence initializer intent has unsafe metadata"
    );
    let fd = openat2(
        state.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let metadata = rustix::fs::fstat(&fd)?;
    ensure!(
        metadata.st_dev == named.st_dev
            && metadata.st_ino == named.st_ino
            && metadata.st_uid == named.st_uid
            && metadata.st_mode == named.st_mode
            && metadata.st_nlink == named.st_nlink
            && metadata.st_size == named.st_size,
        "runtime-fence initializer intent changed while it was pinned"
    );
    let mut file = File::from(fd);
    let bytes =
        crate::fs_util::read_bounded(&mut file, MAX_DOCUMENT_BYTES, metadata.st_size as u64)?;
    let after_read = rustix::fs::fstat(&file)?;
    ensure!(
        after_read.st_dev == metadata.st_dev
            && after_read.st_ino == metadata.st_ino
            && after_read.st_uid == metadata.st_uid
            && after_read.st_mode == metadata.st_mode
            && after_read.st_nlink == metadata.st_nlink
            && after_read.st_size == metadata.st_size
            && after_read.st_mtime == metadata.st_mtime
            && after_read.st_mtime_nsec == metadata.st_mtime_nsec
            && after_read.st_ctime == metadata.st_ctime
            && after_read.st_ctime_nsec == metadata.st_ctime_nsec,
        "runtime-fence initializer intent changed while it was read"
    );
    let document: RuntimeFenceInitIntentV1 = strict_json_from_slice(&bytes)?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "runtime-fence initializer intent is not canonical JSON"
    );
    Ok(PinnedRuntimeFenceIntentV1 {
        file,
        metadata,
        document,
        bytes,
    })
}

pub(super) fn validate_runtime_fence_intent(
    intent: &RuntimeFenceInitIntentV1,
    source_commit: &str,
    staging_name: &str,
    state_device: u64,
) -> Result<()> {
    ensure!(
        intent.schema_version == 1
            && intent.source_commit == source_commit
            && intent.staging_name == staging_name
            && match intent.phase {
                RuntimeFenceInitPhaseV1::Authorized => {
                    intent.staging_device.is_none() && intent.staging_inode.is_none()
                }
                RuntimeFenceInitPhaseV1::StagingBound => {
                    intent.staging_device == Some(state_device)
                        && intent.staging_inode.is_some_and(|inode| inode != 0)
                }
            },
        "runtime-fence initializer intent does not bind this exact initialization"
    );
    Ok(())
}

pub(super) fn runtime_fence_bound_identity(
    intent: &RuntimeFenceInitIntentV1,
) -> Result<(u64, u64)> {
    ensure!(
        intent.phase == RuntimeFenceInitPhaseV1::StagingBound,
        "runtime-fence initializer intent has not bound a staging inode"
    );
    Ok((
        intent
            .staging_device
            .context("runtime-fence bound intent omitted its device")?,
        intent
            .staging_inode
            .context("runtime-fence bound intent omitted its inode")?,
    ))
}

pub(super) fn runtime_fence_named_identity(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<Option<(u64, u64)>> {
    use rustix::fs::{AtFlags, FileType, statat};
    use std::os::fd::AsFd as _;

    match statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => {
            ensure!(
                FileType::from_raw_mode(metadata.st_mode).is_dir(),
                "runtime-fence initializer authority name is not a directory"
            );
            Ok(Some((metadata.st_dev, metadata.st_ino)))
        }
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn remove_exact_runtime_fence_intent(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
    pinned: &PinnedRuntimeFenceIntentV1,
) -> Result<()> {
    use rustix::fs::{AtFlags, statat, unlinkat};
    use std::os::fd::AsFd as _;

    let observed = pin_runtime_fence_intent(state, name, &[0o400])?;
    ensure!(
        observed.metadata.st_dev == pinned.metadata.st_dev
            && observed.metadata.st_ino == pinned.metadata.st_ino
            && observed.metadata.st_mode == pinned.metadata.st_mode
            && observed.metadata.st_nlink == pinned.metadata.st_nlink
            && observed.metadata.st_size == pinned.metadata.st_size
            && observed.bytes == pinned.bytes,
        "runtime-fence initializer intent was substituted before removal"
    );
    let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    ensure!(
        named.st_dev == observed.metadata.st_dev
            && named.st_ino == observed.metadata.st_ino
            && named.st_mode == observed.metadata.st_mode
            && named.st_nlink == 1,
        "runtime-fence initializer intent basename changed before removal"
    );
    unlinkat(state.as_fd(), name, AtFlags::empty())?;
    ensure!(
        rustix::fs::fstat(&observed.file)?.st_nlink == 0,
        "runtime-fence initializer intent remains linked after removal"
    );
    rustix::fs::fsync(state)?;
    Ok(())
}

pub(super) fn remove_runtime_fence_writing_scratch(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<()> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat, unlinkat};
    use std::os::fd::AsFd as _;

    let state_metadata = rustix::fs::fstat(state)?;
    let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    fd_policy::ensure_private_regular(
        named.st_mode,
        named.st_uid,
        named.st_nlink as u64,
        &format!("runtime-fence intent writing scratch has unsafe metadata"),
    )?;
    ensure!(
        named.st_dev == state_metadata.st_dev
            && matches!(named.st_mode & 0o777, 0o400 | 0o600)
            && named.st_size >= 0
            && named.st_size as u64 <= MAX_DOCUMENT_BYTES,
        "runtime-fence intent writing scratch has unsafe metadata"
    );
    let scratch = openat2(
        state.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let pinned = rustix::fs::fstat(&scratch)?;
    ensure!(
        pinned.st_dev == named.st_dev
            && pinned.st_ino == named.st_ino
            && pinned.st_uid == named.st_uid
            && pinned.st_mode == named.st_mode
            && pinned.st_nlink == named.st_nlink
            && pinned.st_size == named.st_size,
        "runtime-fence intent writing scratch changed while it was pinned"
    );
    unlinkat(state.as_fd(), name, AtFlags::empty())?;
    ensure!(
        rustix::fs::fstat(&scratch)?.st_nlink == 0,
        "runtime-fence intent writing scratch remains linked after removal"
    );
    rustix::fs::fsync(state)?;
    Ok(())
}

pub(super) fn write_runtime_fence_intent_new<F>(
    state: &std::os::fd::OwnedFd,
    writing_name: &std::ffi::OsStr,
    new_name: &std::ffi::OsStr,
    document: &RuntimeFenceInitIntentV1,
    activation_lock: &PinnedVpsActivationLockV2,
    boundaries: (
        RuntimeFenceInitBoundaryV1,
        RuntimeFenceInitBoundaryV1,
        RuntimeFenceInitBoundaryV1,
    ),
    after_boundary: &mut F,
) -> Result<PinnedRuntimeFenceIntentV1>
where
    F: FnMut(RuntimeFenceInitBoundaryV1) -> Result<()>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, fchmod, openat2, renameat_with};
    use std::os::fd::AsFd as _;

    ensure!(
        !pinned_entry_exists(state, &writing_name.to_os_string())?
            && !pinned_entry_exists(state, &new_name.to_os_string())?,
        "runtime-fence initializer intent scratch is not clean"
    );
    let bytes = canonical_json_bytes(document)?;
    let writing_fd = openat2(
        state.as_fd(),
        writing_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut writing = File::from(writing_fd);
    after_boundary(boundaries.0)?;
    writing.write_all(&bytes)?;
    writing.sync_all()?;
    rustix::fs::fsync(state)?;
    after_boundary(boundaries.1)?;
    fchmod(&writing, Mode::from_raw_mode(0o400))?;
    writing.sync_all()?;
    let pinned = pin_runtime_fence_intent(state, writing_name, &[0o400])?;
    ensure!(
        pinned.document == *document && pinned.bytes == bytes,
        "runtime-fence initializer intent writing scratch changed before publication"
    );
    activation_lock.ensure_canonical()?;
    renameat_with(
        state.as_fd(),
        writing_name,
        state.as_fd(),
        new_name,
        RenameFlags::NOREPLACE,
    )?;
    rustix::fs::fsync(state)?;
    after_boundary(boundaries.2)?;
    activation_lock.ensure_canonical()?;
    let published = pin_runtime_fence_intent(state, new_name, &[0o400])?;
    ensure!(
        published.document == *document && published.bytes == bytes,
        "runtime-fence initializer intent .new differs from complete writing scratch"
    );
    Ok(published)
}

/// Create the two permanent runtime lock inodes through a private,
/// crash-resumable staging directory, then atomically publish that exact
/// directory. Runtime processes only ever adopt the sealed final topology.
pub fn initialize_vps_runtime_fence_v1(
    source_commit: &str,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<()> {
    {
        let activation_lock = pin_inherited_vps_activation_lock_v2(activation_lock_fd)?;
        initialize_vps_runtime_fence_v1_at(
            source_commit,
            &activation_lock,
            Path::new(STATE_ROOT),
            |_| Ok(()),
        )
    }
}

pub(super) fn initialize_vps_runtime_fence_v1_at<F>(
    source_commit: &str,
    activation_lock: &PinnedVpsActivationLockV2,
    state_path: &Path,
    mut after_boundary: F,
) -> Result<()>
where
    F: FnMut(RuntimeFenceInitBoundaryV1) -> Result<()>,
{
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, RenameFlags, ResolveFlags, fchmod, mkdirat,
        openat2, renameat_with, statat,
    };
    use std::ffi::{OsStr, OsString};
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStringExt as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    ensure!(
        valid_source_commit(source_commit),
        "invalid runtime-fence source commit"
    );
    activation_lock.ensure_canonical()?;
    let state_metadata = fs::symlink_metadata(state_path)?;
    ensure!(
        state_metadata.is_dir()
            && !state_metadata.file_type().is_symlink()
            && fs::canonicalize(state_path)? == state_path
            && state_metadata.uid() == rustix::process::geteuid().as_raw()
            && state_metadata.permissions().mode() & 0o777 == 0o700,
        "runtime-fence state root is not canonical owner-only mode 0700"
    );
    let state = openat2(
        rustix::fs::CWD,
        state_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let state_pinned = rustix::fs::fstat(&state)?;
    ensure!(
        state_pinned.st_dev == state_metadata.dev() && state_pinned.st_ino == state_metadata.ino(),
        "runtime-fence state root changed while it was pinned"
    );
    let final_name = OsString::from("runtime-fence");
    let staging_name = OsString::from(format!(".runtime-fence-{source_commit}.partial"));
    let staging_name_text = staging_name
        .to_str()
        .context("runtime-fence staging name is not UTF-8")?;
    let intent_name = OsString::from(RUNTIME_FENCE_INTENT_NAME);
    let intent_temporary_name = OsString::from(RUNTIME_FENCE_INTENT_TEMPORARY_NAME);
    let intent_writing_name = OsString::from(RUNTIME_FENCE_INTENT_WRITING_NAME);

    let scan = openat2(
        state.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut buffer = Vec::with_capacity(4096);
    let mut directory = RawDir::new(&scan, buffer.spare_capacity_mut());
    while let Some(entry) = directory.next() {
        let entry = entry?;
        let bytes = entry.file_name().to_bytes();
        if bytes.starts_with(b".runtime-fence-") && bytes.ends_with(b".partial") {
            ensure!(
                bytes == staging_name.as_encoded_bytes(),
                "foreign runtime-fence staging evidence exists"
            );
        }
    }

    let validate_fence = |root: &std::os::fd::OwnedFd,
                          mode: u32,
                          allow_subset: bool|
     -> Result<()> {
        let root_metadata = rustix::fs::fstat(root)?;
        ensure!(
            FileType::from_raw_mode(root_metadata.st_mode).is_dir()
                && root_metadata.st_uid == rustix::process::geteuid().as_raw()
                && root_metadata.st_dev == state_pinned.st_dev
                && root_metadata.st_mode & 0o777 == mode,
            "runtime-fence directory has unsafe identity, owner, device, or mode"
        );
        let scan = openat2(
            root.as_fd(),
            ".",
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let mut buffer = Vec::with_capacity(4096);
        let mut directory = RawDir::new(&scan, buffer.spare_capacity_mut());
        let mut names = BTreeSet::new();
        while let Some(entry) = directory.next() {
            let entry = entry?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = OsString::from_vec(bytes.to_vec());
            ensure!(
                name == "db-admission.lock" || name == "db-quiescence.lock",
                "runtime-fence contains an unexpected entry"
            );
            ensure!(
                names.insert(name.clone()),
                "runtime-fence entry is duplicated"
            );
            let named = statat(root.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            fd_policy::ensure_private_regular(
                named.st_mode,
                named.st_uid,
                named.st_nlink as u64,
                &format!("runtime-fence leaf has unsafe type, owner, device, links, size, or mode"),
            )?;
            ensure!(
                named.st_dev == state_pinned.st_dev
                    && named.st_size == 0
                    && named.st_mode & 0o777 == 0o400,
                "runtime-fence leaf has unsafe type, owner, device, links, size, or mode"
            );
            let leaf = openat2(
                root.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&leaf)?;
            ensure!(
                pinned.st_dev == named.st_dev
                    && pinned.st_ino == named.st_ino
                    && pinned.st_uid == named.st_uid
                    && pinned.st_mode == named.st_mode
                    && pinned.st_nlink == named.st_nlink
                    && pinned.st_size == named.st_size,
                "runtime-fence leaf changed while it was pinned"
            );
        }
        ensure!(
            allow_subset
                || names
                    == BTreeSet::from([
                        OsString::from("db-admission.lock"),
                        OsString::from("db-quiescence.lock"),
                    ]),
            "runtime-fence final inventory is incomplete"
        );
        Ok(())
    };

    let open_fence = |name: &OsStr| -> Result<std::os::fd::OwnedFd> {
        use rustix::fs::StatxFlags;

        let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
        let state_mount = rustix::fs::statx(
            state.as_fd(),
            ".",
            AtFlags::NO_AUTOMOUNT,
            StatxFlags::MNT_ID,
        )?;
        let named_mount = rustix::fs::statx(
            state.as_fd(),
            name,
            AtFlags::NO_AUTOMOUNT | AtFlags::SYMLINK_NOFOLLOW,
            StatxFlags::MNT_ID,
        )?;
        ensure!(
            state_mount.stx_mask & StatxFlags::MNT_ID.bits() != 0
                && named_mount.stx_mask & StatxFlags::MNT_ID.bits() != 0
                && state_mount.stx_mnt_id == named_mount.stx_mnt_id,
            "runtime-fence staging is a nested mount"
        );
        let root = openat2(
            state.as_fd(),
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let pinned = rustix::fs::fstat(&root)?;
        ensure!(
            pinned.st_dev == named.st_dev
                && pinned.st_ino == named.st_ino
                && pinned.st_uid == named.st_uid
                && pinned.st_mode == named.st_mode,
            "runtime-fence directory changed while it was pinned"
        );
        Ok(root)
    };

    if pinned_entry_exists(&state, &final_name)? {
        ensure!(
            !pinned_entry_exists(&state, &staging_name)?
                && !pinned_entry_exists(&state, &intent_temporary_name)?
                && !pinned_entry_exists(&state, &intent_writing_name)?,
            "sealed runtime-fence coexists with non-terminal initializer evidence"
        );
        let final_root = open_fence(&final_name)?;
        validate_fence(&final_root, 0o500, false)?;
        if pinned_entry_exists(&state, &intent_name)? {
            let pinned_intent = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
            validate_runtime_fence_intent(
                &pinned_intent.document,
                source_commit,
                staging_name_text,
                state_pinned.st_dev,
            )?;
            let final_metadata = rustix::fs::fstat(&final_root)?;
            ensure!(
                (final_metadata.st_dev, final_metadata.st_ino)
                    == runtime_fence_bound_identity(&pinned_intent.document)?,
                "sealed runtime-fence differs from its durable initializer intent"
            );
            if pinned_entry_exists(&state, &intent_temporary_name)? {
                let old = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
                validate_runtime_fence_intent(
                    &old.document,
                    source_commit,
                    staging_name_text,
                    state_pinned.st_dev,
                )?;
                ensure!(
                    old.document.phase == RuntimeFenceInitPhaseV1::Authorized,
                    "sealed runtime-fence retained a non-authorized predecessor intent"
                );
                activation_lock.ensure_canonical()?;
                remove_exact_runtime_fence_intent(&state, &intent_temporary_name, &old)?;
            }
            activation_lock.ensure_canonical()?;
            remove_exact_runtime_fence_intent(&state, &intent_name, &pinned_intent)?;
            after_boundary(RuntimeFenceInitBoundaryV1::IntentRemoved)?;
        } else {
            ensure!(
                !pinned_entry_exists(&state, &intent_temporary_name)?,
                "sealed runtime-fence lacks its primary intent but retains .new"
            );
        }
        activation_lock.ensure_canonical()?;
        return Ok(());
    }

    let mut intent = if pinned_entry_exists(&state, &intent_name)? {
        let current = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
        validate_runtime_fence_intent(
            &current.document,
            source_commit,
            staging_name_text,
            state_pinned.st_dev,
        )?;
        Some(current)
    } else {
        None
    };

    if pinned_entry_exists(&state, &intent_writing_name)? {
        ensure!(
            intent.is_some()
                || (!pinned_entry_exists(&state, &staging_name)?
                    && !pinned_entry_exists(&state, &intent_temporary_name)?),
            "unauthorized runtime-fence intent scratch coexists with mutated state"
        );
        activation_lock.ensure_canonical()?;
        remove_runtime_fence_writing_scratch(&state, &intent_writing_name)?;
        activation_lock.ensure_canonical()?;
    }

    if intent.is_none() {
        ensure!(
            !pinned_entry_exists(&state, &staging_name)?,
            "runtime-fence staging exists before a durable authorization intent"
        );
        if pinned_entry_exists(&state, &intent_temporary_name)? {
            let authorized = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
            validate_runtime_fence_intent(
                &authorized.document,
                source_commit,
                staging_name_text,
                state_pinned.st_dev,
            )?;
            ensure!(
                authorized.document.phase == RuntimeFenceInitPhaseV1::Authorized,
                "initial runtime-fence .new is not an authorization intent"
            );
        } else {
            let authorized = RuntimeFenceInitIntentV1 {
                schema_version: 1,
                phase: RuntimeFenceInitPhaseV1::Authorized,
                source_commit: source_commit.to_owned(),
                staging_name: staging_name_text.to_owned(),
                staging_device: None,
                staging_inode: None,
            };
            write_runtime_fence_intent_new(
                &state,
                &intent_writing_name,
                &intent_temporary_name,
                &authorized,
                activation_lock,
                (
                    RuntimeFenceInitBoundaryV1::IntentWritingCreated,
                    RuntimeFenceInitBoundaryV1::IntentWritingSynced,
                    RuntimeFenceInitBoundaryV1::IntentNewPublished,
                ),
                &mut after_boundary,
            )?;
        }
        activation_lock.ensure_canonical()?;
        renameat_with(
            state.as_fd(),
            &intent_temporary_name,
            state.as_fd(),
            &intent_name,
            RenameFlags::NOREPLACE,
        )?;
        rustix::fs::fsync(&state)?;
        after_boundary(RuntimeFenceInitBoundaryV1::AuthorizedIntentPublished)?;
        intent = Some(pin_runtime_fence_intent(&state, &intent_name, &[0o400])?);
    }

    let mut intent = intent.context("runtime-fence initialization lacks a durable intent")?;
    if pinned_entry_exists(&state, &intent_temporary_name)? {
        let adjacent = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
        validate_runtime_fence_intent(
            &adjacent.document,
            source_commit,
            staging_name_text,
            state_pinned.st_dev,
        )?;
        match (intent.document.phase, adjacent.document.phase) {
            (RuntimeFenceInitPhaseV1::Authorized, RuntimeFenceInitPhaseV1::StagingBound) => {
                ensure!(
                    runtime_fence_named_identity(&state, &staging_name)?
                        == Some(runtime_fence_bound_identity(&adjacent.document)?),
                    "bound .new intent differs from the retained staging inode"
                );
                activation_lock.ensure_canonical()?;
                renameat_with(
                    state.as_fd(),
                    &intent_name,
                    state.as_fd(),
                    &intent_temporary_name,
                    RenameFlags::EXCHANGE,
                )?;
                rustix::fs::fsync(&state)?;
                after_boundary(RuntimeFenceInitBoundaryV1::BoundIntentExchanged)?;
                intent = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
            }
            (RuntimeFenceInitPhaseV1::StagingBound, RuntimeFenceInitPhaseV1::Authorized) => {}
            _ => anyhow::bail!("runtime-fence intents are not an exact adjacent transition"),
        }
        let predecessor = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
        ensure!(
            predecessor.document.phase == RuntimeFenceInitPhaseV1::Authorized,
            "runtime-fence .new does not contain the exact authorized predecessor"
        );
        activation_lock.ensure_canonical()?;
        remove_exact_runtime_fence_intent(&state, &intent_temporary_name, &predecessor)?;
        after_boundary(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)?;
    }

    if intent.document.phase == RuntimeFenceInitPhaseV1::Authorized {
        if !pinned_entry_exists(&state, &staging_name)? {
            activation_lock.ensure_canonical()?;
            mkdirat(state.as_fd(), &staging_name, Mode::from_raw_mode(0o700))?;
            rustix::fs::fsync(&state)?;
            after_boundary(RuntimeFenceInitBoundaryV1::StagingCreated)?;
        }
        let staging = open_fence(&staging_name)?;
        validate_fence(&staging, 0o700, true)?;
        let staging_metadata = rustix::fs::fstat(&staging)?;
        let bound = RuntimeFenceInitIntentV1 {
            schema_version: 1,
            phase: RuntimeFenceInitPhaseV1::StagingBound,
            source_commit: source_commit.to_owned(),
            staging_name: staging_name_text.to_owned(),
            staging_device: Some(staging_metadata.st_dev),
            staging_inode: Some(staging_metadata.st_ino),
        };
        write_runtime_fence_intent_new(
            &state,
            &intent_writing_name,
            &intent_temporary_name,
            &bound,
            activation_lock,
            (
                RuntimeFenceInitBoundaryV1::BoundIntentWritingCreated,
                RuntimeFenceInitBoundaryV1::BoundIntentWritingSynced,
                RuntimeFenceInitBoundaryV1::BoundIntentNewPublished,
            ),
            &mut after_boundary,
        )?;
        activation_lock.ensure_canonical()?;
        renameat_with(
            state.as_fd(),
            &intent_name,
            state.as_fd(),
            &intent_temporary_name,
            RenameFlags::EXCHANGE,
        )?;
        rustix::fs::fsync(&state)?;
        after_boundary(RuntimeFenceInitBoundaryV1::BoundIntentExchanged)?;
        let predecessor = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
        ensure!(
            predecessor.document == intent.document,
            "runtime-fence bound-intent exchange did not retain its exact predecessor"
        );
        intent = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
        ensure!(
            intent.document == bound,
            "runtime-fence bound-intent exchange did not publish its exact successor"
        );
        activation_lock.ensure_canonical()?;
        remove_exact_runtime_fence_intent(&state, &intent_temporary_name, &predecessor)?;
        after_boundary(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)?;
    }

    let staging_identity = runtime_fence_named_identity(&state, &staging_name)?;
    ensure!(
        staging_identity == Some(runtime_fence_bound_identity(&intent.document)?),
        "runtime-fence staging differs from its durable initializer intent"
    );
    let staging = open_fence(&staging_name)?;
    let staging_mode = rustix::fs::fstat(&staging)?.st_mode & 0o777;
    ensure!(
        staging_mode == 0o700 || staging_mode == 0o500,
        "runtime-fence staging has an unsafe mode"
    );
    validate_fence(&staging, staging_mode, true)?;
    activation_lock.ensure_canonical()?;
    fchmod(&staging, Mode::from_raw_mode(0o700))?;
    for (leaf_name, boundary) in [
        (
            "db-admission.lock",
            RuntimeFenceInitBoundaryV1::AdmissionLeafSynced,
        ),
        (
            "db-quiescence.lock",
            RuntimeFenceInitBoundaryV1::QuiescenceLeafSynced,
        ),
    ] {
        let leaf = match openat2(
            staging.as_fd(),
            leaf_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o400),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        ) {
            Ok(leaf) => leaf,
            Err(rustix::io::Errno::EXIST) => openat2(
                staging.as_fd(),
                leaf_name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?,
            Err(error) => return Err(error.into()),
        };
        let metadata = rustix::fs::fstat(&leaf)?;
        fd_policy::ensure_private_regular(
            metadata.st_mode,
            metadata.st_uid,
            metadata.st_nlink as u64,
            &format!("runtime-fence leaf creation did not produce exact authority"),
        )?;
        ensure!(
            metadata.st_dev == state_pinned.st_dev
                && metadata.st_size == 0
                && metadata.st_mode & 0o777 == 0o400,
            "runtime-fence leaf creation did not produce exact authority"
        );
        rustix::fs::fsync(&leaf)?;
        rustix::fs::fsync(&staging)?;
        after_boundary(boundary)?;
    }
    activation_lock.ensure_canonical()?;
    fchmod(&staging, Mode::from_raw_mode(0o500))?;
    rustix::fs::fsync(&staging)?;
    validate_fence(&staging, 0o500, false)?;
    after_boundary(RuntimeFenceInitBoundaryV1::StagingSealed)?;
    let staging_metadata = rustix::fs::fstat(&staging)?;
    activation_lock.ensure_canonical()?;
    let rename = renameat_with(
        state.as_fd(),
        &staging_name,
        state.as_fd(),
        &final_name,
        RenameFlags::NOREPLACE,
    );
    if let Err(error) = rename {
        let final_metadata = statat(state.as_fd(), &final_name, AtFlags::SYMLINK_NOFOLLOW);
        if !final_metadata.as_ref().is_ok_and(|metadata| {
            metadata.st_dev == staging_metadata.st_dev && metadata.st_ino == staging_metadata.st_ino
        }) {
            return Err(error.into());
        }
    }
    rustix::fs::fsync(&state)?;
    after_boundary(RuntimeFenceInitBoundaryV1::FinalPublished)?;
    let final_root = open_fence(&final_name)?;
    let final_metadata = rustix::fs::fstat(&final_root)?;
    ensure!(
        (final_metadata.st_dev, final_metadata.st_ino)
            == runtime_fence_bound_identity(&intent.document)?,
        "runtime-fence final differs from its durable initializer intent"
    );
    validate_fence(&final_root, 0o500, false)?;
    ensure!(
        !pinned_entry_exists(&state, &staging_name)?,
        "runtime-fence staging remains after publication"
    );
    activation_lock.ensure_canonical()?;
    remove_exact_runtime_fence_intent(&state, &intent_name, &intent)?;
    after_boundary(RuntimeFenceInitBoundaryV1::IntentRemoved)?;
    activation_lock.ensure_canonical()?;
    Ok(())
}
