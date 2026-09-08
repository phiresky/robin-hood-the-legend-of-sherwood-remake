use super::super::filesystem::write_private_file;
use super::super::fixtures::write_test_release_manifest;
use super::super::policy::RELEASE_AUTHORITY_STORE;
use super::*;
use sha2::Sha256;

#[tokio::test]
async fn pinned_restore_sources_reject_path_swaps_and_in_place_mutation() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

    let secret = directory.path().join("cursor-hmac.key");
    write_private_file(&secret, &[0x11; 32]).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o400)).unwrap();
    let pinned = pin_restore_source(&secret, 0o400, Some(32), None, "test restore secret").unwrap();
    let displaced = directory.path().join("cursor-hmac.displaced");
    std::fs::rename(&secret, &displaced).unwrap();
    std::fs::write(&secret, [0x22; 32]).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o400)).unwrap();
    let swapped_target = directory.path().join("archive/swapped.key");
    assert!(
        copy_pinned_restore_source(&pinned, &swapped_target)
            .await
            .is_err(),
        "a pathname replacement after admission must not be copied"
    );
    assert!(!swapped_target.exists());
    std::fs::remove_file(&secret).unwrap();
    std::fs::rename(&displaced, &secret).unwrap();

    #[cfg(unix)]
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mutation = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&secret)
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o400)).unwrap();
    let pinned = pin_restore_source(
        &secret,
        0o400,
        Some(32),
        None,
        "test mutable restore secret",
    )
    .unwrap();
    let mutated_target = directory.path().join("archive/mutated.key");
    assert!(
        copy_pinned_restore_source_with_hook(&pinned, &mutated_target, || {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt as _;
                mutation.write_all_at(&[0x33; 32], 0)?;
                mutation.sync_all()?;
            }
            Ok(())
        })
        .await
        .is_err(),
        "in-place mutation during a descriptor copy must fail before any manifest is authored"
    );
    assert!(
        !directory
            .path()
            .join("archive/backup-manifest.json")
            .exists()
    );

    let unit = directory.path().join("robin-highscores-api.service");
    let unit_bytes = b"[Service]\nType=notify\n";
    write_private_file(&unit, unit_bytes).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&unit, std::fs::Permissions::from_mode(0o440)).unwrap();
    let pinned_unit = pin_restore_source(
        &unit,
        0o440,
        Some(u64::try_from(unit_bytes.len()).unwrap()),
        Some(&hex::encode(Sha256::digest(unit_bytes))),
        "test installed unit",
    )
    .unwrap();
    let old_unit = directory.path().join("old-api.service");
    std::fs::rename(&unit, &old_unit).unwrap();
    std::fs::write(&unit, unit_bytes).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&unit, std::fs::Permissions::from_mode(0o440)).unwrap();
    assert!(
        copy_pinned_restore_source(
            &pinned_unit,
            &directory.path().join("archive/substituted.service"),
        )
        .await
        .is_err(),
        "an identical-byte installed-unit inode substitution must be rejected"
    );
}

#[tokio::test]
async fn preserved_release_authority_is_atomic_idempotent_and_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let backup_root = directory.path().join("backups");
    tokio::fs::create_dir(&backup_root).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&backup_root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let release_manifest = directory.path().join("vps-release-manifest-v2.json");
    let identity = write_test_release_manifest(&release_manifest).await;
    let release_bytes = tokio::fs::read(&release_manifest).await.unwrap();
    let name = release_authority_file_name(&identity).unwrap();
    let store = backup_root.join(RELEASE_AUTHORITY_STORE);
    let final_path = store.join(&name);
    let partial_path = store.join(format!(".{name}.partial"));

    preserve_release_authority(&backup_root, &release_manifest, &identity)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&final_path).await.unwrap(), release_bytes);
    assert!(!partial_path.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(
            std::fs::metadata(&store).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let metadata = std::fs::metadata(&final_path).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o400);
        assert_eq!(metadata.nlink(), 1);
    }
    preserve_release_authority(&backup_root, &release_manifest, &identity)
        .await
        .unwrap();

    tokio::fs::remove_file(&final_path).await.unwrap();
    write_private_file(&partial_path, b"truncated crash residue")
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&partial_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    preserve_release_authority(&backup_root, &release_manifest, &identity)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&final_path).await.unwrap(), release_bytes);
    assert!(!partial_path.exists());

    write_private_file(&partial_path, &release_bytes)
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&partial_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    preserve_release_authority(&backup_root, &release_manifest, &identity)
        .await
        .unwrap();
    assert!(
        !partial_path.exists(),
        "a leftover exact partial must be reconciled"
    );

    #[cfg(unix)]
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    tokio::fs::write(&final_path, b"forged authority")
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        preserve_release_authority(&backup_root, &release_manifest, &identity)
            .await
            .is_err(),
        "a digest-named but forged final authority must never be replaced or accepted"
    );

    let wrong_mode_root = directory.path().join("wrong-mode-root");
    tokio::fs::create_dir(&wrong_mode_root).await.unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&wrong_mode_root, std::fs::Permissions::from_mode(0o700)).unwrap();
    tokio::fs::create_dir(wrong_mode_root.join(RELEASE_AUTHORITY_STORE))
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        wrong_mode_root.join(RELEASE_AUTHORITY_STORE),
        std::fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    assert!(
        preserve_release_authority(&wrong_mode_root, &release_manifest, &identity)
            .await
            .is_err(),
        "a non-private authority store must be rejected"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let symlink_root = directory.path().join("symlink-root");
        std::fs::create_dir(&symlink_root).unwrap();
        std::fs::set_permissions(&symlink_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let external = directory.path().join("external-authority-store");
        std::fs::create_dir(&external).unwrap();
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&external, symlink_root.join(RELEASE_AUTHORITY_STORE)).unwrap();
        assert!(
            preserve_release_authority(&symlink_root, &release_manifest, &identity)
                .await
                .is_err(),
            "an authority-store symlink must be rejected"
        );
        assert_eq!(std::fs::read_dir(&external).unwrap().count(), 0);
    }
}
