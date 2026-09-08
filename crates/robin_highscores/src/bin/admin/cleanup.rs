//! Authenticated exact-owned backup cleanup, recovery, and retention.

use super::filesystem::BackupTreePaths;
use super::filesystem::FileIdentity;
use super::filesystem::backup_tree_paths_cap;
use super::filesystem::cap_entry_exists;
use super::filesystem::cleanup_tombstone_relative;
use super::filesystem::duplicate_pinned_file;
use super::filesystem::expected_backup_cleanup_file_mode;
use super::filesystem::metadata_identity;
use super::filesystem::metadata_identity_std;
use super::filesystem::open_cap_directory_nofollow;
use super::filesystem::open_cap_regular_nofollow;
use super::filesystem::pin_directory_capability;
use super::filesystem::read_bounded_pinned_file;
use super::filesystem::read_cap_regular_bounded;
use super::filesystem::read_cap_regular_bounded_with_mode;
use super::filesystem::record_cap_file_with_identity;
use super::filesystem::remove_pinned_regular_via_tombstone;
use super::filesystem::sync_cap_directory;
use super::filesystem::unlink_pinned_regular;
use super::filesystem::unlink_pinned_regular_with_hook;
use super::filesystem::valid_complete_backup_name;
use super::filesystem::valid_partial_backup_name;
use super::filesystem::validate_managed_directory_tree;
use super::filesystem::validate_managed_metadata;
use super::filesystem::validate_private_pinned_file;
use super::sources::load_preserved_release_authority;
use super::verification::verify_backup_with_schema_policy;
use robin_highscores::Database;
use robin_highscores::backup::BackupCleanupJournalV1;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupReleaseIdentityV2;
use robin_highscores::backup::BackupVerificationEnvelopeV2;
use robin_highscores::backup::parse_backup_id;
use robin_run_protocol::canonical_json_bytes;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

pub(super) fn recover_stale_partial_backups(backup_root: &Path) -> anyhow::Result<()> {
    let root = pin_directory_capability(backup_root)?;
    let root_metadata = root.dir_metadata()?;
    let mut visited = 0_usize;
    for entry in root.entries()? {
        let entry = entry?;
        visited = visited
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("partial recovery entry count overflows"))?;
        anyhow::ensure!(
            visited <= 4_096,
            "backup root has too many entries to recover safely"
        );
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(".backup-v4-") {
            continue;
        }
        anyhow::ensure!(
            valid_partial_backup_name(name),
            "malformed managed partial backup name"
        );
        let directory = open_cap_directory_nofollow(&root, Path::new(name))?;
        validate_managed_directory_tree(&directory, &root_metadata)?;
        directory.remove_open_dir_all()?;
        match root.symlink_metadata(name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => anyhow::bail!(
                "managed partial backup name was substituted during cleanup; replacement was preserved"
            ),
            Err(error) => return Err(error.into()),
        }
        sync_cap_directory(&root)?;
    }
    Ok(())
}

