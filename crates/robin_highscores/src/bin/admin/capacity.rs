//! Conservative backup capacity accounting; does not grant write authority.

use super::policy::BACKUP_MANIFEST_SCHEMA_VERSION;
use super::policy::RELEASE_AUTHORITY_STORE;
use super::policy::SYSTEMD_UNIT_FILES;
use super::policy::SYSTEMD_USER_ROOT;
use super::sources::release_authority_file_name;
use super::sources::validate_backup_restore_source_contract;
use super::sources::validate_installed_unit_source;
use super::sources::validate_secret_source;
use robin_highscores::CampaignStore;
use robin_highscores::ReplayStore;
use robin_highscores::ServerConfig;
use robin_highscores::backup::BackupFileV4 as BackupFile;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupReleaseIdentityV2;
use robin_highscores::backup::BackupRestoreSourceV4 as RestoreSource;
use robin_highscores::backup::BackupSpaceEstimateV1;
use robin_highscores::backup::BackupStatusV4;
use robin_highscores::backup::canonical_backup_directories_v4;
use robin_run_protocol::canonical_json_bytes;
use sha2::Digest as _;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

const FIXED_BACKUP_DIRECTORY_COUNT: u64 = 7;

const FIXED_BACKUP_FILE_COUNT: u64 = 3;

#[derive(Debug, Default)]
struct BackupCopyTopology {
    regular_files: u64,
    directories: u64,
    dense_file_bytes: u64,
}

