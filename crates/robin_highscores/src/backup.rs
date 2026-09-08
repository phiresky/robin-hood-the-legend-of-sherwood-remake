//! Authenticated publication format for the most recent verified backup.
//!
//! The readiness envelope is deliberately separate from the backup payload.
//! The administration process writes one canonical authenticated file only
//! after completing and independently verifying a backup. The compact HMAC
//! summary binds the protected payload manifest by digest without duplicating
//! its potentially large file inventory into the API-readable status file.

use ring::hmac;
use robin_run_protocol::{ArtifactRefV1, Digest32, Validate as _, canonical_json_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt as _;

pub const BACKUP_STATUS_SCHEMA_VERSION: u32 = 4;
pub const BACKUP_SPACE_ESTIMATE_SCHEMA_VERSION: u32 = 1;
pub const MAX_BACKUP_STATUS_BYTES: usize = 64 * 1024;
pub const MAX_BACKUP_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_BACKUP_MANIFEST_FILES: usize = 1_000_000;
pub const MAX_BACKUP_TREE_DEPTH: usize = 32;
pub const MAX_BACKUP_COMPONENT_BYTES: usize = 255;
pub const MAX_BACKUP_RELATIVE_PATH_BYTES: usize = 4_096;
const BACKUP_STATUS_DOMAIN: &[u8] = b"robinhood/highscores/backup-status/4\0";
const BACKUP_VERIFICATION_ENVELOPE_DOMAIN: &[u8] =
    b"robinhood/highscores/backup-verification-envelope/2\0";
const BACKUP_CLEANUP_JOURNAL_DOMAIN: &[u8] = b"robinhood/highscores/backup-cleanup-journal/1\0";
const RELEASE_MANIFEST_SCHEMA_VERSION: u32 = 2;
pub const VPS_RELEASE_V2_MIN_DATABASE_SCHEMA_VERSION: i64 = 2;
const MAX_RELEASE_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// Canonical, machine-readable proof of the additional filesystem capacity
/// required to create one backup generation. Existing releases and retained
/// backups are deliberately absent: their allocation is already reflected in
/// `observed_available_bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupSpaceEstimateV1 {
    pub schema_version: u32,
    pub backup_root: String,
    pub status_path: String,
    pub release_identity: BackupReleaseIdentityV2,
    pub effective_config_sha256: String,
    pub restore_source_map_count: u64,
    pub restore_source_map_sha256: String,
    pub destination_device_id: u64,
    pub destination_filesystem_id: u64,
    pub allocation_granularity_bytes: u64,
    pub copied_file_count: u64,
    pub copied_directory_count: u64,
    /// Peak extra inode used by either SQLite scrub journaling or atomic
    /// status publication. Those phases are sequential, so this is not two.
    pub maximum_transient_file_count: u64,
    pub dense_payload_bytes: u64,
    pub directory_and_entry_overhead_bytes: u64,
    pub manifest_logical_upper_bound_bytes: u64,
    pub manifest_allocation_upper_bound_bytes: u64,
    pub status_temp_logical_upper_bound_bytes: u64,
    pub status_temp_allocation_upper_bound_bytes: u64,
    pub maximum_concurrent_uploads: u64,
    pub maximum_concurrent_requests: u64,
    pub maximum_replay_bytes: u64,
    pub maximum_campaign_bytes: u64,
    pub maximum_metadata_bytes: u64,
    pub concurrent_object_margin_bytes: u64,
    pub concurrent_database_margin_bytes: u64,
    pub required_scratch_bytes: u64,
    pub minimum_storage_free_bytes: u64,
    pub required_available_bytes: u64,
    pub observed_available_bytes: u64,
    pub required_inode_count: u64,
    pub observed_available_inode_count: u64,
}

fn round_backup_allocation(bytes: u64, allocation: u64) -> anyhow::Result<u64> {
    anyhow::ensure!(allocation > 0, "backup allocation granularity is zero");
    if bytes == 0 {
        return Ok(0);
    }
    bytes
        .checked_add(allocation - 1)
        .map(|value| value / allocation)
        .and_then(|fragments| fragments.checked_mul(allocation))
        .ok_or_else(|| anyhow::anyhow!("backup-space allocation rounding overflows"))
}

impl BackupSpaceEstimateV1 {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == BACKUP_SPACE_ESTIMATE_SCHEMA_VERSION,
            "unsupported backup-space estimate schema"
        );
        anyhow::ensure!(
            Path::new(&self.backup_root).is_absolute()
                && Path::new(&self.status_path).is_absolute(),
            "backup-space estimate paths must be absolute"
        );
        self.release_identity.validate()?;
        validate_digest(
            &self.effective_config_sha256,
            "backup-space effective-config digest",
        )?;
        anyhow::ensure!(
            self.restore_source_map_count > 0,
            "backup-space estimate has no restore-source maps"
        );
        validate_digest(
            &self.restore_source_map_sha256,
            "backup-space restore-source-map digest",
        )?;
        anyhow::ensure!(
            self.allocation_granularity_bytes > 0
                && self.allocation_granularity_bytes.is_power_of_two(),
            "backup-space allocation granularity is invalid"
        );
        anyhow::ensure!(
            self.copied_file_count > 0 && self.copied_directory_count > 0,
            "backup-space estimate has an empty copy topology"
        );
        anyhow::ensure!(
            self.maximum_transient_file_count == 1,
            "backup-space estimate must reserve the single transient file peak"
        );
        anyhow::ensure!(
            (1..=u64::try_from(MAX_BACKUP_MANIFEST_BYTES)?)
                .contains(&self.manifest_logical_upper_bound_bytes),
            "backup-space manifest logical bound exceeds the compiled limit"
        );
        anyhow::ensure!(
            self.manifest_allocation_upper_bound_bytes
                == round_backup_allocation(
                    self.manifest_logical_upper_bound_bytes,
                    self.allocation_granularity_bytes,
                )?,
            "backup-space manifest allocation bound is inconsistent"
        );
        anyhow::ensure!(
            (1..=u64::try_from(MAX_BACKUP_STATUS_BYTES)?)
                .contains(&self.status_temp_logical_upper_bound_bytes),
            "backup-space status logical bound is invalid"
        );
        anyhow::ensure!(
            self.status_temp_allocation_upper_bound_bytes
                == round_backup_allocation(
                    self.status_temp_logical_upper_bound_bytes,
                    self.allocation_granularity_bytes,
                )?,
            "backup-space status allocation bound is inconsistent"
        );
        let required_scratch = self
            .dense_payload_bytes
            .checked_add(self.directory_and_entry_overhead_bytes)
            .and_then(|bytes| bytes.checked_add(self.manifest_allocation_upper_bound_bytes))
            .and_then(|bytes| bytes.checked_add(self.status_temp_allocation_upper_bound_bytes))
            .and_then(|bytes| bytes.checked_add(self.concurrent_object_margin_bytes))
            .and_then(|bytes| bytes.checked_add(self.concurrent_database_margin_bytes))
            .ok_or_else(|| anyhow::anyhow!("backup-space scratch requirement overflows"))?;
        anyhow::ensure!(
            self.required_scratch_bytes == required_scratch,
            "backup-space scratch total is inconsistent"
        );
        anyhow::ensure!(
            self.required_available_bytes
                == self
                    .required_scratch_bytes
                    .checked_add(self.minimum_storage_free_bytes)
                    .ok_or_else(|| anyhow::anyhow!("backup-space available total overflows"))?,
            "backup-space available total is inconsistent"
        );
        anyhow::ensure!(
            self.required_inode_count
                == self
                    .copied_file_count
                    .checked_add(self.copied_directory_count)
                    .and_then(|count| count.checked_add(self.maximum_transient_file_count))
                    .ok_or_else(|| anyhow::anyhow!("backup-space inode count overflows"))?,
            "backup-space inode requirement is inconsistent"
        );
        anyhow::ensure!(
            self.directory_and_entry_overhead_bytes
                == self
                    .required_inode_count
                    .checked_mul(self.allocation_granularity_bytes)
                    .ok_or_else(|| anyhow::anyhow!("backup-space entry overhead overflows"))?,
            "backup-space entry overhead is inconsistent"
        );
        anyhow::ensure!(
            self.maximum_concurrent_uploads > 0
                && self.maximum_concurrent_requests > 0
                && self.maximum_replay_bytes > 0
                && self.maximum_campaign_bytes > 0
                && self.maximum_metadata_bytes > 0,
            "backup-space concurrency limits must be positive"
        );
        anyhow::ensure!(
            self.concurrent_object_margin_bytes == {
                let replays = round_backup_allocation(
                    self.maximum_replay_bytes,
                    self.allocation_granularity_bytes,
                )?
                .checked_mul(self.maximum_concurrent_uploads)
                .ok_or_else(|| anyhow::anyhow!("backup-space replay margin overflows"))?;
                let campaigns = round_backup_allocation(
                    self.maximum_campaign_bytes,
                    self.allocation_granularity_bytes,
                )?
                .checked_mul(
                    self.maximum_concurrent_uploads
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("backup-space slot count overflows"))?,
                )
                .ok_or_else(|| anyhow::anyhow!("backup-space campaign margin overflows"))?;
                replays
                    .checked_add(campaigns)
                    .ok_or_else(|| anyhow::anyhow!("backup-space object margin overflows"))?
            },
            "backup-space concurrent-object margin is inconsistent"
        );
        anyhow::ensure!(
            self.concurrent_database_margin_bytes
                == round_backup_allocation(
                    self.maximum_concurrent_uploads
                        .checked_mul(
                            self.maximum_metadata_bytes
                                .checked_add(
                                    crate::storage_admission::MULTIPART_FRAMING_HEADROOM_BYTES,
                                )
                                .ok_or_else(|| {
                                    anyhow::anyhow!("backup-space per-request DB margin overflows")
                                })?,
                        )
                        .and_then(|bytes| {
                            self.maximum_concurrent_requests
                                .checked_add(
                                    crate::storage_admission::MAXIMUM_AUXILIARY_DATABASE_WRITERS,
                                )
                                .and_then(|writers| {
                                    writers.checked_mul(self.maximum_metadata_bytes)
                                })
                                .and_then(|metadata| bytes.checked_add(metadata))
                        })
                        .and_then(|bytes| {
                            bytes.checked_add(crate::storage_admission::SQLITE_WAL_HEADROOM_BYTES)
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!("backup-space database margin overflows")
                        })?,
                    self.allocation_granularity_bytes,
                )?,
            "backup-space database margin differs from shared admission policy"
        );
        Ok(())
    }

    pub fn ensure_available(&self) -> anyhow::Result<()> {
        self.validate()?;
        anyhow::ensure!(
            self.observed_available_bytes >= self.required_available_bytes,
            "insufficient backup capacity: {} bytes available, {} required for one backup scratch plus the configured floor",
            self.observed_available_bytes,
            self.required_available_bytes
        );
        // Some filesystems report no inode accounting. A nonzero report is
        // authoritative and must satisfy the exact prospective node count.
        if self.observed_available_inode_count > 0 {
            anyhow::ensure!(
                self.observed_available_inode_count >= self.required_inode_count,
                "insufficient backup inode capacity: {} available, {} required",
                self.observed_available_inode_count,
                self.required_inode_count
            );
        }
        Ok(())
    }
}