pub(super) fn recover_interrupted_complete_cleanups(
    backup_root: &Path,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<()> {
    let root = pin_directory_capability(backup_root)?;
    let backup_root_identity = metadata_identity(&root.dir_metadata()?);
    let mut journals = Vec::new();
    let mut cleanup_directories = BTreeSet::new();
    let mut terminal_cleanup_directories = BTreeSet::new();
    let mut partial_journals = Vec::new();
    let mut discarded_partial_journals = Vec::new();
    let mut visited = 0_usize;
    for entry in root.entries()? {
        let entry = entry?;
        visited = visited
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("cleanup recovery entry count overflows"))?;
        anyhow::ensure!(
            visited <= 4_096,
            "backup root has too many entries to recover cleanup safely"
        );
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with(".cleanup-terminal-backup-v4-") {
            let backup_id = name
                .strip_prefix(".cleanup-terminal-")
                .ok_or_else(|| anyhow::anyhow!("malformed terminal cleanup directory name"))?;
            anyhow::ensure!(
                parse_backup_id(backup_id).is_some()
                    && name == format!(".cleanup-terminal-{backup_id}"),
                "noncanonical terminal cleanup directory name"
            );
            terminal_cleanup_directories.insert(name);
        } else if name.starts_with("..cleanup-backup-v4-")
            && name.ends_with(".journal.json.partial.discard")
        {
            let backup_id = name
                .strip_prefix("..cleanup-")
                .and_then(|value| value.strip_suffix(".journal.json.partial.discard"))
                .ok_or_else(|| anyhow::anyhow!("malformed discarded cleanup-journal partial"))?;
            let (_, _, expected_partial) = cleanup_names(backup_id)?;
            anyhow::ensure!(
                name == format!("{expected_partial}.discard"),
                "noncanonical discarded cleanup-journal partial"
            );
            discarded_partial_journals.push(name);
        } else if name.starts_with("..cleanup-backup-v4-") {
            let backup_id = name
                .strip_prefix("..cleanup-")
                .and_then(|value| value.strip_suffix(".journal.json.partial"))
                .ok_or_else(|| anyhow::anyhow!("malformed cleanup-journal partial name"))?;
            let (_, _, expected_partial) = cleanup_names(backup_id)?;
            anyhow::ensure!(
                name == expected_partial,
                "noncanonical cleanup-journal partial name"
            );
            partial_journals.push(name);
        } else if name.starts_with(".cleanup-backup-v4-") {
            if let Some(backup_id) = name
                .strip_prefix(".cleanup-")
                .and_then(|value| value.strip_suffix(".journal.json"))
            {
                let (_, expected_journal, _) = cleanup_names(backup_id)?;
                anyhow::ensure!(
                    name == expected_journal,
                    "noncanonical cleanup-journal name"
                );
                journals.push(name);
            } else {
                let backup_id = name
                    .strip_prefix(".cleanup-")
                    .ok_or_else(|| anyhow::anyhow!("malformed cleanup directory name"))?;
                let (expected_directory, _, _) = cleanup_names(backup_id)?;
                anyhow::ensure!(
                    name == expected_directory,
                    "noncanonical cleanup directory name"
                );
                cleanup_directories.insert(name);
            }
        }
    }
    for discard_name in discarded_partial_journals {
        let backup_id = discard_name
            .strip_prefix("..cleanup-")
            .and_then(|value| value.strip_suffix(".journal.json.partial.discard"))
            .ok_or_else(|| anyhow::anyhow!("malformed discarded cleanup-journal partial"))?;
        let (cleanup_name, final_name, _) = cleanup_names(backup_id)?;
        let discard = open_cap_regular_nofollow(&root, Path::new(&discard_name))?;
        validate_private_pinned_file(&discard, 0o400, "discarded cleanup journal partial")?;
        let discard_identity = metadata_identity_std(&discard.metadata()?);
        let discard_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&discard, false)?, 64 * 1024)?;
        let authenticated = serde_json::from_slice::<BackupCleanupJournalV1>(&discard_bytes)
            .ok()
            .filter(|journal| canonical_json_bytes(journal).ok().as_deref() == Some(&discard_bytes))
            .filter(|journal| journal.verify(backup_authority_key).is_ok())
            .is_some_and(|journal| journal.backup_id == backup_id);
        anyhow::ensure!(
            authenticated
                || (cap_entry_exists(&root, Path::new(backup_id))?
                    && !cap_entry_exists(&root, Path::new(&cleanup_name))?
                    && !cap_entry_exists(
                        &root,
                        Path::new(&format!(".cleanup-terminal-{backup_id}")),
                    )?
                    && !cap_entry_exists(&root, Path::new(&final_name))?),
            "discarded cleanup-journal partial has no authenticated recovery state"
        );
        unlink_pinned_regular_with_hook(
            &root,
            &discard_name,
            &discard,
            discard_identity,
            0o400,
            "discarded cleanup-journal partial",
            || Ok(()),
        )?;
    }
    for partial_name in partial_journals {
        let backup_id = partial_name
            .strip_prefix("..cleanup-")
            .and_then(|value| value.strip_suffix(".journal.json.partial"))
            .ok_or_else(|| anyhow::anyhow!("malformed cleanup-journal partial name"))?;
        let (cleanup_name, final_name, expected_partial) = cleanup_names(backup_id)?;
        anyhow::ensure!(
            partial_name == expected_partial,
            "noncanonical cleanup-journal partial"
        );
        let partial = open_cap_regular_nofollow(&root, Path::new(&partial_name))?;
        validate_private_pinned_file(&partial, 0o400, "cleanup journal partial")?;
        let partial_identity = metadata_identity_std(&partial.metadata()?);
        let partial_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&partial, false)?, 64 * 1024)?;
        let authenticated_partial =
            serde_json::from_slice::<BackupCleanupJournalV1>(&partial_bytes)
                .ok()
                .filter(|journal| {
                    canonical_json_bytes(journal).ok().as_deref() == Some(&partial_bytes)
                })
                .filter(|journal| journal.verify(backup_authority_key).is_ok())
                .filter(|journal| journal.backup_id == backup_id);
        let final_exists = cap_entry_exists(&root, Path::new(&final_name))?;
        if final_exists {
            anyhow::ensure!(
                authenticated_partial.is_some(),
                "an installed cleanup journal has a truncated or unauthenticated partial"
            );
            let final_file = open_cap_regular_nofollow(&root, Path::new(&final_name))?;
            validate_private_pinned_file(&final_file, 0o400, "cleanup journal")?;
            anyhow::ensure!(
                read_bounded_pinned_file(duplicate_pinned_file(&final_file, false)?, 64 * 1024)?
                    == partial_bytes,
                "cleanup journal final and partial bytes differ"
            );
        } else if authenticated_partial.is_none() {
            anyhow::ensure!(
                cap_entry_exists(&root, Path::new(backup_id))?
                    && !cap_entry_exists(&root, Path::new(&cleanup_name))?,
                "an unauthenticated cleanup-journal partial is not in a safe pre-rename state"
            );
            let complete = open_cap_directory_nofollow(&root, Path::new(backup_id))?;
            let complete_metadata = complete.dir_metadata()?;
            validate_managed_metadata(&complete_metadata, &root.dir_metadata()?, true)?;
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt as _;
                anyhow::ensure!(
                    complete_metadata.permissions().mode() & 0o777 == 0o700,
                    "pre-rename complete backup has noncanonical mode"
                );
            }
        }
        anyhow::ensure!(
            metadata_identity(&root.symlink_metadata(&partial_name)?) == partial_identity,
            "cleanup journal partial was substituted before removal"
        );
        remove_pinned_regular_via_tombstone(
            &root,
            &partial_name,
            &format!("{partial_name}.discard"),
            &partial,
            partial_identity,
            0o400,
            &partial_bytes,
            64 * 1024,
            "cleanup journal partial",
        )?;
    }
    for journal_name in journals {
        let journal_file = open_cap_regular_nofollow(&root, Path::new(&journal_name))?;
        validate_private_pinned_file(&journal_file, 0o400, "cleanup journal")?;
        let journal_identity = metadata_identity_std(&journal_file.metadata()?);
        let journal_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&journal_file, false)?, 64 * 1024)?;
        let journal: BackupCleanupJournalV1 = serde_json::from_slice(&journal_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&journal)? == journal_bytes,
            "cleanup journal is not canonical JSON"
        );
        journal.verify(backup_authority_key)?;
        anyhow::ensure!(
            backup_root_identity
                == FileIdentity {
                    device: journal.backup_root_device_id,
                    inode: journal.backup_root_inode,
                    owner: journal.backup_root_owner,
                },
            "cleanup journal belongs to a different backup-root inode"
        );
        anyhow::ensure!(
            journal_name == format!("{}.journal.json", journal.cleanup_directory_name),
            "cleanup journal filename differs from its authenticated directory"
        );
        let complete_exists = cap_entry_exists(&root, Path::new(&journal.backup_id))?;
        let cleanup_exists = cap_entry_exists(&root, Path::new(&journal.cleanup_directory_name))?;
        let terminal_exists =
            cap_entry_exists(&root, Path::new(&journal.terminal_cleanup_directory_name))?;
        anyhow::ensure!(
            usize::from(complete_exists)
                + usize::from(cleanup_exists)
                + usize::from(terminal_exists)
                <= 1,
            "cleanup journal names multiple live cleanup states"
        );
        if complete_exists {
            let complete = open_cap_directory_nofollow(&root, Path::new(&journal.backup_id))?;
            anyhow::ensure!(
                metadata_identity(&complete.dir_metadata()?)
                    == FileIdentity {
                        device: journal.cleanup_root_device_id,
                        inode: journal.cleanup_root_inode,
                        owner: journal.cleanup_root_owner,
                    },
                "pre-rename cleanup journal differs from the complete backup inode"
            );
            remove_pinned_cleanup_journal(
                &root,
                &journal_name,
                &journal_file,
                journal_identity,
                &journal_bytes,
            )?;
            continue;
        }
        if cleanup_exists || terminal_exists {
            resume_authenticated_cleanup(&root, &journal, backup_authority_key)?;
            cleanup_directories.remove(&journal.cleanup_directory_name);
            terminal_cleanup_directories.remove(&journal.terminal_cleanup_directory_name);
            continue;
        }
        remove_pinned_cleanup_journal(
            &root,
            &journal_name,
            &journal_file,
            journal_identity,
            &journal_bytes,
        )?;
    }
    anyhow::ensure!(
        cleanup_directories.is_empty() && terminal_cleanup_directories.is_empty(),
        "quarantined backup lacks an authenticated cleanup journal"
    );
    Ok(())
}

