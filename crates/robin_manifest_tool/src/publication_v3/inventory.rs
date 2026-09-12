//! inventory responsibilities of the admitted release pipeline.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PublicationNodeIdentityV3 {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) group: u32,
    pub(crate) links: u64,
    pub(crate) mode: u32,
    pub(crate) length: u64,
    pub(crate) modified_seconds: i64,
    pub(crate) modified_nanoseconds: i64,
    pub(crate) changed_seconds: i64,
    pub(crate) changed_nanoseconds: i64,
}

#[derive(Debug)]
pub(super) struct PublicationFileInventoryV3 {
    pub(super) path: String,
    pub(super) file: fs::File,
    pub(super) artifact: ArtifactRefV1,
    pub(super) unix_mode: u32,
    pub(super) identity: PublicationNodeIdentityV3,
}

#[derive(Debug)]
pub(super) struct PublicationTreeInventoryV3 {
    pub(super) files: Vec<PublicationFileInventoryV3>,
    pub(super) directories: Vec<PublicationDirectoryV3>,
    pub(super) directory_identities: Vec<(String, PublicationNodeIdentityV3)>,
    pub(super) directory_files: Vec<(String, fs::File)>,
}

/// One fully validated PublicationV3 authority that retains the exact root,
/// file, and directory descriptors used for semantic validation. Consumers
/// must keep this value alive through extraction and call `ensure_live` at
/// their acceptance boundary.
#[derive(Debug)]
pub(crate) struct ValidatedPublicationV3 {
    pub(super) root_path: PathBuf,
    pub(super) root: fs::File,
    pub(super) root_parent_path: PathBuf,
    pub(super) root_parent: fs::File,
    pub(super) root_parent_identity: PublicationNodeIdentityV3,
    pub(super) root_name: std::ffi::OsString,
    pub(super) inventory: PublicationTreeInventoryV3,
    pub(super) lock_sha256: Digest32,
}

impl ValidatedPublicationV3 {
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn synthetic_for_consumer_test(root_path: &Path) -> Result<Self> {
        let root = open_publication_root_v3(root_path)?;
        let (root_parent_path, root_parent, root_name) = pin_publication_root_parent_v3(root_path)?;
        Ok(Self {
            root_path: root_path.to_path_buf(),
            root: root.try_clone()?,
            root_parent_path,
            root_parent_identity: publication_node_identity_v3(&root_parent.metadata()?),
            root_parent,
            root_name,
            inventory: publication_tree_inventory_v3_from_fd(root_path, &root)?,
            lock_sha256: Digest32::digest_bytes(b"synthetic PublicationV3 consumer lock"),
        })
    }

    pub(crate) const fn lock_sha256(&self) -> Digest32 {
        self.lock_sha256
    }

