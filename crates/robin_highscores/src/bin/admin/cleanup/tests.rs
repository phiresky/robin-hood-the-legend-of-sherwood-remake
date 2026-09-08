use super::super::filesystem::backup_tree_paths_cap;
use super::super::filesystem::metadata_identity_std;
use super::super::filesystem::open_cap_directory_nofollow;
use super::super::filesystem::open_cap_regular_nofollow;
use super::super::filesystem::pin_directory_capability;
use super::super::filesystem::unlink_pinned_regular;
use super::super::filesystem::unlink_pinned_regular_with_hook;
use super::super::filesystem::write_private_file;
use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

#[test]
fn direct_pinned_unlink_rejects_substitution_and_proves_unlinked_inode() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = pin_directory_capability(directory.path()).unwrap();
    let source_path = directory.path().join("discard");
    let displaced_path = directory.path().join("displaced");
    std::fs::write(&source_path, b"authority").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&source_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let pinned = open_cap_regular_nofollow(&parent, Path::new("discard")).unwrap();
    let identity = metadata_identity_std(&pinned.metadata().unwrap());
    assert!(
        unlink_pinned_regular_with_hook(
            &parent,
            "discard",
            &pinned,
            identity,
            0o400,
            "test discard",
            || {
                std::fs::rename(&source_path, &displaced_path)?;
                std::fs::write(&source_path, b"replacement")?;
                #[cfg(unix)]
                std::fs::set_permissions(&source_path, std::fs::Permissions::from_mode(0o400))?;
                Ok(())
            },
        )
        .is_err(),
        "a replacement installed immediately before unlink must be preserved"
    );
    assert_eq!(std::fs::read(&source_path).unwrap(), b"replacement");
    assert_eq!(std::fs::read(&displaced_path).unwrap(), b"authority");

    let terminal_path = directory.path().join("terminal");
    std::fs::write(&terminal_path, b"terminal").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&terminal_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let terminal = open_cap_regular_nofollow(&parent, Path::new("terminal")).unwrap();
    let terminal_identity = metadata_identity_std(&terminal.metadata().unwrap());
    unlink_pinned_regular(
        &parent,
        "terminal",
        &terminal,
        terminal_identity,
        0o400,
        "test terminal",
    )
    .unwrap();
    assert!(!terminal_path.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(terminal.metadata().unwrap().nlink(), 0);
    }
}

#[test]
fn exact_complete_cleanup_preserves_unverified_insertions_and_substitutions() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let backup = directory
        .path()
        .join("backup-v4-1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    std::fs::create_dir(&backup).unwrap();
    std::fs::create_dir(backup.join("empty")).unwrap();
    std::fs::write(backup.join("backup-manifest.json"), b"manifest").unwrap();
    std::fs::write(
        backup.join("backup-verification-envelope.json"),
        b"envelope",
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(backup.join("empty"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        std::fs::set_permissions(
            backup.join("backup-manifest.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        std::fs::set_permissions(
            backup.join("backup-verification-envelope.json"),
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
    }
    let root = pin_directory_capability(directory.path()).unwrap();
    let child = open_cap_directory_nofollow(
        &root,
        Path::new("backup-v4-1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )
    .unwrap();
    let verified = backup_tree_paths_cap(&child).unwrap();
    std::fs::write(backup.join("unverified"), b"do not delete").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        backup.join("unverified"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(
        remove_exact_verified_tree(
            &root,
            Path::new("backup-v4-1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            &verified,
            &[0x31; 32],
        )
        .is_err()
    );
    assert!(backup.join("unverified").exists());
    assert!(backup.join("backup-manifest.json").exists());
    std::fs::remove_file(backup.join("unverified")).unwrap();
    let displaced = backup.join("displaced-manifest");
    std::fs::rename(backup.join("backup-manifest.json"), &displaced).unwrap();
    std::fs::write(backup.join("backup-manifest.json"), b"manifest").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        backup.join("backup-manifest.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(
        remove_exact_verified_tree(
            &root,
            Path::new("backup-v4-1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            &verified,
            &[0x31; 32],
        )
        .is_err(),
        "identical bytes on a substituted inode must be preserved"
    );
    assert!(backup.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn stale_partial_cleanup_rejects_hardlinks_and_malformed_managed_names() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let partial = root
        .path()
        .join(format!(".backup-v4-1-{}.partial", "a".repeat(32)));
    std::fs::create_dir(&partial).unwrap();
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o700)).unwrap();
    write_private_file(&partial.join("bytes"), b"owned")
        .await
        .unwrap();
    std::fs::hard_link(partial.join("bytes"), partial.join("bytes-alias")).unwrap();
    assert!(recover_stale_partial_backups(root.path()).is_err());
    assert_eq!(std::fs::read(partial.join("bytes")).unwrap(), b"owned");

    std::fs::remove_file(partial.join("bytes-alias")).unwrap();
    recover_stale_partial_backups(root.path()).unwrap();
    assert!(!partial.exists());
    let malformed = root.path().join(".backup-v4-junk.partial");
    std::fs::create_dir(&malformed).unwrap();
    std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(recover_stale_partial_backups(root.path()).is_err());
    assert!(malformed.exists());
}