fn remove_pinned_cleanup_journal(
    backup_root: &cap_std::fs::Dir,
    journal_name: &str,
    journal_file: &std::fs::File,
    journal_identity: FileIdentity,
    journal_bytes: &[u8],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata_identity_std(&journal_file.metadata()?) == journal_identity
            && metadata_identity(&backup_root.symlink_metadata(journal_name)?) == journal_identity
            && read_bounded_pinned_file(duplicate_pinned_file(journal_file, false)?, 64 * 1024)?
                == journal_bytes,
        "cleanup journal was substituted before removal"
    );
    let backup_id = journal_name
        .strip_prefix(".cleanup-")
        .and_then(|value| value.strip_suffix(".journal.json"))
        .ok_or_else(|| anyhow::anyhow!("cleanup journal has a noncanonical filename"))?;
    let (_, _, partial_name) = cleanup_names(backup_id)?;
    remove_pinned_regular_via_tombstone(
        backup_root,
        journal_name,
        &partial_name,
        journal_file,
        journal_identity,
        0o400,
        journal_bytes,
        64 * 1024,
        "cleanup journal",
    )?;
    anyhow::ensure!(
        !cap_entry_exists(backup_root, Path::new(journal_name))?
            && !cap_entry_exists(backup_root, Path::new(&partial_name))?,
        "cleanup journal name remains after removal"
    );
    Ok(())
}

