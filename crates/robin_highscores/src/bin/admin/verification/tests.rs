use super::super::cleanup::cleanup_journal_for_verified_tree;
use super::super::cleanup::publish_cleanup_journal;
use super::super::cleanup::recover_interrupted_complete_cleanups;
use super::super::cleanup::rename_verified_entry_to_tombstone_with_hooks;
use super::super::cleanup::resume_authenticated_cleanup_with_hooks;
use super::super::execution::backup;
use super::super::filesystem::backup_tree_paths_cap;
use super::super::filesystem::cleanup_tombstone_relative;
use super::super::filesystem::open_cap_directory_nofollow;
use super::super::filesystem::pin_directory_capability;
use super::super::filesystem::write_private_file;
use super::super::fixtures::refresh_database_manifest_entry;
use super::super::fixtures::remove_test_database_sidecars;
use super::super::fixtures::test_restore_sources;
use super::super::fixtures::write_test_release_manifest;
use super::super::policy::RELEASE_AUTHORITY_STORE;
use super::super::sources::preserve_release_authority;
use super::super::sources::release_authority_file_name;
use super::*;
use robin_highscores::CampaignStore;
use robin_highscores::Database;
use robin_highscores::ReplayStore;
use robin_highscores::ServerConfig;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupVerificationEnvelopeV2;
use robin_run_protocol::canonical_json_bytes;
use sha2::Digest as _;
use sha2::Sha256;
use sqlx::Connection as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