pub fn parse_backup_id(value: &str) -> Option<u64> {
    let body = value.strip_prefix("backup-v4-")?;
    let (timestamp, identifier) = body.split_once('-')?;
    let parsed = timestamp.parse::<u64>().ok()?;
    (parsed > 0
        && timestamp == parsed.to_string()
        && identifier.len() == 32
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(parsed)
}

/// Exact immutable release which must be installed before restoring the
/// mutable state in a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupReleaseIdentityV2 {
    pub source_commit: String,
    pub database_schema_version: i64,
    pub vps_release_manifest_sha256: String,
    pub publication_lock_sha256: String,
    pub installed_user_units: Vec<BackupReleaseUnitV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupReleaseUnitV2 {
    pub release_relative_path: String,
    pub artifact: ArtifactRefV1,
    pub unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupRestoreSourceV4 {
    pub original_absolute_path: String,
    pub archive_relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupFileV4 {
    pub relative_path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupDirectoryV4 {
    pub relative_path: String,
    pub unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifestProjectionV4 {
    pub schema_version: u32,
    pub created_at_unix_ms: u64,
    pub database_schema_version: i64,
    pub release_identity: BackupReleaseIdentityV2,
    pub root_unix_mode: u32,
    pub restore_sources: Vec<BackupRestoreSourceV4>,
    pub directories: Vec<BackupDirectoryV4>,
    pub files: Vec<BackupFileV4>,
}

pub fn canonical_backup_directories_v4(
    files: &[BackupFileV4],
) -> anyhow::Result<Vec<BackupDirectoryV4>> {
    let mut paths = BTreeSet::from([
        "campaigns".to_owned(),
        "replays".to_owned(),
        "restore".to_owned(),
        "restore/state".to_owned(),
        "restore/systemd".to_owned(),
        "restore/systemd/user".to_owned(),
    ]);
    for file in files {
        validate_safe_relative_path(&file.relative_path, "backup file path")?;
        let mut parent = Path::new(&file.relative_path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            paths.insert(
                directory
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("backup directory path is not UTF-8"))?
                    .replace('\\', "/"),
            );
            parent = directory.parent();
        }
    }
    anyhow::ensure!(
        paths
            .len()
            .checked_add(files.len())
            .is_some_and(|total| total <= MAX_BACKUP_MANIFEST_FILES),
        "backup directory inventory exceeds its bound"
    );
    Ok(paths
        .into_iter()
        .map(|relative_path| BackupDirectoryV4 {
            relative_path,
            unix_mode: 0o700,
        })
        .collect())
}

/// Full canonical payload manifest. The projection is byte-for-byte the same
/// typed document when loaded from the protected backup directory.
pub type BackupManifestV4 = BackupManifestProjectionV4;

impl BackupManifestProjectionV4 {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 4,
            "unsupported backup manifest schema"
        );
        anyhow::ensure!(
            self.created_at_unix_ms > 0,
            "backup manifest timestamp is empty"
        );
        anyhow::ensure!(
            self.database_schema_version > 0,
            "backup database schema version is invalid"
        );
        self.release_identity.validate()?;
        anyhow::ensure!(self.root_unix_mode == 0o700, "backup root mode is invalid");
        anyhow::ensure!(
            self.database_schema_version == self.release_identity.database_schema_version,
            "backup database schema differs from its authenticated release identity"
        );
        anyhow::ensure!(
            !self.restore_sources.is_empty()
                && self
                    .restore_sources
                    .windows(2)
                    .all(|pair| { pair[0].archive_relative_path < pair[1].archive_relative_path }),
            "backup manifest restore sources are empty, duplicate, or unsorted"
        );
        let mut originals = BTreeSet::new();
        let mut archives = BTreeSet::new();
        for source in &self.restore_sources {
            anyhow::ensure!(
                Path::new(&source.original_absolute_path).is_absolute(),
                "backup restore source is not absolute"
            );
            validate_safe_relative_path(
                &source.archive_relative_path,
                "backup restore archive path",
            )?;
            anyhow::ensure!(
                originals.insert(source.original_absolute_path.as_str()),
                "backup manifest repeats an original restore path"
            );
            anyhow::ensure!(
                archives.insert(source.archive_relative_path.as_str()),
                "backup manifest repeats an archive restore path"
            );
        }
        let required_archives = BTreeSet::from([
            "highscores.sqlite3",
            "replays",
            "campaigns",
            "restore/state/cursor-hmac.key",
            "restore/state/competition-run-grant.key",
            "restore/state/run-preflight-grant.key",
            "restore/state/moderation-bearer.token",
            "restore/systemd/user/robin-highscores.target",
            "restore/systemd/user/robin-highscores-api.service",
            "restore/systemd/user/robin-highscores-worker.service",
            "restore/systemd/user/robin-highscores-backup.service",
            "restore/systemd/user/robin-highscores-backup.timer",
        ]);
        anyhow::ensure!(
            archives == required_archives,
            "backup manifest restore sources differ from the exact mutable allowlist"
        );
        anyhow::ensure!(
            !self.files.is_empty()
                && self.files.len() <= MAX_BACKUP_MANIFEST_FILES
                && self
                    .files
                    .windows(2)
                    .all(|pair| pair[0].relative_path < pair[1].relative_path),
            "backup manifest files are empty, duplicate, or unsorted"
        );
        anyhow::ensure!(
            self.directories == canonical_backup_directories_v4(&self.files)?,
            "backup manifest directory inventory is not the exact canonical closure"
        );
        for file in &self.files {
            validate_safe_relative_path(&file.relative_path, "backup file path")?;
            anyhow::ensure!(
                file.byte_length > 0,
                "backup manifest contains an empty file"
            );
            validate_digest(&file.sha256, "backup file digest")?;
            anyhow::ensure!(
                file.relative_path == "highscores.sqlite3"
                    || file.relative_path.starts_with("replays/")
                    || file.relative_path.starts_with("campaigns/")
                    || matches!(
                        file.relative_path.as_str(),
                        "restore/state/cursor-hmac.key"
                            | "restore/state/competition-run-grant.key"
                            | "restore/state/run-preflight-grant.key"
                            | "restore/state/moderation-bearer.token"
                            | "restore/systemd/user/robin-highscores.target"
                            | "restore/systemd/user/robin-highscores-api.service"
                            | "restore/systemd/user/robin-highscores-worker.service"
                            | "restore/systemd/user/robin-highscores-backup.service"
                            | "restore/systemd/user/robin-highscores-backup.timer"
                    ),
                "backup manifest contains a non-mutable payload path"
            );
        }
        let file_paths = self
            .files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<BTreeSet<_>>();
        for required_file in [
            "highscores.sqlite3",
            "restore/state/cursor-hmac.key",
            "restore/state/competition-run-grant.key",
            "restore/state/run-preflight-grant.key",
            "restore/state/moderation-bearer.token",
            "restore/systemd/user/robin-highscores.target",
            "restore/systemd/user/robin-highscores-api.service",
            "restore/systemd/user/robin-highscores-worker.service",
            "restore/systemd/user/robin-highscores-backup.service",
            "restore/systemd/user/robin-highscores-backup.timer",
        ] {
            anyhow::ensure!(
                file_paths.contains(required_file),
                "backup manifest omits required mutable file {required_file}"
            );
        }
        for key_file in [
            "restore/state/cursor-hmac.key",
            "restore/state/competition-run-grant.key",
            "restore/state/run-preflight-grant.key",
        ] {
            let file = self
                .files
                .iter()
                .find(|file| file.relative_path == key_file)
                .expect("required key file presence was checked above");
            anyhow::ensure!(
                file.byte_length == 32,
                "backup manifest key file {key_file} has the wrong length"
            );
        }
        let moderation = self
            .files
            .iter()
            .find(|file| file.relative_path == "restore/state/moderation-bearer.token")
            .expect("required moderation token presence was checked above");
        anyhow::ensure!(
            (1..=64 * 1024).contains(&moderation.byte_length),
            "backup manifest moderation token is empty or oversized"
        );
        for unit in &self.release_identity.installed_user_units {
            let name = unit
                .release_relative_path
                .strip_prefix("systemd/user/")
                .expect("release identity user-unit paths were validated above");
            let archive = format!("restore/systemd/user/{name}");
            let file = self
                .files
                .iter()
                .find(|file| file.relative_path == archive)
                .expect("required archived user-unit presence was checked above");
            anyhow::ensure!(
                file.byte_length == unit.artifact.byte_length
                    && file.sha256 == unit.artifact.sha256.to_string(),
                "archived user unit differs from the exact active release artifact"
            );
        }
        for source in &self.restore_sources {
            if matches!(
                source.archive_relative_path.as_str(),
                "replays" | "campaigns"
            ) {
                continue;
            }
            let prefix = format!("{}/", source.archive_relative_path);
            anyhow::ensure!(
                file_paths.contains(source.archive_relative_path.as_str())
                    || file_paths.iter().any(|path| path.starts_with(&prefix)),
                "backup manifest restore mapping has no archived bytes"
            );
        }
        self.total_bytes()?;
        Ok(())
    }

    pub fn sha256(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(hex::encode(Sha256::digest(canonical_json_bytes(self)?)))
    }

    pub fn total_bytes(&self) -> anyhow::Result<u64> {
        self.files.iter().try_fold(0_u64, |total, file| {
            total
                .checked_add(file.byte_length)
                .ok_or_else(|| anyhow::anyhow!("backup manifest byte total overflows"))
        })
    }

    /// Exact directory inventory count, including the backup root whose mode
    /// is authenticated separately from descendant directory entries.
    pub fn directory_count(&self) -> anyhow::Result<u64> {
        u64::try_from(self.directories.len())?
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("backup directory count overflows"))
    }

    pub fn validate_production_restore_paths(&self) -> anyhow::Result<()> {
        let state = "/home/robinhood/.local/share/robin-highscores";
        let units = "/home/robinhood/.config/systemd/user";
        let expected = BTreeSet::from([
            (
                format!("{state}/database/highscores.sqlite3"),
                "highscores.sqlite3".to_owned(),
            ),
            (format!("{state}/replays"), "replays".to_owned()),
            (format!("{state}/campaign-states"), "campaigns".to_owned()),
            (
                format!("{state}/api-secrets/cursor-hmac.key"),
                "restore/state/cursor-hmac.key".to_owned(),
            ),
            (
                format!("{state}/api-secrets/competition-run-grant.key"),
                "restore/state/competition-run-grant.key".to_owned(),
            ),
            (
                format!("{state}/api-secrets/run-preflight-grant.key"),
                "restore/state/run-preflight-grant.key".to_owned(),
            ),
            (
                format!("{state}/api-secrets/moderation-bearer.token"),
                "restore/state/moderation-bearer.token".to_owned(),
            ),
            (
                format!("{units}/robin-highscores.target"),
                "restore/systemd/user/robin-highscores.target".to_owned(),
            ),
            (
                format!("{units}/robin-highscores-api.service"),
                "restore/systemd/user/robin-highscores-api.service".to_owned(),
            ),
            (
                format!("{units}/robin-highscores-worker.service"),
                "restore/systemd/user/robin-highscores-worker.service".to_owned(),
            ),
            (
                format!("{units}/robin-highscores-backup.service"),
                "restore/systemd/user/robin-highscores-backup.service".to_owned(),
            ),
            (
                format!("{units}/robin-highscores-backup.timer"),
                "restore/systemd/user/robin-highscores-backup.timer".to_owned(),
            ),
        ]);
        let actual = self
            .restore_sources
            .iter()
            .map(|source| {
                (
                    source.original_absolute_path.clone(),
                    source.archive_relative_path.clone(),
                )
            })
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            actual == expected,
            "backup manifest original-to-archive mappings differ from the exact production mutable layout"
        );
        Ok(())
    }
}