    pub(crate) fn load_document<T>(&mut self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
    {
        load_inventory_document_v3(&mut self.inventory, path)
    }

    pub(crate) fn artifact(&self, path: &str, media_type: &str) -> Result<ArtifactRefV1> {
        inventory_artifact_v3(&self.inventory, path, media_type)
    }

    pub(crate) fn relative_files(&self, prefix: &str) -> Vec<String> {
        inventory_relative_files_v3(&self.inventory, prefix)
            .into_iter()
            .collect()
    }

    pub(crate) fn relative_directories(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.trim_end_matches('/');
        let nested_prefix = format!("{prefix}/");
        self.inventory
            .directories
            .iter()
            .filter_map(|directory| {
                if directory.path == prefix {
                    Some(".".to_owned())
                } else {
                    directory
                        .path
                        .strip_prefix(&nested_prefix)
                        .map(str::to_owned)
                }
            })
            .collect()
    }

    pub(crate) fn copy_file_to(
        &mut self,
        path: &str,
        output: &mut fs::File,
    ) -> Result<ArtifactRefV1> {
        use std::os::unix::fs::MetadataExt as _;

        let source = self
            .inventory
            .files
            .iter_mut()
            .find(|file| file.path == path)
            .with_context(|| format!("validated PublicationV3 omits {path}"))?;
        let output_before = output.metadata()?;
        ensure!(
            output_before.is_file()
                && output_before.nlink() == 1
                && output_before.uid() == rustix::process::geteuid().as_raw()
                && output_before.len() == 0,
            "PublicationV3 extraction output is not an empty owned singleton file"
        );
        source.file.seek(std::io::SeekFrom::Start(0))?;
        output.seek(std::io::SeekFrom::Start(0))?;
        let copied = std::io::copy(&mut source.file, output)?;
        ensure!(
            copied == source.artifact.byte_length
                && publication_node_identity_v3(&source.file.metadata()?) == source.identity,
            "validated PublicationV3 source changed while extracting {path}"
        );
        output.sync_all()?;
        let output_identity = publication_node_identity_v3(&output.metadata()?);
        ensure!(
            stable_publication_file_artifact_v3(output, &output_identity, path)? == source.artifact,
            "PublicationV3 extraction changed bytes at {path}"
        );
        Ok(source.artifact.clone())
    }

    pub(crate) fn ensure_live(&self) -> Result<()> {
        let rebound_parent = open_publication_root_v3(&self.root_parent_path)?;
        ensure!(
            publication_same_stable_node_v3(
                &publication_node_identity_v3(&self.root_parent.metadata()?),
                &self.root_parent_identity,
            ) && publication_same_stable_node_v3(
                &publication_node_identity_v3(&rebound_parent.metadata()?),
                &self.root_parent_identity,
            ),
            "validated PublicationV3 root parent was substituted"
        );
        let rebound_root = open_publication_child_v3(&rebound_parent, Path::new(&self.root_name))?;
        ensure!(
            publication_node_identity_v3(&rebound_root.metadata()?)
                == publication_node_identity_v3(&self.root.metadata()?),
            "validated PublicationV3 root path was substituted"
        );
        ensure!(
            publication_tree_inventory_v3_from_fd(&self.root_path, &self.root)?.snapshot()
                == self.inventory.snapshot(),
            "validated PublicationV3 changed before consumer acceptance"
        );
        Ok(())
    }

    pub(super) fn sync_exact_tree(&self) -> Result<()> {
        for file in &self.inventory.files {
            file.file.sync_all()?;
        }
        let mut directories = self.inventory.directory_files.iter().collect::<Vec<_>>();
        directories
            .sort_by_key(|(path, _)| std::cmp::Reverse(Path::new(path).components().count()));
        for (_, directory) in directories {
            directory.sync_all()?;
        }
        self.ensure_live()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicationTreeSnapshotV3 {
    pub(crate) files: Vec<(String, ArtifactRefV1, u32, PublicationNodeIdentityV3)>,
    pub(crate) directories: Vec<(PublicationDirectoryV3, PublicationNodeIdentityV3)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicationTreeAuthorityV3 {
    pub(super) files: Vec<(String, ArtifactRefV1, u32)>,
    pub(super) directories: Vec<PublicationDirectoryV3>,
}

impl PublicationTreeInventoryV3 {
    pub(super) fn snapshot(&self) -> PublicationTreeSnapshotV3 {
        PublicationTreeSnapshotV3 {
            files: self
                .files
                .iter()
                .map(|file| {
                    (
                        file.path.clone(),
                        file.artifact.clone(),
                        file.unix_mode,
                        file.identity.clone(),
                    )
                })
                .collect(),
            directories: self
                .directories
                .iter()
                .cloned()
                .zip(
                    self.directory_identities
                        .iter()
                        .map(|(_, identity)| identity.clone()),
                )
                .collect(),
        }
    }

    pub(super) fn authority(&self) -> PublicationTreeAuthorityV3 {
        PublicationTreeAuthorityV3 {
            files: self
                .files
                .iter()
                .map(|file| (file.path.clone(), file.artifact.clone(), file.unix_mode))
                .collect(),
            directories: self.directories.clone(),
        }
    }
}

pub(super) fn publication_inventory_matches_after_root_rename_v3(
    before: &PublicationTreeInventoryV3,
    after: &PublicationTreeInventoryV3,
) -> bool {
    before.authority() == after.authority()
        && before
            .files
            .iter()
            .zip(&after.files)
            .all(|(left, right)| left.path == right.path && left.identity == right.identity)
        && before
            .directory_identities
            .iter()
            .zip(&after.directory_identities)
            .all(|((left_path, left), (right_path, right))| {
                left_path == right_path
                    && if left_path == "." {
                        publication_same_stable_node_v3(left, right)
                    } else {
                        left == right
                    }
            })
}

#[cfg(target_os = "linux")]
pub(super) fn read_inventory_file_v3(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
    maximum: u64,
) -> Result<Vec<u8>> {
    let file = inventory
        .files
        .iter_mut()
        .find(|file| file.path == path)
        .with_context(|| format!("PublicationV3 inventory omits {path}"))?;
    ensure!(
        file.artifact.byte_length <= maximum,
        "PublicationV3 file {path} exceeds its read bound"
    );
    let capacity = usize::try_from(file.artifact.byte_length)
        .context("PublicationV3 file length does not fit usize")?;
    file.file.seek(std::io::SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.file.read_to_end(&mut bytes)?;
    ensure!(
        u64::try_from(bytes.len()).ok() == Some(file.artifact.byte_length)
            && Digest32::digest_bytes(&bytes) == file.artifact.sha256
            && publication_node_identity_v3(&file.file.metadata()?) == file.identity,
        "PublicationV3 file changed while read at {path}"
    );
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub(super) fn load_inventory_canonical_document_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = read_inventory_file_v3(inventory, path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical PublicationV3 document {path}"))?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "PublicationV3 document is not byte-for-byte canonical at {path}"
    );
    Ok(document)
}

#[cfg(target_os = "linux")]
pub(super) fn load_inventory_document_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let document: T = load_inventory_canonical_document_v3(inventory, path)?;
    document.validate()?;
    Ok(document)
}

#[cfg(target_os = "linux")]
pub(super) fn inventory_artifact_v3(
    inventory: &PublicationTreeInventoryV3,
    path: &str,
    media_type: &str,
) -> Result<ArtifactRefV1> {
    let file = inventory
        .files
        .iter()
        .find(|file| file.path == path)
        .with_context(|| format!("PublicationV3 inventory omits {path}"))?;
    let mut artifact = file.artifact.clone();
    artifact.media_type = media_type.into();
    Ok(artifact)
}

pub(super) fn inventory_relative_files_v3(
    inventory: &PublicationTreeInventoryV3,
    prefix: &str,
) -> BTreeSet<String> {
    let prefix = format!("{}/", prefix.trim_end_matches('/'));
    inventory
        .files
        .iter()
        .filter_map(|file| file.path.strip_prefix(&prefix).map(str::to_owned))
        .collect()
}

pub(super) fn inventory_has_directory_v3(
    inventory: &PublicationTreeInventoryV3,
    path: &str,
) -> bool {
    inventory
        .directories
        .binary_search_by(|directory| directory.path.as_str().cmp(path))
        .is_ok()
}

#[cfg(target_os = "linux")]
pub(super) fn publication_node_identity_v3(metadata: &fs::Metadata) -> PublicationNodeIdentityV3 {
    use std::os::unix::fs::MetadataExt as _;

    PublicationNodeIdentityV3 {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        group: metadata.gid(),
        links: metadata.nlink(),
        mode: metadata.mode(),
        length: metadata.size(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

pub(super) fn publication_same_stable_node_v3(
    left: &PublicationNodeIdentityV3,
    right: &PublicationNodeIdentityV3,
) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.owner == right.owner
        && left.group == right.group
        && left.mode == right.mode
}

#[cfg(target_os = "linux")]
pub(super) fn open_publication_root_v3(root: &Path) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

    let descriptor = openat2(
        rustix::fs::CWD,
        root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .with_context(|| format!("pin PublicationV3 root {}", root.display()))?;
    Ok(fs::File::from(descriptor))
}

#[cfg(target_os = "linux")]
pub(super) fn pin_publication_root_parent_v3(
    root: &Path,
) -> Result<(PathBuf, fs::File, std::ffi::OsString)> {
    let parent_path = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let name = root
        .file_name()
        .context("PublicationV3 root has no basename")?
        .to_owned();
    let parent = open_publication_root_v3(&parent_path)?;
    Ok((parent_path, parent, name))
}

#[cfg(target_os = "linux")]
pub(super) fn open_publication_child_v3(parent: &fs::File, name: &Path) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let descriptor = openat2(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    Ok(fs::File::from(descriptor))
}

#[cfg(target_os = "linux")]
pub(super) fn open_publication_child_identity_v3(
    parent: &fs::File,
    name: &Path,
) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let descriptor = openat2(
        parent.as_fd(),
        name,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    Ok(fs::File::from(descriptor))
}

#[cfg(target_os = "linux")]
pub(super) fn open_optional_publication_child_identity_v3(
    parent: &fs::File,
    name: &Path,
) -> Result<Option<fs::File>> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    match openat2(
        parent.as_fd(),
        name,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    ) {
        Ok(descriptor) => Ok(Some(fs::File::from(descriptor))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn publication_directory_entries_v3(
    directory: &fs::File,
) -> Result<Vec<(std::ffi::OsString, u64, rustix::fs::FileType)>> {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let mut reader = rustix::fs::Dir::read_from(directory)?;
    let mut entries = Vec::new();
    for entry in &mut reader {
        let entry = entry?;
        let bytes = entry.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        ensure!(
            !bytes.is_empty() && !bytes.contains(&b'/'),
            "PublicationV3 directory contains an invalid entry name"
        );
        entries.push((
            std::ffi::OsString::from_vec(bytes.to_vec()),
            entry.ino(),
            entry.file_type(),
        ));
    }
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(entries)
}

#[cfg(target_os = "linux")]
pub(super) fn validate_publication_node_v3(
    identity: &PublicationNodeIdentityV3,
    expected_uid: u32,
    expected_device: u64,
    path: &str,
) -> Result<()> {
    ensure!(
        identity.owner == expected_uid && identity.device == expected_device,
        "PublicationV3 contains mixed ownership or devices at {path}"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn stable_publication_file_artifact_v3(
    file: &mut fs::File,
    expected_identity: &PublicationNodeIdentityV3,
    path: &str,
) -> Result<ArtifactRefV1> {
    stable_publication_file_artifact_v3_with(file, expected_identity, path, || {})
}

#[cfg(target_os = "linux")]
pub(super) fn stable_publication_file_artifact_v3_with<F>(
    file: &mut fs::File,
    expected_identity: &PublicationNodeIdentityV3,
    path: &str,
    between_hashes: F,
) -> Result<ArtifactRefV1>
where
    F: FnOnce(),
{
    file.seek(std::io::SeekFrom::Start(0))?;
    let first = Digest32::digest_reader(BufReader::new(&mut *file))?;
    let after_first = publication_node_identity_v3(&file.metadata()?);
    between_hashes();
    file.seek(std::io::SeekFrom::Start(0))?;
    let second = Digest32::digest_reader(BufReader::new(&mut *file))?;
    let after_second = publication_node_identity_v3(&file.metadata()?);
    ensure!(
        expected_identity == &after_first && expected_identity == &after_second && first == second,
        "PublicationV3 file changed while pinned and hashed at {path}"
    );
    Ok(ArtifactRefV1 {
        sha256: first,
        byte_length: expected_identity.length,
        media_type: "application/octet-stream".into(),
    })
}

/// Enumerate and hash the complete PublicationV3 tree through pinned dirfds.
#[cfg(target_os = "linux")]
pub(super) fn publication_tree_inventory_v3_from_fd(
    root_path: &Path,
    root: &fs::File,
) -> Result<PublicationTreeInventoryV3> {
    publication_tree_inventory_v3_from_fd_with(root_path, root, || {})
}

#[cfg(target_os = "linux")]
pub(super) fn publication_tree_inventory_v3_from_fd_with<F>(
    root_path: &Path,
    root: &fs::File,
    after_walk: F,
) -> Result<PublicationTreeInventoryV3>
where
    F: FnOnce(),
{
    let root_metadata = root.metadata()?;
    ensure!(
        root_metadata.is_dir(),
        "PublicationV3 root is not a directory"
    );
    let root_identity = publication_node_identity_v3(&root_metadata);
    let (root_parent_path, root_parent, root_name) = pin_publication_root_parent_v3(root_path)?;
    let root_parent_identity = publication_node_identity_v3(&root_parent.metadata()?);
    let rooted = open_publication_child_v3(&root_parent, Path::new(&root_name))?;
    ensure!(
        publication_node_identity_v3(&rooted.metadata()?) == root_identity,
        "PublicationV3 root path differs from its pinned descriptor"
    );
    let expected_uid = rustix::process::geteuid().as_raw();
    let expected_device = root_identity.device;
    validate_publication_node_v3(&root_identity, expected_uid, expected_device, ".")?;
    ensure!(expected_device != 0, "PublicationV3 root device is zero");

    struct InventoryBuilder {
        files: Vec<PublicationFileInventoryV3>,
        directories: Vec<PublicationDirectoryV3>,
        directory_identities: Vec<(String, PublicationNodeIdentityV3)>,
        directory_files: Vec<(String, fs::File)>,
        seen: usize,
        expected_uid: u32,
        expected_device: u64,
    }

    fn walk(
        directory: &fs::File,
        relative: &Path,
        depth: usize,
        builder: &mut InventoryBuilder,
    ) -> Result<()> {
        use rustix::fs::FileType;

        ensure!(
            depth <= MAX_PUBLICATION_TREE_DEPTH,
            "PublicationV3 tree exceeds its depth bound"
        );
        let before = publication_node_identity_v3(&directory.metadata()?);
        let path = if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            path_to_manifest(relative)?
        };
        validate_publication_node_v3(
            &before,
            builder.expected_uid,
            builder.expected_device,
            &path,
        )?;
        ensure!(
            directory.metadata()?.is_dir(),
            "PublicationV3 node is not a directory"
        );
        builder.directories.push(PublicationDirectoryV3 {
            path: path.clone(),
            unix_mode: before.mode & 0o7777,
        });
        builder
            .directory_identities
            .push((path.clone(), before.clone()));
        builder.directory_files.push((path, directory.try_clone()?));

        let entries = publication_directory_entries_v3(directory)?;
        for (name, observed_inode, observed_type) in &entries {
            builder.seen = builder
                .seen
                .checked_add(1)
                .context("PublicationV3 entry count overflow")?;
            ensure!(
                builder.seen <= MAX_PUBLICATION_TREE_ENTRIES,
                "PublicationV3 tree exceeds its entry bound"
            );
            let child_relative = relative.join(name);
            let child_path = path_to_manifest(&child_relative)?;
            let observed_child = open_publication_child_identity_v3(directory, Path::new(name))
                .with_context(|| format!("pin PublicationV3 child {child_path}"))?;
            let child_metadata = observed_child.metadata()?;
            let identity = publication_node_identity_v3(&child_metadata);
            validate_publication_node_v3(
                &identity,
                builder.expected_uid,
                builder.expected_device,
                &child_path,
            )?;
            ensure!(
                *observed_inode == 0 || *observed_inode == identity.inode,
                "PublicationV3 directory entry inode changed at {child_path}"
            );
            let opened_type = if child_metadata.is_dir() {
                FileType::Directory
            } else if child_metadata.is_file() {
                FileType::RegularFile
            } else {
                FileType::Unknown
            };
            ensure!(
                *observed_type == FileType::Unknown || *observed_type == opened_type,
                "PublicationV3 directory entry type changed at {child_path}"
            );
            match opened_type {
                FileType::Directory => {
                    let child = open_publication_child_v3(directory, Path::new(name))?;
                    ensure!(
                        publication_node_identity_v3(&child.metadata()?) == identity,
                        "PublicationV3 directory was substituted while opened at {child_path}"
                    );
                    walk(&child, &child_relative, depth + 1, builder)?;
                }
                FileType::RegularFile => {
                    ensure!(
                        identity.links == 1,
                        "PublicationV3 contains hard-linked file {child_path}"
                    );
                    let mut child = open_publication_child_v3(directory, Path::new(name))?;
                    ensure!(
                        publication_node_identity_v3(&child.metadata()?) == identity,
                        "PublicationV3 file was substituted while opened at {child_path}"
                    );
                    let artifact =
                        stable_publication_file_artifact_v3(&mut child, &identity, &child_path)?;
                    let unix_mode = identity.mode & 0o7777;
                    builder.files.push(PublicationFileInventoryV3 {
                        path: child_path.clone(),
                        file: child,
                        artifact,
                        unix_mode,
                        identity: identity.clone(),
                    });
                }
                _ => anyhow::bail!("PublicationV3 contains special node {child_path}"),
            }
            let rebound = open_publication_child_v3(directory, Path::new(name))?;
            ensure!(
                publication_node_identity_v3(&rebound.metadata()?) == identity,
                "PublicationV3 child was substituted after use at {child_path}"
            );
        }
        let rebound_entries = publication_directory_entries_v3(directory)?;
        ensure!(
            rebound_entries == entries,
            "PublicationV3 directory entries changed while traversing {relative:?}"
        );
        ensure!(
            publication_node_identity_v3(&directory.metadata()?) == before,
            "PublicationV3 directory changed while traversing {relative:?}"
        );
        Ok(())
    }

    let mut builder = InventoryBuilder {
        files: Vec::new(),
        directories: Vec::new(),
        directory_identities: Vec::new(),
        directory_files: Vec::new(),
        seen: 1,
        expected_uid,
        expected_device,
    };
    walk(root, Path::new(""), 0, &mut builder)?;
    after_walk();
    builder
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    builder
        .directories
        .sort_by(|left, right| left.path.cmp(&right.path));
    builder
        .directory_identities
        .sort_by(|left, right| left.0.cmp(&right.0));
    builder
        .directory_files
        .sort_by(|left, right| left.0.cmp(&right.0));
    ensure!(
        builder
            .directories
            .first()
            .is_some_and(|entry| entry.path == ".")
            && builder
                .directories
                .iter()
                .map(|entry| entry.path.as_str())
                .eq(builder
                    .directory_identities
                    .iter()
                    .map(|(path, _)| path.as_str()))
            && builder
                .directories
                .iter()
                .map(|entry| entry.path.as_str())
                .eq(builder
                    .directory_files
                    .iter()
                    .map(|(path, _)| path.as_str())),
        "PublicationV3 directory inventory is incomplete or misbound"
    );
    let rebound_parent = open_publication_root_v3(&root_parent_path)?;
    ensure!(
        publication_same_stable_node_v3(
            &publication_node_identity_v3(&rebound_parent.metadata()?),
            &root_parent_identity,
        ),
        "PublicationV3 root parent path was substituted during traversal"
    );
    let rebound_root = open_publication_child_v3(&rebound_parent, Path::new(&root_name))?;
    ensure!(
        publication_node_identity_v3(&rebound_root.metadata()?) == root_identity,
        "PublicationV3 root path was substituted during traversal"
    );
    for (directory, (_, expected_identity)) in builder
        .directories
        .iter()
        .zip(&builder.directory_identities)
    {
        if directory.path == "." {
            continue;
        }
        let rebound = open_publication_child_v3(root, Path::new(&directory.path))?;
        ensure!(
            rebound.metadata()?.is_dir()
                && publication_node_identity_v3(&rebound.metadata()?) == *expected_identity,
            "PublicationV3 directory failed final root-relative rebind at {}",
            directory.path
        );
    }
    for expected in &builder.files {
        let rebound = open_publication_child_v3(root, Path::new(&expected.path))?;
        ensure!(
            rebound.metadata()?.is_file()
                && publication_node_identity_v3(&rebound.metadata()?) == expected.identity,
            "PublicationV3 file failed final root-relative rebind at {}",
            expected.path
        );
    }
    Ok(PublicationTreeInventoryV3 {
        files: builder.files,
        directories: builder.directories,
        directory_identities: builder.directory_identities,
        directory_files: builder.directory_files,
    })
}

pub(super) fn publication_tree_inventory_v3(root: &Path) -> Result<PublicationTreeInventoryV3> {
    #[cfg(target_os = "linux")]
    {
        let descriptor = open_publication_root_v3(root)?;
        publication_tree_inventory_v3_from_fd(root, &descriptor)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        anyhow::bail!("PublicationV3 requires Linux openat2 filesystem authority")
    }
}

#[cfg(test)]
pub(super) fn publication_directories(root: &Path) -> Result<Vec<PublicationDirectoryV3>> {
    Ok(publication_tree_inventory_v3(root)?.directories)
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
pub(super) fn reject_publication_mounts_in_v3(
    canonical_root: &Path,
    mountinfo: &[u8],
) -> Result<()> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    for line in mountinfo
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let encoded = line
            .split(|byte| *byte == b' ')
            .nth(4)
            .context("malformed /proc/self/mountinfo line")?;
        let mut decoded = Vec::with_capacity(encoded.len());
        let mut index = 0;
        while index < encoded.len() {
            if encoded[index] == b'\\'
                && index + 3 < encoded.len()
                && encoded[index + 1..index + 4]
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'7'))
            {
                decoded.push(
                    (encoded[index + 1] - b'0') * 64
                        + (encoded[index + 2] - b'0') * 8
                        + (encoded[index + 3] - b'0'),
                );
                index += 4;
            } else {
                decoded.push(encoded[index]);
                index += 1;
            }
        }
        let mount = PathBuf::from(OsString::from_vec(decoded));
        ensure!(
            mount != canonical_root && !mount.starts_with(canonical_root),
            "PublicationV3 contains mount point {}",
            mount.display()
        );
    }
    Ok(())
}

#[cfg(all(unix, test))]
pub(super) fn publication_unix_mode(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
}

#[cfg(all(not(unix), test))]
pub(super) fn publication_unix_mode(_path: &Path) -> Result<u32> {
    anyhow::bail!("operator publications require Unix permission semantics")
}

pub(super) fn file_artifacts(root: &Path) -> Result<BTreeMap<String, ArtifactRefV1>> {
    publication_tree_inventory_v3(root)?
        .files
        .into_iter()
        .map(|file| Ok((file.path, file.artifact)))
        .collect()
}

pub(super) fn mutable_transition_path(path: &str) -> bool {
    path.starts_with("backend/manifests/published-rulesets/")
        || matches!(
            path,
            "backend/publication-v3.json"
                | "publication-manifest-v3.json"
                | "publication-manifest-v3.sha256"
                | "publication-lock-v3.json"
                | "publication-lock-v3.sha256"
        )
}

pub(super) fn immutable_transition_files(
    files: &BTreeMap<String, ArtifactRefV1>,
) -> BTreeMap<&str, &ArtifactRefV1> {
    files
        .iter()
        .filter(|(path, _)| !mutable_transition_path(path))
        .map(|(path, artifact)| (path.as_str(), artifact))
        .collect()
}

pub(super) fn status_transition_files(
    files: &BTreeMap<String, ArtifactRefV1>,
) -> BTreeMap<&str, &ArtifactRefV1> {
    files
        .iter()
        .filter(|(path, _)| path.starts_with("backend/manifests/published-rulesets/"))
        .map(|(path, artifact)| (path.as_str(), artifact))
        .collect()
}