fn resume_authenticated_cleanup(
    backup_root: &cap_std::fs::Dir,
    journal: &BackupCleanupJournalV1,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<()> {
    resume_authenticated_cleanup_with_hooks(
        backup_root,
        journal,
        backup_authority_key,
        || Ok(()),
        || Ok(()),
    )
}

pub(super) fn resume_authenticated_cleanup_with_hooks<F, G>(
    backup_root: &cap_std::fs::Dir,
    journal: &BackupCleanupJournalV1,
    backup_authority_key: &[u8; 32],
    after_terminal_rename: F,
    after_terminal_root_remove: G,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
    G: FnOnce() -> anyhow::Result<()>,
{
    anyhow::ensure!(
        metadata_identity(&backup_root.dir_metadata()?)
            == FileIdentity {
                device: journal.backup_root_device_id,
                inode: journal.backup_root_inode,
                owner: journal.backup_root_owner,
            },
        "cleanup journal belongs to a different backup-root inode"
    );
    let (_, journal_name, _) = cleanup_names(&journal.backup_id)?;
    let journal_file = open_cap_regular_nofollow(backup_root, Path::new(&journal_name))?;
    validate_private_pinned_file(&journal_file, 0o400, "cleanup journal")?;
    let journal_identity = metadata_identity_std(&journal_file.metadata()?);
    let journal_bytes =
        read_bounded_pinned_file(duplicate_pinned_file(&journal_file, false)?, 64 * 1024)?;
    anyhow::ensure!(
        canonical_json_bytes(journal)? == journal_bytes,
        "cleanup journal path differs from the authenticated deletion plan"
    );
    let cleanup_exists = cap_entry_exists(backup_root, Path::new(&journal.cleanup_directory_name))?;
    let terminal_exists = cap_entry_exists(
        backup_root,
        Path::new(&journal.terminal_cleanup_directory_name),
    )?;
    anyhow::ensure!(
        cleanup_exists ^ terminal_exists,
        "cleanup journal must name exactly one quarantined root"
    );
    let active_cleanup_name = if cleanup_exists {
        journal.cleanup_directory_name.as_str()
    } else {
        journal.terminal_cleanup_directory_name.as_str()
    };
    let directory = open_cap_directory_nofollow(backup_root, Path::new(active_cleanup_name))?;
    let expected_root = FileIdentity {
        device: journal.cleanup_root_device_id,
        inode: journal.cleanup_root_inode,
        owner: journal.cleanup_root_owner,
    };
    anyhow::ensure!(
        metadata_identity(&directory.dir_metadata()?) == expected_root,
        "quarantined backup root differs from its cleanup journal"
    );
    let cleanup_metadata = directory.dir_metadata()?;
    validate_managed_metadata(&cleanup_metadata, &backup_root.dir_metadata()?, true)?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        anyhow::ensure!(
            cleanup_metadata.permissions().mode() & 0o777 == 0o700,
            "quarantined backup root has noncanonical mode"
        );
    }
    let manifest_tombstone = cleanup_tombstone_relative("backup-manifest.json")?;
    let envelope_tombstone = cleanup_tombstone_relative("backup-verification-envelope.json")?;
    let manifest_relative = if cap_entry_exists(&directory, Path::new("backup-manifest.json"))? {
        Some("backup-manifest.json".to_owned())
    } else if cap_entry_exists(&directory, Path::new(&manifest_tombstone))? {
        Some(manifest_tombstone.clone())
    } else {
        None
    };
    let envelope_relative =
        if cap_entry_exists(&directory, Path::new("backup-verification-envelope.json"))? {
            Some("backup-verification-envelope.json".to_owned())
        } else if cap_entry_exists(&directory, Path::new(&envelope_tombstone))? {
            Some(envelope_tombstone.clone())
        } else {
            None
        };
    let actual_tree = backup_tree_paths_cap(&directory)?;
    let mut tombstoned_paths = BTreeSet::new();
    if let Some(manifest_relative) = manifest_relative.as_deref() {
        anyhow::ensure!(
            envelope_relative.is_some(),
            "cleanup lost its verification envelope before its manifest"
        );
        let manifest_bytes = read_cap_regular_bounded(
            &directory,
            Path::new(manifest_relative),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        )?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(&manifest_bytes)) == journal.backup_manifest_sha256,
            "cleanup manifest differs from its journal"
        );
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&manifest)? == manifest_bytes,
            "cleanup manifest is not canonical JSON"
        );
        manifest.validate()?;
        let envelope_bytes = read_cap_regular_bounded_with_mode(
            &directory,
            Path::new(envelope_relative.as_deref().expect("checked envelope")),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            0o400,
        )?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(&envelope_bytes)) == journal.verification_envelope_sha256,
            "cleanup verification envelope differs from its journal"
        );
        let envelope: BackupVerificationEnvelopeV2 = serde_json::from_slice(&envelope_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&envelope)? == envelope_bytes,
            "cleanup verification envelope is not canonical JSON"
        );
        envelope.verify_manifest(backup_authority_key, &journal.backup_id, &manifest)?;
        let mut allowed_files = BTreeSet::new();
        let mut manifest_files = BTreeMap::new();
        for file in &manifest.files {
            allowed_files.insert(file.relative_path.clone());
            manifest_files.insert(file.relative_path.clone(), file);
            let tombstone = cleanup_tombstone_relative(&file.relative_path)?;
            allowed_files.insert(tombstone.clone());
            manifest_files.insert(tombstone, file);
        }
        for authority in ["backup-manifest.json", "backup-verification-envelope.json"] {
            allowed_files.insert(authority.to_owned());
            allowed_files.insert(cleanup_tombstone_relative(authority)?);
        }
        let mut allowed_directories = BTreeSet::new();
        for expected in &manifest.directories {
            allowed_directories.insert(expected.relative_path.clone());
            allowed_directories.insert(cleanup_tombstone_relative(&expected.relative_path)?);
        }
        anyhow::ensure!(
            (*actual_tree.files())
                .iter()
                .all(|path| allowed_files.contains(path))
                && (*actual_tree.directories())
                    .iter()
                    .all(|path| allowed_directories.contains(path)),
            "cleanup quarantine contains an unverified insertion"
        );
        tombstoned_paths.extend(
            (*actual_tree.files())
                .iter()
                .chain((*actual_tree.directories()).iter())
                .filter(|path| {
                    path.contains("/.cleanup-unlink-v1-") || path.starts_with(".cleanup-unlink-v1-")
                })
                .cloned(),
        );
        for path in (*actual_tree.files()).iter().filter(|path| {
            path.as_str() != manifest_relative
                && Some(path.as_str()) != envelope_relative.as_deref()
        }) {
            let expected = manifest_files
                .get(path)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("cleanup payload is absent from its manifest"))?;
            let actual = record_cap_file_with_identity(&directory, Path::new(path))?.0;
            anyhow::ensure!(
                actual.byte_length == expected.byte_length && actual.sha256 == expected.sha256,
                "cleanup payload differs from its authenticated manifest: {path}"
            );
        }
    } else {
        let allowed_terminal_envelope = envelope_relative.as_deref();
        anyhow::ensure!(
            (*actual_tree.directories()).is_empty()
                && (*actual_tree.files())
                    .iter()
                    .all(|path| Some(path.as_str()) == allowed_terminal_envelope),
            "terminal cleanup state contains unexpected payload"
        );
        if let Some(envelope_relative) = envelope_relative.as_deref() {
            if envelope_relative == envelope_tombstone {
                tombstoned_paths.insert(envelope_relative.to_owned());
            }
            let envelope_bytes = read_cap_regular_bounded_with_mode(
                &directory,
                Path::new(envelope_relative),
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
                0o400,
            )?;
            anyhow::ensure!(
                hex::encode(Sha256::digest(&envelope_bytes))
                    == journal.verification_envelope_sha256,
                "terminal cleanup envelope differs from its journal"
            );
            let envelope: BackupVerificationEnvelopeV2 = serde_json::from_slice(&envelope_bytes)?;
            envelope.verify(backup_authority_key)?;
            anyhow::ensure!(
                envelope.backup_id == journal.backup_id
                    && envelope.backup_manifest_sha256 == journal.backup_manifest_sha256,
                "terminal cleanup envelope differs from its journal identity"
            );
        }
    }
    remove_remaining_quarantined_tree(
        backup_root,
        journal,
        &actual_tree,
        &journal_name,
        &journal_file,
        journal_identity,
        &journal_bytes,
        &tombstoned_paths,
        active_cleanup_name,
        after_terminal_rename,
        after_terminal_root_remove,
    )
}