impl BackupReleaseIdentityV2 {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.source_commit.len() == 40
                && self
                    .source_commit
                    .bytes()
                    .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) }),
            "backup release source commit is not canonical lowercase Git SHA-1"
        );
        anyhow::ensure!(
            self.database_schema_version > 0,
            "backup release database schema version is invalid"
        );
        validate_digest(
            &self.vps_release_manifest_sha256,
            "VPS release manifest digest",
        )?;
        validate_digest(&self.publication_lock_sha256, "publication lock digest")?;
        let expected_units = [
            "systemd/user/robin-highscores-api.service",
            "systemd/user/robin-highscores-backup.service",
            "systemd/user/robin-highscores-backup.timer",
            "systemd/user/robin-highscores-worker.service",
            "systemd/user/robin-highscores.target",
        ];
        anyhow::ensure!(
            self.installed_user_units.len() == expected_units.len()
                && self
                    .installed_user_units
                    .iter()
                    .zip(expected_units)
                    .all(|(unit, expected)| unit.release_relative_path == expected),
            "backup release identity does not bind the exact five user units"
        );
        for unit in &self.installed_user_units {
            unit.artifact
                .validate()
                .map_err(|error| anyhow::anyhow!(error))?;
            anyhow::ensure!(
                unit.unix_mode == 0o440,
                "backup release user unit has a noncanonical mode"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledReleaseManifestIdentity {
    schema_version: u32,
    source_commit: String,
    database_schema_version: i64,
    deployment: InstalledReleaseDeployment,
    publication_lock_sha256: Digest32,
    publication_manifest_sha256: Digest32,
    verifier_sha256: Digest32,
    files: Vec<InstalledReleaseFile>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledReleaseDeployment {
    user: String,
    home: PathBuf,
    install_root: PathBuf,
    persistent_state_root: PathBuf,
    current_link: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledReleaseFile {
    path: String,
    artifact: ArtifactRefV1,
    unix_mode: u32,
}

/// Load the identity from the exact installed canonical VPS release manifest.
pub async fn load_backup_release_identity(path: &Path) -> anyhow::Result<BackupReleaseIdentityV2> {
    load_backup_release_identity_inner(path, true, true).await
}

/// Load an exact installed historical release for retention. V2 format,
/// canonical path, private metadata, source basename and artifact authority
/// remain mandatory; only the positive database schema may precede this
/// binary's compiled schema.
pub async fn load_backup_release_identity_historical(
    path: &Path,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    load_backup_release_identity_inner(path, true, false).await
}

/// Load an out-of-band release manifest used by the config-free offline
/// verifier. It validates the complete canonical authority but does not
/// require the file itself to be installed below the production release root.
pub async fn load_backup_release_identity_oob(
    path: &Path,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    load_backup_release_identity_inner(path, false, true).await
}

/// Load an out-of-band release authority from an already pinned inherited
/// regular-file descriptor. The caller retains path-admission authority; this
/// function validates exact private metadata and canonical bytes without
/// reopening a pathname.
pub async fn load_backup_release_identity_oob_file(
    file: std::fs::File,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    load_backup_release_identity_oob_file_with_policy(file, true, 0o440).await
}

/// Historical offline restore authority. Schema 1 predates the canonical V2
/// release/backup trust chain and future schemas are never interpreted by an
/// older verifier.
pub async fn load_backup_release_identity_oob_file_historical(
    file: std::fs::File,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    load_backup_release_identity_oob_file_with_policy(file, false, 0o440).await
}

/// Load an append-only owner-only copy retained beside backup generations.
pub async fn load_backup_release_identity_preserved_file(
    file: std::fs::File,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    load_backup_release_identity_oob_file_with_policy(file, false, 0o400).await
}

async fn load_backup_release_identity_oob_file_with_policy(
    file: std::fs::File,
    require_current_schema: bool,
    expected_mode: u32,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= MAX_RELEASE_MANIFEST_BYTES,
        "pinned release manifest is not a bounded regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.nlink() == 1
                && metadata.permissions().mode() & 0o777 == expected_mode
                && metadata.uid() == rustix::process::geteuid().as_raw(),
            "pinned release manifest has the wrong owner, mode, or link count"
        );
    }
    let file = tokio::fs::File::from_std(file);
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(MAX_RELEASE_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    anyhow::ensure!(
        bytes.len() <= usize::try_from(MAX_RELEASE_MANIFEST_BYTES)?,
        "pinned release manifest exceeds its byte limit"
    );
    release_identity_from_bytes(bytes, None, require_current_schema)
}

async fn load_backup_release_identity_inner(
    path: &Path,
    require_installed_path: bool,
    require_current_schema: bool,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    anyhow::ensure!(path.is_absolute(), "release manifest path must be absolute");
    if require_installed_path {
        installed_release_commit(path)?;
    }
    #[cfg(target_os = "linux")]
    let file = {
        let descriptor = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )?;
        tokio::fs::File::from_std(std::fs::File::from(descriptor))
    };
    #[cfg(not(target_os = "linux"))]
    let file = {
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        options.open(path).await?
    };
    let metadata = file.metadata().await?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= MAX_RELEASE_MANIFEST_BYTES,
        "release manifest is not a bounded regular file"
    );
    #[cfg(unix)]
    if require_installed_path {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.nlink() == 1
                && metadata.permissions().mode() & 0o777 == 0o440
                && metadata.uid() == rustix::process::geteuid().as_raw(),
            "installed VPS release manifest has the wrong owner, mode, or link count"
        );
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(MAX_RELEASE_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    anyhow::ensure!(
        bytes.len() <= usize::try_from(MAX_RELEASE_MANIFEST_BYTES)?,
        "release manifest exceeds its byte limit"
    );
    release_identity_from_bytes(
        bytes,
        require_installed_path.then_some(path),
        require_current_schema,
    )
}

fn release_identity_from_bytes(
    bytes: Vec<u8>,
    installed_path: Option<&Path>,
    require_current_schema: bool,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    release_identity_from_bytes_with_compiled_schema(
        bytes,
        installed_path,
        require_current_schema,
        robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
    )
}

fn release_identity_from_bytes_with_compiled_schema(
    bytes: Vec<u8>,
    installed_path: Option<&Path>,
    require_current_schema: bool,
    compiled_schema: i64,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&value)? == bytes,
        "installed VPS release manifest is not canonical JSON"
    );
    let document: InstalledReleaseManifestIdentity = serde_json::from_value(value)?;
    if let Some(installed_path) = installed_path {
        validate_installed_release_manifest_source(installed_path, &document.source_commit)?;
    }
    anyhow::ensure!(
        document.schema_version == RELEASE_MANIFEST_SCHEMA_VERSION,
        "unsupported installed VPS release manifest schema"
    );
    validate_release_database_schema(
        document.database_schema_version,
        compiled_schema,
        require_current_schema,
    )?;
    anyhow::ensure!(
        document.deployment
            == InstalledReleaseDeployment {
                user: "robinhood".to_owned(),
                home: PathBuf::from("/home/robinhood"),
                install_root: PathBuf::from("/home/robinhood/.local/opt/robin-highscores",),
                persistent_state_root: PathBuf::from(
                    "/home/robinhood/.local/share/robin-highscores",
                ),
                current_link: PathBuf::from("/home/robinhood/.local/opt/robin-highscores/current",),
            },
        "installed VPS release manifest has the wrong deployment identity"
    );
    anyhow::ensure!(
        !document.publication_lock_sha256.is_zero()
            && !document.publication_manifest_sha256.is_zero()
            && !document.verifier_sha256.is_zero(),
        "installed VPS release manifest contains a zero identity"
    );
    anyhow::ensure!(
        !document.files.is_empty()
            && document
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path),
        "installed VPS release inventory is empty, duplicate, or unsorted: {:?}",
        document
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>()
    );
    for file in &document.files {
        let relative = Path::new(&file.path);
        anyhow::ensure!(
            !file.path.is_empty()
                && !relative.is_absolute()
                && relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
                && relative.to_str() == Some(file.path.as_str()),
            "installed VPS release inventory path is unsafe"
        );
        file.artifact
            .validate()
            .map_err(|error| anyhow::anyhow!(error))?;
        let executable = matches!(
            file.path.as_str(),
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
        );
        anyhow::ensure!(
            file.unix_mode == if executable { 0o550 } else { 0o440 },
            "installed VPS release inventory has a noncanonical mode"
        );
    }
    let installed_user_units = document
        .files
        .iter()
        .filter(|file| file.path.starts_with("systemd/user/"))
        .map(|file| BackupReleaseUnitV2 {
            release_relative_path: file.path.clone(),
            artifact: file.artifact.clone(),
            unix_mode: file.unix_mode,
        })
        .collect::<Vec<_>>();
    let identity = BackupReleaseIdentityV2 {
        source_commit: document.source_commit,
        database_schema_version: document.database_schema_version,
        vps_release_manifest_sha256: hex::encode(Sha256::digest(&bytes)),
        publication_lock_sha256: document.publication_lock_sha256.to_string(),
        installed_user_units,
    };
    identity.validate()?;
    Ok(identity)
}