async fn prospective_backup_document_lengths(
    config: &ServerConfig,
    release_identity: &BackupReleaseIdentityV2,
    canonical_backup_root: &Path,
    readable_sources: &BTreeMap<PathBuf, PathBuf>,
    admission_demand: robin_highscores::storage_admission::CapacityDemandBytes,
) -> anyhow::Result<(u64, u64)> {
    let placeholder_sha256 = "01".repeat(32);
    let mut files = Vec::new();
    let mut database_upper = admission_demand.database;
    for path in [
        config.database_path.clone(),
        PathBuf::from(format!("{}-wal", config.database_path.display())),
        PathBuf::from(format!("{}-shm", config.database_path.display())),
    ] {
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) => {
                anyhow::ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "prospective database source is not a regular file"
                );
                database_upper = database_upper
                    .checked_add(metadata.len())
                    .ok_or_else(|| anyhow::anyhow!("prospective database length overflows"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    files.push(BackupFile {
        relative_path: "highscores.sqlite3".to_owned(),
        byte_length: database_upper.max(1),
        sha256: placeholder_sha256.clone(),
    });

    let mut replay_digests = BTreeSet::new();
    if tokio::fs::try_exists(&config.replay_directory).await? {
        let replay =
            ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes).await?;
        let mut cursor = None;
        loop {
            let page = replay.inventory_page(cursor, 10_000).await?;
            if page.is_empty() {
                break;
            }
            for entry in &page {
                let digest = hex::encode(entry.sha256);
                anyhow::ensure!(
                    replay_digests.insert(digest.clone()),
                    "replay inventory repeats"
                );
                files.push(BackupFile {
                    relative_path: format!(
                        "replays/{}/{}/{}.rhrec",
                        &digest[..2],
                        &digest[2..4],
                        digest
                    ),
                    byte_length: entry.bytes,
                    sha256: digest,
                });
            }
            cursor = page.last().map(|entry| entry.sha256);
            if page.len() < 10_000 {
                break;
            }
        }
    }
    let mut campaign_digests = BTreeSet::new();
    if tokio::fs::try_exists(&config.campaign_state_directory).await? {
        let campaign = CampaignStore::create(
            config.campaign_state_directory.clone(),
            config.max_campaign_bytes,
        )
        .await?;
        for entry in campaign.inventory().await? {
            let digest = hex::encode(entry.sha256);
            anyhow::ensure!(
                campaign_digests.insert(digest.clone()),
                "campaign inventory repeats"
            );
            files.push(BackupFile {
                relative_path: format!("campaigns/{}/{}.campaign", &digest[..2], digest),
                byte_length: entry.bytes,
                sha256: digest,
            });
        }
    }

    let concurrent_slots = u64::try_from(config.max_concurrent_uploads)?;
    for slot in 0..concurrent_slots {
        let mut nonce = slot;
        let digest = loop {
            let digest = hex::encode(Sha256::digest(
                format!("prospective-replay-{nonce}").as_bytes(),
            ));
            if replay_digests.insert(digest.clone()) {
                break digest;
            }
            nonce = nonce
                .checked_add(concurrent_slots)
                .ok_or_else(|| anyhow::anyhow!("prospective replay nonce overflows"))?;
        };
        files.push(BackupFile {
            relative_path: format!(
                "replays/{}/{}/{}.rhrec",
                &digest[..2],
                &digest[2..4],
                digest
            ),
            byte_length: config.max_replay_bytes,
            sha256: digest,
        });
    }
    for slot in 0..=concurrent_slots {
        let mut nonce = slot;
        let digest = loop {
            let digest = hex::encode(Sha256::digest(
                format!("prospective-campaign-{nonce}").as_bytes(),
            ));
            if campaign_digests.insert(digest.clone()) {
                break digest;
            }
            nonce =
                nonce
                    .checked_add(concurrent_slots.checked_add(1).ok_or_else(|| {
                        anyhow::anyhow!("prospective campaign slot count overflows")
                    })?)
                    .ok_or_else(|| anyhow::anyhow!("prospective campaign nonce overflows"))?;
        };
        files.push(BackupFile {
            relative_path: format!("campaigns/{}/{}.campaign", &digest[..2], digest),
            byte_length: config.max_campaign_bytes,
            sha256: digest,
        });
    }

    for (original, archive, exact_length) in [
        (
            &config.cursor_secret_path,
            "restore/state/cursor-hmac.key",
            Some(32_u64),
        ),
        (
            &config.competition_run_grant_secret_path,
            "restore/state/competition-run-grant.key",
            Some(32_u64),
        ),
        (
            &config.run_preflight_grant_secret_path,
            "restore/state/run-preflight-grant.key",
            Some(32_u64),
        ),
    ] {
        let readable = readable_sources
            .get(original)
            .map(PathBuf::as_path)
            .unwrap_or(original);
        let length = std::fs::metadata(readable)?.len();
        anyhow::ensure!(exact_length.is_none_or(|expected| length == expected));
        files.push(BackupFile {
            relative_path: archive.to_owned(),
            byte_length: length,
            sha256: placeholder_sha256.clone(),
        });
    }
    let moderation = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("prospective backup requires moderation authority"))?;
    let moderation_readable = readable_sources
        .get(moderation)
        .map(PathBuf::as_path)
        .unwrap_or(moderation);
    files.push(BackupFile {
        relative_path: "restore/state/moderation-bearer.token".to_owned(),
        byte_length: std::fs::metadata(moderation_readable)?.len(),
        sha256: placeholder_sha256,
    });
    for unit in &release_identity.installed_user_units {
        let name = unit
            .release_relative_path
            .strip_prefix("systemd/user/")
            .ok_or_else(|| anyhow::anyhow!("release unit path is not canonical"))?;
        files.push(BackupFile {
            relative_path: format!("restore/systemd/user/{name}"),
            byte_length: unit.artifact.byte_length,
            sha256: unit.artifact.sha256.to_string(),
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    anyhow::ensure!(
        files.len() <= robin_highscores::backup::MAX_BACKUP_MANIFEST_FILES,
        "prospective backup manifest exceeds its file-count limit"
    );

    let moderation_original = config
        .moderation_bearer_token_path
        .as_ref()
        .expect("moderation authority checked above");
    let mut restore_sources = vec![
        RestoreSource {
            original_absolute_path: config.database_path.to_string_lossy().into_owned(),
            archive_relative_path: "highscores.sqlite3".to_owned(),
        },
        RestoreSource {
            original_absolute_path: config.replay_directory.to_string_lossy().into_owned(),
            archive_relative_path: "replays".to_owned(),
        },
        RestoreSource {
            original_absolute_path: config
                .campaign_state_directory
                .to_string_lossy()
                .into_owned(),
            archive_relative_path: "campaigns".to_owned(),
        },
        RestoreSource {
            original_absolute_path: config.cursor_secret_path.to_string_lossy().into_owned(),
            archive_relative_path: "restore/state/cursor-hmac.key".to_owned(),
        },
        RestoreSource {
            original_absolute_path: config
                .competition_run_grant_secret_path
                .to_string_lossy()
                .into_owned(),
            archive_relative_path: "restore/state/competition-run-grant.key".to_owned(),
        },
        RestoreSource {
            original_absolute_path: config
                .run_preflight_grant_secret_path
                .to_string_lossy()
                .into_owned(),
            archive_relative_path: "restore/state/run-preflight-grant.key".to_owned(),
        },
        RestoreSource {
            original_absolute_path: moderation_original.to_string_lossy().into_owned(),
            archive_relative_path: "restore/state/moderation-bearer.token".to_owned(),
        },
    ];
    restore_sources.extend(SYSTEMD_UNIT_FILES.into_iter().map(|unit| {
        RestoreSource {
            original_absolute_path: Path::new(SYSTEMD_USER_ROOT)
                .join(unit)
                .to_string_lossy()
                .into_owned(),
            archive_relative_path: format!("restore/systemd/user/{unit}"),
        }
    }));
    restore_sources
        .sort_by(|left, right| left.archive_relative_path.cmp(&right.archive_relative_path));
    let manifest = BackupManifest {
        schema_version: BACKUP_MANIFEST_SCHEMA_VERSION,
        created_at_unix_ms: u64::MAX,
        database_schema_version: robin_highscores::db::CURRENT_SCHEMA_VERSION,
        release_identity: release_identity.clone(),
        root_unix_mode: 0o700,
        restore_sources,
        directories: canonical_backup_directories_v4(&files)?,
        files,
    };
    manifest.validate()?;
    let manifest_bytes = canonical_json_bytes(&manifest)?;
    let status = BackupStatusV4::new_authenticated(
        format!("backup-v4-{}-{}", u64::MAX, "f".repeat(32)),
        canonical_backup_root
            .join(format!("backup-v4-{}-{}", u64::MAX, "f".repeat(32)))
            .to_string_lossy()
            .into_owned(),
        manifest,
        &[0_u8; 32],
    )?;
    let status_bytes = canonical_json_bytes(&status)?;
    anyhow::ensure!(
        manifest_bytes.len() <= robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES
            && status_bytes.len() <= robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
        "prospective canonical backup manifest or compact status exceeds its byte limit"
    );
    Ok((
        u64::try_from(manifest_bytes.len())?,
        u64::try_from(status_bytes.len())?,
    ))
}

pub(super) async fn estimate_backup_space(
    config: &ServerConfig,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<BackupSpaceEstimateV1> {
    release_identity.validate()?;
    anyhow::ensure!(
        release_identity.database_schema_version == robin_highscores::db::CURRENT_SCHEMA_VERSION,
        "backup-space release database schema differs from the running authority"
    );
    validate_backup_restore_source_contract(config, restore_sources)?;
    let canonical_backup_root = tokio::fs::canonicalize(backup_root).await?;
    anyhow::ensure!(
        canonical_backup_root == backup_root,
        "backup-space root must be canonical"
    );
    let status_parent = status_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup status path has no parent"))?;
    let canonical_status_parent = tokio::fs::canonicalize(status_parent).await?;
    anyhow::ensure!(
        canonical_status_parent == status_parent,
        "backup-space status parent must be canonical"
    );
    #[cfg(unix)]
    let destination_device_id = {
        use std::os::unix::fs::MetadataExt as _;
        let backup_metadata = std::fs::metadata(&canonical_backup_root)?;
        anyhow::ensure!(
            backup_metadata.dev() == std::fs::metadata(&canonical_status_parent)?.dev(),
            "backup payload and status must use one filesystem capacity authority"
        );
        backup_metadata.dev()
    };
    #[cfg(not(unix))]
    let destination_device_id = 0;
    #[cfg(unix)]
    let (
        allocation_granularity,
        observed_available_bytes,
        observed_available_inode_count,
        destination_filesystem_id,
    ) = {
        let filesystem = rustix::fs::statvfs(&canonical_backup_root)?;
        (
            filesystem.f_frsize,
            filesystem
                .f_frsize
                .checked_mul(filesystem.f_bavail)
                .ok_or_else(|| anyhow::anyhow!("backup available-space snapshot overflows"))?,
            filesystem.f_favail,
            filesystem.f_fsid,
        )
    };
    #[cfg(not(unix))]
    let (
        allocation_granularity,
        observed_available_bytes,
        observed_available_inode_count,
        destination_filesystem_id,
    ) = {
        let filesystem = fs2::statvfs(&canonical_backup_root)?;
        (
            filesystem.allocation_granularity(),
            filesystem.available_space(),
            0,
            0,
        )
    };
    anyhow::ensure!(
        allocation_granularity > 0,
        "backup filesystem reports zero allocation granularity"
    );
    for (path, exact_length) in [
        (&config.cursor_secret_path, Some(32)),
        (&config.competition_run_grant_secret_path, Some(32)),
        (&config.run_preflight_grant_secret_path, Some(32)),
    ] {
        let readable = restore_sources
            .get(path)
            .map(PathBuf::as_path)
            .unwrap_or(path);
        validate_secret_source(readable, exact_length)?;
    }
    let moderation = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup-space estimate requires moderation authority"))?;
    validate_secret_source(
        restore_sources
            .get(moderation)
            .map(PathBuf::as_path)
            .unwrap_or(moderation),
        None,
    )?;
    for unit in SYSTEMD_UNIT_FILES {
        let original = Path::new(SYSTEMD_USER_ROOT).join(unit);
        let readable = restore_sources
            .get(&original)
            .map(PathBuf::as_path)
            .unwrap_or(&original);
        let authority = release_identity
            .installed_user_units
            .iter()
            .find(|authority| authority.release_relative_path == format!("systemd/user/{unit}"))
            .ok_or_else(|| anyhow::anyhow!("release omits backup unit authority for {unit}"))?;
        validate_installed_unit_source(readable, authority)?;
    }
    let mut paths = vec![
        config.database_path.clone(),
        PathBuf::from(format!("{}-wal", config.database_path.display())),
        PathBuf::from(format!("{}-shm", config.database_path.display())),
        config.replay_directory.clone(),
        config.campaign_state_directory.clone(),
        restore_sources
            .get(&config.cursor_secret_path)
            .cloned()
            .unwrap_or_else(|| config.cursor_secret_path.clone()),
        restore_sources
            .get(&config.competition_run_grant_secret_path)
            .cloned()
            .unwrap_or_else(|| config.competition_run_grant_secret_path.clone()),
        restore_sources
            .get(&config.run_preflight_grant_secret_path)
            .cloned()
            .unwrap_or_else(|| config.run_preflight_grant_secret_path.clone()),
    ];
    if let Some(path) = &config.moderation_bearer_token_path {
        paths.push(
            restore_sources
                .get(path)
                .cloned()
                .unwrap_or_else(|| path.clone()),
        );
    }
    for unit in SYSTEMD_UNIT_FILES {
        let original = Path::new(SYSTEMD_USER_ROOT).join(unit);
        paths.push(restore_sources.get(&original).cloned().unwrap_or(original));
    }
    let mut canonical_seen = BTreeSet::new();
    for path in paths {
        let canonical = match tokio::fs::canonicalize(&path).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        canonical_seen.insert(canonical);
    }
    let canonical_roots = canonical_seen
        .iter()
        .filter(|candidate| {
            !canonical_seen
                .iter()
                .any(|other| other != *candidate && candidate.starts_with(other))
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut topology = BackupCopyTopology::default();
    for root in canonical_roots {
        add_regular_tree_capacity(&root, allocation_granularity, &mut topology).await?;
    }
    let release_authority_root = canonical_backup_root.join(RELEASE_AUTHORITY_STORE);
    let release_authority_file =
        release_authority_root.join(release_authority_file_name(release_identity)?);
    if !tokio::fs::try_exists(&release_authority_root).await? {
        topology.directories = topology
            .directories
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("release-authority directory count overflows"))?;
    }
    if !tokio::fs::try_exists(&release_authority_file).await? {
        let release_bytes = tokio::fs::metadata(
            config
                .release_manifest_path
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("backup-space estimate lacks release manifest"))?,
        )
        .await?
        .len();
        topology.regular_files = topology
            .regular_files
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("release-authority file count overflows"))?;
        topology.dense_file_bytes = topology
            .dense_file_bytes
            .checked_add(round_up_to_allocation(
                release_bytes,
                allocation_granularity,
            )?)
            .ok_or_else(|| anyhow::anyhow!("release-authority capacity overflows"))?;
    }
    anyhow::ensure!(
        topology
            .regular_files
            .checked_add(topology.directories)
            .is_some_and(|nodes| {
                nodes
                    <= u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_FILES)
                        .expect("manifest file limit fits u64")
            }),
        "backup-space source closure exceeds the managed topology limit"
    );
    let concurrent_slots = u64::try_from(config.max_concurrent_uploads)?;
    let concurrent_file_count = concurrent_slots
        .checked_mul(2)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| anyhow::anyhow!("backup-space concurrent file count overflows"))?;
    // One replay can require two digest-shard directories and one campaign
    // object one shard. Assume none of those directories existed at admission.
    let concurrent_directory_count = concurrent_slots
        .checked_mul(3)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| anyhow::anyhow!("backup-space concurrent directory count overflows"))?;
    let copied_file_count = topology
        .regular_files
        .checked_add(FIXED_BACKUP_FILE_COUNT)
        .and_then(|count| count.checked_add(concurrent_file_count))
        .ok_or_else(|| anyhow::anyhow!("backup-space file count overflows"))?;
    let copied_directory_count = topology
        .directories
        .checked_add(FIXED_BACKUP_DIRECTORY_COUNT)
        .and_then(|count| count.checked_add(concurrent_directory_count))
        .ok_or_else(|| anyhow::anyhow!("backup-space directory count overflows"))?;
    anyhow::ensure!(
        copied_file_count <= u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_FILES)?,
        "backup-space prospective files exceed the manifest entry limit"
    );
    let required_inode_count = copied_file_count
        .checked_add(copied_directory_count)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| anyhow::anyhow!("backup-space inode count overflows"))?;
    // Charging a whole destination fragment for every prospective directory
    // and directory entry bounds tiny-file topology rather than pretending
    // that one million one-byte objects need only one megabyte of scratch.
    let directory_and_entry_overhead_bytes =
        conservative_entry_overhead(required_inode_count, allocation_granularity)?;
    let admission_demand =
        robin_highscores::storage_admission::maximum_capacity_demand_bytes(config)?;
    let (manifest_logical_upper_bound_bytes, status_temp_logical_upper_bound_bytes) =
        prospective_backup_document_lengths(
            config,
            release_identity,
            &canonical_backup_root,
            restore_sources,
            admission_demand,
        )
        .await?;
    let manifest_allocation_upper_bound_bytes =
        round_up_to_allocation(manifest_logical_upper_bound_bytes, allocation_granularity)?;
    let status_temp_allocation_upper_bound_bytes = round_up_to_allocation(
        status_temp_logical_upper_bound_bytes,
        allocation_granularity,
    )?;
    let concurrent_object_margin_bytes =
        round_up_to_allocation(config.max_replay_bytes, allocation_granularity)?
            .checked_mul(concurrent_slots)
            .and_then(|replays| {
                round_up_to_allocation(config.max_campaign_bytes, allocation_granularity)
                    .ok()?
                    .checked_mul(concurrent_slots.checked_add(1)?)
                    .and_then(|campaigns| replays.checked_add(campaigns))
            })
            .ok_or_else(|| anyhow::anyhow!("backup-space concurrent object margin overflows"))?;
    anyhow::ensure!(
        admission_demand.replay
            == concurrent_slots
                .checked_mul(config.max_replay_bytes)
                .ok_or_else(|| anyhow::anyhow!("backup-space replay demand overflows"))?
            && admission_demand.campaign
                == concurrent_slots
                    .checked_add(1)
                    .and_then(|slots| slots.checked_mul(config.max_campaign_bytes))
                    .ok_or_else(|| anyhow::anyhow!("backup-space campaign demand overflows"))?,
        "backup-space object allowance drifted from shared admission policy"
    );
    let concurrent_database_margin_bytes =
        round_up_to_allocation(admission_demand.database, allocation_granularity)?;
    let required_scratch_bytes = topology
        .dense_file_bytes
        .checked_add(directory_and_entry_overhead_bytes)
        .and_then(|bytes| bytes.checked_add(manifest_allocation_upper_bound_bytes))
        .and_then(|bytes| bytes.checked_add(status_temp_allocation_upper_bound_bytes))
        .and_then(|bytes| bytes.checked_add(concurrent_object_margin_bytes))
        .and_then(|bytes| bytes.checked_add(concurrent_database_margin_bytes))
        .ok_or_else(|| anyhow::anyhow!("backup-space scratch total overflows"))?;
    let required_available_bytes = required_scratch_bytes
        .checked_add(config.minimum_storage_free_bytes)
        .ok_or_else(|| anyhow::anyhow!("backup-space available total overflows"))?;
    let restore_source_map_bytes = canonical_json_bytes(
        &restore_sources
            .iter()
            .map(|(original, readable)| {
                (
                    original.to_string_lossy().into_owned(),
                    readable.to_string_lossy().into_owned(),
                )
            })
            .collect::<Vec<_>>(),
    )?;
    let effective_config_sha256 = hex::encode(Sha256::digest(canonical_json_bytes(config)?));
    let estimate = BackupSpaceEstimateV1 {
        schema_version: robin_highscores::backup::BACKUP_SPACE_ESTIMATE_SCHEMA_VERSION,
        backup_root: canonical_backup_root.to_string_lossy().into_owned(),
        status_path: status_path.to_string_lossy().into_owned(),
        release_identity: release_identity.clone(),
        effective_config_sha256,
        restore_source_map_count: u64::try_from(restore_sources.len())?,
        restore_source_map_sha256: hex::encode(Sha256::digest(&restore_source_map_bytes)),
        destination_device_id,
        destination_filesystem_id,
        allocation_granularity_bytes: allocation_granularity,
        copied_file_count,
        copied_directory_count,
        maximum_transient_file_count: 1,
        dense_payload_bytes: topology.dense_file_bytes,
        directory_and_entry_overhead_bytes,
        manifest_logical_upper_bound_bytes,
        manifest_allocation_upper_bound_bytes,
        status_temp_logical_upper_bound_bytes,
        status_temp_allocation_upper_bound_bytes,
        maximum_concurrent_uploads: concurrent_slots,
        maximum_concurrent_requests: u64::try_from(config.max_concurrent_requests)?,
        maximum_replay_bytes: config.max_replay_bytes,
        maximum_campaign_bytes: config.max_campaign_bytes,
        maximum_metadata_bytes: u64::try_from(config.max_metadata_bytes)?,
        concurrent_object_margin_bytes,
        concurrent_database_margin_bytes,
        required_scratch_bytes,
        minimum_storage_free_bytes: config.minimum_storage_free_bytes,
        required_available_bytes,
        observed_available_bytes,
        required_inode_count,
        observed_available_inode_count,
    };
    estimate.validate()?;
    Ok(estimate)
}