fn remove_remaining_quarantined_tree<F, G>(
    backup_root: &cap_std::fs::Dir,
    journal: &BackupCleanupJournalV1,
    actual_tree: &BackupTreePaths,
    journal_name: &str,
    journal_file: &std::fs::File,
    journal_identity: FileIdentity,
    journal_bytes: &[u8],
    tombstoned_paths: &BTreeSet<String>,
    active_cleanup_name: &str,
    after_terminal_rename: F,
    after_terminal_root_remove: G,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
    G: FnOnce() -> anyhow::Result<()>,
{
    let directory = open_cap_directory_nofollow(backup_root, Path::new(active_cleanup_name))?;
    let authority_paths = BTreeSet::from([
        "backup-manifest.json".to_owned(),
        cleanup_tombstone_relative("backup-manifest.json")?,
        "backup-verification-envelope.json".to_owned(),
        cleanup_tombstone_relative("backup-verification-envelope.json")?,
    ]);
    for relative in (*actual_tree.files())
        .iter()
        .filter(|relative| !authority_paths.contains(relative.as_str()))
    {
        remove_verified_cleanup_entry(
            &directory,
            relative,
            *(*actual_tree.file_identities())
                .get(relative)
                .expect("tree identity"),
            false,
            tombstoned_paths.contains(relative),
        )?;
    }
    let mut directories = (*actual_tree.directories()).iter().collect::<Vec<_>>();
    directories.sort_by_key(|path| std::cmp::Reverse(Path::new(path).components().count()));
    for relative in directories {
        remove_verified_cleanup_entry(
            &directory,
            relative,
            *(*actual_tree.directory_identities())
                .get(relative)
                .expect("tree directory identity"),
            true,
            tombstoned_paths.contains(relative),
        )?;
    }
    let terminal_tree = backup_tree_paths_cap(&directory)?;
    anyhow::ensure!(
        terminal_tree.root_identity() == actual_tree.root_identity()
            && (*terminal_tree.directories()).is_empty()
            && (*terminal_tree.files())
                .iter()
                .all(|path| authority_paths.contains(path)),
        "cleanup terminal topology changed before authority removal"
    );
    for original_authority in ["backup-manifest.json", "backup-verification-envelope.json"] {
        let tombstone = cleanup_tombstone_relative(original_authority)?;
        let authority = if (*actual_tree.files()).contains(original_authority) {
            Some(original_authority)
        } else if (*actual_tree.files()).contains(&tombstone) {
            Some(tombstone.as_str())
        } else {
            None
        };
        if let Some(authority) = authority {
            remove_verified_cleanup_entry(
                &directory,
                authority,
                *(*actual_tree.file_identities())
                    .get(authority)
                    .ok_or_else(|| {
                        anyhow::anyhow!("cleanup authority leaf was absent from the verified tree")
                    })?,
                false,
                tombstoned_paths.contains(authority),
            )?;
        }
    }
    anyhow::ensure!(
        directory.entries()?.next().transpose()?.is_none(),
        "cleanup root is not empty"
    );
    let current_directory =
        open_cap_directory_nofollow(backup_root, Path::new(active_cleanup_name))?;
    anyhow::ensure!(
        metadata_identity(&current_directory.dir_metadata()?) == actual_tree.root_identity(),
        "cleanup root was substituted before removal"
    );
    anyhow::ensure!(
        metadata_identity(&backup_root.symlink_metadata(active_cleanup_name)?)
            == actual_tree.root_identity(),
        "cleanup root path was substituted before removal"
    );
    anyhow::ensure!(
        metadata_identity_std(&journal_file.metadata()?) == journal_identity
            && metadata_identity(&backup_root.symlink_metadata(journal_name)?) == journal_identity,
        "cleanup journal was substituted before terminal removal"
    );
    if active_cleanup_name == journal.cleanup_directory_name {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                backup_root.as_fd(),
                Path::new(&journal.cleanup_directory_name),
                backup_root.as_fd(),
                Path::new(&journal.terminal_cleanup_directory_name),
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        anyhow::bail!("terminal cleanup-root quarantine requires Linux renameat2");
        sync_cap_directory(backup_root)?;
    }
    after_terminal_rename()?;
    let terminal_directory = open_cap_directory_nofollow(
        backup_root,
        Path::new(&journal.terminal_cleanup_directory_name),
    )?;
    anyhow::ensure!(
        metadata_identity(&terminal_directory.dir_metadata()?) == actual_tree.root_identity(),
        "cleanup root was substituted while moving to its terminal name"
    );
    backup_root.remove_dir(&journal.terminal_cleanup_directory_name)?;
    sync_cap_directory(backup_root)?;
    after_terminal_root_remove()?;
    anyhow::ensure!(
        !cap_entry_exists(backup_root, Path::new(&journal.cleanup_directory_name))?
            && !cap_entry_exists(
                backup_root,
                Path::new(&journal.terminal_cleanup_directory_name),
            )?,
        "cleanup root name remains after removal"
    );
    remove_pinned_cleanup_journal(
        backup_root,
        journal_name,
        journal_file,
        journal_identity,
        journal_bytes,
    )?;
    Ok(())
}

fn remove_verified_cleanup_entry(
    root: &cap_std::fs::Dir,
    relative: &str,
    expected_identity: FileIdentity,
    is_directory: bool,
    already_tombstoned: bool,
) -> anyhow::Result<()> {
    if !already_tombstoned {
        return rename_verified_entry_to_tombstone(root, relative, expected_identity, is_directory);
    }
    let (parent, child_name) = open_cap_parent_for_relative(root, Path::new(relative))?;
    let actual_identity = if is_directory {
        let child = open_cap_directory_nofollow(&parent, &child_name)?;
        let metadata = child.dir_metadata()?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt as _;
            anyhow::ensure!(
                metadata.permissions().mode() & 0o777 == 0o700,
                "cleanup tombstone directory has a noncanonical mode: {relative}"
            );
        }
        metadata_identity(&metadata)
    } else {
        let child = open_cap_regular_nofollow(&parent, &child_name)?;
        let expected_mode = expected_backup_cleanup_file_mode(relative)?;
        validate_private_pinned_file(&child, expected_mode, "cleanup tombstone file")?;
        let actual_identity = metadata_identity_std(&child.metadata()?);
        anyhow::ensure!(
            actual_identity == expected_identity,
            "cleanup tombstone was substituted before removal: {relative}"
        );
        unlink_pinned_regular(
            &parent,
            child_name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("cleanup tombstone filename is not UTF-8"))?,
            &child,
            expected_identity,
            expected_mode,
            "cleanup tombstone file",
        )?;
        return Ok(());
    };
    anyhow::ensure!(
        actual_identity == expected_identity,
        "cleanup tombstone was substituted before removal: {relative}"
    );
    if is_directory {
        parent.remove_dir(&child_name)?;
    }
    sync_cap_directory(&parent)?;
    Ok(())
}