fn validate_release_database_schema(
    schema: i64,
    compiled_schema: i64,
    require_current_schema: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        (VPS_RELEASE_V2_MIN_DATABASE_SCHEMA_VERSION..=compiled_schema).contains(&schema),
        "VPS V2 release database schema is unsupported by this verifier"
    );
    if require_current_schema {
        anyhow::ensure!(
            schema == compiled_schema,
            "installed VPS release manifest targets a different database schema"
        );
    }
    Ok(())
}

fn installed_release_commit(path: &Path) -> anyhow::Result<String> {
    let release_root = Path::new("/home/robinhood/.local/opt/robin-highscores/releases");
    anyhow::ensure!(
        path.file_name().and_then(|name| name.to_str()) == Some("vps-release-manifest-v2.json")
            && path.parent().and_then(Path::parent) == Some(release_root),
        "installed VPS release manifest path is not the exact commit-named layout"
    );
    let commit = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("installed release path has no commit basename"))?;
    anyhow::ensure!(
        commit.len() == 40
            && commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "installed release directory is not a canonical source commit"
    );
    Ok(commit.to_owned())
}

fn validate_installed_release_manifest_source(
    path: &Path,
    source_commit: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        installed_release_commit(path)? == source_commit,
        "installed VPS release manifest source commit differs from its release directory"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupStatusV4 {
    pub schema_version: u32,
    pub created_at_unix_ms: u64,
    pub backup_id: String,
    pub backup_directory: String,
    pub backup_manifest_sha256: String,
    pub release_identity: BackupReleaseIdentityV2,
    pub database_schema_version: i64,
    pub file_count: u64,
    pub directory_count: u64,
    pub total_bytes: u64,
    pub hmac_sha256: String,
}

/// Canonical stdout result of one config-free, inherited-FD verification.
/// Deployment transactions persist these exact bytes only after exit status 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupVerificationReceiptV2 {
    pub schema_version: u32,
    pub verification_envelope_sha256: String,
    pub verification_envelope_byte_length: u64,
    pub current_status: Option<BackupCurrentStatusEvidenceV2>,
    pub backup_id: String,
    pub backup_directory: String,
    pub backup_manifest_sha256: String,
    pub release_identity: BackupReleaseIdentityV2,
    pub database_schema_version: i64,
    pub file_count: u64,
    pub directory_count: u64,
    pub total_bytes: u64,
}