#[tokio::test]
async fn coordinated_backup_verifies_database_objects_and_cursor_key() {
    use bytes::Bytes;
    use futures_util::stream;

    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let release_manifest_path = directory.path().join("vps-release-manifest-v2.json");
    let release_identity = write_test_release_manifest(&release_manifest_path).await;
    let mut config = ServerConfig::default();
    config.database_path = directory.path().join("data/highscores.sqlite3");
    config.replay_directory = directory.path().join("data/replays");
    config.campaign_state_directory = directory.path().join("data/campaigns");
    config.cursor_secret_path = directory.path().join("data/cursor-hmac.key");
    config.competition_run_grant_secret_path =
        directory.path().join("data/competition-run-grant.key");
    config.run_preflight_grant_secret_path = directory.path().join("data/run-preflight-grant.key");
    config.moderation_bearer_token_path =
        Some(directory.path().join("data/moderation-bearer.token"));
    tokio::fs::create_dir_all(config.database_path.parent().unwrap())
        .await
        .unwrap();
    write_private_file(&config.cursor_secret_path, &[7; 32])
        .await
        .unwrap();
    write_private_file(&config.competition_run_grant_secret_path, &[8; 32])
        .await
        .unwrap();
    write_private_file(&config.run_preflight_grant_secret_path, &[9; 32])
        .await
        .unwrap();
    write_private_file(
        config.moderation_bearer_token_path.as_ref().unwrap(),
        b"moderation-secret",
    )
    .await
    .unwrap();
    #[cfg(unix)]
    for secret in [
        &config.cursor_secret_path,
        &config.competition_run_grant_secret_path,
        &config.run_preflight_grant_secret_path,
        config.moderation_bearer_token_path.as_ref().unwrap(),
    ] {
        std::fs::set_permissions(secret, std::fs::Permissions::from_mode(0o400)).unwrap();
    }
    let database = Database::migrate(&config).await.unwrap();
    let replay_store = ReplayStore::create(config.replay_directory.clone(), 1024)
        .await
        .unwrap();
    let replay_bytes = Bytes::from_static(b"canonical compact replay");
    let replay_digest: [u8; 32] = Sha256::digest(&replay_bytes).into();
    replay_store
        .store_stream(
            stream::iter([Ok::<_, std::convert::Infallible>(replay_bytes.clone())]),
            replay_digest,
            replay_bytes.len() as u64,
        )
        .await
        .unwrap();
    database
        .register_replay_object(&replay_digest, replay_bytes.len() as u64)
        .await
        .unwrap();
    let campaign_store = CampaignStore::create(config.campaign_state_directory.clone(), 1024)
        .await
        .unwrap();
    let campaign_bytes = b"exact starting campaign";
    let campaign_digest: [u8; 32] = Sha256::digest(campaign_bytes).into();
    campaign_store
        .import_bytes(&campaign_digest, campaign_bytes)
        .await
        .unwrap();
    database
        .register_campaign_object(&campaign_digest, campaign_bytes.len() as u64)
        .await
        .unwrap();
    let restore_sources = test_restore_sources(&config, directory.path()).await;
    let created_at_unix_ms =
        u64::try_from(robin_highscores::model::now_epoch_ms().unwrap()).unwrap();
    let destination = directory
        .path()
        .join(format!("backup-v4-{created_at_unix_ms}-{}", "a".repeat(32)));
    backup(
        &config,
        &config.campaign_state_directory,
        1024,
        &destination,
        created_at_unix_ms,
        &release_identity,
        &restore_sources,
    )
    .await
    .unwrap();
    let backup_id = destination.file_name().unwrap().to_str().unwrap();
    let backup_parent = pin_directory_capability(directory.path()).unwrap();
    let backup_directory =
        open_cap_directory_nofollow(&backup_parent, Path::new(backup_id)).unwrap();
    let verified_tree = backup_tree_paths_cap(&backup_directory).unwrap();
    let (_cleanup_journal, cleanup_journal_bytes, cleanup_name, journal_name, partial_name) =
        cleanup_journal_for_verified_tree(
            &backup_parent,
            &backup_directory,
            backup_id,
            &verified_tree,
            &[0x31; 32],
        )
        .unwrap();

    publish_cleanup_journal(
        &backup_parent,
        &journal_name,
        &partial_name,
        &cleanup_journal_bytes,
    )
    .unwrap();
    recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).unwrap();
    assert!(
        destination.is_dir(),
        "pre-rename recovery must preserve the complete backup"
    );
    assert!(!directory.path().join(&journal_name).exists());

    std::fs::write(directory.path().join(&partial_name), []).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        directory.path().join(&partial_name),
        std::fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).unwrap();
    assert!(
        !directory.path().join(&partial_name).exists(),
        "a crash-truncated pre-rename journal partial must not poison retry"
    );

    publish_cleanup_journal(
        &backup_parent,
        &journal_name,
        &partial_name,
        &cleanup_journal_bytes,
    )
    .unwrap();
    std::fs::write(directory.path().join(&partial_name), &cleanup_journal_bytes).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        directory.path().join(&partial_name),
        std::fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).unwrap();
    assert!(!directory.path().join(&partial_name).exists());
    assert!(!directory.path().join(&journal_name).exists());

    publish_cleanup_journal(
        &backup_parent,
        &journal_name,
        &partial_name,
        &cleanup_journal_bytes,
    )
    .unwrap();
    std::fs::rename(&destination, directory.path().join(&cleanup_name)).unwrap();
    let manifest: BackupManifest = serde_json::from_slice(
        &std::fs::read(
            directory
                .path()
                .join(&cleanup_name)
                .join("backup-manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let interrupted_payload = &manifest.files[0].relative_path;
    assert!(
        rename_verified_entry_to_tombstone_with_hooks(
            &backup_directory,
            interrupted_payload,
            *verified_tree
                .file_identities
                .get(interrupted_payload)
                .unwrap(),
            false,
            || Ok(()),
            || anyhow::bail!("injected crash after cleanup tombstone rename"),
        )
        .is_err()
    );
    assert!(
        directory
            .path()
            .join(&cleanup_name)
            .join(cleanup_tombstone_relative(interrupted_payload).unwrap())
            .is_file(),
        "the durable cleanup tombstone must make an interrupted unlink resumable"
    );
    recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).unwrap();
    assert!(!directory.path().join(&cleanup_name).exists());
    assert!(!directory.path().join(&journal_name).exists());

    backup(
        &config,
        &config.campaign_state_directory,
        1024,
        &destination,
        created_at_unix_ms,
        &release_identity,
        &restore_sources,
    )
    .await
    .unwrap();
    let backup_directory =
        open_cap_directory_nofollow(&backup_parent, Path::new(backup_id)).unwrap();
    let verified_tree = backup_tree_paths_cap(&backup_directory).unwrap();
    let manifest: BackupManifest =
        serde_json::from_slice(&std::fs::read(destination.join("backup-manifest.json")).unwrap())
            .unwrap();
    let substituted_payload = manifest.files[0].relative_path.clone();
    let original_payload = destination.join(&substituted_payload);
    let linked_payload = directory.path().join("cleanup-payload-hardlink");
    assert!(
        rename_verified_entry_to_tombstone_with_hooks(
            &backup_directory,
            &substituted_payload,
            *verified_tree
                .file_identities
                .get(&substituted_payload)
                .unwrap(),
            false,
            || {
                std::fs::hard_link(&original_payload, &linked_payload)?;
                Ok(())
            },
            || Ok(()),
        )
        .is_err(),
        "a newly hard-linked payload must fail the post-rename nlink=1 boundary"
    );
    let linked_tombstone =
        destination.join(cleanup_tombstone_relative(&substituted_payload).unwrap());
    assert!(linked_tombstone.is_file() && linked_payload.is_file());
    std::fs::remove_file(linked_tombstone).unwrap();
    std::fs::rename(&linked_payload, &original_payload).unwrap();
    assert_eq!(
        backup_tree_paths_cap(&backup_directory).unwrap(),
        verified_tree,
        "failed hard-link deletion must leave the exact verified tree recoverable"
    );

    let displaced_payload = directory.path().join("displaced-cleanup-payload");
    let replacement_bytes = std::fs::read(&original_payload).unwrap();
    assert!(
        rename_verified_entry_to_tombstone_with_hooks(
            &backup_directory,
            &substituted_payload,
            *verified_tree
                .file_identities
                .get(&substituted_payload)
                .unwrap(),
            false,
            || {
                std::fs::rename(&original_payload, &displaced_payload)?;
                std::fs::write(&original_payload, &replacement_bytes)?;
                #[cfg(unix)]
                std::fs::set_permissions(
                    &original_payload,
                    std::fs::Permissions::from_mode(0o600),
                )?;
                Ok(())
            },
            || Ok(()),
        )
        .is_err(),
        "a pathname substitution at the unlink boundary must be moved aside and preserved"
    );
    let substituted_tombstone =
        destination.join(cleanup_tombstone_relative(&substituted_payload).unwrap());
    assert!(substituted_tombstone.is_file());
    std::fs::remove_file(&substituted_tombstone).unwrap();
    std::fs::rename(&displaced_payload, &original_payload).unwrap();
    assert_eq!(
        backup_tree_paths_cap(&backup_directory).unwrap(),
        verified_tree
    );
    let final_tombstone =
        destination.join(cleanup_tombstone_relative(&substituted_payload).unwrap());
    let displaced_final_tombstone = directory.path().join("displaced-final-tombstone");
    assert!(
        rename_verified_entry_to_tombstone_with_hooks(
            &backup_directory,
            &substituted_payload,
            *verified_tree
                .file_identities
                .get(&substituted_payload)
                .unwrap(),
            false,
            || Ok(()),
            || {
                std::fs::rename(&final_tombstone, &displaced_final_tombstone)?;
                std::fs::write(&final_tombstone, b"replacement-at-final-unlink")?;
                #[cfg(unix)]
                std::fs::set_permissions(&final_tombstone, std::fs::Permissions::from_mode(0o600))?;
                Ok(())
            },
        )
        .is_err(),
        "a replacement installed at the final tombstone unlink boundary must be preserved"
    );
    assert_eq!(
        std::fs::read(&final_tombstone).unwrap(),
        b"replacement-at-final-unlink"
    );
    std::fs::remove_file(&final_tombstone).unwrap();
    std::fs::rename(&displaced_final_tombstone, &original_payload).unwrap();
    assert_eq!(
        backup_tree_paths_cap(&backup_directory).unwrap(),
        verified_tree
    );
    let (cleanup_journal, cleanup_journal_bytes, cleanup_name, journal_name, partial_name) =
        cleanup_journal_for_verified_tree(
            &backup_parent,
            &backup_directory,
            backup_id,
            &verified_tree,
            &[0x31; 32],
        )
        .unwrap();
    publish_cleanup_journal(
        &backup_parent,
        &journal_name,
        &partial_name,
        &cleanup_journal_bytes,
    )
    .unwrap();
    std::fs::rename(&destination, directory.path().join(&cleanup_name)).unwrap();
    std::fs::write(
        directory.path().join(&cleanup_name).join("unexpected"),
        b"preserve",
    )
    .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(
        directory.path().join(&cleanup_name).join("unexpected"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(
        recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).is_err(),
        "recovery must preserve an unverified insertion"
    );
    assert!(
        directory
            .path()
            .join(&cleanup_name)
            .join("unexpected")
            .is_file()
    );
    std::fs::remove_file(directory.path().join(&cleanup_name).join("unexpected")).unwrap();
    assert!(
        resume_authenticated_cleanup_with_hooks(
            &backup_parent,
            &cleanup_journal,
            &[0x31; 32],
            || anyhow::bail!("injected crash after terminal cleanup-root rename"),
            || Ok(()),
        )
        .is_err()
    );
    assert!(
        directory
            .path()
            .join(&cleanup_journal.terminal_cleanup_directory_name)
            .is_dir(),
        "terminal cleanup root must remain recoverable after a crash"
    );
    recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).unwrap();
    assert!(!directory.path().join(&cleanup_name).exists());
    assert!(!directory.path().join(&journal_name).exists());

    backup(
        &config,
        &config.campaign_state_directory,
        1024,
        &destination,
        created_at_unix_ms,
        &release_identity,
        &restore_sources,
    )
    .await
    .unwrap();
    let backup_directory =
        open_cap_directory_nofollow(&backup_parent, Path::new(backup_id)).unwrap();
    let verified_tree = backup_tree_paths_cap(&backup_directory).unwrap();
    let (cleanup_journal, cleanup_journal_bytes, cleanup_name, journal_name, partial_name) =
        cleanup_journal_for_verified_tree(
            &backup_parent,
            &backup_directory,
            backup_id,
            &verified_tree,
            &[0x31; 32],
        )
        .unwrap();
    publish_cleanup_journal(
        &backup_parent,
        &journal_name,
        &partial_name,
        &cleanup_journal_bytes,
    )
    .unwrap();
    std::fs::rename(&destination, directory.path().join(&cleanup_name)).unwrap();
    assert!(
        resume_authenticated_cleanup_with_hooks(
            &backup_parent,
            &cleanup_journal,
            &[0x31; 32],
            || Ok(()),
            || anyhow::bail!("injected crash after terminal cleanup-root removal"),
        )
        .is_err()
    );
    assert!(!directory.path().join(&cleanup_name).exists());
    assert!(directory.path().join(&journal_name).is_file());
    recover_interrupted_complete_cleanups(directory.path(), &[0x31; 32]).unwrap();
    assert!(!directory.path().join(&journal_name).exists());

    backup(
        &config,
        &config.campaign_state_directory,
        1024,
        &destination,
        created_at_unix_ms,
        &release_identity,
        &restore_sources,
    )
    .await
    .unwrap();
    for secret in [
        &config.cursor_secret_path,
        &config.competition_run_grant_secret_path,
        &config.run_preflight_grant_secret_path,
        config.moderation_bearer_token_path.as_ref().unwrap(),
    ] {
        tokio::fs::remove_file(secret).await.unwrap();
    }
    let verified = verify_backup(&destination).await.unwrap();
    preserve_release_authority(directory.path(), &release_manifest_path, &release_identity)
        .await
        .unwrap();
    assert_eq!(
        verify_historical_backup_chain_with_compiled_schema(
            directory.path(),
            &destination,
            &[0x31; 32],
            3,
        )
        .await
        .unwrap()
        .release_identity,
        release_identity,
        "schema-2 verification under simulated schema 3 must bind the full payload, HMAC envelope, and independent VpsV2 authority"
    );
    assert!(
        verify_historical_backup_chain_with_compiled_schema(
            directory.path(),
            &destination,
            &[0x32; 32],
            3,
        )
        .await
        .is_err(),
        "a wrong fifth-secret key must reject the historical envelope"
    );
    let authority_store = directory.path().join(RELEASE_AUTHORITY_STORE);
    let displaced_authority_store = directory.path().join("displaced-release-authorities");
    std::fs::rename(&authority_store, &displaced_authority_store).unwrap();
    assert!(
        verify_historical_backup_chain_with_compiled_schema(
            directory.path(),
            &destination,
            &[0x31; 32],
            3,
        )
        .await
        .is_err(),
        "a missing preserved release authority must reject historical verification"
    );
    std::fs::rename(&displaced_authority_store, &authority_store).unwrap();
    let authority_file =
        authority_store.join(release_authority_file_name(&release_identity).unwrap());
    let displaced_authority = authority_store.join("displaced-authority");
    std::fs::rename(&authority_file, &displaced_authority).unwrap();
    std::fs::write(&authority_file, b"{}").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&authority_file, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        verify_historical_backup_chain_with_compiled_schema(
            directory.path(),
            &destination,
            &[0x31; 32],
            3,
        )
        .await
        .is_err(),
        "forged bytes at the indexed authority path must be rejected"
    );
    std::fs::remove_file(&authority_file).unwrap();
    std::fs::rename(&displaced_authority, &authority_file).unwrap();

    let manifest_path = destination.join("backup-manifest.json");
    let envelope_path = destination.join("backup-verification-envelope.json");
    let original_manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let original_envelope_bytes = std::fs::read(&envelope_path).unwrap();
    let mut mismatched_manifest: BackupManifest =
        serde_json::from_slice(&original_manifest_bytes).unwrap();
    mismatched_manifest
        .release_identity
        .vps_release_manifest_sha256 = "56".repeat(32);
    std::fs::write(
        &manifest_path,
        canonical_json_bytes(&mismatched_manifest).unwrap(),
    )
    .unwrap();
    let mismatched_envelope = BackupVerificationEnvelopeV2::new_authenticated(
        backup_id.to_owned(),
        &mismatched_manifest,
        &[0x31; 32],
    )
    .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(
        &envelope_path,
        canonical_json_bytes(&mismatched_envelope).unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        verify_historical_backup_chain_with_compiled_schema(
            directory.path(),
            &destination,
            &[0x31; 32],
            3,
        )
        .await
        .is_err(),
        "a re-signed backup identity without its exact independently preserved authority must fail"
    );
    std::fs::write(&manifest_path, &original_manifest_bytes).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&envelope_path, &original_envelope_bytes).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let backup_root = pin_directory_capability(&destination).unwrap();
    assert!(
        verify_backup_capability_with_compiled_schema(
            &backup_root,
            destination.file_name().unwrap().to_str().unwrap(),
            &[0x31; 32],
            3,
            true,
        )
        .await
        .is_err(),
        "a schema-2 backup must be historical, not current, under a simulated schema-3 binary"
    );
    assert!(
        verify_backup_capability_with_compiled_schema(
            &backup_root,
            destination.file_name().unwrap().to_str().unwrap(),
            &[0x31; 32],
            1,
            false,
        )
        .await
        .is_err(),
        "pre-VpsV2 schema policy must reject canonical backups"
    );
    assert!(
        verify_backup_with_expected(&destination, &verified.manifest_sha256, &release_identity,)
            .await
            .is_err(),
        "production restore verification must reject a test-only destination layout"
    );
    assert!(
        verify_backup_with_expected(&destination, &"fe".repeat(32), &release_identity,)
            .await
            .is_err()
    );
    let mut wrong_release = release_identity.clone();
    wrong_release.source_commit.replace_range(0..1, "f");
    assert!(
        verify_backup_with_expected(&destination, &verified.manifest_sha256, &wrong_release,)
            .await
            .is_err()
    );

    let backup_database = destination.join("highscores.sqlite3");
    let current_database_bytes = tokio::fs::read(&backup_database).await.unwrap();
    let database_url = format!("sqlite://{}", backup_database.display());
    let mut connection = sqlx::SqliteConnection::connect(&database_url)
        .await
        .unwrap();
    sqlx::query("PRAGMA journal_mode = DELETE")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?")
        .bind(robin_highscores::db::CURRENT_SCHEMA_VERSION)
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    remove_test_database_sidecars(&backup_database).await;
    refresh_database_manifest_entry(&destination).await;
    assert!(verify_backup(&destination).await.is_err());
    tokio::fs::write(&backup_database, &current_database_bytes)
        .await
        .unwrap();
    refresh_database_manifest_entry(&destination).await;
    verify_backup(&destination).await.unwrap();

    let mut connection = sqlx::SqliteConnection::connect(&database_url)
        .await
        .unwrap();
    sqlx::query("PRAGMA journal_mode = DELETE")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET version = ? WHERE version = ?")
        .bind(robin_highscores::db::CURRENT_SCHEMA_VERSION + 1)
        .bind(robin_highscores::db::CURRENT_SCHEMA_VERSION)
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    remove_test_database_sidecars(&backup_database).await;
    refresh_database_manifest_entry(&destination).await;
    assert!(verify_backup(&destination).await.is_err());

    // A backup from a different database schema is never eligible for
    // retention or automatic restore under the no-compatibility contract.
    let mut connection = sqlx::SqliteConnection::connect(&database_url)
        .await
        .unwrap();
    sqlx::query("DROP VIEW campaign_object_submission_references")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    remove_test_database_sidecars(&backup_database).await;
    refresh_database_manifest_entry(&destination).await;
    let manifest_path = destination.join("backup-manifest.json");
    let mut other_schema: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(&manifest_path).await.unwrap()).unwrap();
    other_schema.database_schema_version = robin_highscores::db::CURRENT_SCHEMA_VERSION + 1;
    tokio::fs::write(&manifest_path, canonical_json_bytes(&other_schema).unwrap())
        .await
        .unwrap();
    assert!(verify_backup(&destination).await.is_err());

    // Restore a valid backup before exercising non-database manifest
    // corruption so each assertion has one unambiguous cause.
    tokio::fs::write(&backup_database, &current_database_bytes)
        .await
        .unwrap();
    let mut current_schema: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(&manifest_path).await.unwrap()).unwrap();
    current_schema.database_schema_version = robin_highscores::db::CURRENT_SCHEMA_VERSION;
    tokio::fs::write(
        &manifest_path,
        canonical_json_bytes(&current_schema).unwrap(),
    )
    .await
    .unwrap();
    refresh_database_manifest_entry(&destination).await;
    verify_backup(&destination).await.unwrap();

    let mut connection = sqlx::SqliteConnection::connect(&database_url)
        .await
        .unwrap();
    sqlx::query("UPDATE replay_objects SET byte_length = byte_length + 1")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    remove_test_database_sidecars(&backup_database).await;
    refresh_database_manifest_entry(&destination).await;
    assert!(verify_backup(&destination).await.is_err());
    assert!(
        verify_historical_backup_chain_with_compiled_schema(
            directory.path(),
            &destination,
            &[0x31; 32],
            3,
        )
        .await
        .is_err(),
        "the schema-2 verifier retained by a simulated schema-3 binary must enforce DB/object relational closure"
    );

    tokio::fs::write(&backup_database, &current_database_bytes)
        .await
        .unwrap();
    refresh_database_manifest_entry(&destination).await;
    verify_backup(&destination).await.unwrap();

    let mut connection = sqlx::SqliteConnection::connect(&database_url)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE campaign_objects SET purge_state = 'purging', \
         purge_token = 'backup-test-token-0001', purge_claimed_at_ms = 1",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();
    remove_test_database_sidecars(&backup_database).await;
    refresh_database_manifest_entry(&destination).await;
    assert!(verify_backup(&destination).await.is_err());

    tokio::fs::write(&backup_database, &current_database_bytes)
        .await
        .unwrap();
    refresh_database_manifest_entry(&destination).await;
    verify_backup(&destination).await.unwrap();

    // A purged row has no relational file requirement. Immutable bytes
    // that appeared after the SQLite snapshot may remain as a verified,
    // unreferenced physical extra and are safe for restored GC to reclaim.
    let mut connection = sqlx::SqliteConnection::connect(&database_url)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE replay_objects SET purge_state = 'purged', purged_at_ms = created_at_ms, \
         purge_token = NULL, purge_claimed_at_ms = NULL",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();
    remove_test_database_sidecars(&backup_database).await;
    refresh_database_manifest_entry(&destination).await;
    verify_backup(&destination).await.unwrap();

    tokio::fs::write(&backup_database, &current_database_bytes)
        .await
        .unwrap();
    refresh_database_manifest_entry(&destination).await;
    verify_backup(&destination).await.unwrap();

    tokio::fs::write(destination.join("unexpected"), b"unlisted")
        .await
        .unwrap();
    assert!(verify_backup(&destination).await.is_err());
    tokio::fs::remove_file(destination.join("unexpected"))
        .await
        .unwrap();
    tokio::fs::write(destination.join("restore/state/cursor-hmac.key"), [9; 32])
        .await
        .unwrap();
    assert!(verify_backup(&destination).await.is_err());
    tokio::fs::write(destination.join("restore/state/cursor-hmac.key"), [7; 32])
        .await
        .unwrap();
    verify_backup(&destination).await.unwrap();

    // Removing both a live object and its manifest entry must still fail:
    // relational closure is checked against the restored database, not
    // inferred merely from the archive's self-consistent file inventory.
    let replay_relative = format!(
        "replays/{}/{}/{}.rhrec",
        &hex::encode(replay_digest)[..2],
        &hex::encode(replay_digest)[2..4],
        hex::encode(replay_digest)
    );
    tokio::fs::remove_file(destination.join(&replay_relative))
        .await
        .unwrap();
    let manifest_path = destination.join("backup-manifest.json");
    let mut manifest: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(&manifest_path).await.unwrap()).unwrap();
    manifest
        .files
        .retain(|entry| entry.relative_path != replay_relative);
    tokio::fs::write(&manifest_path, canonical_json_bytes(&manifest).unwrap())
        .await
        .unwrap();
    assert!(verify_backup(&destination).await.is_err());
}