pub(super) fn remove_owned_complete_backup(
    backup_root: &Path,
    complete: &Path,
    expected_identifier: &str,
    verified_tree: &BackupTreePaths,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        parse_backup_id(expected_identifier).is_some()
            && complete == backup_root.join(expected_identifier),
        "complete-backup cleanup target is outside its exact managed name"
    );
    let root = pin_directory_capability(backup_root)?;
    let name = Path::new(expected_identifier);
    let path_metadata = root.symlink_metadata(name)?;
    let directory = open_cap_directory_nofollow(&root, name)?;
    anyhow::ensure!(
        metadata_identity(&directory.dir_metadata()?) == metadata_identity(&path_metadata)
            && metadata_identity(&directory.dir_metadata()?) == verified_tree.root_identity(),
        "complete backup was substituted before cleanup"
    );
    remove_exact_verified_tree(&root, name, verified_tree, backup_authority_key)?;
    match root.symlink_metadata(name) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => anyhow::bail!(
            "complete backup name was substituted during cleanup; replacement was preserved"
        ),
        Err(error) => return Err(error.into()),
    }
    sync_cap_directory(&root)?;
    Ok(())
}

fn open_cap_parent_for_relative(
    root: &cap_std::fs::Dir,
    relative: &Path,
) -> anyhow::Result<(cap_std::fs::Dir, PathBuf)> {
    anyhow::ensure!(
        !relative.as_os_str().is_empty()
            && !relative.is_absolute()
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "verified-tree deletion path is unsafe"
    );
    let name = relative
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("verified-tree deletion path has no filename"))?;
    let mut parent = root.try_clone()?;
    if let Some(parent_path) = relative.parent() {
        for component in parent_path.components() {
            let std::path::Component::Normal(component) = component else {
                anyhow::bail!("verified-tree deletion parent is unsafe");
            };
            parent = open_cap_directory_nofollow(&parent, Path::new(component))?;
        }
    }
    Ok((parent, PathBuf::from(name)))
}

fn cleanup_names(backup_id: &str) -> anyhow::Result<(String, String, String)> {
    anyhow::ensure!(
        parse_backup_id(backup_id).is_some(),
        "cleanup backup ID is invalid"
    );
    let directory = format!(".cleanup-{backup_id}");
    let journal = format!("{directory}.journal.json");
    let partial = format!(".{journal}.partial");
    Ok((directory, journal, partial))
}

fn rename_verified_entry_to_tombstone(
    root: &cap_std::fs::Dir,
    relative: &str,
    expected_identity: FileIdentity,
    is_directory: bool,
) -> anyhow::Result<()> {
    rename_verified_entry_to_tombstone_with_hooks(
        root,
        relative,
        expected_identity,
        is_directory,
        || Ok(()),
        || Ok(()),
    )
}

pub(super) fn rename_verified_entry_to_tombstone_with_hooks<F, G>(
    root: &cap_std::fs::Dir,
    relative: &str,
    expected_identity: FileIdentity,
    is_directory: bool,
    before_rename: F,
    after_rename: G,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
    G: FnOnce() -> anyhow::Result<()>,
{
    let tombstone = cleanup_tombstone_relative(relative)?;
    if relative == tombstone {
        anyhow::bail!("cleanup tombstone path cannot tombstone itself");
    }
    let (source_parent, source_name) = open_cap_parent_for_relative(root, Path::new(relative))?;
    let (tombstone_parent, tombstone_name) =
        open_cap_parent_for_relative(root, Path::new(&tombstone))?;
    anyhow::ensure!(
        metadata_identity(&source_parent.dir_metadata()?)
            == metadata_identity(&tombstone_parent.dir_metadata()?),
        "cleanup tombstone must remain in the source parent"
    );
    before_rename()?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsFd as _;
        rustix::fs::renameat_with(
            source_parent.as_fd(),
            &source_name,
            tombstone_parent.as_fd(),
            &tombstone_name,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    anyhow::bail!("verified cleanup unlink requires Linux renameat2");
    sync_cap_directory(&source_parent)?;
    after_rename()?;
    if is_directory {
        let moved = open_cap_directory_nofollow(&tombstone_parent, &tombstone_name)?;
        let metadata = moved.dir_metadata()?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt as _;
            anyhow::ensure!(
                metadata.permissions().mode() & 0o777 == 0o700,
                "cleanup source directory has a noncanonical mode after tombstoning"
            );
        }
        anyhow::ensure!(
            metadata_identity(&metadata) == expected_identity,
            "cleanup source was substituted while moving it to a safe tombstone; the replacement was preserved"
        );
        tombstone_parent.remove_dir(&tombstone_name)?;
        sync_cap_directory(&tombstone_parent)?;
    } else {
        let moved = open_cap_regular_nofollow(&tombstone_parent, &tombstone_name)?;
        let expected_mode = expected_backup_cleanup_file_mode(relative)?;
        validate_private_pinned_file(
            &moved,
            expected_mode,
            "cleanup source file after tombstoning",
        )?;
        anyhow::ensure!(
            metadata_identity_std(&moved.metadata()?) == expected_identity,
            "cleanup source was substituted while moving it to a safe tombstone; the replacement was preserved"
        );
        unlink_pinned_regular(
            &tombstone_parent,
            tombstone_name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("cleanup tombstone filename is not UTF-8"))?,
            &moved,
            expected_identity,
            expected_mode,
            "cleanup source file after tombstoning",
        )?;
    }
    Ok(())
}