/// Additional proof present only when a transaction verifies the currently
/// published latest backup. Historical restore receipts deliberately omit it:
/// the per-backup verification envelope is their durable authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupCurrentStatusEvidenceV2 {
    pub sha256: String,
    pub byte_length: u64,
}

/// Durable per-generation proof that the backup authority completed full
/// verification before publication. Unlike compact latest-status this file
/// remains with its payload and permits future retention to authenticate a
/// same-release historical schema against preserved immutable V2 authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupVerificationEnvelopeV2 {
    pub schema_version: u32,
    pub created_at_unix_ms: u64,
    pub backup_id: String,
    pub backup_manifest_sha256: String,
    pub release_identity: BackupReleaseIdentityV2,
    pub database_schema_version: i64,
    pub file_count: u64,
    pub directory_count: u64,
    pub total_bytes: u64,
    pub result: BackupVerificationResultV2,
    pub hmac_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupVerificationResultV2 {
    Verified,
}

/// Compact crash-recovery authority for pruning one already verified backup.
/// The authenticated manifest and verification envelope remain inside the
/// quarantined directory until every payload leaf has been removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupCleanupJournalV1 {
    pub schema_version: u32,
    pub backup_id: String,
    pub cleanup_directory_name: String,
    pub terminal_cleanup_directory_name: String,
    pub backup_root_device_id: u64,
    pub backup_root_inode: u64,
    pub backup_root_owner: u32,
    pub cleanup_root_device_id: u64,
    pub cleanup_root_inode: u64,
    pub cleanup_root_owner: u32,
    pub backup_manifest_sha256: String,
    pub verification_envelope_sha256: String,
    pub hmac_sha256: String,
}

#[derive(Serialize)]
struct UnsignedBackupCleanupJournal<'a> {
    schema_version: u32,
    backup_id: &'a str,
    cleanup_directory_name: &'a str,
    terminal_cleanup_directory_name: &'a str,
    backup_root_device_id: u64,
    backup_root_inode: u64,
    backup_root_owner: u32,
    cleanup_root_device_id: u64,
    cleanup_root_inode: u64,
    cleanup_root_owner: u32,
    backup_manifest_sha256: &'a str,
    verification_envelope_sha256: &'a str,
}

impl BackupCleanupJournalV1 {
    pub fn new_authenticated(
        backup_id: String,
        backup_root_device_id: u64,
        backup_root_inode: u64,
        backup_root_owner: u32,
        cleanup_root_device_id: u64,
        cleanup_root_inode: u64,
        cleanup_root_owner: u32,
        backup_manifest_sha256: String,
        verification_envelope_sha256: String,
        key: &[u8; 32],
    ) -> anyhow::Result<Self> {
        let mut value = Self {
            schema_version: 1,
            cleanup_directory_name: format!(".cleanup-{backup_id}"),
            terminal_cleanup_directory_name: format!(".cleanup-terminal-{backup_id}"),
            backup_id,
            backup_root_device_id,
            backup_root_inode,
            backup_root_owner,
            cleanup_root_device_id,
            cleanup_root_inode,
            cleanup_root_owner,
            backup_manifest_sha256,
            verification_envelope_sha256,
            hmac_sha256: String::new(),
        };
        value.validate_fields()?;
        value.hmac_sha256 = hex::encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, key),
            &value.signing_bytes()?,
        ));
        Ok(value)
    }

    pub fn verify(&self, key: &[u8; 32]) -> anyhow::Result<()> {
        self.validate_fields()?;
        validate_digest(&self.hmac_sha256, "backup cleanup-journal HMAC")?;
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, key),
            &self.signing_bytes()?,
            &hex::decode(&self.hmac_sha256)?,
        )
        .map_err(|_| anyhow::anyhow!("backup cleanup-journal authentication failed"))
    }

    fn validate_fields(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 1,
            "unsupported backup cleanup journal"
        );
        anyhow::ensure!(
            parse_backup_id(&self.backup_id).is_some()
                && self.cleanup_directory_name == format!(".cleanup-{}", self.backup_id),
            "backup cleanup journal has a noncanonical backup name"
        );
        anyhow::ensure!(
            self.terminal_cleanup_directory_name == format!(".cleanup-terminal-{}", self.backup_id),
            "backup cleanup journal has a noncanonical terminal name"
        );
        anyhow::ensure!(
            self.backup_root_device_id > 0
                && self.backup_root_inode > 0
                && self.backup_root_owner == rustix::process::geteuid().as_raw()
                && self.cleanup_root_device_id > 0
                && self.cleanup_root_inode > 0
                && self.cleanup_root_owner == rustix::process::geteuid().as_raw(),
            "backup cleanup journal has an invalid root identity"
        );
        validate_digest(
            &self.backup_manifest_sha256,
            "cleanup backup-manifest digest",
        )?;
        validate_digest(
            &self.verification_envelope_sha256,
            "cleanup verification-envelope digest",
        )?;
        Ok(())
    }

    fn signing_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let mut bytes = BACKUP_CLEANUP_JOURNAL_DOMAIN.to_vec();
        bytes.extend(canonical_json_bytes(&UnsignedBackupCleanupJournal {
            schema_version: self.schema_version,
            backup_id: &self.backup_id,
            cleanup_directory_name: &self.cleanup_directory_name,
            terminal_cleanup_directory_name: &self.terminal_cleanup_directory_name,
            backup_root_device_id: self.backup_root_device_id,
            backup_root_inode: self.backup_root_inode,
            backup_root_owner: self.backup_root_owner,
            cleanup_root_device_id: self.cleanup_root_device_id,
            cleanup_root_inode: self.cleanup_root_inode,
            cleanup_root_owner: self.cleanup_root_owner,
            backup_manifest_sha256: &self.backup_manifest_sha256,
            verification_envelope_sha256: &self.verification_envelope_sha256,
        })?);
        Ok(bytes)
    }
}

#[derive(Serialize)]
struct UnsignedBackupVerificationEnvelope<'a> {
    schema_version: u32,
    created_at_unix_ms: u64,
    backup_id: &'a str,
    backup_manifest_sha256: &'a str,
    release_identity: &'a BackupReleaseIdentityV2,
    database_schema_version: i64,
    file_count: u64,
    directory_count: u64,
    total_bytes: u64,
    result: BackupVerificationResultV2,
}