pub(super) fn round_up_to_allocation(bytes: u64, allocation: u64) -> anyhow::Result<u64> {
    anyhow::ensure!(allocation > 0, "allocation granularity must be positive");
    if bytes == 0 {
        return Ok(0);
    }
    bytes
        .checked_add(allocation - 1)
        .map(|value| value / allocation * allocation)
        .ok_or_else(|| anyhow::anyhow!("backup-space allocation rounding overflows"))
}

fn conservative_entry_overhead(node_count: u64, allocation: u64) -> anyhow::Result<u64> {
    anyhow::ensure!(allocation > 0, "allocation granularity must be positive");
    node_count
        .checked_mul(allocation)
        .ok_or_else(|| anyhow::anyhow!("backup-space entry overhead overflows"))
}

async fn add_regular_tree_capacity(
    root: &Path,
    allocation: u64,
    total: &mut BackupCopyTopology,
) -> anyhow::Result<()> {
    let metadata = tokio::fs::symlink_metadata(root).await?;
    anyhow::ensure!(
        !metadata.file_type().is_symlink(),
        "backup size source must not be a symlink"
    );
    if metadata.is_file() {
        total.regular_files = total
            .regular_files
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("backup-space file count overflows"))?;
        total.dense_file_bytes = total
            .dense_file_bytes
            .checked_add(round_up_to_allocation(metadata.len(), allocation)?)
            .ok_or_else(|| anyhow::anyhow!("backup-space dense bytes overflow"))?;
        return Ok(());
    }
    anyhow::ensure!(metadata.is_dir(), "backup size source must be regular");
    total.directories = total
        .directories
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("backup-space directory count overflows"))?;
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "backup size tree contains a symlink"
            );
            if metadata.is_dir() {
                total.directories = total
                    .directories
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("backup-space directory count overflows"))?;
                pending.push(entry.path());
            } else {
                anyhow::ensure!(metadata.is_file(), "backup size tree is not regular");
                total.regular_files = total
                    .regular_files
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("backup-space file count overflows"))?;
                total.dense_file_bytes = total
                    .dense_file_bytes
                    .checked_add(round_up_to_allocation(metadata.len(), allocation)?)
                    .ok_or_else(|| anyhow::anyhow!("backup-space dense bytes overflow"))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