pub(super) fn publish_cleanup_journal(
    backup_root: &cap_std::fs::Dir,
    journal_name: &str,
    partial_name: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    match backup_root.symlink_metadata(partial_name) {
        Ok(metadata) => {
            validate_managed_metadata(&metadata, &backup_root.dir_metadata()?, false)?;
            let partial = open_cap_regular_nofollow(backup_root, Path::new(partial_name))?;
            validate_private_pinned_file(&partial, 0o400, "cleanup journal partial")?;
            let partial_identity = metadata_identity_std(&partial.metadata()?);
            let existing =
                read_bounded_pinned_file(duplicate_pinned_file(&partial, false)?, 64 * 1024)?;
            if existing != bytes {
                remove_pinned_regular_via_tombstone(
                    backup_root,
                    partial_name,
                    &format!("{partial_name}.discard"),
                    &partial,
                    partial_identity,
                    0o400,
                    &existing,
                    64 * 1024,
                    "cleanup journal partial",
                )?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if !cap_entry_exists(backup_root, Path::new(journal_name))?
        && !cap_entry_exists(backup_root, Path::new(partial_name))?
    {
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            options.mode(0o400);
        }
        let mut file = backup_root.open_with(partial_name, &options)?.into_std();
        file.write_all(bytes)?;
        file.sync_all()?;
        validate_private_pinned_file(&file, 0o400, "cleanup journal partial")?;
        sync_cap_directory(backup_root)?;
    }
    if !cap_entry_exists(backup_root, Path::new(journal_name))? {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                backup_root.as_fd(),
                Path::new(partial_name),
                backup_root.as_fd(),
                Path::new(journal_name),
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        anyhow::bail!("cleanup-journal NOREPLACE publication requires Linux renameat2");
        sync_cap_directory(backup_root)?;
    }
    let journal = open_cap_regular_nofollow(backup_root, Path::new(journal_name))?;
    validate_private_pinned_file(&journal, 0o400, "cleanup journal")?;
    anyhow::ensure!(
        read_bounded_pinned_file(duplicate_pinned_file(&journal, false)?, 64 * 1024)? == bytes,
        "existing cleanup journal differs from the authenticated deletion plan"
    );
    if cap_entry_exists(backup_root, Path::new(partial_name))? {
        let partial = open_cap_regular_nofollow(backup_root, Path::new(partial_name))?;
        validate_private_pinned_file(&partial, 0o400, "cleanup journal partial")?;
        let partial_identity = metadata_identity_std(&partial.metadata()?);
        let partial_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&partial, false)?, 64 * 1024)?;
        anyhow::ensure!(
            partial_bytes == bytes,
            "cleanup journal final and partial bytes differ during reconciliation"
        );
        remove_pinned_regular_via_tombstone(
            backup_root,
            partial_name,
            &format!("{partial_name}.discard"),
            &partial,
            partial_identity,
            0o400,
            &partial_bytes,
            64 * 1024,
            "cleanup journal partial",
        )?;
    }
    Ok(())
}