impl BackupVerificationEnvelopeV2 {
    pub fn new_authenticated(
        backup_id: String,
        manifest: &BackupManifestV4,
        key: &[u8; 32],
    ) -> anyhow::Result<Self> {
        manifest.validate()?;
        let mut value = Self {
            schema_version: 2,
            created_at_unix_ms: manifest.created_at_unix_ms,
            backup_id,
            backup_manifest_sha256: manifest.sha256()?,
            release_identity: manifest.release_identity.clone(),
            database_schema_version: manifest.database_schema_version,
            file_count: u64::try_from(manifest.files.len())?,
            directory_count: manifest.directory_count()?,
            total_bytes: manifest.total_bytes()?,
            result: BackupVerificationResultV2::Verified,
            hmac_sha256: String::new(),
        };
        value.validate_fields()?;
        value.hmac_sha256 = hex::encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, key),
            &value.signing_bytes()?,
        ));
        Ok(value)
    }

    pub fn verify(&self, key: &[u8; 32]) -> anyhow::Result<()> {
        self.validate_fields()?;
        validate_digest(&self.hmac_sha256, "backup verification-envelope HMAC")?;
        let tag = hex::decode(&self.hmac_sha256)?;
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, key),
            &self.signing_bytes()?,
            &tag,
        )
        .map_err(|_| anyhow::anyhow!("backup verification-envelope authentication failed"))
    }

    pub fn verify_manifest(
        &self,
        key: &[u8; 32],
        backup_id: &str,
        manifest: &BackupManifestV4,
    ) -> anyhow::Result<()> {
        self.verify(key)?;
        manifest.validate()?;
        anyhow::ensure!(
            self.backup_id == backup_id
                && self.created_at_unix_ms == manifest.created_at_unix_ms
                && self.backup_manifest_sha256 == manifest.sha256()?
                && self.release_identity == manifest.release_identity
                && self.database_schema_version == manifest.database_schema_version
                && self.file_count == u64::try_from(manifest.files.len())?
                && self.directory_count == manifest.directory_count()?
                && self.total_bytes == manifest.total_bytes()?,
            "backup verification envelope differs from its protected manifest"
        );
        Ok(())
    }

    fn validate_fields(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 2,
            "unsupported backup verification envelope"
        );
        anyhow::ensure!(
            parse_backup_id(&self.backup_id) == Some(self.created_at_unix_ms),
            "backup verification envelope has a noncanonical ID"
        );
        validate_digest(&self.backup_manifest_sha256, "backup manifest digest")?;
        self.release_identity.validate()?;
        anyhow::ensure!(
            self.database_schema_version == self.release_identity.database_schema_version
                && self.file_count > 0
                && self.directory_count > 0
                && self.total_bytes > 0
                && self.result == BackupVerificationResultV2::Verified,
            "backup verification envelope has an invalid verified result"
        );
        Ok(())
    }

    fn signing_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let unsigned = UnsignedBackupVerificationEnvelope {
            schema_version: self.schema_version,
            created_at_unix_ms: self.created_at_unix_ms,
            backup_id: &self.backup_id,
            backup_manifest_sha256: &self.backup_manifest_sha256,
            release_identity: &self.release_identity,
            database_schema_version: self.database_schema_version,
            file_count: self.file_count,
            directory_count: self.directory_count,
            total_bytes: self.total_bytes,
            result: self.result,
        };
        let encoded = canonical_json_bytes(&unsigned)?;
        let mut bytes =
            Vec::with_capacity(BACKUP_VERIFICATION_ENVELOPE_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(BACKUP_VERIFICATION_ENVELOPE_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}

impl BackupVerificationReceiptV2 {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 2,
            "unsupported backup verification receipt"
        );
        validate_digest(
            &self.verification_envelope_sha256,
            "backup verification envelope digest",
        )?;
        anyhow::ensure!(
            self.verification_envelope_byte_length > 0
                && self.verification_envelope_byte_length
                    <= u64::try_from(MAX_BACKUP_STATUS_BYTES)?,
            "backup verification envelope length is invalid"
        );
        if let Some(status) = &self.current_status {
            validate_digest(&status.sha256, "current backup status digest")?;
            anyhow::ensure!(
                status.byte_length > 0
                    && status.byte_length <= u64::try_from(MAX_BACKUP_STATUS_BYTES)?,
                "current backup status length is invalid"
            );
        }
        anyhow::ensure!(
            parse_backup_id(&self.backup_id).is_some(),
            "backup verification ID is invalid"
        );
        anyhow::ensure!(
            Path::new(&self.backup_directory).is_absolute()
                && Path::new(&self.backup_directory)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(self.backup_id.as_str()),
            "backup verification directory is not bound to its ID"
        );
        validate_digest(
            &self.backup_manifest_sha256,
            "backup verification manifest digest",
        )?;
        self.release_identity.validate()?;
        anyhow::ensure!(
            self.database_schema_version == self.release_identity.database_schema_version
                && self.file_count > 0
                && self.directory_count > 0
                && self.total_bytes > 0,
            "backup verification receipt has an empty database, file, or byte identity"
        );
        Ok(())
    }
}

#[derive(Serialize)]
struct UnsignedBackupStatus<'a> {
    schema_version: u32,
    created_at_unix_ms: u64,
    backup_id: &'a str,
    backup_directory: &'a str,
    backup_manifest_sha256: &'a str,
    release_identity: &'a BackupReleaseIdentityV2,
    database_schema_version: i64,
    file_count: u64,
    directory_count: u64,
    total_bytes: u64,
}

impl BackupStatusV4 {
    pub fn new_authenticated(
        backup_id: String,
        backup_directory: String,
        backup_manifest: BackupManifestProjectionV4,
        backup_authority_hmac_key: &[u8; 32],
    ) -> anyhow::Result<Self> {
        backup_manifest.validate()?;
        let created_at_unix_ms = backup_manifest.created_at_unix_ms;
        let backup_manifest_sha256 = backup_manifest.sha256()?;
        let release_identity = backup_manifest.release_identity.clone();
        let database_schema_version = backup_manifest.database_schema_version;
        let file_count = u64::try_from(backup_manifest.files.len())?;
        let directory_count = backup_manifest.directory_count()?;
        let total_bytes = backup_manifest.total_bytes()?;
        let mut status = Self {
            schema_version: BACKUP_STATUS_SCHEMA_VERSION,
            created_at_unix_ms,
            backup_id,
            backup_directory,
            backup_manifest_sha256,
            release_identity,
            database_schema_version,
            file_count,
            directory_count,
            total_bytes,
            hmac_sha256: String::new(),
        };
        status.validate_fields()?;
        status.hmac_sha256 = hex::encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, backup_authority_hmac_key),
            &status.signing_bytes()?,
        ));
        Ok(status)
    }

    pub fn verify(&self, backup_authority_hmac_key: &[u8; 32]) -> anyhow::Result<()> {
        self.validate_fields()?;
        validate_digest(&self.hmac_sha256, "backup status HMAC")?;
        let tag = hex::decode(&self.hmac_sha256)?;
        anyhow::ensure!(tag.len() == 32, "backup status HMAC has the wrong length");
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, backup_authority_hmac_key),
            &self.signing_bytes()?,
            &tag,
        )
        .map_err(|_| anyhow::anyhow!("backup status authentication failed"))?;
        Ok(())
    }

    fn validate_fields(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == BACKUP_STATUS_SCHEMA_VERSION,
            "unsupported backup status schema"
        );
        anyhow::ensure!(self.created_at_unix_ms > 0, "backup timestamp is empty");
        anyhow::ensure!(
            parse_backup_id(&self.backup_id) == Some(self.created_at_unix_ms),
            "backup id is not the exact canonical timestamped identifier"
        );
        anyhow::ensure!(
            std::path::Path::new(&self.backup_directory).is_absolute(),
            "backup directory is not absolute"
        );
        anyhow::ensure!(
            self.backup_directory.len() <= 4_096,
            "backup directory is too long"
        );
        validate_digest(&self.backup_manifest_sha256, "backup manifest digest")?;
        self.release_identity.validate()?;
        anyhow::ensure!(
            self.database_schema_version == self.release_identity.database_schema_version,
            "backup status database schema differs from its authenticated release identity"
        );
        anyhow::ensure!(self.file_count > 0, "backup status has no files");
        anyhow::ensure!(self.directory_count > 0, "backup status has no directories");
        anyhow::ensure!(self.total_bytes > 0, "backup status has no bytes");
        Ok(())
    }

    fn signing_bytes(&self) -> anyhow::Result<Vec<u8>> {
        let unsigned = UnsignedBackupStatus {
            schema_version: self.schema_version,
            created_at_unix_ms: self.created_at_unix_ms,
            backup_id: &self.backup_id,
            backup_directory: &self.backup_directory,
            backup_manifest_sha256: &self.backup_manifest_sha256,
            release_identity: &self.release_identity,
            database_schema_version: self.database_schema_version,
            file_count: self.file_count,
            directory_count: self.directory_count,
            total_bytes: self.total_bytes,
        };
        let encoded = canonical_json_bytes(&unsigned)?;
        let mut bytes = Vec::with_capacity(BACKUP_STATUS_DOMAIN.len() + encoded.len());
        bytes.extend_from_slice(BACKUP_STATUS_DOMAIN);
        bytes.extend_from_slice(&encoded);
        Ok(bytes)
    }
}

fn validate_safe_relative_path(value: &str, label: &str) -> anyhow::Result<()> {
    let relative = Path::new(value);
    let components = relative.components().collect::<Vec<_>>();
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= MAX_BACKUP_RELATIVE_PATH_BYTES
            && !relative.is_absolute()
            && components.len() <= MAX_BACKUP_TREE_DEPTH
            && components.iter().all(|component| matches!(component, Component::Normal(value) if value.as_encoded_bytes().len() <= MAX_BACKUP_COMPONENT_BYTES))
            && relative.to_str() == Some(value),
        "{label} is unsafe"
    );
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} is not canonical lowercase SHA-256"
    );
    anyhow::ensure!(value.bytes().any(|byte| byte != b'0'), "{label} is zero");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> BackupReleaseIdentityV2 {
        BackupReleaseIdentityV2 {
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            database_schema_version: robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
            vps_release_manifest_sha256: "12".repeat(32),
            publication_lock_sha256: "34".repeat(32),
            installed_user_units: [
                "robin-highscores-api.service",
                "robin-highscores-backup.service",
                "robin-highscores-backup.timer",
                "robin-highscores-worker.service",
                "robin-highscores.target",
            ]
            .into_iter()
            .map(|name| {
                let bytes = format!("fixture {name}\n");
                BackupReleaseUnitV2 {
                    release_relative_path: format!("systemd/user/{name}"),
                    artifact: ArtifactRefV1 {
                        sha256: Digest32::digest_bytes(bytes.as_bytes()),
                        byte_length: u64::try_from(bytes.len()).unwrap(),
                        media_type: "text/plain".to_owned(),
                    },
                    unix_mode: 0o440,
                }
            })
            .collect(),
        }
    }

    fn manifest(created_at_unix_ms: u64) -> BackupManifestProjectionV4 {
        let mut restore_sources = [
            "campaigns",
            "highscores.sqlite3",
            "replays",
            "restore/state/competition-run-grant.key",
            "restore/state/cursor-hmac.key",
            "restore/state/moderation-bearer.token",
            "restore/state/run-preflight-grant.key",
            "restore/systemd/user/robin-highscores-api.service",
            "restore/systemd/user/robin-highscores-backup.service",
            "restore/systemd/user/robin-highscores-backup.timer",
            "restore/systemd/user/robin-highscores-worker.service",
            "restore/systemd/user/robin-highscores.target",
        ]
        .into_iter()
        .map(|archive| BackupRestoreSourceV4 {
            original_absolute_path: format!("/var/lib/robin-highscores/{archive}"),
            archive_relative_path: archive.to_owned(),
        })
        .collect::<Vec<_>>();
        restore_sources
            .sort_by(|left, right| left.archive_relative_path.cmp(&right.archive_relative_path));
        let mut files = [
            "highscores.sqlite3",
            "restore/state/competition-run-grant.key",
            "restore/state/cursor-hmac.key",
            "restore/state/moderation-bearer.token",
            "restore/state/run-preflight-grant.key",
            "restore/systemd/user/robin-highscores-api.service",
            "restore/systemd/user/robin-highscores-backup.service",
            "restore/systemd/user/robin-highscores-backup.timer",
            "restore/systemd/user/robin-highscores-worker.service",
            "restore/systemd/user/robin-highscores.target",
        ]
        .into_iter()
        .map(|relative_path| BackupFileV4 {
            relative_path: relative_path.to_owned(),
            byte_length: if relative_path.starts_with("restore/systemd/user/") {
                let bytes = format!(
                    "fixture {}\n",
                    relative_path.trim_start_matches("restore/systemd/user/")
                );
                u64::try_from(bytes.len()).unwrap()
            } else if relative_path.ends_with(".key") {
                32
            } else {
                1
            },
            sha256: if relative_path.starts_with("restore/systemd/user/") {
                Digest32::digest_bytes(
                    format!(
                        "fixture {}\n",
                        relative_path.trim_start_matches("restore/systemd/user/")
                    )
                    .as_bytes(),
                )
                .to_string()
            } else {
                "ab".repeat(32)
            },
        })
        .collect::<Vec<_>>();
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        BackupManifestProjectionV4 {
            schema_version: 4,
            created_at_unix_ms,
            database_schema_version: crate::db::CURRENT_SCHEMA_VERSION,
            release_identity: identity(),
            root_unix_mode: 0o700,
            restore_sources,
            directories: canonical_backup_directories_v4(&files).unwrap(),
            files,
        }
    }

    #[test]
    fn status_authentication_covers_every_readiness_field() {
        let key = [0x42; 32];
        let backup_id = format!("backup-v4-123-{}", "a".repeat(32));
        let status = BackupStatusV4::new_authenticated(
            backup_id.clone(),
            format!("/var/lib/robin-highscores/backups/{backup_id}"),
            manifest(123),
            &key,
        )
        .unwrap();
        status.verify(&key).unwrap();

        let mut uppercase_hmac = status.clone();
        uppercase_hmac.hmac_sha256.make_ascii_uppercase();
        assert!(uppercase_hmac.verify(&key).is_err());
        for invalid in [
            format!("backup-v4-0123-{}", "a".repeat(32)),
            format!("backup-v4-0-{}", "a".repeat(32)),
            format!("backup-v4-123_{}", "a".repeat(32)),
            format!("backup-v4-18446744073709551616-{}", "a".repeat(32)),
        ] {
            assert!(parse_backup_id(&invalid).is_none(), "accepted {invalid}");
        }

        for tampered in [
            {
                let mut value = status.clone();
                value.created_at_unix_ms += 1;
                value
            },
            {
                let mut value = status.clone();
                value.backup_id.push('x');
                value
            },
            {
                let mut value = status.clone();
                value.backup_directory.push('x');
                value
            },
            {
                let mut value = status.clone();
                value.backup_manifest_sha256.replace_range(0..2, "cd");
                value
            },
            {
                let mut value = status.clone();
                value
                    .release_identity
                    .source_commit
                    .replace_range(0..1, "f");
                value
            },
            {
                let mut value = status.clone();
                value
                    .release_identity
                    .vps_release_manifest_sha256
                    .replace_range(0..2, "56");
                value
            },
            {
                let mut value = status.clone();
                value
                    .release_identity
                    .publication_lock_sha256
                    .replace_range(0..2, "78");
                value
            },
            {
                let mut value = status.clone();
                value.file_count += 1;
                value
            },
            {
                let mut value = status.clone();
                value.total_bytes += 1;
                value
            },
            {
                let mut value = status.clone();
                value.database_schema_version += 1;
                value
            },
            {
                let mut value = status.clone();
                value.created_at_unix_ms += 1;
                value
            },
        ] {
            assert!(tampered.verify(&key).is_err());
        }

        let mut mismatched_schema = status.clone();
        mismatched_schema.database_schema_version += 1;
        mismatched_schema.hmac_sha256 = hex::encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, &key),
            &mismatched_schema.signing_bytes().unwrap(),
        ));
        assert!(
            mismatched_schema.verify(&key).is_err(),
            "a valid HMAC must not authorize a database/release schema mismatch"
        );

        let mut old_domain_bytes = b"robinhood/highscores/backup-status/3\0".to_vec();
        old_domain_bytes
            .extend_from_slice(&status.signing_bytes().unwrap()[BACKUP_STATUS_DOMAIN.len()..]);
        let mut old_domain = status.clone();
        old_domain.hmac_sha256 = hex::encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, &key),
            &old_domain_bytes,
        ));
        assert!(
            old_domain.verify(&key).is_err(),
            "the obsolete V3 HMAC domain must not authenticate V4 status"
        );

        let mut manifest_schema_mismatch = manifest(123);
        manifest_schema_mismatch.database_schema_version += 1;
        assert!(manifest_schema_mismatch.validate().is_err());

        let mut receipt = BackupVerificationReceiptV2 {
            schema_version: 2,
            verification_envelope_sha256: "56".repeat(32),
            verification_envelope_byte_length: 1,
            current_status: Some(BackupCurrentStatusEvidenceV2 {
                sha256: "78".repeat(32),
                byte_length: 1,
            }),
            backup_id,
            backup_directory: status.backup_directory,
            backup_manifest_sha256: status.backup_manifest_sha256,
            release_identity: status.release_identity,
            database_schema_version: status.database_schema_version,
            file_count: status.file_count,
            directory_count: status.directory_count,
            total_bytes: status.total_bytes,
        };
        receipt.validate().unwrap();
        receipt.database_schema_version += 1;
        assert!(receipt.validate().is_err());
    }

    #[test]
    fn verification_envelope_authenticates_exact_manifest_and_topology() {
        let key = [0x42; 32];
        let manifest = manifest(123);
        assert_eq!(
            manifest.directory_count().unwrap(),
            u64::try_from(manifest.directories.len()).unwrap() + 1,
            "the authenticated directory count must include the backup root"
        );
        let backup_id = format!("backup-v4-123-{}", "a".repeat(32));
        let envelope =
            BackupVerificationEnvelopeV2::new_authenticated(backup_id.clone(), &manifest, &key)
                .unwrap();
        envelope
            .verify_manifest(&key, &backup_id, &manifest)
            .unwrap();
        assert!(envelope.verify(&[0x43; 32]).is_err());

        let canonical = canonical_json_bytes(&envelope).unwrap();
        assert_eq!(
            serde_json::from_slice::<BackupVerificationEnvelopeV2>(&canonical).unwrap(),
            envelope
        );
        let mut with_lf = canonical.clone();
        with_lf.push(b'\n');
        assert_ne!(
            canonical_json_bytes(
                &serde_json::from_slice::<BackupVerificationEnvelopeV2>(&with_lf).unwrap()
            )
            .unwrap(),
            with_lf
        );
        let mut unknown: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        unknown["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<BackupVerificationEnvelopeV2>(unknown).is_err());

        let tampered = [
            {
                let mut value = envelope.clone();
                value.created_at_unix_ms += 1;
                value
            },
            {
                let mut value = envelope.clone();
                value.backup_id.push('x');
                value
            },
            {
                let mut value = envelope.clone();
                value.backup_manifest_sha256.replace_range(0..2, "cd");
                value
            },
            {
                let mut value = envelope.clone();
                value
                    .release_identity
                    .source_commit
                    .replace_range(0..1, "f");
                value
            },
            {
                let mut value = envelope.clone();
                value.database_schema_version += 1;
                value
            },
            {
                let mut value = envelope.clone();
                value.file_count += 1;
                value
            },
            {
                let mut value = envelope.clone();
                value.directory_count += 1;
                value
            },
            {
                let mut value = envelope.clone();
                value.total_bytes += 1;
                value
            },
        ];
        for value in tampered {
            assert!(value.verify(&key).is_err());
        }

        let mut uppercase = envelope.clone();
        uppercase.hmac_sha256.make_ascii_uppercase();
        assert!(uppercase.verify(&key).is_err());
        let mut obsolete_domain = b"robinhood/highscores/backup-verification-envelope/1\0".to_vec();
        obsolete_domain.extend_from_slice(
            &envelope.signing_bytes().unwrap()[BACKUP_VERIFICATION_ENVELOPE_DOMAIN.len()..],
        );
        let mut old = envelope.clone();
        old.hmac_sha256 = hex::encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, &key),
            &obsolete_domain,
        ));
        assert!(old.verify(&key).is_err());

        let mut different_manifest = manifest.clone();
        different_manifest.files[0].sha256 = "cd".repeat(32);
        assert!(
            envelope
                .verify_manifest(&key, &backup_id, &different_manifest)
                .is_err()
        );
    }

    #[test]
    fn cleanup_journal_authenticates_root_and_backup_identity() {
        let key = [0x51; 32];
        let backup_id = format!("backup-v4-123-{}", "a".repeat(32));
        let owner = rustix::process::geteuid().as_raw();
        let journal = BackupCleanupJournalV1::new_authenticated(
            backup_id,
            11,
            12,
            owner,
            11,
            13,
            owner,
            "12".repeat(32),
            "34".repeat(32),
            &key,
        )
        .unwrap();
        journal.verify(&key).unwrap();
        assert!(journal.verify(&[0x52; 32]).is_err());
        let canonical = canonical_json_bytes(&journal).unwrap();
        assert_eq!(
            canonical_json_bytes(
                &serde_json::from_slice::<BackupCleanupJournalV1>(&canonical).unwrap()
            )
            .unwrap(),
            canonical
        );
        let mut unknown: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
        unknown["unknown"] = serde_json::json!(true);
        assert!(serde_json::from_value::<BackupCleanupJournalV1>(unknown).is_err());

        let tampered = [
            {
                let mut value = journal.clone();
                value.schema_version += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.backup_id.push('x');
                value
            },
            {
                let mut value = journal.clone();
                value.cleanup_directory_name.push('x');
                value
            },
            {
                let mut value = journal.clone();
                value.terminal_cleanup_directory_name.push('x');
                value
            },
            {
                let mut value = journal.clone();
                value.backup_root_device_id += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.backup_root_inode += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.backup_root_owner += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.cleanup_root_device_id += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.cleanup_root_inode += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.cleanup_root_owner += 1;
                value
            },
            {
                let mut value = journal.clone();
                value.backup_manifest_sha256.replace_range(0..2, "56");
                value
            },
            {
                let mut value = journal.clone();
                value.verification_envelope_sha256.replace_range(0..2, "78");
                value
            },
        ];
        for value in tampered {
            assert!(value.verify(&key).is_err());
        }
    }

    #[tokio::test]
    async fn installed_manifest_identity_requires_canonical_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("vps-release-manifest-v2.json");
        let mut release_files = vec![serde_json::json!({
            "artifact": {
                "byte_length": 1,
                "media_type": "application/octet-stream",
                "sha256": "9a".repeat(32),
            },
            "path": "README.md",
            "unix_mode": 0o440,
        })];
        release_files.extend(identity().installed_user_units.into_iter().map(|unit| {
            serde_json::json!({
                "artifact": unit.artifact,
                "path": unit.release_relative_path,
                "unix_mode": unit.unix_mode,
            })
        }));
        let document = serde_json::json!({
            "database_schema_version": robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
            "deployment": {
                "current_link": "/home/robinhood/.local/opt/robin-highscores/current",
                "home": "/home/robinhood",
                "install_root": "/home/robinhood/.local/opt/robin-highscores",
                "persistent_state_root": "/home/robinhood/.local/share/robin-highscores",
                "user": "robinhood",
            },
            "files": release_files,
            "publication_lock_sha256": "34".repeat(32),
            "publication_manifest_sha256": "56".repeat(32),
            "schema_version": 2,
            "source_commit": "0123456789abcdef0123456789abcdef01234567",
            "verifier_sha256": "78".repeat(32),
        });
        let bytes = canonical_json_bytes(&document).unwrap();
        assert_eq!(
            release_identity_from_bytes_with_compiled_schema(bytes.clone(), None, false, 3,)
                .unwrap()
                .database_schema_version,
            2,
            "a canonical schema-2 VpsManifestV2 remains historical authority under schema 3"
        );
        assert!(
            release_identity_from_bytes_with_compiled_schema(bytes.clone(), None, true, 3,)
                .is_err(),
            "current-release admission must reject a historical downgrade"
        );
        let mut pre_vps_v2_schema = document.clone();
        pre_vps_v2_schema["database_schema_version"] = serde_json::json!(1);
        assert!(
            release_identity_from_bytes_with_compiled_schema(
                canonical_json_bytes(&pre_vps_v2_schema).unwrap(),
                None,
                false,
                3,
            )
            .is_err(),
            "historical admission must reject schemas older than the VpsManifestV2 minimum"
        );
        let mut future_schema = document.clone();
        future_schema["database_schema_version"] = serde_json::json!(4);
        assert!(
            release_identity_from_bytes_with_compiled_schema(
                canonical_json_bytes(&future_schema).unwrap(),
                None,
                false,
                3,
            )
            .is_err(),
            "historical admission must reject schemas newer than the supplied compiled target"
        );
        let mut newer_than_compiled_schema = document.clone();
        newer_than_compiled_schema["database_schema_version"] = serde_json::json!(3);
        assert!(
            release_identity_from_bytes_with_compiled_schema(
                canonical_json_bytes(&newer_than_compiled_schema).unwrap(),
                None,
                false,
                2,
            )
            .is_err(),
            "a downlevel verifier must reject a release from a future schema"
        );
        tokio::fs::write(&path, &bytes).await.unwrap();
        let loaded = load_backup_release_identity_oob(&path).await.unwrap();
        assert_eq!(loaded.source_commit, identity().source_commit);
        assert_eq!(
            loaded.publication_lock_sha256,
            identity().publication_lock_sha256
        );
        assert_eq!(
            loaded.vps_release_manifest_sha256,
            hex::encode(Sha256::digest(&bytes))
        );

        let mut wrong_database_schema = document.clone();
        wrong_database_schema["database_schema_version"] =
            serde_json::json!(robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION + 1);
        tokio::fs::write(&path, canonical_json_bytes(&wrong_database_schema).unwrap())
            .await
            .unwrap();
        assert!(load_backup_release_identity_oob(&path).await.is_err());
        tokio::fs::write(&path, &bytes).await.unwrap();

        tokio::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap())
            .await
            .unwrap();
        assert!(load_backup_release_identity_oob(&path).await.is_err());

        let commit_a = "0123456789abcdef0123456789abcdef01234567";
        let commit_b = "f123456789abcdef0123456789abcdef01234567";
        let installed_a = PathBuf::from(format!(
            "/home/robinhood/.local/opt/robin-highscores/releases/{commit_a}/vps-release-manifest-v2.json"
        ));
        let installed_b = PathBuf::from(format!(
            "/home/robinhood/.local/opt/robin-highscores/releases/{commit_b}/vps-release-manifest-v2.json"
        ));
        assert_eq!(installed_release_commit(&installed_a).unwrap(), commit_a);
        assert_ne!(
            installed_release_commit(&installed_b).unwrap(),
            identity().source_commit,
            "a manifest for source A copied below release B must not inherit B's path identity"
        );
        assert!(
            validate_installed_release_manifest_source(&installed_b, commit_a).is_err(),
            "a manifest for source A copied below release B must be rejected"
        );
    }

    #[test]
    fn historical_v2_schema_policy_accepts_two_under_simulated_compiled_three() {
        let mut historical = manifest(123);
        historical.database_schema_version = 2;
        historical.release_identity.database_schema_version = 2;
        historical.validate().unwrap();
        validate_release_database_schema(historical.database_schema_version, 3, false).unwrap();
        assert!(validate_release_database_schema(2, 3, true).is_err());
        assert!(validate_release_database_schema(1, 3, false).is_err());
        assert!(validate_release_database_schema(4, 3, false).is_err());
        let mut mismatch = historical;
        mismatch.release_identity.database_schema_version = 3;
        assert!(mismatch.validate().is_err());
    }
}