fn remove_exact_verified_tree(
    backup_root: &cap_std::fs::Dir,
    name: &Path,
    verified_tree: &BackupTreePaths,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<()> {
    let directory = open_cap_directory_nofollow(backup_root, name)?;
    anyhow::ensure!(
        backup_tree_paths_cap(&directory)? == *verified_tree,
        "complete backup changed after verification; refusing recursive cleanup"
    );
    let backup_id = name
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("complete backup name is not UTF-8"))?;
    let (journal, journal_bytes, cleanup_name, journal_name, partial_journal_name) =
        cleanup_journal_for_verified_tree(
            backup_root,
            &directory,
            backup_id,
            verified_tree,
            backup_authority_key,
        )?;
    publish_cleanup_journal(
        backup_root,
        &journal_name,
        &partial_journal_name,
        &journal_bytes,
    )?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsFd as _;
        if let Err(error) = rustix::fs::renameat_with(
            backup_root.as_fd(),
            name,
            backup_root.as_fd(),
            Path::new(&cleanup_name),
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            // Preserve the already durable authenticated journal. Recovery
            // will prove that the original complete inode is still present
            // before removing the journal; deleting it here would introduce
            // a pathname-substitution deletion race on the failure path.
            return Err(error.into());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    anyhow::bail!("complete-backup quarantine requires Linux renameat2");
    sync_cap_directory(backup_root)?;
    let directory = open_cap_directory_nofollow(backup_root, Path::new(&cleanup_name))?;
    anyhow::ensure!(
        metadata_identity(&directory.dir_metadata()?) == verified_tree.root_identity(),
        "quarantined backup root differs from the verified inode"
    );
    resume_authenticated_cleanup(backup_root, &journal, backup_authority_key)
}

pub(super) fn cleanup_journal_for_verified_tree(
    backup_root: &cap_std::fs::Dir,
    directory: &cap_std::fs::Dir,
    backup_id: &str,
    verified_tree: &BackupTreePaths,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<(BackupCleanupJournalV1, Vec<u8>, String, String, String)> {
    let (cleanup_name, journal_name, partial_journal_name) = cleanup_names(backup_id)?;
    let manifest_bytes = read_cap_regular_bounded(
        &directory,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )?;
    let envelope_bytes = read_cap_regular_bounded_with_mode(
        &directory,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let backup_root_identity = metadata_identity(&backup_root.dir_metadata()?);
    let journal = BackupCleanupJournalV1::new_authenticated(
        backup_id.to_owned(),
        backup_root_identity.device(),
        backup_root_identity.inode(),
        backup_root_identity.owner(),
        verified_tree.root_identity().device(),
        verified_tree.root_identity().inode(),
        verified_tree.root_identity().owner(),
        hex::encode(Sha256::digest(&manifest_bytes)),
        hex::encode(Sha256::digest(&envelope_bytes)),
        backup_authority_key,
    )?;
    let journal_bytes = canonical_json_bytes(&journal)?;
    Ok((
        journal,
        journal_bytes,
        cleanup_name,
        journal_name,
        partial_journal_name,
    ))
}

pub(super) async fn cleanup_failed_partial_backup(
    database: &Database,
    backup_root: &Path,
    partial: &Path,
    original: anyhow::Error,
) -> anyhow::Error {
    // VACUUM INTO uses the live pool's SQLite worker. An error can reach its
    // awaiter before the statement and checked-out connection finish cleanup.
    // Keep the partial in place until that worker has returned to idle.
    if let Err(error) = database.wait_for_idle().await {
        return original.context(format!(
            "partial preserved because source SQL drain failed: {error:#}"
        ));
    }
    match remove_owned_partial_backup(backup_root, partial) {
        Ok(()) => original,
        Err(error) => original.context(format!("partial backup cleanup also failed: {error:#}")),
    }
}

pub(super) fn remove_owned_partial_backup(
    backup_root: &Path,
    partial: &Path,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        partial.parent() == Some(backup_root)
            && partial
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(valid_partial_backup_name),
        "refusing to remove an unowned partial backup path"
    );
    let root = pin_directory_capability(backup_root)?;
    let name = partial
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("partial backup has no filename"))?;
    match root.symlink_metadata(name) {
        Ok(_) => {
            let directory = open_cap_directory_nofollow(&root, Path::new(name))?;
            validate_managed_directory_tree(&directory, &root.dir_metadata()?)?;
            directory.remove_open_dir_all()?;
            match root.symlink_metadata(name) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => anyhow::bail!(
                    "partial backup name was substituted during failure cleanup; replacement was preserved"
                ),
                Err(error) => return Err(error.into()),
            }
            sync_cap_directory(&root)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(super) async fn retain_complete_backups(
    backup_root: &Path,
    current_identifier: &str,
    retain_complete: usize,
    maximum_admitted_generations: usize,
    active_release_identity: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        retain_complete >= 1,
        "backup retention must keep one backup"
    );
    anyhow::ensure!(
        maximum_admitted_generations >= retain_complete,
        "retention scan bound is smaller than the desired keep count"
    );
    anyhow::ensure!(
        valid_complete_backup_name(current_identifier),
        "current backup identifier is not canonical"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_metadata = root.dir_metadata()?;
    let mut complete = Vec::new();
    let mut visited = 0_usize;
    let mut verified_topology_entries = 0_usize;
    let maximum_verified_topology_entries = (robin_highscores::backup::MAX_BACKUP_MANIFEST_FILES
        + 2)
    .checked_mul(maximum_admitted_generations)
    .ok_or_else(|| anyhow::anyhow!("aggregate retention topology bound overflows"))?;
    let mut managed_complete_generations = 0_usize;
    for entry in root.entries()? {
        let entry = entry?;
        visited = visited
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("retention root entry count overflows"))?;
        anyhow::ensure!(
            visited <= 4_096,
            "backup root has too many entries for bounded retention"
        );
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("backup-v4-") {
            continue;
        }
        managed_complete_generations = managed_complete_generations
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("retention generation count overflows"))?;
        anyhow::ensure!(
            managed_complete_generations <= maximum_admitted_generations,
            "backup root has excess complete generations for the configured retention policy"
        );
        anyhow::ensure!(
            valid_complete_backup_name(&name),
            "managed complete backup has a malformed name"
        );
        let directory = open_cap_directory_nofollow(&root, Path::new(&name))?;
        let directory_metadata = directory.dir_metadata()?;
        #[cfg(unix)]
        {
            use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
            anyhow::ensure!(
                directory_metadata.uid() == rustix::process::geteuid().as_raw()
                    && directory_metadata.dev() == root_metadata.dev()
                    && directory_metadata.permissions().mode() & 0o777 == 0o700,
                "complete backup root has the wrong owner, device, or mode"
            );
        }
        let identity = metadata_identity(&directory_metadata);
        let path = backup_root.join(&name);
        let verified = verify_backup_with_schema_policy(&path, false, backup_authority_key).await?;
        verified_topology_entries = verified_topology_entries
            .checked_add((*verified.tree().files()).len())
            .and_then(|value| value.checked_add((*verified.tree().directories()).len()))
            .ok_or_else(|| anyhow::anyhow!("retention topology count overflows"))?;
        anyhow::ensure!(
            verified_topology_entries <= maximum_verified_topology_entries,
            "aggregate retained-backup topology exceeds its verification bound"
        );
        let preserved =
            load_preserved_release_authority(backup_root, &(*verified.release_identity())).await?;
        anyhow::ensure!(
            preserved == (*verified.release_identity()),
            "retained backup differs from its independent preserved release authority"
        );
        if (*verified.release_identity()) == *active_release_identity {
            anyhow::ensure!(
                preserved == *active_release_identity,
                "current retained backup differs from active release authority"
            );
        }
        let reopened = open_cap_directory_nofollow(&root, Path::new(&name))?;
        anyhow::ensure!(
            metadata_identity(&reopened.dir_metadata()?) == identity,
            "managed complete backup was substituted during verification"
        );
        complete.push((name, identity, verified.into_tree()));
    }
    anyhow::ensure!(
        complete
            .iter()
            .any(|(name, _, _)| name == current_identifier),
        "newly completed backup disappeared before publication"
    );
    let current_identity = complete
        .iter()
        .find(|(name, _, _)| name == current_identifier)
        .map(|(_, identity, _)| *identity)
        .expect("current backup presence was checked above");
    let mut older = complete
        .into_iter()
        .filter(|(name, _, _)| name != current_identifier)
        .collect::<Vec<_>>();
    older.sort_by(|left, right| {
        parse_backup_id(&right.0)
            .expect("retention names were validated above")
            .cmp(&parse_backup_id(&left.0).expect("retention names were validated above"))
            .then_with(|| right.0.cmp(&left.0))
    });
    for (name, expected_identity, verified_tree) in older.into_iter().skip(retain_complete - 1) {
        anyhow::ensure!(
            expected_identity != current_identity,
            "retention candidate aliases the current backup inode"
        );
        let directory = open_cap_directory_nofollow(&root, Path::new(&name))?;
        anyhow::ensure!(
            metadata_identity(&directory.dir_metadata()?) == expected_identity,
            "retention candidate was substituted before deletion"
        );
        remove_exact_verified_tree(
            &root,
            Path::new(&name),
            &verified_tree,
            backup_authority_key,
        )?;
        match root.symlink_metadata(&name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => anyhow::bail!(
                "retention candidate name was substituted during deletion; replacement was preserved"
            ),
            Err(error) => return Err(error.into()),
        }
    }
    sync_cap_directory(&root)?;
    Ok(())
}

#[cfg(test)]
mod tests;
