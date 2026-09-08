use anyhow::Context as _;
use clap::{Parser, Subcommand};
#[cfg(test)]
use robin_highscores::backup::load_backup_release_identity_oob;
use robin_highscores::backup::{
    BackupCleanupJournalV1, BackupCurrentStatusEvidenceV2, BackupFileV4 as BackupFile,
    BackupManifestV4 as BackupManifest, BackupReleaseIdentityV2,
    BackupRestoreSourceV4 as RestoreSource, BackupSpaceEstimateV1, BackupStatusV4,
    BackupVerificationEnvelopeV2, BackupVerificationReceiptV2, canonical_backup_directories_v4,
    load_backup_release_identity, load_backup_release_identity_oob_file,
    load_backup_release_identity_preserved_file, parse_backup_id,
};
use robin_highscores::db_fence::{
    ExclusiveAdmissionGuard, ExclusiveQuiescenceGuard, RuntimeDatabaseFence,
};
use robin_highscores::live_schema::{LiveDatabaseSchemaProbeV2, verify_live_database_schema_v2};
use robin_highscores::runtime_authority::{
    BackupAuthorityStateV2, CandidateSelfRoleV2, attest_candidate_release_root_v2,
    probe_runtime_authority_v2,
};
use robin_highscores::{CampaignStore, Database, ReplayStore, ServerConfig};
use robin_run_protocol::canonical_json_bytes;
#[cfg(test)]
use robin_run_protocol::{ArtifactRefV1, Digest32};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Connection as _, Row as _};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

const BACKUP_LOCK_TTL: Duration = Duration::from_secs(10 * 60);
const BACKUP_WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const BACKUP_MANIFEST_SCHEMA_VERSION: u32 = 4;
const DEFAULT_BACKUP_ROOT: &str = "/home/robinhood/.local/share/robin-highscores/backups";
const DEFAULT_BACKUP_STATUS: &str =
    "/home/robinhood/.local/share/robin-highscores/status/backup-status.json";
const DEFAULT_BACKUP_AUTHORITY_KEY: &str =
    "/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key";
const VPS_ACTIVATION_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores";
const INSTALLED_RELEASE_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores/releases";
const RELEASE_AUTHORITY_STORE: &str = ".release-authorities-v2";
const SYSTEMD_USER_ROOT: &str = "/home/robinhood/.config/systemd/user";
const SYSTEMD_UNIT_FILES: [&str; 5] = [
    "robin-highscores.target",
    "robin-highscores-api.service",
    "robin-highscores-worker.service",
    "robin-highscores-backup.service",
    "robin-highscores-backup.timer",
];
const FIXED_BACKUP_DIRECTORY_COUNT: u64 = 7;
const FIXED_BACKUP_FILE_COUNT: u64 = 3;

#[derive(Debug, Parser)]
#[command(about = "Explicit high-score database and moderation administration")]
struct Arguments {
    #[arg(
        long,
        env = "ROBIN_HIGHSCORES_CONFIG",
        default_value = "highscores-server.toml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply all reviewed SQL migrations. Serving processes never do this.
    Migrate,
    /// Create the durable cursor key without printing or otherwise exposing it.
    InitializeCursorKey,
    /// Create the durable Ed25519 seed used only for scheduled-run grants.
    InitializeCompetitionRunGrantKey,
    /// Create the durable Ed25519 seed used only for run-preflight grants.
    InitializeRunPreflightGrantKey,
    /// Prepare or resume the transaction-bound fifth backup HMAC authority.
    InitializeBackupAuthorityKeyV2 {
        /// Exact 40-character source commit owning this initialization.
        #[arg(long)]
        source_commit: String,
        /// Inherited canonical activation.lock descriptor holding exclusion.
        #[arg(long)]
        activation_lock_fd: u32,
    },
    /// Complete fifth-key initialization after the full runtime authority exists.
    CompleteBackupAuthorityKeyV2 {
        /// Exact 40-character source commit owning this initialization.
        #[arg(long)]
        source_commit: String,
        /// Inherited canonical activation.lock descriptor holding exclusion.
        #[arg(long)]
        activation_lock_fd: u32,
        /// Inherited descriptor for the immutable candidate release root.
        #[arg(long)]
        candidate_release_root_fd: u32,
        /// Out-of-band SHA-256 of canonical vps-release-manifest-v2.json.
        #[arg(long)]
        expected_vps_release_manifest_sha256: String,
    },
    /// Read-only, descriptor-pinned pre-activation runtime authority probe.
    ProbeRuntimeAuthorityV2 {
        /// Inherited descriptor for the exact immutable candidate root.
        #[arg(long)]
        candidate_release_root_fd: u32,
        /// Out-of-band SHA-256 of canonical vps-release-manifest-v2.json.
        #[arg(long)]
        expected_vps_release_manifest_sha256: String,
        /// Require the dedicated fifth secret to be absent or present.
        #[arg(long, value_enum)]
        backup_authority_state: BackupAuthorityStateV2,
    },
    /// Verify the live SQLite schema from a private copy under both kernel fences.
    VerifyLiveDatabaseSchemaV2 {
        /// Inherited descriptor for the exact immutable candidate root.
        #[arg(long)]
        candidate_release_root_fd: u32,
        /// Out-of-band SHA-256 of canonical vps-release-manifest-v2.json.
        #[arg(long)]
        expected_vps_release_manifest_sha256: String,
    },
    Reports {
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    Moderate {
        report_id: String,
        #[arg(long)]
        state: String,
        #[arg(long)]
        detail: String,
    },
    Audit {
        #[arg(long)]
        report_id: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// Verify every backup byte plus SQLite integrity and migration level.
    VerifyBackup {
        /// Inherited, already pinned canonical backup-root descriptor.
        #[arg(long)]
        backup_root_fd: u32,
        /// Inherited, already pinned owner-only backup-directory descriptor.
        #[arg(long)]
        backup_directory_fd: u32,
        /// Inherited, already pinned dedicated backup-authority key descriptor.
        #[arg(long)]
        backup_authority_key_fd: u32,
        #[arg(long)]
        expected_backup_manifest_sha256: String,
        #[arg(long)]
        expected_source_commit: String,
        #[arg(long)]
        expected_vps_release_manifest_sha256: String,
        #[arg(long)]
        expected_publication_lock_sha256: String,
    },
    /// Verify the freshly published transaction backup, deriving its manifest
    /// digest only from the pinned HMAC-authenticated status envelope.
    VerifyTransactionBackup {
        #[arg(long)]
        backup_root_fd: u32,
        #[arg(long)]
        status_envelope_fd: u32,
        #[arg(long)]
        backup_authority_key_fd: u32,
        #[arg(long)]
        expected_release_manifest_fd: u32,
        #[arg(long)]
        expected_source_commit: String,
        #[arg(long)]
        expected_vps_release_manifest_sha256: String,
        #[arg(long)]
        expected_publication_lock_sha256: String,
    },
    /// Emit a canonical typed upper bound for one additional backup
    /// generation, optionally failing when the live volume cannot satisfy it.
    EstimateBackupSpace {
        #[arg(long)]
        release_manifest_path: PathBuf,
        #[arg(long, default_value = DEFAULT_BACKUP_ROOT)]
        backup_root: PathBuf,
        #[arg(long, default_value = DEFAULT_BACKUP_STATUS)]
        status_path: PathBuf,
        #[arg(long)]
        require_available: bool,
        /// Restore path and readable source, as ORIGINAL_ABSOLUTE=SOURCE_ABSOLUTE.
        #[arg(long = "restore-source-map")]
        restore_source_maps: Vec<RestoreSourceMap>,
    },
    /// Create, verify, retain, and atomically publish a readiness backup.
    BackupAndPublishStatus {
        /// Exact installed immutable release manifest bound into the backup.
        #[arg(long)]
        release_manifest_path: PathBuf,
        #[arg(long, default_value = DEFAULT_BACKUP_ROOT)]
        backup_root: PathBuf,
        #[arg(long, default_value = DEFAULT_BACKUP_STATUS)]
        status_path: PathBuf,
        /// Number of complete local backups to retain. Must be at least one.
        #[arg(long, default_value_t = 2)]
        retain_complete: usize,
        /// Restore path and readable source, as ORIGINAL_ABSOLUTE=SOURCE_ABSOLUTE.
        /// Map mode-0400 API secrets to their exact operator-readable sources.
        #[arg(long = "restore-source-map")]
        restore_source_maps: Vec<RestoreSourceMap>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreSourceMap {
    original: PathBuf,
    readable_source: PathBuf,
}

impl FromStr for RestoreSourceMap {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (original, readable_source) = value
            .split_once('=')
            .ok_or_else(|| "restore source map must be ORIGINAL=SOURCE".to_owned())?;
        let original = PathBuf::from(original);
        let readable_source = PathBuf::from(readable_source);
        if !original.is_absolute() || !readable_source.is_absolute() {
            return Err("restore source map paths must be absolute".to_owned());
        }
        Ok(Self {
            original,
            readable_source,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedBackup {
    created_at_unix_ms: u64,
    database_schema_version: i64,
    manifest_sha256: String,
    release_identity: BackupReleaseIdentityV2,
    file_count: u64,
    directory_count: u64,
    total_bytes: u64,
    tree: BackupTreePaths,
}

#[derive(Debug)]
enum BackupInstallOutcome {
    Installed,
    InstalledButParentSyncFailed(anyhow::Error),
}

#[derive(Debug)]
enum StatusPublicationOutcome {
    Published,
    PublishedButParentSyncFailed(anyhow::Error),
    PublishedButIdentityUncertain(anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
#[error(
    "authenticated backup readiness envelope was installed at {path} with SHA-256 {envelope_sha256}, but status-parent durability sync failed: {source}"
)]
struct StatusPublicationDurabilityUncertain {
    path: PathBuf,
    envelope_sha256: String,
    #[source]
    source: anyhow::Error,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "authenticated backup readiness envelope may have been installed at {path} with SHA-256 {envelope_sha256}, but its post-publication path identity could not be proven: {source}"
)]
struct StatusPublicationIdentityUncertain {
    path: PathBuf,
    envelope_sha256: String,
    #[source]
    source: anyhow::Error,
}

struct GuardedBackupVerificationReceipt {
    receipt: BackupVerificationReceiptV2,
    operation_lock: std::fs::File,
    trusted_backup_root: PathBuf,
    _status_guard: Option<std::fs::File>,
    _backup_directory_guard: std::fs::File,
}

struct GuardedVerifierCommand {
    backup: GuardedBackupVerificationReceipt,
    release_guard: std::fs::File,
    release_store_guard: Option<std::fs::File>,
    release_target: PathBuf,
    release_expected_mode: u32,
    expected_release: BackupReleaseIdentityV2,
    historical_release_authority: bool,
    backup_authority_key_guard: std::fs::File,
    expected_backup_authority_key: [u8; 32],
    backup_root_guard: std::fs::File,
}

impl GuardedVerifierCommand {
    async fn canonical_receipt_bytes(&self) -> anyhow::Result<Vec<u8>> {
        revalidate_pinned_root_directory(
            &self.backup_root_guard,
            Path::new(DEFAULT_BACKUP_ROOT),
            "backup root",
        )?;
        revalidate_operation_lock_path(
            &self.backup.trusted_backup_root,
            &self.backup.operation_lock,
        )?;
        if let Some(store) = &self.release_store_guard {
            revalidate_pinned_directory_path(
                store,
                &Path::new(DEFAULT_BACKUP_ROOT).join(RELEASE_AUTHORITY_STORE),
                Path::new(DEFAULT_BACKUP_ROOT),
                "preserved release-authority store",
            )?;
        }
        revalidate_pinned_regular_path(
            &self.release_guard,
            &self.release_target,
            self.release_expected_mode,
            Some(if self.historical_release_authority {
                0o700
            } else {
                0o550
            }),
            "out-of-band release manifest",
        )?;
        revalidate_pinned_regular_path(
            &self.backup_authority_key_guard,
            Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
            0o400,
            Some(0o700),
            "backup authority HMAC key",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(&self.backup_authority_key_guard, false)?,
                32,
            )? == self.expected_backup_authority_key,
            "backup authority HMAC key changed during verification"
        );
        let final_release = if self.historical_release_authority {
            load_backup_release_identity_preserved_file(duplicate_pinned_file(
                &self.release_guard,
                false,
            )?)
            .await?
        } else {
            load_backup_release_identity_oob_file(duplicate_pinned_file(
                &self.release_guard,
                false,
            )?)
            .await?
        };
        anyhow::ensure!(
            final_release == self.expected_release,
            "out-of-band release authority changed during verification"
        );
        revalidate_pinned_regular_path(
            &self.release_guard,
            &self.release_target,
            self.release_expected_mode,
            Some(if self.historical_release_authority {
                0o700
            } else {
                0o550
            }),
            "out-of-band release manifest",
        )?;
        revalidate_pinned_regular_path(
            &self.backup_authority_key_guard,
            Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
            0o400,
            Some(0o700),
            "backup authority HMAC key",
        )?;
        revalidate_operation_lock_path(
            &self.backup.trusted_backup_root,
            &self.backup.operation_lock,
        )?;
        if let Some(store) = &self.release_store_guard {
            revalidate_pinned_directory_path(
                store,
                &Path::new(DEFAULT_BACKUP_ROOT).join(RELEASE_AUTHORITY_STORE),
                Path::new(DEFAULT_BACKUP_ROOT),
                "preserved release-authority store",
            )?;
        }
        revalidate_pinned_root_directory(
            &self.backup_root_guard,
            Path::new(DEFAULT_BACKUP_ROOT),
            "backup root",
        )?;
        Ok(canonical_json_bytes(&self.backup.receipt)?)
    }
}

#[derive(Clone, Copy)]
enum BackupManifestExpectation<'a> {
    Explicit(&'a str),
    AuthenticatedStatus,
}

enum BackupVerificationMode<'a> {
    Offline {
        backup_root_fd: u32,
        backup_directory_fd: u32,
        expected_manifest_sha256: &'a str,
    },
    Transaction {
        backup_root_fd: u32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    match &arguments.command {
        Command::InitializeBackupAuthorityKeyV2 {
            source_commit,
            activation_lock_fd,
        } => {
            initialize_backup_authority_key_v2(source_commit, *activation_lock_fd)?;
            println!("backup authority HMAC key transaction is prepared");
            return Ok(());
        }
        Command::CompleteBackupAuthorityKeyV2 {
            source_commit,
            activation_lock_fd,
            candidate_release_root_fd,
            expected_vps_release_manifest_sha256,
        } => {
            let root = duplicate_inherited_fd(*candidate_release_root_fd, true)?;
            complete_backup_authority_key_v2(source_commit, *activation_lock_fd, move || {
                let receipt = probe_runtime_authority_v2(
                    root,
                    expected_vps_release_manifest_sha256,
                    BackupAuthorityStateV2::Present,
                )?;
                anyhow::ensure!(
                    receipt.source_commit == source_commit.as_str(),
                    "runtime authority belongs to a different source commit"
                );
                Ok(())
            })?;
            println!("backup authority HMAC key transaction is complete");
            return Ok(());
        }
        Command::ProbeRuntimeAuthorityV2 {
            candidate_release_root_fd,
            expected_vps_release_manifest_sha256,
            backup_authority_state,
        } => {
            let root = duplicate_inherited_fd(*candidate_release_root_fd, true)?;
            let receipt = probe_runtime_authority_v2(
                root,
                expected_vps_release_manifest_sha256,
                *backup_authority_state,
            )?;
            std::io::stdout().write_all(&canonical_json_bytes(&receipt)?)?;
            return Ok(());
        }
        Command::VerifyLiveDatabaseSchemaV2 {
            candidate_release_root_fd,
            expected_vps_release_manifest_sha256,
        } => {
            let root = duplicate_inherited_fd(*candidate_release_root_fd, true)?;
            let (initial_attestation, retained_root) = attest_candidate_release_root_v2(
                root,
                expected_vps_release_manifest_sha256,
                CandidateSelfRoleV2::Admin,
            )?;
            let database_schema_version =
                verify_live_database_schema_v2(initial_attestation.database_schema_version).await?;
            let (final_attestation, _retained_root) = attest_candidate_release_root_v2(
                retained_root,
                expected_vps_release_manifest_sha256,
                CandidateSelfRoleV2::Admin,
            )?;
            anyhow::ensure!(
                final_attestation == initial_attestation,
                "candidate release authority changed during live-schema verification"
            );
            let receipt = LiveDatabaseSchemaProbeV2::from_attestation(
                &initial_attestation,
                database_schema_version,
            )?;
            std::io::stdout().write_all(&canonical_json_bytes(&receipt)?)?;
            return Ok(());
        }
        Command::VerifyBackup {
            backup_root_fd,
            backup_directory_fd,
            backup_authority_key_fd,
            expected_backup_manifest_sha256,
            expected_source_commit,
            expected_vps_release_manifest_sha256,
            expected_publication_lock_sha256,
        } => {
            let verification = run_pinned_verifier_command(
                BackupVerificationMode::Offline {
                    backup_root_fd: *backup_root_fd,
                    backup_directory_fd: *backup_directory_fd,
                    expected_manifest_sha256: expected_backup_manifest_sha256,
                },
                None,
                *backup_authority_key_fd,
                None,
                expected_source_commit,
                expected_vps_release_manifest_sha256,
                expected_publication_lock_sha256,
            )
            .await?;
            std::io::stdout().write_all(&verification.canonical_receipt_bytes().await?)?;
            return Ok(());
        }
        Command::VerifyTransactionBackup {
            backup_root_fd,
            status_envelope_fd,
            backup_authority_key_fd,
            expected_release_manifest_fd,
            expected_source_commit,
            expected_vps_release_manifest_sha256,
            expected_publication_lock_sha256,
        } => {
            let verification = run_pinned_verifier_command(
                BackupVerificationMode::Transaction {
                    backup_root_fd: *backup_root_fd,
                },
                Some(*status_envelope_fd),
                *backup_authority_key_fd,
                Some(*expected_release_manifest_fd),
                expected_source_commit,
                expected_vps_release_manifest_sha256,
                expected_publication_lock_sha256,
            )
            .await?;
            std::io::stdout().write_all(&verification.canonical_receipt_bytes().await?)?;
            return Ok(());
        }
        Command::InitializeCursorKey => {
            let config = load_secret_bootstrap_config(&arguments.config, "cursor_secret_path")?;
            config.load_or_create_cursor_key()?;
            println!("cursor key is initialized");
            return Ok(());
        }
        Command::InitializeCompetitionRunGrantKey => {
            let config = load_secret_bootstrap_config(
                &arguments.config,
                "competition_run_grant_secret_path",
            )?;
            let secret = config.load_or_create_competition_run_grant_key()?;
            let public = ed25519_dalek::SigningKey::from_bytes(&secret)
                .verifying_key()
                .to_bytes();
            println!(
                "competition run grant key is initialized; manifest public key: {}",
                hex::encode(public)
            );
            return Ok(());
        }
        Command::InitializeRunPreflightGrantKey => {
            let config =
                load_secret_bootstrap_config(&arguments.config, "run_preflight_grant_secret_path")?;
            let secret = config.load_or_create_run_preflight_grant_key()?;
            let public = ed25519_dalek::SigningKey::from_bytes(&secret)
                .verifying_key()
                .to_bytes();
            println!(
                "run preflight grant key is initialized; ruleset manifest public key: {}",
                hex::encode(public)
            );
            return Ok(());
        }
        _ => {}
    }
    let backup_sources = match &arguments.command {
        Command::BackupAndPublishStatus {
            restore_source_maps,
            ..
        }
        | Command::EstimateBackupSpace {
            restore_source_maps,
            ..
        } => restore_source_map(restore_source_maps)?,
        _ => BTreeMap::new(),
    };
    let config = if backup_sources.is_empty() {
        ServerConfig::load(&arguments.config)?
    } else {
        ServerConfig::load_with_backup_credentials(&arguments.config, &backup_sources)?
    };
    match arguments.command {
        Command::Migrate => {
            let database = Database::migrate(&config).await?;
            database.close_fenced().await?;
            println!("database migrations applied successfully");
        }
        Command::InitializeCursorKey
        | Command::InitializeCompetitionRunGrantKey
        | Command::InitializeRunPreflightGrantKey
        | Command::InitializeBackupAuthorityKeyV2 { .. }
        | Command::CompleteBackupAuthorityKeyV2 { .. }
        | Command::VerifyBackup { .. }
        | Command::VerifyTransactionBackup { .. } => {
            unreachable!("bootstrap commands returned above")
        }
        Command::ProbeRuntimeAuthorityV2 { .. } | Command::VerifyLiveDatabaseSchemaV2 { .. } => {
            unreachable!("config-free runtime probe returned above")
        }
        Command::Reports { state, limit } => {
            let db = Database::connect(&config).await?;
            let reports = db
                .run_fenced_operation(async {
                    Ok(db.moderation_reports(state.as_deref(), limit).await?)
                })
                .await;
            db.close_fenced().await?;
            println!("{}", serde_json::to_string_pretty(&reports?)?);
        }
        Command::Moderate {
            report_id,
            state,
            detail,
        } => {
            let db = Database::connect(&config).await?;
            let result: anyhow::Result<()> = db
                .run_fenced_operation(async {
                    let lease = db
                        .acquire_maintenance_write_lease(
                            robin_highscores::db::MaintenanceWriteClass::Admin,
                            "robin-highscores-admin-moderation",
                            Duration::from_secs(5 * 60),
                        )
                        .await?;
                    let mutation = db
                        .moderate_report(
                            &report_id,
                            &state,
                            &detail,
                            &config.moderation_operator_id,
                        )
                        .await;
                    let release = db.release_maintenance_write_lease(&lease).await;
                    match (mutation, release) {
                        (Ok(()), Ok(true)) => {}
                        (Ok(()), Ok(false)) => {
                            anyhow::bail!("moderation write lease disappeared before release")
                        }
                        (Ok(()), Err(error)) => return Err(error.into()),
                        (Err(operation), Ok(true)) => return Err(operation.into()),
                        (Err(operation), Ok(false)) => {
                            return Err(anyhow::Error::from(operation)
                                .context("moderation failed and its write lease disappeared"));
                        }
                        (Err(operation), Err(release)) => {
                            return Err(anyhow::Error::from(operation).context(format!(
                        "moderation failed and releasing its write lease also failed: {release}"
                    )));
                        }
                    }
                    Ok(())
                })
                .await;
            db.close_fenced().await?;
            result?;
            println!("moderation action recorded");
        }
        Command::Audit { report_id, limit } => {
            let db = Database::connect(&config).await?;
            let audit = db
                .run_fenced_operation(async {
                    Ok(db.moderation_audit(report_id.as_deref(), limit).await?)
                })
                .await;
            db.close_fenced().await?;
            println!("{}", serde_json::to_string_pretty(&audit?)?);
        }
        Command::EstimateBackupSpace {
            release_manifest_path,
            backup_root,
            status_path,
            require_available,
            restore_source_maps: _,
        } => {
            let release_identity = load_backup_release_identity(&release_manifest_path).await?;
            validate_backup_restore_source_contract(&config, &backup_sources)?;
            let estimate = estimate_backup_space(
                &config,
                &release_identity,
                &backup_root,
                &status_path,
                &backup_sources,
            )
            .await?;
            if require_available {
                estimate.ensure_available()?;
            }
            std::io::stdout().write_all(&canonical_json_bytes(&estimate)?)?;
        }
        Command::BackupAndPublishStatus {
            release_manifest_path,
            backup_root,
            status_path,
            retain_complete,
            restore_source_maps: _,
        } => {
            let release_identity = load_backup_release_identity(&release_manifest_path).await?;
            let directory = backup_and_publish_status(
                &config,
                &release_manifest_path,
                &release_identity,
                &backup_root,
                &status_path,
                retain_complete,
                &backup_sources,
            )
            .await?;
            println!("verified backup published from {}", directory.display());
        }
    }
    Ok(())
}

fn load_secret_bootstrap_config(path: &Path, field: &str) -> anyhow::Result<ServerConfig> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= 4 * 1024 * 1024,
        "bootstrap config must be a bounded regular file"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "bootstrap config is too large"
    );
    let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    let secret_path = PathBuf::from(
        value
            .get(field)
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("bootstrap config omits {field}"))?,
    );
    anyhow::ensure!(
        secret_path.is_absolute(),
        "bootstrap secret path is not absolute"
    );
    let mut config = ServerConfig::default();
    match field {
        "cursor_secret_path" => config.cursor_secret_path = secret_path,
        "competition_run_grant_secret_path" => {
            config.competition_run_grant_secret_path = secret_path;
        }
        "run_preflight_grant_secret_path" => {
            config.run_preflight_grant_secret_path = secret_path;
        }
        _ => anyhow::bail!("unsupported bootstrap secret field"),
    }
    Ok(config)
}

#[cfg(test)]
async fn verify_backup_with_expected(
    directory: &Path,
    expected_manifest_sha256: &str,
    expected_release: &BackupReleaseIdentityV2,
) -> anyhow::Result<VerifiedBackup> {
    expected_release.validate()?;
    anyhow::ensure!(
        expected_manifest_sha256.len() == 64
            && expected_manifest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && expected_manifest_sha256.bytes().any(|byte| byte != b'0'),
        "expected backup manifest digest is not canonical nonzero SHA-256"
    );
    let verified = verify_backup(directory).await?;
    let manifest_bytes = read_bounded_regular_nofollow(
        &directory.join("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )
    .await?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.validate_production_restore_paths()?;
    anyhow::ensure!(
        verified.manifest_sha256 == expected_manifest_sha256,
        "backup manifest differs from the out-of-band expected digest"
    );
    anyhow::ensure!(
        &verified.release_identity == expected_release,
        "backup release identity differs from the out-of-band expected identity"
    );
    Ok(verified)
}

const BACKUP_AUTHORITY_INTENT_SCHEMA_VERSION: u32 = 1;
const BACKUP_AUTHORITY_INTENT_NAME: &str = ".backup-authority-hmac-key.intent-v1.json";
const BACKUP_AUTHORITY_INTENT_TEMP_NAME: &str = ".backup-authority-hmac-key.intent-v1.json.new";
const MAX_BACKUP_AUTHORITY_INTENT_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupAuthorityKeyIntentV1 {
    schema_version: u32,
    source_commit: String,
    activation_lock_device: u64,
    activation_lock_inode: u64,
    secret_parent_device: u64,
    secret_parent_inode: u64,
    key_device: u64,
    key_inode: u64,
    key_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupAuthorityKeyBoundary {
    AnonymousKeySynced,
    IntentTemporaryCreated,
    IntentTemporaryWritten,
    IntentTemporarySynced,
    IntentPublished,
    KeyLinked,
    KeyDirectorySynced,
    KeyValidated,
    RecoveryTemporaryRemoved,
    RecoveryIntentRemoved,
    OuterAuthorityValidated,
    CompletionIntentRemoved,
    CompletionDirectorySynced,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct OwnedRawFd(i32);

#[cfg(target_os = "linux")]
impl Drop for OwnedRawFd {
    fn drop(&mut self) {
        let _ = nix_legacy::unistd::close(self.0);
    }
}

#[cfg(target_os = "linux")]
struct PinnedActivationLock {
    inherited: OwnedRawFd,
    opt_root: PathBuf,
    opt_device: u64,
    opt_inode: u64,
    lock_device: u64,
    lock_inode: u64,
}

#[cfg(target_os = "linux")]
impl PinnedActivationLock {
    fn ensure_canonical(&self) -> anyhow::Result<()> {
        use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
        use rustix::io::Errno;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let root_metadata = std::fs::symlink_metadata(&self.opt_root)?;
        anyhow::ensure!(
            root_metadata.is_dir()
                && !root_metadata.file_type().is_symlink()
                && std::fs::canonicalize(&self.opt_root)? == self.opt_root
                && root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.permissions().mode() & 0o777 == 0o750
                && root_metadata.dev() == self.opt_device
                && root_metadata.ino() == self.opt_inode,
            "activation opt root changed while the fifth-key transaction was active"
        );
        let root = openat2(
            rustix::fs::CWD,
            &self.opt_root,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let canonical = openat2(
            &root,
            "activation.lock",
            OFlags::RDWR | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let canonical_metadata = rustix::fs::fstat(&canonical)?;
        let inherited_metadata = nix_legacy::sys::stat::fstat(self.inherited.0)?;
        anyhow::ensure!(
            rustix::fs::FileType::from_raw_mode(canonical_metadata.st_mode).is_file()
                && canonical_metadata.st_uid == rustix::process::geteuid().as_raw()
                && canonical_metadata.st_nlink == 1
                && canonical_metadata.st_mode & 0o777 == 0o600
                && canonical_metadata.st_size == 0
                && canonical_metadata.st_dev == self.opt_device
                && canonical_metadata.st_dev == self.lock_device
                && canonical_metadata.st_ino == self.lock_inode
                && inherited_metadata.st_dev == self.lock_device
                && inherited_metadata.st_ino == self.lock_inode
                && inherited_metadata.st_uid == canonical_metadata.st_uid
                && inherited_metadata.st_nlink == 1
                && inherited_metadata.st_mode & 0o777 == 0o600
                && inherited_metadata.st_size == 0,
            "inherited activation lock is no longer the canonical owner-only lock inode"
        );
        match flock(&canonical, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {
                flock(&canonical, FlockOperation::Unlock)?;
                anyhow::bail!("inherited activation lock no longer holds exclusion");
            }
            Err(Errno::WOULDBLOCK) => {}
            Err(error) => return Err(error.into()),
        }
        #[allow(deprecated)]
        nix_legacy::fcntl::flock(
            self.inherited.0,
            nix_legacy::fcntl::FlockArg::LockExclusiveNonblock,
        )
        .context("inherited activation lock is not the open file description holding exclusion")?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn pin_activation_lock_at(
    opt_root: &Path,
    activation_lock_fd: u32,
) -> anyhow::Result<PinnedActivationLock> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        activation_lock_fd >= 3,
        "inherited activation lock descriptor must be at least 3"
    );
    let raw = i32::try_from(activation_lock_fd)?;
    let inherited = OwnedRawFd(nix_legacy::unistd::dup(raw)?);
    nix_legacy::fcntl::fcntl(
        inherited.0,
        nix_legacy::fcntl::FcntlArg::F_SETFD(nix_legacy::fcntl::FdFlag::FD_CLOEXEC),
    )?;
    let root_metadata = std::fs::symlink_metadata(opt_root)?;
    anyhow::ensure!(
        root_metadata.is_dir()
            && !root_metadata.file_type().is_symlink()
            && std::fs::canonicalize(opt_root)? == opt_root
            && root_metadata.uid() == rustix::process::geteuid().as_raw()
            && root_metadata.permissions().mode() & 0o777 == 0o750,
        "activation opt root must be canonical, EUID-owned, and mode 0750"
    );
    let root = openat2(
        rustix::fs::CWD,
        opt_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_root = rustix::fs::fstat(&root)?;
    anyhow::ensure!(
        pinned_root.st_dev == root_metadata.dev() && pinned_root.st_ino == root_metadata.ino(),
        "activation opt root changed while it was pinned"
    );
    let canonical = openat2(
        &root,
        "activation.lock",
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let canonical_metadata = rustix::fs::fstat(&canonical)?;
    let inherited_metadata = nix_legacy::sys::stat::fstat(inherited.0)?;
    anyhow::ensure!(
        rustix::fs::FileType::from_raw_mode(inherited_metadata.st_mode).is_file()
            && inherited_metadata.st_uid == rustix::process::geteuid().as_raw()
            && inherited_metadata.st_nlink == 1
            && inherited_metadata.st_mode & 0o777 == 0o600
            && inherited_metadata.st_size == 0
            && inherited_metadata.st_dev == pinned_root.st_dev
            && inherited_metadata.st_dev == canonical_metadata.st_dev
            && inherited_metadata.st_ino == canonical_metadata.st_ino,
        "inherited activation lock is not the canonical owner-only lock inode"
    );
    let pinned = PinnedActivationLock {
        inherited,
        opt_root: opt_root.to_path_buf(),
        opt_device: pinned_root.st_dev,
        opt_inode: pinned_root.st_ino,
        lock_device: inherited_metadata.st_dev,
        lock_inode: inherited_metadata.st_ino,
    };
    pinned.ensure_canonical()?;
    Ok(pinned)
}

#[cfg(target_os = "linux")]
struct PinnedSecretParent {
    fd: std::os::fd::OwnedFd,
    path: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
}

#[cfg(target_os = "linux")]
impl PinnedSecretParent {
    fn ensure_canonical(&self) -> anyhow::Result<()> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let metadata = std::fs::symlink_metadata(&self.path)?;
        anyhow::ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && std::fs::canonicalize(&self.path)? == self.path
                && metadata.uid() == self.uid
                && metadata.permissions().mode() & 0o777 == 0o700
                && metadata.dev() == self.device
                && metadata.ino() == self.inode,
            "backup-authority secret parent changed during initialization"
        );
        let reopened = openat2(
            rustix::fs::CWD,
            &self.path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let reopened = rustix::fs::fstat(&reopened)?;
        anyhow::ensure!(
            reopened.st_dev == self.device && reopened.st_ino == self.inode,
            "backup-authority secret parent path was replaced"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn pin_secret_parent(key_path: &Path, expected_uid: u32) -> anyhow::Result<PinnedSecretParent> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        key_path.is_absolute(),
        "backup-authority key path is not absolute"
    );
    let parent_path = key_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup-authority key has no parent"))?;
    anyhow::ensure!(
        key_path.file_name().and_then(|name| name.to_str()) == Some("backup-authority-hmac.key"),
        "backup-authority key has a non-canonical filename"
    );
    let metadata = std::fs::symlink_metadata(parent_path)?;
    anyhow::ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && std::fs::canonicalize(parent_path)? == parent_path
            && metadata.uid() == expected_uid
            && metadata.permissions().mode() & 0o777 == 0o700,
        "backup-authority secret parent must be canonical, expected-user-owned, and mode 0700"
    );
    let fd = openat2(
        rustix::fs::CWD,
        parent_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned = rustix::fs::fstat(&fd)?;
    anyhow::ensure!(
        pinned.st_dev == metadata.dev() && pinned.st_ino == metadata.ino(),
        "backup-authority secret parent changed while it was pinned"
    );
    Ok(PinnedSecretParent {
        fd,
        path: parent_path.to_path_buf(),
        device: pinned.st_dev,
        inode: pinned.st_ino,
        uid: expected_uid,
    })
}

#[cfg(target_os = "linux")]
fn named_entry_exists(parent: &PinnedSecretParent, name: &str) -> anyhow::Result<bool> {
    use rustix::fs::{AtFlags, statat};
    match statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PinnedKeyIdentity {
    device: u64,
    inode: u64,
    sha256: String,
}

#[cfg(target_os = "linux")]
fn load_published_backup_authority_key(
    parent: &PinnedSecretParent,
) -> anyhow::Result<PinnedKeyIdentity> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::os::fd::AsFd as _;

    let name = "backup-authority-hmac.key";
    let named = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        FileType::from_raw_mode(named.st_mode).is_file()
            && named.st_uid == parent.uid
            && named.st_dev == parent.device
            && named.st_nlink == 1
            && named.st_size == 32
            && named.st_mode & 0o777 == 0o400,
        "backup-authority key has unsafe type, owner, device, links, size, or mode"
    );
    let fd = openat2(
        parent.fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let mut file = std::fs::File::from(fd);
    let opened = file.metadata()?;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    anyhow::ensure!(
        opened.is_file()
            && opened.dev() == named.st_dev
            && opened.ino() == named.st_ino
            && opened.uid() == parent.uid
            && opened.nlink() == 1
            && opened.len() == 32
            && opened.permissions().mode() & 0o777 == 0o400,
        "backup-authority key changed while it was pinned"
    );
    let mut key = [0_u8; 32];
    file.read_exact(&mut key)?;
    let mut trailing = [0_u8; 1];
    anyhow::ensure!(
        file.read(&mut trailing)? == 0 && key != [0; 32],
        "backup-authority key is not an exact nonzero 32-byte key"
    );
    let observed = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        observed.st_dev == named.st_dev
            && observed.st_ino == named.st_ino
            && observed.st_uid == named.st_uid
            && observed.st_nlink == 1
            && observed.st_size == 32
            && observed.st_mode & 0o777 == 0o400,
        "backup-authority key path changed while it was read"
    );
    Ok(PinnedKeyIdentity {
        device: named.st_dev,
        inode: named.st_ino,
        sha256: hex::encode(Sha256::digest(key)),
    })
}

#[cfg(target_os = "linux")]
struct PinnedIntent {
    document: BackupAuthorityKeyIntentV1,
    bytes: Vec<u8>,
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
fn load_backup_authority_intent(
    parent: &PinnedSecretParent,
    name: &str,
) -> anyhow::Result<PinnedIntent> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let named = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        FileType::from_raw_mode(named.st_mode).is_file()
            && named.st_uid == parent.uid
            && named.st_dev == parent.device
            && named.st_nlink == 1
            && (1..=MAX_BACKUP_AUTHORITY_INTENT_BYTES as i64).contains(&named.st_size)
            && named.st_mode & 0o777 == 0o400,
        "backup-authority intent has unsafe type, owner, device, links, size, or mode"
    );
    let fd = openat2(
        parent.fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let file = std::fs::File::from(fd);
    let opened = file.metadata()?;
    anyhow::ensure!(
        opened.dev() == named.st_dev
            && opened.ino() == named.st_ino
            && opened.uid() == parent.uid
            && opened.nlink() == 1
            && opened.permissions().mode() & 0o777 == 0o400
            && opened.len() == u64::try_from(named.st_size)?,
        "backup-authority intent changed while it was pinned"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(opened.len())?);
    file.take(MAX_BACKUP_AUTHORITY_INTENT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? == opened.len(),
        "backup-authority intent changed length while it was read"
    );
    let document: BackupAuthorityKeyIntentV1 = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&document)? == bytes,
        "backup-authority intent is not exact canonical JSON"
    );
    let observed = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        observed.st_dev == named.st_dev
            && observed.st_ino == named.st_ino
            && observed.st_uid == named.st_uid
            && observed.st_nlink == 1
            && observed.st_size == named.st_size
            && observed.st_mode & 0o777 == 0o400,
        "backup-authority intent path changed while it was read"
    );
    Ok(PinnedIntent {
        document,
        bytes,
        device: named.st_dev,
        inode: named.st_ino,
    })
}

#[cfg(target_os = "linux")]
fn validate_backup_authority_intent(
    intent: &BackupAuthorityKeyIntentV1,
    source_commit: &str,
    parent: &PinnedSecretParent,
    activation_lock: &PinnedActivationLock,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        intent.schema_version == BACKUP_AUTHORITY_INTENT_SCHEMA_VERSION
            && intent.source_commit == source_commit
            && intent.activation_lock_device == activation_lock.lock_device
            && intent.activation_lock_inode == activation_lock.lock_inode
            && intent.secret_parent_device == parent.device
            && intent.secret_parent_inode == parent.inode
            && intent.key_device == parent.device
            && intent.key_inode != 0
            && intent.key_sha256.len() == 64
            && intent
                .key_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && intent.key_sha256.bytes().any(|byte| byte != b'0'),
        "backup-authority intent is stale or does not bind this exact transaction"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_key_against_intent(
    key: &PinnedKeyIdentity,
    intent: &BackupAuthorityKeyIntentV1,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        key.device == intent.key_device
            && key.inode == intent.key_inode
            && key.sha256 == intent.key_sha256,
        "published backup-authority key differs from its durable intent"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_exact_named_entry(
    parent: &PinnedSecretParent,
    name: &str,
    expected_device: u64,
    expected_inode: u64,
) -> anyhow::Result<()> {
    use rustix::fs::{AtFlags, Mode, OFlags, ResolveFlags, openat2, statat, unlinkat};
    use std::os::fd::AsFd as _;

    let pinned = openat2(
        parent.fd.as_fd(),
        name,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let pinned_metadata = rustix::fs::fstat(&pinned)?;
    let named = statat(parent.fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    anyhow::ensure!(
        pinned_metadata.st_dev == expected_device
            && pinned_metadata.st_ino == expected_inode
            && named.st_dev == expected_device
            && named.st_ino == expected_inode,
        "backup-authority recovery entry changed before removal"
    );
    unlinkat(parent.fd.as_fd(), name, AtFlags::empty())?;
    anyhow::ensure!(
        rustix::fs::fstat(&pinned)?.st_nlink == 0,
        "backup-authority recovery inode remained linked after removal"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn reconcile_intent_temporary<H>(
    parent: &PinnedSecretParent,
    intent_exists: bool,
    key_exists: bool,
    hook: &mut H,
) -> anyhow::Result<()>
where
    H: FnMut(BackupAuthorityKeyBoundary) -> anyhow::Result<()>,
{
    use rustix::fs::{AtFlags, FileType, statat};
    use std::os::fd::AsFd as _;

    if !named_entry_exists(parent, BACKUP_AUTHORITY_INTENT_TEMP_NAME)? {
        return Ok(());
    }
    anyhow::ensure!(
        !intent_exists && !key_exists,
        "backup-authority intent temporary coexists with published authority evidence"
    );
    let temporary = statat(
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        AtFlags::SYMLINK_NOFOLLOW,
    )?;
    anyhow::ensure!(
        FileType::from_raw_mode(temporary.st_mode).is_file()
            && temporary.st_uid == parent.uid
            && temporary.st_dev == parent.device
            && temporary.st_nlink == 1
            && temporary.st_size >= 0
            && temporary.st_size <= MAX_BACKUP_AUTHORITY_INTENT_BYTES as i64
            && matches!(temporary.st_mode & 0o777, 0o000 | 0o200 | 0o400 | 0o600),
        "backup-authority intent temporary has unsafe type, owner, device, links, size, or mode"
    );
    remove_exact_named_entry(
        parent,
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        temporary.st_dev,
        temporary.st_ino,
    )?;
    rustix::fs::fsync(&parent.fd)?;
    hook(BackupAuthorityKeyBoundary::RecoveryTemporaryRemoved)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn valid_backup_authority_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(target_os = "linux")]
fn publish_anonymous_backup_authority_key_with<F>(
    anonymous: &std::fs::File,
    parent: &PinnedSecretParent,
    target: &str,
    proc_fd_root: &Path,
    direct_link: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> rustix::io::Result<()>,
{
    use rustix::fs::{AtFlags, Mode, OFlags, ResolveFlags, linkat, openat2};
    use std::os::fd::{AsFd as _, AsRawFd as _};

    let retained = rustix::fs::fstat(anonymous)?;
    match direct_link() {
        Ok(()) => {}
        Err(error) if error == rustix::io::Errno::NOENT => {
            let proc_path = proc_fd_root.join(anonymous.as_raw_fd().to_string());
            linkat(
                rustix::fs::CWD,
                &proc_path,
                parent.fd.as_fd(),
                target,
                AtFlags::SYMLINK_FOLLOW,
            )
            .with_context(|| {
                format!(
                    "publish anonymous backup-authority key through {}",
                    proc_path.display()
                )
            })?;
        }
        Err(error) => return Err(error).context("publish anonymous backup-authority key"),
    }

    let published = openat2(
        parent.fd.as_fd(),
        target,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .context("pin published backup-authority key")?;
    let observed = rustix::fs::fstat(&published)?;
    if observed.st_dev != retained.st_dev
        || observed.st_ino != retained.st_ino
        || observed.st_uid != retained.st_uid
        || observed.st_nlink != 1
        || observed.st_mode & 0o777 != retained.st_mode & 0o777
        || observed.st_size != retained.st_size
    {
        drop(published);
        remove_exact_named_entry(parent, target, observed.st_dev, observed.st_ino)?;
        rustix::fs::fsync(&parent.fd)?;
        anyhow::bail!("published backup-authority key is not the retained anonymous inode");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn initialize_backup_authority_key_v2_at<H>(
    opt_root: &Path,
    key_path: &Path,
    expected_uid: u32,
    source_commit: &str,
    activation_lock_fd: u32,
    mut hook: H,
) -> anyhow::Result<()>
where
    H: FnMut(BackupAuthorityKeyBoundary) -> anyhow::Result<()>,
{
    use rustix::fs::{
        AtFlags, Mode, OFlags, RenameFlags, ResolveFlags, fchmod, linkat, openat, openat2,
        renameat_with,
    };
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    anyhow::ensure!(
        valid_backup_authority_source_commit(source_commit),
        "backup-authority source commit is not exact lowercase 40-character hexadecimal"
    );
    let activation_lock = pin_activation_lock_at(opt_root, activation_lock_fd)
        .context("pin activation lock for backup-authority initialization")?;
    let parent =
        pin_secret_parent(key_path, expected_uid).context("pin backup-authority secret parent")?;
    activation_lock
        .ensure_canonical()
        .context("revalidate activation lock before backup-authority initialization")?;
    parent
        .ensure_canonical()
        .context("revalidate secret parent before backup-authority initialization")?;

    loop {
        let intent_exists = named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
        let key_exists = named_entry_exists(&parent, "backup-authority-hmac.key")?;
        reconcile_intent_temporary(&parent, intent_exists, key_exists, &mut hook)?;

        if intent_exists {
            let intent = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
            validate_backup_authority_intent(
                &intent.document,
                source_commit,
                &parent,
                &activation_lock,
            )?;
            if key_exists {
                let key = load_published_backup_authority_key(&parent)?;
                validate_key_against_intent(&key, &intent.document)?;
                activation_lock.ensure_canonical()?;
                parent.ensure_canonical()?;
                return Ok(());
            }
            activation_lock.ensure_canonical()?;
            remove_exact_named_entry(
                &parent,
                BACKUP_AUTHORITY_INTENT_NAME,
                intent.device,
                intent.inode,
            )?;
            rustix::fs::fsync(&parent.fd)?;
            hook(BackupAuthorityKeyBoundary::RecoveryIntentRemoved)?;
            continue;
        }
        anyhow::ensure!(
            !key_exists,
            "backup-authority key exists without its transaction intent"
        );
        break;
    }

    activation_lock.ensure_canonical()?;
    let anonymous = openat(
        parent.fd.as_fd(),
        ".",
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::TMPFILE,
        Mode::from_raw_mode(0o400),
    )
    .context("create anonymous backup-authority key")?;
    fchmod(&anonymous, Mode::from_raw_mode(0o400))
        .context("chmod anonymous backup-authority key")?;
    let mut anonymous = std::fs::File::from(anonymous);
    let key = loop {
        let candidate: [u8; 32] = rand::random();
        if candidate != [0; 32] {
            break candidate;
        }
    };
    anonymous.write_all(&key)?;
    anonymous
        .sync_all()
        .context("sync anonymous backup-authority key")?;
    let key_metadata = anonymous.metadata()?;
    anyhow::ensure!(
        key_metadata.is_file()
            && key_metadata.dev() == parent.device
            && key_metadata.uid() == parent.uid
            && key_metadata.nlink() == 0
            && key_metadata.len() == 32
            && key_metadata.permissions().mode() & 0o777 == 0o400,
        "anonymous backup-authority key has unsafe identity, owner, links, size, or mode"
    );
    hook(BackupAuthorityKeyBoundary::AnonymousKeySynced)?;
    let intent = BackupAuthorityKeyIntentV1 {
        schema_version: BACKUP_AUTHORITY_INTENT_SCHEMA_VERSION,
        source_commit: source_commit.to_owned(),
        activation_lock_device: activation_lock.lock_device,
        activation_lock_inode: activation_lock.lock_inode,
        secret_parent_device: parent.device,
        secret_parent_inode: parent.inode,
        key_device: key_metadata.dev(),
        key_inode: key_metadata.ino(),
        key_sha256: hex::encode(Sha256::digest(key)),
    };
    validate_backup_authority_intent(&intent, source_commit, &parent, &activation_lock)?;
    let intent_bytes = canonical_json_bytes(&intent)?;
    anyhow::ensure!(
        u64::try_from(intent_bytes.len())? <= MAX_BACKUP_AUTHORITY_INTENT_BYTES,
        "backup-authority intent exceeds its byte limit"
    );
    let temporary = openat2(
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o400),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .context("create backup-authority intent temporary")?;
    fchmod(&temporary, Mode::from_raw_mode(0o400))
        .context("chmod backup-authority intent temporary")?;
    let mut temporary = std::fs::File::from(temporary);
    hook(BackupAuthorityKeyBoundary::IntentTemporaryCreated)?;
    temporary.write_all(&intent_bytes)?;
    hook(BackupAuthorityKeyBoundary::IntentTemporaryWritten)?;
    temporary
        .sync_all()
        .context("sync backup-authority intent temporary")?;
    rustix::fs::fsync(&parent.fd)
        .context("sync secret parent before backup-authority intent publication")?;
    hook(BackupAuthorityKeyBoundary::IntentTemporarySynced)?;
    activation_lock.ensure_canonical()?;
    renameat_with(
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_TEMP_NAME,
        parent.fd.as_fd(),
        BACKUP_AUTHORITY_INTENT_NAME,
        RenameFlags::NOREPLACE,
    )
    .context("publish backup-authority intent")?;
    rustix::fs::fsync(&parent.fd)
        .context("sync secret parent after backup-authority intent publication")?;
    let published_intent = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
    anyhow::ensure!(
        published_intent.document == intent && published_intent.bytes == intent_bytes,
        "published backup-authority intent differs from the retained transaction intent"
    );
    hook(BackupAuthorityKeyBoundary::IntentPublished)?;
    activation_lock.ensure_canonical()?;
    publish_anonymous_backup_authority_key_with(
        &anonymous,
        &parent,
        "backup-authority-hmac.key",
        Path::new("/proc/self/fd"),
        || {
            linkat(
                anonymous.as_fd(),
                "",
                parent.fd.as_fd(),
                "backup-authority-hmac.key",
                AtFlags::EMPTY_PATH,
            )
        },
    )?;
    hook(BackupAuthorityKeyBoundary::KeyLinked)?;
    rustix::fs::fsync(&parent.fd)
        .context("sync secret parent after backup-authority key publication")?;
    hook(BackupAuthorityKeyBoundary::KeyDirectorySynced)?;
    let published_key = load_published_backup_authority_key(&parent)?;
    validate_key_against_intent(&published_key, &intent)?;
    anyhow::ensure!(
        published_key.device == key_metadata.dev() && published_key.inode == key_metadata.ino(),
        "published backup-authority key is not the retained anonymous inode"
    );
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    hook(BackupAuthorityKeyBoundary::KeyValidated)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn complete_backup_authority_key_v2_at<V, H>(
    opt_root: &Path,
    key_path: &Path,
    expected_uid: u32,
    source_commit: &str,
    activation_lock_fd: u32,
    validate_outer_authority: V,
    mut hook: H,
) -> anyhow::Result<()>
where
    V: FnOnce() -> anyhow::Result<()>,
    H: FnMut(BackupAuthorityKeyBoundary) -> anyhow::Result<()>,
{
    anyhow::ensure!(
        valid_backup_authority_source_commit(source_commit),
        "backup-authority source commit is not exact lowercase 40-character hexadecimal"
    );
    let activation_lock = pin_activation_lock_at(opt_root, activation_lock_fd)?;
    let parent = pin_secret_parent(key_path, expected_uid)?;
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    anyhow::ensure!(
        !named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_TEMP_NAME)?,
        "backup-authority completion found an unresolved intent temporary"
    );
    let key = load_published_backup_authority_key(&parent)?;
    let intent = if named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_NAME)? {
        let intent = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
        validate_backup_authority_intent(
            &intent.document,
            source_commit,
            &parent,
            &activation_lock,
        )?;
        validate_key_against_intent(&key, &intent.document)?;
        Some(intent)
    } else {
        None
    };
    validate_outer_authority()?;
    hook(BackupAuthorityKeyBoundary::OuterAuthorityValidated)?;
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    anyhow::ensure!(
        load_published_backup_authority_key(&parent)? == key,
        "backup-authority key changed during outer authority validation"
    );
    if let Some(intent) = intent {
        let reloaded = load_backup_authority_intent(&parent, BACKUP_AUTHORITY_INTENT_NAME)?;
        anyhow::ensure!(
            reloaded.device == intent.device
                && reloaded.inode == intent.inode
                && reloaded.bytes == intent.bytes,
            "backup-authority intent changed during outer authority validation"
        );
        remove_exact_named_entry(
            &parent,
            BACKUP_AUTHORITY_INTENT_NAME,
            intent.device,
            intent.inode,
        )?;
        hook(BackupAuthorityKeyBoundary::CompletionIntentRemoved)?;
        rustix::fs::fsync(&parent.fd)?;
        hook(BackupAuthorityKeyBoundary::CompletionDirectorySynced)?;
        anyhow::ensure!(
            !named_entry_exists(&parent, BACKUP_AUTHORITY_INTENT_NAME)?,
            "backup-authority intent remains after completion"
        );
    } else {
        // An already completed invocation has only the durable key. It is
        // accepted here, never by initialization, because the caller just
        // re-proved the entire production runtime authority as present.
        rustix::fs::fsync(&parent.fd)?;
    }
    activation_lock.ensure_canonical()?;
    parent.ensure_canonical()?;
    anyhow::ensure!(
        load_published_backup_authority_key(&parent)? == key,
        "backup-authority key changed during completion"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn initialize_backup_authority_key_v2(
    source_commit: &str,
    activation_lock_fd: u32,
) -> anyhow::Result<()> {
    initialize_backup_authority_key_v2_at(
        Path::new(VPS_ACTIVATION_ROOT),
        Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
        rustix::process::geteuid().as_raw(),
        source_commit,
        activation_lock_fd,
        |_| Ok(()),
    )
}

#[cfg(not(target_os = "linux"))]
fn initialize_backup_authority_key_v2(
    _source_commit: &str,
    _activation_lock_fd: u32,
) -> anyhow::Result<()> {
    anyhow::bail!("transactional backup-authority initialization requires Linux")
}

#[cfg(target_os = "linux")]
fn complete_backup_authority_key_v2<V>(
    source_commit: &str,
    activation_lock_fd: u32,
    validate_outer_authority: V,
) -> anyhow::Result<()>
where
    V: FnOnce() -> anyhow::Result<()>,
{
    complete_backup_authority_key_v2_at(
        Path::new(VPS_ACTIVATION_ROOT),
        Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
        rustix::process::geteuid().as_raw(),
        source_commit,
        activation_lock_fd,
        validate_outer_authority,
        |_| Ok(()),
    )
}

#[cfg(not(target_os = "linux"))]
fn complete_backup_authority_key_v2<V>(
    _source_commit: &str,
    _activation_lock_fd: u32,
    _validate_outer_authority: V,
) -> anyhow::Result<()>
where
    V: FnOnce() -> anyhow::Result<()>,
{
    anyhow::bail!("transactional backup-authority completion requires Linux")
}

#[cfg(target_os = "linux")]
fn duplicate_inherited_fd(fd: u32, directory: bool) -> anyhow::Result<std::fs::File> {
    anyhow::ensure!(fd >= 3, "inherited verifier descriptor must be at least 3");
    let path = PathBuf::from(format!("/proc/self/fd/{fd}"));
    let file = std::fs::File::open(&path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "inherited verifier descriptor has the wrong file type"
    );
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
fn duplicate_inherited_fd(_fd: u32, _directory: bool) -> anyhow::Result<std::fs::File> {
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

#[cfg(target_os = "linux")]
fn inherited_fd_target(fd: u32) -> anyhow::Result<PathBuf> {
    let target = std::fs::read_link(format!("/proc/self/fd/{fd}"))?;
    anyhow::ensure!(
        target.is_absolute() && !target.to_string_lossy().ends_with(" (deleted)"),
        "inherited verifier descriptor has no stable absolute name"
    );
    Ok(target)
}

#[cfg(not(target_os = "linux"))]
fn inherited_fd_target(_fd: u32) -> anyhow::Result<PathBuf> {
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

fn pinned_file_target(file: &std::fs::File) -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;
        let fd = u32::try_from(file.as_raw_fd())?;
        return inherited_fd_target(fd);
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

fn require_pinned_file_name(file: &std::fs::File, expected: &str) -> anyhow::Result<PathBuf> {
    let target = pinned_file_target(file)?;
    anyhow::ensure!(
        target.file_name().and_then(|name| name.to_str()) == Some(expected),
        "inherited verifier descriptor has the wrong canonical filename"
    );
    Ok(target)
}

fn duplicate_pinned_file(file: &std::fs::File, directory: bool) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;
        return duplicate_inherited_fd(u32::try_from(file.as_raw_fd())?, directory);
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("inherited-FD backup verification requires Linux procfs")
}

fn pin_transaction_backup_from_status(
    backup_root_fd: u32,
    status_envelope_fd: u32,
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
) -> anyhow::Result<(std::fs::File, std::fs::File)> {
    let root_guard = duplicate_inherited_fd(backup_root_fd, true)?;
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "transaction backup root")?;
    let status_file = duplicate_inherited_fd(status_envelope_fd, false)?;
    anyhow::ensure!(
        require_pinned_file_name(&status_file, "backup-status.json")? == trusted_status_path,
        "transaction status envelope is outside the trusted authority"
    );
    validate_private_pinned_file(&status_file, 0o400, "transaction status envelope")?;
    let status_bytes = read_bounded_pinned_file(
        duplicate_pinned_file(&status_file, false)?,
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
    )?;
    let status: BackupStatusV4 = serde_json::from_slice(&status_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&status)? == status_bytes,
        "transaction status envelope is not canonical JSON"
    );
    let timestamp = parse_backup_id(&status.backup_id)
        .ok_or_else(|| anyhow::anyhow!("transaction status has an invalid backup ID"))?;
    let exact_child_path = trusted_backup_root.join(&status.backup_id);
    anyhow::ensure!(
        timestamp == status.created_at_unix_ms
            && status.backup_directory == exact_child_path.to_string_lossy(),
        "transaction status does not select the exact canonical backup child"
    );

    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(&root_guard, true)?);
    let child = open_cap_directory_nofollow(&root, Path::new(&status.backup_id))?.into_std_file();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = child.metadata()?;
        anyhow::ensure!(
            metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o777 == 0o700,
            "transaction backup child has the wrong owner or mode"
        );
    }
    anyhow::ensure!(
        pinned_file_target(&child)? == exact_child_path,
        "transaction backup child is not linked at its authenticated path"
    );
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "transaction backup root")?;
    Ok((root_guard, child))
}

fn pin_offline_backup_from_root(
    backup_root_fd: u32,
    backup_directory_fd: u32,
    trusted_backup_root: &Path,
) -> anyhow::Result<(std::fs::File, std::fs::File, String)> {
    let root_guard = duplicate_inherited_fd(backup_root_fd, true)?;
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "offline backup root")?;
    let directory_guard = duplicate_inherited_fd(backup_directory_fd, true)?;
    let target = pinned_file_target(&directory_guard)?;
    let backup_id = target
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| parse_backup_id(name).is_some())
        .ok_or_else(|| anyhow::anyhow!("offline backup child has a noncanonical identifier"))?
        .to_owned();
    anyhow::ensure!(
        target == trusted_backup_root.join(&backup_id),
        "offline backup child is outside the canonical backup root"
    );
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(&root_guard, true)?);
    let current = open_cap_directory_nofollow(&root, Path::new(&backup_id))?.into_std_file();
    let root_metadata = root_guard.metadata()?;
    let child_metadata = directory_guard.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            child_metadata.uid() == rustix::process::geteuid().as_raw()
                && child_metadata.dev() == root_metadata.dev()
                && child_metadata.permissions().mode() & 0o777 == 0o700,
            "offline backup child has the wrong owner, device, or mode"
        );
    }
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == metadata_identity_std(&child_metadata),
        "offline backup child descriptor differs from the canonical root entry"
    );
    revalidate_pinned_root_directory(&root_guard, trusted_backup_root, "offline backup root")?;
    Ok((root_guard, directory_guard, backup_id))
}

fn authenticated_release_identity_from_backup(
    directory: &std::fs::File,
    backup_id: &str,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<(BackupReleaseIdentityV2, String)> {
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(directory, true)?);
    let manifest_bytes = read_cap_regular_bounded(
        &root,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&manifest)? == manifest_bytes,
        "offline backup manifest is not canonical JSON"
    );
    manifest.validate()?;
    anyhow::ensure!(
        parse_backup_id(backup_id) == Some(manifest.created_at_unix_ms),
        "offline backup ID differs from its manifest timestamp"
    );
    let envelope_bytes = read_cap_regular_bounded_with_mode(
        &root,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let envelope: BackupVerificationEnvelopeV2 = serde_json::from_slice(&envelope_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&envelope)? == envelope_bytes,
        "offline backup verification envelope is not canonical JSON"
    );
    envelope.verify_manifest(backup_authority_key, backup_id, &manifest)?;
    Ok((
        manifest.release_identity,
        hex::encode(Sha256::digest(&manifest_bytes)),
    ))
}

fn pin_preserved_release_authority_from_root(
    root_guard: &std::fs::File,
    release_identity: &BackupReleaseIdentityV2,
    trusted_backup_root: &Path,
) -> anyhow::Result<(std::fs::File, std::fs::File, PathBuf)> {
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(root_guard, true)?);
    let root_metadata = root.dir_metadata()?;
    let store = open_cap_directory_nofollow(&root, Path::new(RELEASE_AUTHORITY_STORE))?;
    let store_metadata = store.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            store_metadata.uid() == rustix::process::geteuid().as_raw()
                && store_metadata.dev() == root_metadata.dev()
                && store_metadata.permissions().mode() & 0o777 == 0o700,
            "preserved release-authority store has the wrong owner, device, or mode"
        );
    }
    let store_guard = store.try_clone()?.into_std_file();
    let store_path = trusted_backup_root.join(RELEASE_AUTHORITY_STORE);
    anyhow::ensure!(
        pinned_file_target(&store_guard)? == store_path,
        "preserved release-authority store is outside the canonical backup root"
    );
    let name = release_authority_file_name(release_identity)?;
    let target = store_path.join(&name);
    let authority = open_cap_regular_nofollow(&store, Path::new(&name))?;
    validate_private_pinned_file(&authority, 0o400, "preserved release authority")?;
    anyhow::ensure!(
        metadata_identity_std(&authority.metadata()?).device
            == metadata_identity(&root_metadata).device
            && pinned_file_target(&authority)? == target,
        "preserved release authority has the wrong device or canonical path"
    );
    Ok((store_guard, authority, target))
}

fn revalidate_pinned_root_directory(
    pinned: &std::fs::File,
    expected_path: &Path,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected_path.is_absolute() && pinned_file_target(pinned)? == expected_path,
        "{label} is no longer linked at its canonical path"
    );
    let pinned_metadata = pinned.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned_metadata.is_dir()
                && pinned_metadata.uid() == rustix::process::geteuid().as_raw()
                && pinned_metadata.permissions().mode() & 0o777 == 0o700,
            "{label} has the wrong owner or mode"
        );
    }
    let current = open_directory_nofollow(expected_path)?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == metadata_identity_std(&pinned_metadata),
        "{label} canonical path no longer names the pinned inode"
    );
    Ok(())
}

async fn run_pinned_verifier_command(
    mode: BackupVerificationMode<'_>,
    status_envelope_fd: Option<u32>,
    backup_authority_key_fd: u32,
    expected_release_manifest_fd: Option<u32>,
    expected_source_commit: &str,
    expected_vps_release_manifest_sha256: &str,
    expected_publication_lock_sha256: &str,
) -> anyhow::Result<GuardedVerifierCommand> {
    let backup_authority_key_guard = duplicate_inherited_fd(backup_authority_key_fd, false)?;
    let backup_authority_key_target =
        require_pinned_file_name(&backup_authority_key_guard, "backup-authority-hmac.key")?;
    anyhow::ensure!(
        backup_authority_key_target == Path::new(DEFAULT_BACKUP_AUTHORITY_KEY),
        "pinned backup-authority key is outside the trusted production authority"
    );
    validate_private_pinned_file(
        &backup_authority_key_guard,
        0o400,
        "backup authority HMAC key",
    )?;
    let backup_authority_key: [u8; 32] = read_bounded_pinned_file(
        duplicate_pinned_file(&backup_authority_key_guard, false)?,
        32,
    )?
    .try_into()
    .map_err(|_| anyhow::anyhow!("backup authority HMAC key has the wrong length"))?;
    anyhow::ensure!(
        backup_authority_key.iter().any(|byte| *byte != 0),
        "backup authority HMAC key is all zero"
    );
    let (
        backup,
        pinned_release,
        release_store_guard,
        release_target,
        release_expected_mode,
        expected_release,
        historical_release_authority,
        backup_root_guard,
    ) = match mode {
        BackupVerificationMode::Offline {
            backup_root_fd,
            backup_directory_fd,
            expected_manifest_sha256,
        } => {
            anyhow::ensure!(
                status_envelope_fd.is_none() && expected_release_manifest_fd.is_none(),
                "offline verification derives authority from the authenticated backup envelope"
            );
            let (root_guard, directory_guard, backup_id) = pin_offline_backup_from_root(
                backup_root_fd,
                backup_directory_fd,
                Path::new(DEFAULT_BACKUP_ROOT),
            )?;
            let (authenticated_release, authenticated_manifest_sha256) =
                authenticated_release_identity_from_backup(
                    &directory_guard,
                    &backup_id,
                    &backup_authority_key,
                )?;
            anyhow::ensure!(
                authenticated_manifest_sha256 == expected_manifest_sha256,
                "authenticated backup manifest differs from the explicit expected digest"
            );
            anyhow::ensure!(
                authenticated_release.source_commit == expected_source_commit
                    && authenticated_release.vps_release_manifest_sha256
                        == expected_vps_release_manifest_sha256
                    && authenticated_release.publication_lock_sha256
                        == expected_publication_lock_sha256,
                "authenticated backup release differs from the explicit expected identity"
            );
            let (store_guard, release_guard, release_target) =
                pin_preserved_release_authority_from_root(
                    &root_guard,
                    &authenticated_release,
                    Path::new(DEFAULT_BACKUP_ROOT),
                )?;
            let expected_release = load_backup_release_identity_preserved_file(
                duplicate_pinned_file(&release_guard, false)?,
            )
            .await?;
            anyhow::ensure!(
                expected_release == authenticated_release,
                "preserved release authority differs from the authenticated backup envelope"
            );
            let backup = verify_backup_pinned_offline_with_expected(
                backup_directory_fd,
                expected_manifest_sha256,
                &expected_release,
                &backup_authority_key,
                Path::new(DEFAULT_BACKUP_ROOT),
                true,
            )
            .await?;
            (
                backup,
                release_guard,
                Some(store_guard),
                release_target,
                0o400,
                expected_release,
                true,
                root_guard,
            )
        }
        BackupVerificationMode::Transaction { backup_root_fd } => {
            let release_fd = expected_release_manifest_fd.ok_or_else(|| {
                anyhow::anyhow!("transaction verification requires a pinned release manifest")
            })?;
            let pinned_release = duplicate_inherited_fd(release_fd, false)?;
            let release_target =
                require_pinned_file_name(&pinned_release, "vps-release-manifest-v2.json")?;
            anyhow::ensure!(
                release_target
                    == Path::new(INSTALLED_RELEASE_ROOT)
                        .join(expected_source_commit)
                        .join("vps-release-manifest-v2.json"),
                "transaction release manifest is outside the exact installed source release"
            );
            validate_private_pinned_file(&pinned_release, 0o440, "out-of-band release authority")?;
            let expected_release = load_backup_release_identity_oob_file(duplicate_pinned_file(
                &pinned_release,
                false,
            )?)
            .await?;
            anyhow::ensure!(
                expected_release.source_commit == expected_source_commit
                    && expected_release.vps_release_manifest_sha256
                        == expected_vps_release_manifest_sha256
                    && expected_release.publication_lock_sha256 == expected_publication_lock_sha256,
                "out-of-band release manifest differs from the explicit expected release identity"
            );
            let status_envelope_fd = status_envelope_fd.ok_or_else(|| {
                anyhow::anyhow!("transaction verification requires a pinned status envelope")
            })?;
            let (backup, root_guard) = verify_transaction_backup_from_root_pinned(
                backup_root_fd,
                status_envelope_fd,
                &expected_release,
                &backup_authority_key,
                Path::new(DEFAULT_BACKUP_ROOT),
                Path::new(DEFAULT_BACKUP_STATUS),
                true,
            )
            .await?;
            (
                backup,
                pinned_release,
                None,
                release_target,
                0o440,
                expected_release,
                false,
                root_guard,
            )
        }
    };
    Ok(GuardedVerifierCommand {
        backup,
        release_guard: pinned_release,
        release_store_guard,
        release_target,
        release_expected_mode,
        expected_release,
        historical_release_authority,
        backup_authority_key_guard,
        expected_backup_authority_key: backup_authority_key,
        backup_root_guard,
    })
}

#[cfg(test)]
async fn verify_backup_pinned_with_expected(
    backup_directory_fd: u32,
    status_envelope_fd: u32,
    expected_manifest_sha256: &str,
    expected_release: &BackupReleaseIdentityV2,
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<GuardedBackupVerificationReceipt> {
    verify_backup_pinned_with_expected_and_hook(
        backup_directory_fd,
        status_envelope_fd,
        BackupManifestExpectation::Explicit(expected_manifest_sha256),
        expected_release,
        trusted_backup_root,
        trusted_status_path,
        require_production_restore_paths,
        || Ok(()),
    )
    .await
}

async fn verify_backup_pinned_offline_with_expected(
    backup_directory_fd: u32,
    expected_manifest_sha256: &str,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<GuardedBackupVerificationReceipt> {
    verify_backup_pinned_core(
        backup_directory_fd,
        None,
        BackupManifestExpectation::Explicit(expected_manifest_sha256),
        expected_release,
        backup_authority_key,
        trusted_backup_root,
        None,
        require_production_restore_paths,
        || Ok(()),
    )
    .await
}

async fn verify_transaction_backup_pinned(
    backup_directory_fd: u32,
    status_envelope_fd: u32,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<GuardedBackupVerificationReceipt> {
    verify_backup_pinned_core(
        backup_directory_fd,
        Some(status_envelope_fd),
        BackupManifestExpectation::AuthenticatedStatus,
        expected_release,
        backup_authority_key,
        trusted_backup_root,
        Some(trusted_status_path),
        require_production_restore_paths,
        || Ok(()),
    )
    .await
}

async fn verify_transaction_backup_from_root_pinned(
    backup_root_fd: u32,
    status_envelope_fd: u32,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
) -> anyhow::Result<(GuardedBackupVerificationReceipt, std::fs::File)> {
    let (root_guard, backup_directory) = pin_transaction_backup_from_status(
        backup_root_fd,
        status_envelope_fd,
        trusted_backup_root,
        trusted_status_path,
    )?;
    #[cfg(target_os = "linux")]
    let backup_directory_fd = {
        use std::os::fd::AsRawFd as _;
        u32::try_from(backup_directory.as_raw_fd())?
    };
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("transaction backup verification requires Linux procfs");
    let backup = verify_transaction_backup_pinned(
        backup_directory_fd,
        status_envelope_fd,
        expected_release,
        backup_authority_key,
        trusted_backup_root,
        trusted_status_path,
        require_production_restore_paths,
    )
    .await?;
    Ok((backup, root_guard))
}

#[cfg(test)]
async fn verify_backup_pinned_with_expected_and_hook<F>(
    backup_directory_fd: u32,
    status_envelope_fd: u32,
    manifest_expectation: BackupManifestExpectation<'_>,
    expected_release: &BackupReleaseIdentityV2,
    trusted_backup_root: &Path,
    trusted_status_path: &Path,
    require_production_restore_paths: bool,
    before_final_authority_check: F,
) -> anyhow::Result<GuardedBackupVerificationReceipt>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    verify_backup_pinned_core(
        backup_directory_fd,
        Some(status_envelope_fd),
        manifest_expectation,
        expected_release,
        &[0x31; 32],
        trusted_backup_root,
        Some(trusted_status_path),
        require_production_restore_paths,
        before_final_authority_check,
    )
    .await
}

async fn verify_backup_pinned_core<F>(
    backup_directory_fd: u32,
    status_envelope_fd: Option<u32>,
    manifest_expectation: BackupManifestExpectation<'_>,
    expected_release: &BackupReleaseIdentityV2,
    backup_authority_key: &[u8; 32],
    trusted_backup_root: &Path,
    trusted_status_path: Option<&Path>,
    require_production_restore_paths: bool,
    before_final_authority_check: F,
) -> anyhow::Result<GuardedBackupVerificationReceipt>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    if let BackupManifestExpectation::Explicit(expected_digest) = manifest_expectation {
        validate_expected_digest(expected_digest)?;
    }
    expected_release.validate()?;
    anyhow::ensure!(
        trusted_status_path.is_none_or(Path::is_absolute) && trusted_backup_root.is_absolute(),
        "pinned verifier authority paths must be absolute"
    );
    let operation_lock = acquire_backup_operation_lock(trusted_backup_root)?;
    let status_authority = match (status_envelope_fd, trusted_status_path) {
        (Some(fd), Some(path)) => {
            let file = duplicate_inherited_fd(fd, false)?;
            let target = require_pinned_file_name(&file, "backup-status.json")?;
            anyhow::ensure!(
                target == path,
                "pinned status envelope is outside the trusted production authority"
            );
            validate_private_pinned_file(&file, 0o400, "backup status envelope")?;
            let bytes = read_bounded_pinned_file(
                duplicate_pinned_file(&file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )?;
            let status: BackupStatusV4 = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                canonical_json_bytes(&status)? == bytes,
                "pinned backup status envelope is not canonical JSON"
            );
            status.verify(backup_authority_key)?;
            Some((file, bytes, status, path))
        }
        (None, None) => None,
        _ => anyhow::bail!("status descriptor and trusted path must be supplied together"),
    };

    let directory_guard = duplicate_inherited_fd(backup_directory_fd, true)?;
    let directory_target = pinned_file_target(&directory_guard)?;
    let root_metadata = directory_guard.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.permissions().mode() & 0o777 == 0o700,
            "pinned backup directory has the wrong owner or mode"
        );
    }
    let backup_id = directory_target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("pinned backup directory has no UTF-8 identifier"))?;
    anyhow::ensure!(
        parse_backup_id(backup_id).is_some()
            && directory_target.parent() == Some(trusted_backup_root)
            && status_authority.as_ref().is_none_or(|(_, _, status, _)| {
                directory_target == Path::new(&status.backup_directory)
                    && status.backup_id == backup_id
            }),
        "pinned backup directory name differs from the authenticated backup ID"
    );
    let root = cap_std::fs::Dir::from_std_file(duplicate_pinned_file(&directory_guard, true)?);
    let require_current_schema = matches!(
        manifest_expectation,
        BackupManifestExpectation::AuthenticatedStatus
    );
    let verified = verify_backup_capability(
        &root,
        backup_id,
        backup_authority_key,
        require_current_schema,
    )
    .await?;
    anyhow::ensure!(
        &verified.release_identity == expected_release,
        "pinned backup differs from the out-of-band release identity"
    );
    if let BackupManifestExpectation::Explicit(expected_digest) = manifest_expectation {
        anyhow::ensure!(
            verified.manifest_sha256 == expected_digest,
            "pinned backup differs from the out-of-band manifest digest"
        );
    }
    let manifest_bytes = read_cap_regular_bounded(
        &root,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
    )?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    if require_production_restore_paths {
        manifest.validate_production_restore_paths()?;
    }
    if let Some((_, _, status, _)) = &status_authority {
        anyhow::ensure!(
            status.backup_manifest_sha256 == verified.manifest_sha256
                && status.release_identity == verified.release_identity
                && status.database_schema_version == verified.database_schema_version
                && status.created_at_unix_ms == verified.created_at_unix_ms
                && status.file_count == verified.file_count
                && status.directory_count == verified.directory_count
                && status.total_bytes == verified.total_bytes,
            "authenticated status envelope differs from the pinned verified backup"
        );
    }
    let envelope_bytes = read_cap_regular_bounded_with_mode(
        &root,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let receipt_verified = verified.clone();
    let receipt = BackupVerificationReceiptV2 {
        schema_version: 2,
        verification_envelope_sha256: hex::encode(Sha256::digest(&envelope_bytes)),
        verification_envelope_byte_length: u64::try_from(envelope_bytes.len())?,
        current_status: status_authority.as_ref().map(|(_, bytes, _, _)| {
            BackupCurrentStatusEvidenceV2 {
                sha256: hex::encode(Sha256::digest(bytes)),
                byte_length: u64::try_from(bytes.len()).expect("bounded status length fits u64"),
            }
        }),
        backup_id: backup_id.to_owned(),
        backup_directory: directory_target.to_string_lossy().into_owned(),
        backup_manifest_sha256: verified.manifest_sha256,
        release_identity: verified.release_identity,
        database_schema_version: verified.database_schema_version,
        file_count: verified.file_count,
        directory_count: verified.directory_count,
        total_bytes: verified.total_bytes,
    };
    receipt.validate()?;
    before_final_authority_check()?;
    revalidate_operation_lock_path(trusted_backup_root, &operation_lock)?;
    if let Some((status_file, status_bytes, _, trusted_status_path)) = &status_authority {
        revalidate_pinned_regular_path(
            status_file,
            trusted_status_path,
            0o400,
            Some(0o700),
            "backup status envelope",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == *status_bytes,
            "pinned backup status bytes changed during verification"
        );
        revalidate_pinned_regular_path(
            status_file,
            trusted_status_path,
            0o400,
            Some(0o700),
            "backup status envelope",
        )?;
    }
    revalidate_pinned_directory_path(
        &directory_guard,
        Path::new(&receipt.backup_directory),
        trusted_backup_root,
        "backup payload directory",
    )?;
    let final_verified = verify_backup_capability(
        &root,
        backup_id,
        backup_authority_key,
        require_current_schema,
    )
    .await?;
    anyhow::ensure!(
        final_verified == receipt_verified,
        "pinned backup verification result changed at the receipt boundary"
    );
    revalidate_pinned_directory_path(
        &directory_guard,
        Path::new(&receipt.backup_directory),
        trusted_backup_root,
        "backup payload directory",
    )?;
    revalidate_operation_lock_path(trusted_backup_root, &operation_lock)?;
    Ok(GuardedBackupVerificationReceipt {
        receipt,
        operation_lock,
        trusted_backup_root: trusted_backup_root.to_owned(),
        _status_guard: status_authority.map(|(file, _, _, _)| file),
        _backup_directory_guard: directory_guard,
    })
}

fn revalidate_operation_lock_path(
    trusted_backup_root: &Path,
    held_lock: &std::fs::File,
) -> anyhow::Result<()> {
    let expected_path = trusted_backup_root.join(".backup-operation.lock");
    anyhow::ensure!(
        pinned_file_target(held_lock)? == expected_path,
        "backup operation lock is no longer linked at its canonical path"
    );
    validate_private_pinned_file(held_lock, 0o600, "backup operation lock")?;
    let root = pin_directory_capability(trusted_backup_root)?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        let metadata = root.dir_metadata()?;
        anyhow::ensure!(
            metadata_identity(&metadata).owner == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o777 == 0o700,
            "backup operation lock parent has the wrong owner or mode"
        );
    }
    let current = open_cap_regular_nofollow(&root, Path::new(".backup-operation.lock"))?;
    validate_private_pinned_file(&current, 0o600, "backup operation lock")?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?)
            == metadata_identity_std(&held_lock.metadata()?),
        "canonical backup operation lock path no longer names the held inode"
    );
    Ok(())
}

fn revalidate_pinned_regular_path(
    pinned: &std::fs::File,
    expected_path: &Path,
    expected_mode: u32,
    expected_parent_mode: Option<u32>,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        pinned_file_target(pinned)? == expected_path,
        "{label} is no longer linked at its exact admitted path"
    );
    validate_private_pinned_file(pinned, expected_mode, label)?;
    let parent_path = expected_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} path has no parent"))?;
    let name = expected_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} path has no filename"))?;
    let parent = pin_directory_capability(parent_path)?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        let metadata = parent.dir_metadata()?;
        anyhow::ensure!(
            metadata_identity(&metadata).owner == rustix::process::geteuid().as_raw()
                && metadata_identity_std(&pinned.metadata()?).device
                    == metadata_identity(&metadata).device,
            "{label} or its parent has the wrong owner or device"
        );
        if let Some(expected_parent_mode) = expected_parent_mode {
            anyhow::ensure!(
                metadata.permissions().mode() & 0o777 == expected_parent_mode,
                "{label} parent has the wrong mode"
            );
        }
    }
    let current = open_cap_regular_nofollow(&parent, Path::new(name))?;
    validate_private_pinned_file(&current, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == metadata_identity_std(&pinned.metadata()?),
        "{label} path no longer names the pinned inode"
    );
    Ok(())
}

fn revalidate_pinned_directory_path(
    pinned: &std::fs::File,
    expected_path: &Path,
    trusted_parent_path: &Path,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        expected_path.parent() == Some(trusted_parent_path)
            && pinned_file_target(pinned)? == expected_path,
        "{label} is no longer linked at its exact admitted path"
    );
    let pinned_metadata = pinned.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned_metadata.is_dir()
                && pinned_metadata.uid() == rustix::process::geteuid().as_raw()
                && pinned_metadata.permissions().mode() & 0o777 == 0o700,
            "{label} has the wrong owner or mode"
        );
    }
    let parent = pin_directory_capability(trusted_parent_path)?;
    let parent_metadata = parent.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
        anyhow::ensure!(
            metadata_identity(&parent_metadata).owner == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o777 == 0o700
                && metadata_identity_std(&pinned_metadata).device
                    == metadata_identity(&parent_metadata).device,
            "{label} or its parent has the wrong owner, mode, or device"
        );
    }
    let name = expected_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} path has no filename"))?;
    let current = open_cap_directory_nofollow(&parent, Path::new(name))?;
    anyhow::ensure!(
        metadata_identity(&current.dir_metadata()?) == metadata_identity_std(&pinned_metadata),
        "{label} path no longer names the pinned inode"
    );
    Ok(())
}

fn validate_expected_digest(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.bytes().any(|byte| byte != b'0'),
        "expected backup manifest digest is not canonical nonzero SHA-256"
    );
    Ok(())
}

fn validate_private_pinned_file(
    file: &std::fs::File,
    mode: u32,
    label: &str,
) -> anyhow::Result<()> {
    let metadata = file.metadata()?;
    anyhow::ensure!(metadata.is_file(), "{label} is not a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o777 == mode
                && metadata.nlink() == 1,
            "{label} has the wrong owner, mode, or link count"
        );
    }
    Ok(())
}

fn read_bounded_pinned_file(file: std::fs::File, maximum: u64) -> anyhow::Result<Vec<u8>> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "pinned document is not a bounded regular file"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? <= maximum,
        "pinned document exceeds its byte limit"
    );
    Ok(bytes)
}

fn restore_source_map(mappings: &[RestoreSourceMap]) -> anyhow::Result<BTreeMap<PathBuf, PathBuf>> {
    let mut sources = BTreeMap::new();
    for mapping in mappings {
        anyhow::ensure!(
            sources
                .insert(mapping.original.clone(), mapping.readable_source.clone())
                .is_none(),
            "duplicate restore source map for {}",
            mapping.original.display()
        );
    }
    Ok(sources)
}

fn validate_backup_restore_source_contract(
    config: &ServerConfig,
    sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    let moderation = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup requires a moderation bearer secret path"))?;
    let mut expected = BTreeSet::from([
        config.cursor_secret_path.clone(),
        config.competition_run_grant_secret_path.clone(),
        config.run_preflight_grant_secret_path.clone(),
        moderation.clone(),
    ]);
    expected.extend(
        SYSTEMD_UNIT_FILES
            .into_iter()
            .map(|unit| Path::new(SYSTEMD_USER_ROOT).join(unit)),
    );
    let actual = sources.keys().cloned().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        actual == expected,
        "restore source maps must name exactly the four secrets and five installed user units"
    );
    Ok(())
}

async fn backup_and_publish_status(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<PathBuf> {
    backup_and_publish_status_with_limit(
        config,
        release_manifest_path,
        release_identity,
        backup_root,
        status_path,
        retain_complete,
        restore_sources,
        robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
    )
    .await
}

async fn backup_and_publish_status_with_limit(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
) -> anyhow::Result<PathBuf> {
    backup_and_publish_status_with_limit_and_publisher(
        config,
        release_manifest_path,
        release_identity,
        backup_root,
        status_path,
        retain_complete,
        restore_sources,
        maximum_status_bytes,
        publish_private_atomic,
    )
    .await
}

async fn backup_and_publish_status_with_limit_and_publisher<F>(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
    publish_status: F,
) -> anyhow::Result<PathBuf>
where
    F: FnOnce(&Path, &[u8]) -> anyhow::Result<StatusPublicationOutcome> + Send + 'static,
{
    backup_and_publish_status_with_limit_and_publisher_and_hooks(
        config,
        release_manifest_path,
        release_identity,
        backup_root,
        status_path,
        retain_complete,
        restore_sources,
        maximum_status_bytes,
        publish_status,
        || Ok(()),
        || Ok(()),
    )
    .await
}

async fn backup_and_publish_status_with_limit_and_publisher_and_hooks<F, I, S>(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
    publish_status: F,
    before_install: I,
    before_status_publication: S,
) -> anyhow::Result<PathBuf>
where
    F: FnOnce(&Path, &[u8]) -> anyhow::Result<StatusPublicationOutcome> + Send + 'static,
    I: FnOnce() -> anyhow::Result<()> + Send + 'static,
    S: FnOnce() -> anyhow::Result<()> + Send + 'static,
{
    let config = config.clone();
    let release_manifest_path = release_manifest_path.to_owned();
    let release_identity = release_identity.clone();
    let backup_root = backup_root.to_owned();
    let status_path = status_path.to_owned();
    let restore_sources = restore_sources.clone();
    run_owned_backup(async move {
        backup_and_publish_status_owned(
            &config,
            &release_manifest_path,
            &release_identity,
            &backup_root,
            &status_path,
            retain_complete,
            &restore_sources,
            maximum_status_bytes,
            publish_status,
            before_install,
            before_status_publication,
        )
        .await
    })
    .await
}

/// Own the whole operation, including admission and final cleanup. Dropping the
/// caller's waiter must not release operation.lock or either kernel fence while
/// a backup is still running. This owner is never aborted by its caller.
async fn run_owned_backup<T, F>(operation: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
{
    tokio::spawn(operation)
        .await
        .map_err(|error| anyhow::anyhow!(error).context("backup owner task failed"))?
}

async fn backup_and_publish_status_owned<F, I, S>(
    config: &ServerConfig,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
    backup_root: &Path,
    status_path: &Path,
    retain_complete: usize,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
    maximum_status_bytes: usize,
    publish_status: F,
    before_install: I,
    before_status_publication: S,
) -> anyhow::Result<PathBuf>
where
    F: FnOnce(&Path, &[u8]) -> anyhow::Result<StatusPublicationOutcome>,
    I: FnOnce() -> anyhow::Result<()>,
    S: FnOnce() -> anyhow::Result<()>,
{
    anyhow::ensure!(
        maximum_status_bytes > 0
            && maximum_status_bytes <= robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
        "backup status byte limit is invalid"
    );
    anyhow::ensure!(
        retain_complete >= 1,
        "at least one complete backup is required"
    );
    anyhow::ensure!(backup_root.is_absolute(), "backup root must be absolute");
    anyhow::ensure!(
        config.release_manifest_path.as_deref() == Some(release_manifest_path),
        "backup release manifest path must exactly match release_manifest_path"
    );
    release_identity.validate()?;
    anyhow::ensure!(
        release_identity.database_schema_version == robin_highscores::db::CURRENT_SCHEMA_VERSION,
        "active release database schema differs from the running backup authority"
    );
    validate_backup_restore_source_contract(config, restore_sources)?;
    anyhow::ensure!(
        status_path.is_absolute(),
        "backup status path must be absolute"
    );
    anyhow::ensure!(
        status_path.file_name().and_then(|name| name.to_str()) == Some("backup-status.json"),
        "backup status path must name backup-status.json"
    );
    anyhow::ensure!(
        config.backup_manifest_path.as_deref() == Some(status_path),
        "published status path must exactly match backup_manifest_path"
    );
    let backup_root_metadata = tokio::fs::symlink_metadata(backup_root).await?;
    anyhow::ensure!(
        backup_root_metadata.is_dir() && !backup_root_metadata.file_type().is_symlink(),
        "backup root must be a real directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            backup_root_metadata.uid() == rustix::process::geteuid().as_raw()
                && backup_root_metadata.permissions().mode() & 0o777 == 0o700,
            "backup root has the wrong owner or mode"
        );
    }
    let canonical_backup_root = tokio::fs::canonicalize(backup_root).await?;
    anyhow::ensure!(
        canonical_backup_root == backup_root,
        "backup root must not traverse symlinks or aliases"
    );
    let backup_root = canonical_backup_root.as_path();
    let backup_authority_source = pin_restore_source(
        &config.backup_authority_hmac_secret_path,
        0o400,
        Some(32),
        None,
        "backup authority HMAC key",
    )?;
    let backup_authority_key: [u8; 32] = backup_authority_source
        .bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("backup authority HMAC key has the wrong length"))?;
    anyhow::ensure!(
        backup_authority_key.iter().any(|byte| *byte != 0),
        "backup authority HMAC key must not be all zero"
    );
    let status_parent = status_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup status path has no parent"))?;
    let status_parent_metadata = tokio::fs::symlink_metadata(status_parent).await?;
    anyhow::ensure!(
        status_parent_metadata.is_dir() && !status_parent_metadata.file_type().is_symlink(),
        "backup status parent must be a real directory"
    );
    let canonical_status_parent = tokio::fs::canonicalize(status_parent).await?;
    anyhow::ensure!(
        canonical_status_parent == status_parent,
        "backup status parent must not traverse symlinks or aliases"
    );
    anyhow::ensure!(
        !canonical_status_parent.starts_with(backup_root),
        "backup status path must remain outside the protected backup payload root"
    );
    let status_parent_capability = pin_directory_capability(status_parent)?;
    let status_parent_guard = status_parent_capability.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&status_parent_guard, status_parent, "backup status parent")?;
    let status_name = Path::new("backup-status.json");
    let existing_status = if cap_entry_exists(&status_parent_capability, status_name)? {
        let status_file = open_cap_regular_nofollow(&status_parent_capability, status_name)?;
        revalidate_pinned_regular_path(
            &status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        let status_bytes = read_bounded_pinned_file(
            duplicate_pinned_file(&status_file, false)?,
            u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        )?;
        revalidate_pinned_regular_path(
            &status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(&status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == status_bytes,
            "existing backup status changed during admission"
        );
        let status: BackupStatusV4 = serde_json::from_slice(&status_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&status)? == status_bytes,
            "existing backup status is not canonical JSON"
        );
        status.verify(&backup_authority_key)?;
        anyhow::ensure!(
            Path::new(&status.backup_directory) == backup_root.join(&status.backup_id),
            "existing backup status is outside the canonical backup root"
        );
        Some((status, status_file, status_bytes))
    } else {
        None
    };

    let _operation_lock = acquire_backup_operation_lock(backup_root)?;
    backup_authority_source.revalidate("backup authority HMAC key")?;
    revalidate_pinned_root_directory(&status_parent_guard, status_parent, "backup status parent")?;
    if let Some((_, status_file, status_bytes)) = &existing_status {
        revalidate_pinned_regular_path(
            status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == *status_bytes,
            "existing backup status changed before cleanup admission"
        );
    } else {
        anyhow::ensure!(
            !cap_entry_exists(&status_parent_capability, status_name)?,
            "backup status appeared while acquiring the operation lock"
        );
    }
    recover_interrupted_complete_cleanups(backup_root, &backup_authority_key)?;
    recover_stale_partial_backups(backup_root)?;
    if let Some((status, status_file, status_bytes)) = &existing_status {
        // Reconcile a crash after status publication but before post-publish
        // retention before allocating another generation. Prune only a crash
        // backlog down to the configured steady-state count; healthy retained
        // redundancy remains intact until the replacement is fully published.
        let maximum_admitted_generations = retain_complete
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("retention generation bound overflows"))?;
        backup_authority_source.revalidate("backup authority HMAC key before retention")?;
        retain_complete_backups(
            backup_root,
            &status.backup_id,
            retain_complete,
            maximum_admitted_generations,
            release_identity,
            &backup_authority_key,
        )
        .await?;
        backup_authority_source.revalidate("backup authority HMAC key after retention")?;
        revalidate_pinned_root_directory(
            &status_parent_guard,
            status_parent,
            "backup status parent",
        )?;
        revalidate_pinned_regular_path(
            status_file,
            status_path,
            0o400,
            Some(0o700),
            "existing backup status",
        )?;
        anyhow::ensure!(
            read_bounded_pinned_file(
                duplicate_pinned_file(status_file, false)?,
                u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            )? == *status_bytes,
            "existing backup status changed during pre-backup retention"
        );
    }

    let database = Database::connect(config).await?;
    let (backup_lock, exclusive_fence) = acquire_backup_write_authority(&database).await?;
    let result: anyhow::Result<PathBuf> = run_with_backup_lock_heartbeat(
        &database,
        &backup_lock,
        async {
            wait_for_maintenance_writers(&database, &backup_lock).await?;
            exclusive_fence.revalidate()?;
            anyhow::ensure!(
                database.active_maintenance_write_lease_count().await? == 0,
                "maintenance writer lease appeared after the exclusive TTL drain"
            );
            let replay =
                ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes)
                    .await?;
            let campaign = CampaignStore::create(
                config.campaign_state_directory.clone(),
                config.max_campaign_bytes,
            )
            .await?;

    // Recompute the same typed estimate used by deployment while holding the
    // operation lock and immediately before any partial backup byte is
    // created. Existing retained backups are already charged to live free
    // space; exactly one additional generation is required here.
    estimate_backup_space(
        config,
        release_identity,
        backup_root,
        status_path,
        restore_sources,
    )
    .await?
    .ensure_available()?;
    preserve_release_authority(backup_root, release_manifest_path, release_identity).await?;

    let created_at_unix_ms = u64::try_from(robin_highscores::model::now_epoch_ms()?)?;
    let identifier = format!(
        "backup-v4-{created_at_unix_ms}-{}",
        uuid::Uuid::now_v7().simple()
    );
    let partial = backup_root.join(format!(".{identifier}.partial"));
    let complete = backup_root.join(&identifier);
    create_backup_directory(&partial).await?;
    set_private_directory(&partial).await?;
    let backup_result = backup_locked(
        config,
        &database,
                &replay,
                &campaign,
        &backup_lock,
        &partial,
        created_at_unix_ms,
        release_identity,
        restore_sources,
    )
    .await;
            if let Err(error) = backup_result {
                remove_owned_partial_backup(backup_root, &partial)?;
                return Err(error);
            }
            let manifest_bytes = read_bounded_regular_nofollow(
                &partial.join("backup-manifest.json"),
                u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
            )
            .await?;
            let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
            anyhow::ensure!(
                canonical_json_bytes(&manifest)? == manifest_bytes,
                "new backup manifest is not canonical before envelope publication"
            );
            backup_authority_source
                .revalidate("backup authority HMAC key before envelope publication")?;
            let envelope = BackupVerificationEnvelopeV2::new_authenticated(
                identifier.clone(),
                &manifest,
                &backup_authority_key,
            )?;
            let envelope_path = partial.join("backup-verification-envelope.json");
            write_private_file(&envelope_path, &canonical_json_bytes(&envelope)?).await?;
            #[cfg(unix)]
            set_backup_permissions(
                &envelope_path,
                std::fs::Permissions::from_mode(0o400),
            )
            .await?;
            sync_directory(&partial).await?;
            refresh_backup_lock(&database, &backup_lock).await?;
            let verified = match verify_backup_authenticated(&partial, &backup_authority_key).await {
        Ok(verified) => verified,
        Err(error) => {
            remove_owned_partial_backup(backup_root, &partial)?;
            return Err(error);
        }
    };
    anyhow::ensure!(
        verified.release_identity == *release_identity,
        "verified backup release identity differs from the active installed release"
    );
    if fs2::available_space(backup_root)? < config.minimum_storage_free_bytes {
        remove_owned_partial_backup(backup_root, &partial)?;
        anyhow::bail!("completed backup would violate the configured storage floor");
    }

    // Build and bound the complete authenticated readiness envelope while the
    // backup is still a removable partial. An installed backup must never be
    // left behind merely because its projection cannot be published.
    let status_bytes_result = async {
        let manifest_bytes = read_bounded_regular_nofollow(
            &partial.join("backup-manifest.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        )
        .await?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(&manifest_bytes)) == verified.manifest_sha256,
            "verified backup manifest changed before status publication"
        );
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
        anyhow::ensure!(
            canonical_json_bytes(&manifest)? == manifest_bytes
                && manifest.created_at_unix_ms == verified.created_at_unix_ms
                && manifest.database_schema_version == verified.database_schema_version
                && manifest.release_identity == verified.release_identity
                && u64::try_from(manifest.files.len())? == verified.file_count
                && manifest.directory_count()? == verified.directory_count
                && manifest.total_bytes()? == verified.total_bytes,
            "verified backup manifest changed identity before status publication"
        );
        backup_authority_source
            .revalidate("backup authority HMAC key before status publication")?;
        let status = BackupStatusV4::new_authenticated(
            identifier.clone(),
                        complete.to_string_lossy().into_owned(),
                        manifest,
                        &backup_authority_key,
                    )?;
                    status.verify(&backup_authority_key)?;
        let status_bytes = canonical_json_bytes(&status)?;
        anyhow::ensure!(
            status_bytes.len() <= maximum_status_bytes,
            "authenticated backup readiness envelope exceeds its byte limit"
        );
        Ok::<_, anyhow::Error>(status_bytes)
    }
    .await;
    let status_bytes = match status_bytes_result {
        Ok(bytes) => bytes,
        Err(error) => {
            remove_owned_partial_backup(backup_root, &partial)?;
            return Err(error);
        }
    };
    refresh_backup_lock(&database, &backup_lock).await?;
    if let Err(error) = before_install().and_then(|()| {
        backup_authority_source.revalidate("backup authority HMAC key before backup installation")
    }) {
        remove_owned_partial_backup(backup_root, &partial)?;
        return Err(error.context("backup authority changed before backup installation"));
    }
    match install_verified_partial(&partial, &complete, backup_root) {
        Ok(BackupInstallOutcome::Installed) => {}
        Ok(BackupInstallOutcome::InstalledButParentSyncFailed(error)) => {
            anyhow::bail!(
                "verified backup was installed at {} but backup-root durability sync failed; status was not published: {error:#}",
                complete.display()
            );
        }
        Err(error) => {
            remove_owned_partial_backup(backup_root, &partial)?;
            return Err(error);
        }
    }
    let complete = tokio::fs::canonicalize(&complete).await?;
    anyhow::ensure!(
        complete == backup_root.join(&identifier),
        "completed backup directory is not exactly backup-root/backup-id"
    );

    refresh_backup_lock(&database, &backup_lock).await?;
    if let Err(error) = before_status_publication().and_then(|()| {
        backup_authority_source.revalidate("backup authority HMAC key at status publication")
    }) {
        remove_owned_complete_backup(
            backup_root,
            &complete,
            &identifier,
            &verified.tree,
            &backup_authority_key,
        )?;
        return Err(error.context("backup authority changed before status publication"));
    }
    let publication = match publish_status(status_path, &status_bytes) {
        Ok(outcome) => outcome,
        Err(publication_error) => {
            remove_owned_complete_backup(
                backup_root,
                &complete,
                &identifier,
                &verified.tree,
                &backup_authority_key,
            )
            .with_context(|| {
                format!(
                    "status publication failed before installation ({publication_error:#}); remove exact unreferenced backup {}",
                    complete.display()
                )
            })?;
            return Err(publication_error.context(
                "status publication failed before installation; exact unreferenced backup removed",
            ));
        }
    };
    match publication {
        StatusPublicationOutcome::Published => {}
        StatusPublicationOutcome::PublishedButParentSyncFailed(source) => {
            return Err(StatusPublicationDurabilityUncertain {
                path: status_path.to_owned(),
                envelope_sha256: hex::encode(Sha256::digest(&status_bytes)),
                source,
            }
            .into());
        }
        StatusPublicationOutcome::PublishedButIdentityUncertain(source) => {
            return Err(StatusPublicationIdentityUncertain {
                path: status_path.to_owned(),
                envelope_sha256: hex::encode(Sha256::digest(&status_bytes)),
                source,
            }
            .into());
        }
    }
    // Retention follows publication so the previously published backup is
    // never removed while the old status still names it. If pruning fails,
    // the command fails but the new authenticated status and backup remain a
    // truthful, complete readiness boundary.
    refresh_backup_lock(&database, &backup_lock).await?;
    backup_authority_source.revalidate("backup authority HMAC key before final retention")?;
    retain_complete_backups(
        backup_root,
        &identifier,
                retain_complete,
                retain_complete
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("retention generation bound overflows"))?,
                release_identity,
                &backup_authority_key,
    )
    .await?;
    backup_authority_source.revalidate("backup authority HMAC key at backup completion")?;
    Ok(complete)
        },
    )
    .await;
    let release = release_backup_gate_and_close_pool_under_exclusive_fence(
        &database,
        &backup_lock,
        &exclusive_fence,
    )
    .await;
    match (result, release) {
        (Ok(value), Ok(true)) => Ok(value),
        (Ok(_), Ok(false)) => anyhow::bail!("backup writer gate disappeared before release"),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(operation), Ok(true)) => Err(operation),
        (Err(operation), Ok(false)) => {
            Err(operation.context("backup failed and its writer gate disappeared before release"))
        }
        (Err(operation), Err(release)) => Err(operation.context(format!(
            "backup failed and releasing its writer gate also failed: {release}"
        ))),
    }
}

async fn wait_for_maintenance_writers(
    database: &Database,
    backup_lock: &str,
) -> anyhow::Result<()> {
    wait_for_maintenance_writers_with_timing(
        database,
        backup_lock,
        BACKUP_WRITER_DRAIN_TIMEOUT,
        Duration::from_millis(100),
    )
    .await
}

async fn wait_for_maintenance_writers_with_timing(
    database: &Database,
    backup_lock: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !timeout.is_zero() && !poll_interval.is_zero(),
        "maintenance-writer drain timing must be positive"
    );
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if database.active_maintenance_write_lease_count().await? == 0 {
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out draining state-mutating requests after closing backup admission"
        );
        anyhow::ensure!(
            database
                .refresh_backup_lock(backup_lock, BACKUP_LOCK_TTL)
                .await?,
            "backup writer gate expired while draining admitted requests"
        );
        tokio::time::sleep(poll_interval).await;
    }
}

/// Close a database that will not reach this command's EX phase. The shared
/// guard covers optional gate release, SQLx return/rollback, and awaited pool
/// close. No post-connect error path may rely on `SqlitePool` Drop.
async fn close_pre_exclusive_database_pool(
    database: &Database,
    backup_lock: Option<&str>,
) -> anyhow::Result<Option<bool>> {
    let shared = database.runtime_fence().acquire_one_off_shared().await?;
    let release = match backup_lock {
        Some(token) => Some(database.release_backup_lock(token).await),
        None => None,
    };
    let reconciliation = match (&release, backup_lock) {
        (Some(Err(_)), Some(token)) => Some(database.backup_lock_token_present(token).await),
        _ => None,
    };
    let idle_before_close = database.wait_for_idle().await;
    database.close_pool_under_fence().await;
    let shared_validation = shared.revalidate();
    let runtime_validation = database.runtime_fence().revalidate();
    drop(shared);

    let mut cleanup_error: Option<anyhow::Error> = None;
    for (label, result) in [
        ("draining the pre-exclusive SQLite pool", idle_before_close),
        (
            "revalidating the retained shared database fence",
            shared_validation,
        ),
        (
            "revalidating the runtime database fence",
            runtime_validation,
        ),
    ] {
        if let Err(error) = result {
            cleanup_error = Some(match cleanup_error {
                Some(previous) => previous.context(format!("{label} also failed: {error:#}")),
                None => error.context(label),
            });
        }
    }
    if let Some(error) = cleanup_error {
        return Err(match release {
            Some(Err(release)) => anyhow::Error::from(release)
                .context(format!("pre-exclusive pool cleanup also failed: {error:#}")),
            _ => error,
        });
    }
    match (release, reconciliation) {
        (None, None) => Ok(None),
        (Some(Ok(released)), None) => Ok(Some(released)),
        (Some(Err(release)), Some(Ok(false))) => Err(anyhow::Error::from(release).context(
            "pre-exclusive gate release reported an error, but exact-token reconciliation proves it absent",
        )),
        (Some(Err(release)), Some(Ok(true))) => Err(anyhow::Error::from(release).context(
            "pre-exclusive gate release reported an error and exact-token reconciliation proves it remains",
        )),
        (Some(Err(release)), Some(Err(reconciliation))) => Err(anyhow::Error::from(release)
            .context(format!(
                "pre-exclusive gate release and exact-token reconciliation both failed: {reconciliation}"
            ))),
        _ => anyhow::bail!("pre-exclusive cleanup produced an impossible release state"),
    }
}

async fn acquire_backup_write_authority(
    database: &Database,
) -> anyhow::Result<(String, ExclusiveBackupDatabaseFence)> {
    // Publish the durable gate under the ordinary shared database fence, then
    // release that shared generation before attempting exclusive admission.
    // New processes may briefly pass the turnstile, but must observe this gate
    // and leave without starting their real database operation.
    let mut gate_operation = match database.begin_fenced_operation().await {
        Ok(operation) => operation,
        Err(error) => {
            let original = anyhow::Error::from(error);
            return match close_pre_exclusive_database_pool(&database, None).await {
                Ok(_) => Err(original),
                Err(cleanup) => Err(original.context(format!(
                    "closing the pre-exclusive database pool also failed: {cleanup:#}"
                ))),
            };
        }
    };
    let backup_lock_result = database
        .acquire_backup_lock("robin-highscores-admin", BACKUP_LOCK_TTL)
        .await;
    let gate_finish = database.finish_fenced_operation(&mut gate_operation).await;
    let backup_lock = match (backup_lock_result, gate_finish) {
        (Ok(token), Ok(())) => token,
        (Ok(token), Err(error)) => {
            let original = anyhow::Error::from(error);
            return match close_pre_exclusive_database_pool(&database, Some(&token)).await {
                Ok(Some(true)) => Err(original),
                Ok(Some(false)) => Err(original.context(
                    "gate publication succeeded but its exact token disappeared during pre-exclusive cleanup",
                )),
                Ok(None) => Err(original.context(
                    "pre-exclusive cleanup omitted the published backup-gate token",
                )),
                Err(cleanup) => Err(original.context(format!(
                    "releasing the gate and closing the pre-exclusive pool also failed: {cleanup:#}"
                ))),
            };
        }
        (Err(error), Ok(())) => {
            let original = anyhow::Error::from(error);
            return match close_pre_exclusive_database_pool(&database, None).await {
                Ok(_) => Err(original),
                Err(cleanup) => Err(original.context(format!(
                    "closing the pre-exclusive database pool also failed: {cleanup:#}"
                ))),
            };
        }
        (Err(operation), Err(finish)) => {
            let original = anyhow::Error::from(operation)
                .context(format!("database fence drain also failed: {finish}"));
            return match close_pre_exclusive_database_pool(&database, None).await {
                Ok(_) => Err(original),
                Err(cleanup) => Err(original.context(format!(
                    "closing the pre-exclusive database pool also failed: {cleanup:#}"
                ))),
            };
        }
    };
    let exclusive_fence = match acquire_exclusive_backup_database_fence(&database, &backup_lock)
        .await
    {
        Ok(fence) => fence,
        Err(error) => {
            return match close_pre_exclusive_database_pool(
                    &database,
                    Some(&backup_lock),
                )
                .await
                {
                    Ok(Some(true)) => Err(error),
                    Ok(Some(false)) => Err(error
                        .context("exclusive database fence failed and backup gate disappeared")),
                    Ok(None) => Err(error.context(
                        "exclusive database fence failed and cleanup omitted its backup gate",
                    )),
                    Err(cleanup) => Err(error.context(format!(
                        "exclusive database fence failed and pre-exclusive cleanup also failed: {cleanup:#}"
                    ))),
                };
        }
    };
    Ok((backup_lock, exclusive_fence))
}

struct ExclusiveBackupDatabaseFence {
    runtime: RuntimeDatabaseFence,
    admission: ExclusiveAdmissionGuard,
    quiescence: ExclusiveQuiescenceGuard,
}

impl ExclusiveBackupDatabaseFence {
    fn revalidate(&self) -> anyhow::Result<()> {
        self.runtime
            .validate_exclusive_pair(&self.admission, &self.quiescence)
    }
}

async fn acquire_exclusive_backup_database_fence(
    database: &Database,
    backup_lock: &str,
) -> anyhow::Result<ExclusiveBackupDatabaseFence> {
    let runtime = database.runtime_fence().clone();
    let deadline = tokio::time::Instant::now() + BACKUP_WRITER_DRAIN_TIMEOUT;
    let admission = loop {
        if let Some(guard) = runtime.try_lock_exclusive_admission()? {
            break guard;
        }
        // Never block the gate heartbeat behind flock. A brief shared pass is
        // sufficient to refresh it while all existing operations remain
        // covered by the data fence.
        if let Some(shared) = runtime.try_acquire_one_off_shared()? {
            refresh_backup_lock(database, backup_lock).await?;
            database.wait_for_idle().await?;
            shared.revalidate()?;
            drop(shared);
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out acquiring the exclusive database-admission turnstile"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let quiescence = loop {
        admission.revalidate()?;
        if let Some(guard) = runtime.try_lock_exclusive_quiescence()? {
            break guard;
        }
        // Exclusive admission prevents new joins. Taking the data lock shared
        // while older readers drain keeps each heartbeat itself fenced.
        let shared = runtime
            .acquire_shared_quiescence_while_admission_exclusive(&admission)
            .await?;
        refresh_backup_lock(database, backup_lock).await?;
        database.wait_for_idle().await?;
        shared.revalidate()?;
        drop(shared);
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "timed out draining existing database operations"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    runtime.validate_exclusive_pair(&admission, &quiescence)?;
    refresh_backup_lock(database, backup_lock).await?;
    database.wait_for_idle().await?;
    let pair = ExclusiveBackupDatabaseFence {
        runtime,
        admission,
        quiescence,
    };
    pair.revalidate()?;
    Ok(pair)
}

async fn release_backup_gate_and_close_pool_under_exclusive_fence(
    database: &Database,
    backup_lock: &str,
    fence: &ExclusiveBackupDatabaseFence,
) -> anyhow::Result<bool> {
    close_pool_under_exclusive_fence_after_release(
        database,
        backup_lock,
        fence,
        database.release_backup_lock(backup_lock),
    )
    .await
}

async fn close_pool_under_exclusive_fence_after_release<F>(
    database: &Database,
    backup_lock: &str,
    fence: &ExclusiveBackupDatabaseFence,
    release: F,
) -> anyhow::Result<bool>
where
    F: std::future::Future<Output = Result<bool, robin_highscores::db::DbError>>,
{
    let initial_fence_validation = fence.revalidate();
    let release = if initial_fence_validation.is_ok() {
        Some(release.await)
    } else {
        None
    };
    // DELETE errors can be outcome-uncertain. Reconcile the exact token while
    // the EX pair is still retained and the pool is still usable, but never
    // let a reconciliation error skip the unconditional pool drain below.
    let reconciliation = if release.as_ref().is_some_and(Result::is_err) {
        Some(database.backup_lock_token_present(backup_lock).await)
    } else {
        None
    };
    let idle_before_close = database.wait_for_idle().await;
    database.close_pool_under_fence().await;
    let final_fence_validation = fence.revalidate();

    let mut cleanup_error: Option<anyhow::Error> = None;
    for (label, result) in [
        (
            "validating the retained exclusive database fence before gate release",
            initial_fence_validation,
        ),
        ("draining the backup SQLite pool", idle_before_close),
        (
            "revalidating the retained exclusive database fence",
            final_fence_validation,
        ),
    ] {
        if let Err(error) = result {
            cleanup_error = Some(match cleanup_error {
                Some(previous) => previous.context(format!("{label} also failed: {error:#}")),
                None => error.context(label),
            });
        }
    }
    if let Some(error) = cleanup_error {
        return match release {
            Some(Err(release)) => Err(anyhow::Error::from(release)
                .context(format!("exclusive cleanup also failed: {error:#}"))),
            _ => Err(error),
        };
    }
    match (release, reconciliation) {
        (Some(Ok(released)), None) => Ok(released),
        (Some(Err(release)), Some(Ok(false))) => Err(anyhow::Error::from(release)
            .context("backup gate deletion reported an error, but the exact token is absent after reconciliation; pool was closed under EX")),
        (Some(Err(release)), Some(Ok(true))) => Err(anyhow::Error::from(release)
            .context("backup gate deletion reported an error and the exact token remains present; pool was closed under EX")),
        (Some(Err(release)), Some(Err(reconciliation))) => Err(anyhow::Error::from(release).context(
            format!(
                "backup gate deletion was outcome-uncertain and exact-token reconciliation also failed: {reconciliation}"
            ),
        )),
        _ => anyhow::bail!("backup gate cleanup produced an impossible successful state"),
    }
}

fn acquire_backup_operation_lock(backup_root: &Path) -> anyhow::Result<std::fs::File> {
    let root = pin_directory_capability(backup_root)?;
    let name = Path::new(".backup-operation.lock");
    #[cfg(target_os = "linux")]
    let (file, created) = {
        use std::os::fd::AsFd as _;
        let create = rustix::fs::openat2(
            root.as_fd(),
            name,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            rustix::fs::ResolveFlags::BENEATH
                | rustix::fs::ResolveFlags::NO_SYMLINKS
                | rustix::fs::ResolveFlags::NO_MAGICLINKS
                | rustix::fs::ResolveFlags::NO_XDEV,
        );
        match create {
            Ok(descriptor) => (std::fs::File::from(descriptor), true),
            Err(rustix::io::Errno::EXIST) => {
                let descriptor = rustix::fs::openat2(
                    root.as_fd(),
                    name,
                    rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                    rustix::fs::ResolveFlags::BENEATH
                        | rustix::fs::ResolveFlags::NO_SYMLINKS
                        | rustix::fs::ResolveFlags::NO_MAGICLINKS
                        | rustix::fs::ResolveFlags::NO_XDEV,
                )?;
                (std::fs::File::from(descriptor), false)
            }
            Err(error) => return Err(error.into()),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let (file, created) = {
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        (root.open_with(name, &options)?.into_std(), false)
    };
    #[cfg(unix)]
    if created {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.sync_all()?;
        sync_cap_directory(&root)?;
    }
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file(),
        "backup operation lock is not a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let path_metadata = root.symlink_metadata(name)?;
        anyhow::ensure!(
            metadata.nlink() == 1
                && metadata.permissions().mode() & 0o777 == 0o600
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata_identity_std(&metadata) == metadata_identity(&path_metadata),
            "backup operation lock has the wrong owner, mode, link count, or path identity"
        );
    }
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|error| anyhow::anyhow!("another backup operation is active: {error}"))?;
    Ok(file)
}

fn recover_stale_partial_backups(backup_root: &Path) -> anyhow::Result<()> {
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

fn recover_interrupted_complete_cleanups(
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

fn cap_entry_exists(root: &cap_std::fs::Dir, relative: &Path) -> anyhow::Result<bool> {
    match root.symlink_metadata(relative) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn remove_pinned_regular_via_tombstone(
    parent: &cap_std::fs::Dir,
    source_name: &str,
    tombstone_name: &str,
    pinned: &std::fs::File,
    expected_identity: FileIdentity,
    expected_mode: u32,
    expected_bytes: &[u8],
    maximum_bytes: u64,
    label: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !cap_entry_exists(parent, Path::new(tombstone_name))?,
        "{label} deletion tombstone already exists"
    );
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsFd as _;
        rustix::fs::renameat_with(
            parent.as_fd(),
            Path::new(source_name),
            parent.as_fd(),
            Path::new(tombstone_name),
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    anyhow::bail!("pinned cleanup deletion requires Linux renameat2");
    sync_cap_directory(parent)?;
    let moved = open_cap_regular_nofollow(parent, Path::new(tombstone_name))?;
    validate_private_pinned_file(pinned, expected_mode, label)?;
    validate_private_pinned_file(&moved, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&moved.metadata()?) == expected_identity
            && metadata_identity_std(&pinned.metadata()?) == expected_identity
            && read_bounded_pinned_file(duplicate_pinned_file(&moved, false)?, maximum_bytes)?
                == expected_bytes,
        "{label} was substituted while moving to its deletion tombstone; the replacement was preserved"
    );
    unlink_pinned_regular(
        parent,
        tombstone_name,
        &moved,
        expected_identity,
        expected_mode,
        label,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned.metadata()?.nlink() == 0,
            "{label} original pinned inode remained linked after tombstone unlink"
        );
    }
    Ok(())
}

fn unlink_pinned_regular(
    parent: &cap_std::fs::Dir,
    name: &str,
    pinned: &std::fs::File,
    expected_identity: FileIdentity,
    expected_mode: u32,
    label: &str,
) -> anyhow::Result<()> {
    unlink_pinned_regular_with_hook(
        parent,
        name,
        pinned,
        expected_identity,
        expected_mode,
        label,
        || Ok(()),
    )
}

fn unlink_pinned_regular_with_hook<F>(
    parent: &cap_std::fs::Dir,
    name: &str,
    pinned: &std::fs::File,
    expected_identity: FileIdentity,
    expected_mode: u32,
    label: &str,
    before_unlink: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    validate_private_pinned_file(pinned, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&pinned.metadata()?) == expected_identity,
        "{label} pinned authority changed before removal"
    );
    before_unlink()?;
    let current = open_cap_regular_nofollow(parent, Path::new(name))?;
    validate_private_pinned_file(pinned, expected_mode, label)?;
    validate_private_pinned_file(&current, expected_mode, label)?;
    anyhow::ensure!(
        metadata_identity_std(&pinned.metadata()?) == expected_identity
            && metadata_identity_std(&current.metadata()?) == expected_identity,
        "{label} was substituted at its unlink boundary; the replacement was preserved"
    );
    parent.remove_file(name)?;
    sync_cap_directory(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            pinned.metadata()?.nlink() == 0 && current.metadata()?.nlink() == 0,
            "{label} pinned inode remained linked after unlink"
        );
    }
    anyhow::ensure!(
        !cap_entry_exists(parent, Path::new(name))?,
        "{label} pathname reappeared after unlink"
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

fn resume_authenticated_cleanup_with_hooks<F, G>(
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
            actual_tree
                .files
                .iter()
                .all(|path| allowed_files.contains(path))
                && actual_tree
                    .directories
                    .iter()
                    .all(|path| allowed_directories.contains(path)),
            "cleanup quarantine contains an unverified insertion"
        );
        tombstoned_paths.extend(
            actual_tree
                .files
                .iter()
                .chain(actual_tree.directories.iter())
                .filter(|path| {
                    path.contains("/.cleanup-unlink-v1-") || path.starts_with(".cleanup-unlink-v1-")
                })
                .cloned(),
        );
        for path in actual_tree.files.iter().filter(|path| {
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
            actual_tree.directories.is_empty()
                && actual_tree
                    .files
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
    for relative in actual_tree
        .files
        .iter()
        .filter(|relative| !authority_paths.contains(relative.as_str()))
    {
        remove_verified_cleanup_entry(
            &directory,
            relative,
            *actual_tree
                .file_identities
                .get(relative)
                .expect("tree identity"),
            false,
            tombstoned_paths.contains(relative),
        )?;
    }
    let mut directories = actual_tree.directories.iter().collect::<Vec<_>>();
    directories.sort_by_key(|path| std::cmp::Reverse(Path::new(path).components().count()));
    for relative in directories {
        remove_verified_cleanup_entry(
            &directory,
            relative,
            *actual_tree
                .directory_identities
                .get(relative)
                .expect("tree directory identity"),
            true,
            tombstoned_paths.contains(relative),
        )?;
    }
    let terminal_tree = backup_tree_paths_cap(&directory)?;
    anyhow::ensure!(
        terminal_tree.root_identity == actual_tree.root_identity
            && terminal_tree.directories.is_empty()
            && terminal_tree
                .files
                .iter()
                .all(|path| authority_paths.contains(path)),
        "cleanup terminal topology changed before authority removal"
    );
    for original_authority in ["backup-manifest.json", "backup-verification-envelope.json"] {
        let tombstone = cleanup_tombstone_relative(original_authority)?;
        let authority = if actual_tree.files.contains(original_authority) {
            Some(original_authority)
        } else if actual_tree.files.contains(&tombstone) {
            Some(tombstone.as_str())
        } else {
            None
        };
        if let Some(authority) = authority {
            remove_verified_cleanup_entry(
                &directory,
                authority,
                *actual_tree.file_identities.get(authority).ok_or_else(|| {
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
        metadata_identity(&current_directory.dir_metadata()?) == actual_tree.root_identity,
        "cleanup root was substituted before removal"
    );
    anyhow::ensure!(
        metadata_identity(&backup_root.symlink_metadata(active_cleanup_name)?)
            == actual_tree.root_identity,
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
        metadata_identity(&terminal_directory.dir_metadata()?) == actual_tree.root_identity,
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

fn remove_owned_complete_backup(
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
            && metadata_identity(&directory.dir_metadata()?) == verified_tree.root_identity,
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

fn cleanup_tombstone_relative(relative: &str) -> anyhow::Result<String> {
    let path = Path::new(relative);
    anyhow::ensure!(
        !relative.is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "cleanup tombstone source path is unsafe"
    );
    let tombstone = format!(
        ".cleanup-unlink-v1-{}",
        hex::encode(Sha256::digest(relative.as_bytes()))
    );
    Ok(path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || PathBuf::from(&tombstone),
            |parent| parent.join(&tombstone),
        )
        .to_string_lossy()
        .replace('\\', "/"))
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

fn rename_verified_entry_to_tombstone_with_hooks<F, G>(
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

fn expected_backup_cleanup_file_mode(relative: &str) -> anyhow::Result<u32> {
    Ok(
        if relative == "backup-verification-envelope.json"
            || relative == cleanup_tombstone_relative("backup-verification-envelope.json")?
        {
            0o400
        } else {
            0o600
        },
    )
}

fn publish_cleanup_journal(
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
        metadata_identity(&directory.dir_metadata()?) == verified_tree.root_identity,
        "quarantined backup root differs from the verified inode"
    );
    resume_authenticated_cleanup(backup_root, &journal, backup_authority_key)
}

fn cleanup_journal_for_verified_tree(
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
        backup_root_identity.device,
        backup_root_identity.inode,
        backup_root_identity.owner,
        verified_tree.root_identity.device,
        verified_tree.root_identity.inode,
        verified_tree.root_identity.owner,
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

fn valid_partial_backup_name(name: &str) -> bool {
    let Some(complete) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".partial"))
    else {
        return false;
    };
    parse_backup_id(complete).is_some()
}

fn validate_managed_directory_tree(
    root: &cap_std::fs::Dir,
    backup_root_metadata: &cap_std::fs::Metadata,
) -> anyhow::Result<()> {
    validate_managed_metadata(&root.dir_metadata()?, backup_root_metadata, true)?;
    let mut pending = vec![(root.try_clone()?, 0_usize)];
    let mut validated_directories = Vec::new();
    let mut visited = 0_usize;
    while let Some((directory, depth)) = pending.pop() {
        validated_directories.push(directory.try_clone()?);
        anyhow::ensure!(depth <= 32, "managed partial backup is too deep");
        for entry in directory.entries()? {
            let entry = entry?;
            visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("partial topology count overflows"))?;
            anyhow::ensure!(
                visited <= 1_000_000,
                "managed partial backup is too large to recover safely"
            );
            let name = entry.file_name();
            let metadata = directory.symlink_metadata(&name)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
                "managed partial backup contains a link or special node"
            );
            validate_managed_metadata(&metadata, backup_root_metadata, metadata.is_dir())
                .with_context(|| {
                    format!("validate managed backup node {}", name.to_string_lossy())
                })?;
            if metadata.is_dir() {
                let child = open_cap_directory_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity(&child.dir_metadata()?) == metadata_identity(&metadata),
                    "managed partial backup directory was substituted while opening"
                );
                pending.push((child, depth + 1));
            } else {
                let child = open_cap_regular_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity_std(&child.metadata()?) == metadata_identity(&metadata),
                    "managed partial backup file was substituted while opening"
                );
            }
        }
    }
    #[cfg(unix)]
    for directory in validated_directories {
        directory
            .into_std_file()
            .set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    owner: u32,
}

fn metadata_identity(metadata: &cap_std::fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as _;
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
        }
    }
    #[cfg(not(unix))]
    FileIdentity {
        device: 0,
        inode: 0,
        owner: 0,
    }
}

fn metadata_identity_std(metadata: &std::fs::Metadata) -> FileIdentity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
        }
    }
    #[cfg(not(unix))]
    FileIdentity {
        device: 0,
        inode: 0,
        owner: 0,
    }
}

fn validate_managed_metadata(
    metadata: &cap_std::fs::Metadata,
    backup_root_metadata: &cap_std::fs::Metadata,
    directory: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata_identity(metadata).owner == metadata_identity(backup_root_metadata).owner,
        "managed backup node has a foreign owner"
    );
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.dev() == backup_root_metadata.dev()
                && metadata.permissions().mode() & 0o077 == 0
                && (directory || metadata.nlink() == 1),
            "managed backup node changed filesystem, privacy mode, or link count"
        );
    }
    Ok(())
}

fn pin_directory_capability(path: &Path) -> anyhow::Result<cap_std::fs::Dir> {
    Ok(cap_std::fs::Dir::from_std_file(open_directory_nofollow(
        path,
    )?))
}

fn open_cap_directory_nofollow(
    parent: &cap_std::fs::Dir,
    name: &Path,
) -> anyhow::Result<cap_std::fs::Dir> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsFd as _;
        let descriptor = rustix::fs::openat2(
            parent.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH
                | rustix::fs::ResolveFlags::NO_SYMLINKS
                | rustix::fs::ResolveFlags::NO_MAGICLINKS
                | rustix::fs::ResolveFlags::NO_XDEV,
        )?;
        return Ok(cap_std::fs::Dir::from_std_file(std::fs::File::from(
            descriptor,
        )));
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::ensure!(
            !parent.symlink_metadata(name)?.file_type().is_symlink(),
            "managed directory is a symlink"
        );
        Ok(parent.open_dir(name)?)
    }
}

fn open_cap_regular_nofollow(
    parent: &cap_std::fs::Dir,
    name: &Path,
) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsFd as _;
        let descriptor = rustix::fs::openat2(
            parent.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::BENEATH
                | rustix::fs::ResolveFlags::NO_SYMLINKS
                | rustix::fs::ResolveFlags::NO_MAGICLINKS
                | rustix::fs::ResolveFlags::NO_XDEV,
        )?;
        let file = std::fs::File::from(descriptor);
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "managed node is not a regular file"
        );
        return Ok(file);
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::ensure!(
            !parent.symlink_metadata(name)?.file_type().is_symlink(),
            "managed file is a symlink"
        );
        let file = parent.open(name)?.into_std();
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "managed node is not a regular file"
        );
        Ok(file)
    }
}

fn sync_cap_directory(directory: &cap_std::fs::Dir) -> anyhow::Result<()> {
    directory.try_clone()?.into_std_file().sync_all()?;
    Ok(())
}

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

async fn estimate_backup_space(
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

fn round_up_to_allocation(bytes: u64, allocation: u64) -> anyhow::Result<u64> {
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

fn remove_owned_partial_backup(backup_root: &Path, partial: &Path) -> anyhow::Result<()> {
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

fn valid_complete_backup_name(name: &str) -> bool {
    parse_backup_id(name).is_some()
}

fn release_authority_file_name(
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<String> {
    release_identity.validate()?;
    Ok(format!(
        "{}.vps-release-manifest-v2.json",
        release_identity.vps_release_manifest_sha256
    ))
}

async fn preserve_release_authority(
    backup_root: &Path,
    release_manifest_path: &Path,
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        backup_root.is_absolute() && std::fs::canonicalize(backup_root)? == backup_root,
        "backup root is not its canonical real path"
    );
    let bytes = read_bounded_regular_nofollow(release_manifest_path, 64 * 1024 * 1024).await?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(&bytes)) == release_identity.vps_release_manifest_sha256,
        "release authority bytes differ from the active release identity"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_guard = root.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    let mut directory_builder = cap_std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt as _;
        directory_builder.mode(0o700);
    }
    match root.create_dir_with(RELEASE_AUTHORITY_STORE, &directory_builder) {
        Ok(()) => {
            let authority = open_cap_directory_nofollow(&root, Path::new(RELEASE_AUTHORITY_STORE))?;
            sync_cap_directory(&authority)?;
            sync_cap_directory(&root)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let authority = open_cap_directory_nofollow(&root, Path::new(RELEASE_AUTHORITY_STORE))?;
    let authority_guard = authority.try_clone()?.into_std_file();
    let authority_path = backup_root.join(RELEASE_AUTHORITY_STORE);
    revalidate_pinned_root_directory(&authority_guard, &authority_path, "release-authority store")?;
    let root_metadata = root.dir_metadata()?;
    let authority_metadata = authority.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            authority_metadata.uid() == rustix::process::geteuid().as_raw()
                && authority_metadata.dev() == root_metadata.dev()
                && authority_metadata.permissions().mode() & 0o777 == 0o700,
            "release-authority store has the wrong owner, device, or mode"
        );
    }
    let name = release_authority_file_name(release_identity)?;
    let partial_name = format!(".{name}.partial");
    let discarded_partial_name = format!("{partial_name}.discard");
    if cap_entry_exists(&authority, Path::new(&discarded_partial_name))? {
        let discarded = open_cap_regular_nofollow(&authority, Path::new(&discarded_partial_name))?;
        validate_private_pinned_file(&discarded, 0o400, "discarded release-authority partial")?;
        let discarded_identity = metadata_identity_std(&discarded.metadata()?);
        let discarded_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&discarded, false)?, 64 * 1024 * 1024)?;
        anyhow::ensure!(
            metadata_identity(&authority.symlink_metadata(&discarded_partial_name)?)
                == discarded_identity,
            "discarded release-authority partial was substituted before recovery"
        );
        unlink_pinned_regular(
            &authority,
            &discarded_partial_name,
            &discarded,
            discarded_identity,
            0o400,
            "discarded release-authority partial",
        )?;
        drop(discarded_bytes);
    }
    match authority.symlink_metadata(&partial_name) {
        Ok(metadata) => {
            validate_managed_metadata(&metadata, &root_metadata, false)?;
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt as _;
                anyhow::ensure!(
                    metadata.permissions().mode() & 0o777 == 0o400,
                    "release-authority partial has the wrong mode"
                );
            }
            let partial = open_cap_regular_nofollow(&authority, Path::new(&partial_name))?;
            validate_private_pinned_file(&partial, 0o400, "release-authority partial")?;
            let partial_bytes = read_bounded_pinned_file(
                duplicate_pinned_file(&partial, false)?,
                64 * 1024 * 1024,
            )?;
            if partial_bytes != bytes {
                let partial_identity = metadata_identity_std(&partial.metadata()?);
                remove_pinned_regular_via_tombstone(
                    &authority,
                    &partial_name,
                    &discarded_partial_name,
                    &partial,
                    partial_identity,
                    0o400,
                    &partial_bytes,
                    64 * 1024 * 1024,
                    "release-authority partial",
                )?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if !cap_entry_exists(&authority, Path::new(&partial_name))?
        && !cap_entry_exists(&authority, Path::new(&name))?
    {
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            options.mode(0o400);
        }
        let mut file = authority.open_with(&partial_name, &options)?.into_std();
        file.write_all(&bytes)?;
        file.sync_all()?;
        validate_private_pinned_file(&file, 0o400, "preserved release authority partial")?;
        sync_cap_directory(&authority)?;
    }
    if !cap_entry_exists(&authority, Path::new(&name))? {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                authority.as_fd(),
                Path::new(&partial_name),
                authority.as_fd(),
                Path::new(&name),
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        anyhow::bail!("release-authority NOREPLACE publication requires Linux renameat2");
        sync_cap_directory(&authority)?;
    } else if cap_entry_exists(&authority, Path::new(&partial_name))? {
        let partial = open_cap_regular_nofollow(&authority, Path::new(&partial_name))?;
        validate_private_pinned_file(&partial, 0o400, "release-authority partial")?;
        let partial_identity = metadata_identity_std(&partial.metadata()?);
        let partial_bytes =
            read_bounded_pinned_file(duplicate_pinned_file(&partial, false)?, 64 * 1024 * 1024)?;
        anyhow::ensure!(
            partial_bytes == bytes
                && metadata_identity(&authority.symlink_metadata(&partial_name)?)
                    == partial_identity,
            "release-authority partial changed before reconciliation"
        );
        remove_pinned_regular_via_tombstone(
            &authority,
            &partial_name,
            &discarded_partial_name,
            &partial,
            partial_identity,
            0o400,
            &partial_bytes,
            64 * 1024 * 1024,
            "release-authority partial",
        )?;
    }
    let file = open_cap_regular_nofollow(&authority, Path::new(&name))?;
    validate_private_pinned_file(&file, 0o400, "preserved release authority")?;
    #[cfg(unix)]
    {
        anyhow::ensure!(
            metadata_identity_std(&file.metadata()?).device
                == metadata_identity(&root_metadata).device,
            "preserved release authority crosses a filesystem boundary"
        );
    }
    let preserved =
        read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, 64 * 1024 * 1024)?;
    anyhow::ensure!(
        preserved == bytes,
        "preserved release authority differs from the exact active manifest bytes"
    );
    let final_identity = metadata_identity_std(&file.metadata()?);
    let loaded =
        load_backup_release_identity_preserved_file(duplicate_pinned_file(&file, false)?).await?;
    anyhow::ensure!(
        loaded == *release_identity,
        "preserved release authority has a different typed identity"
    );
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    revalidate_pinned_root_directory(&authority_guard, &authority_path, "release-authority store")?;
    let current = open_cap_regular_nofollow(&authority, Path::new(&name))?;
    validate_private_pinned_file(&current, 0o400, "preserved release authority")?;
    anyhow::ensure!(
        metadata_identity_std(&current.metadata()?) == final_identity
            && read_bounded_pinned_file(duplicate_pinned_file(&current, false)?, 64 * 1024 * 1024,)?
                == bytes,
        "preserved release authority was substituted before publication completed"
    );
    Ok(())
}

async fn load_preserved_release_authority(
    backup_root: &Path,
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<BackupReleaseIdentityV2> {
    anyhow::ensure!(
        backup_root.is_absolute() && std::fs::canonicalize(backup_root)? == backup_root,
        "backup root is not its canonical real path"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_guard = root.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    let root_metadata = root.dir_metadata()?;
    let (store_guard, file, target) =
        pin_preserved_release_authority_from_root(&root_guard, release_identity, backup_root)?;
    let file_identity = metadata_identity_std(&file.metadata()?);
    let authority_bytes =
        read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, 64 * 1024 * 1024)?;
    let loaded =
        load_backup_release_identity_preserved_file(duplicate_pinned_file(&file, false)?).await?;
    anyhow::ensure!(
        loaded == *release_identity,
        "preserved release authority differs from the backup identity"
    );
    revalidate_pinned_root_directory(&root_guard, backup_root, "backup root")?;
    revalidate_pinned_root_directory(
        &store_guard,
        &backup_root.join(RELEASE_AUTHORITY_STORE),
        "release-authority store",
    )?;
    revalidate_pinned_regular_path(
        &file,
        &target,
        0o400,
        Some(0o700),
        "preserved release authority",
    )?;
    anyhow::ensure!(
        file_identity.device == metadata_identity(&root_metadata).device
            && metadata_identity_std(&file.metadata()?) == file_identity
            && read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, 64 * 1024 * 1024,)?
                == authority_bytes,
        "preserved release authority changed during verification"
    );
    Ok(loaded)
}

async fn retain_complete_backups(
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
            .checked_add(verified.tree.files.len())
            .and_then(|value| value.checked_add(verified.tree.directories.len()))
            .ok_or_else(|| anyhow::anyhow!("retention topology count overflows"))?;
        anyhow::ensure!(
            verified_topology_entries <= maximum_verified_topology_entries,
            "aggregate retained-backup topology exceeds its verification bound"
        );
        let preserved =
            load_preserved_release_authority(backup_root, &verified.release_identity).await?;
        anyhow::ensure!(
            preserved == verified.release_identity,
            "retained backup differs from its independent preserved release authority"
        );
        if verified.release_identity == *active_release_identity {
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
        complete.push((name, identity, verified.tree));
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

fn publish_private_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<StatusPublicationOutcome> {
    publish_private_atomic_with(path, bytes, sync_cap_directory)
}

fn publish_private_atomic_with<F>(
    path: &Path,
    bytes: &[u8],
    sync_parent: F,
) -> anyhow::Result<StatusPublicationOutcome>
where
    F: FnOnce(&cap_std::fs::Dir) -> anyhow::Result<()>,
{
    publish_private_atomic_with_hooks(path, bytes, sync_parent, || Ok(()))
}

fn publish_private_atomic_with_hooks<F, G>(
    path: &Path,
    bytes: &[u8],
    sync_parent: F,
    before_rename: G,
) -> anyhow::Result<StatusPublicationOutcome>
where
    F: FnOnce(&cap_std::fs::Dir) -> anyhow::Result<()>,
    G: FnOnce() -> anyhow::Result<()>,
{
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup status path has no parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("private publication path has no filename"))?;
    let directory = pin_directory_capability(parent)?;
    let parent_metadata = directory.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            parent_metadata.uid() == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o777 == 0o700,
            "backup status parent has the wrong owner or mode"
        );
    }
    let parent_identity = metadata_identity(&parent_metadata);
    let temporary = format!(
        ".{}-{}.tmp",
        name.to_string_lossy(),
        uuid::Uuid::now_v7().simple()
    );
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = directory.open_with(&temporary, &options)?.into_std();
    let pre_rename = (|| -> anyhow::Result<()> {
        #[cfg(unix)]
        file.set_permissions(std::fs::Permissions::from_mode(0o400))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        let metadata = file.metadata()?;
        anyhow::ensure!(metadata.is_file(), "backup status temporary is not regular");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            anyhow::ensure!(
                metadata.nlink() == 1 && metadata.permissions().mode() & 0o777 == 0o400,
                "backup status temporary has the wrong mode or link count"
            );
        }
        drop(file);
        before_rename()?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                directory.as_fd(),
                Path::new(&temporary),
                directory.as_fd(),
                Path::new(name),
                rustix::fs::RenameFlags::empty(),
            )?;
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        directory.rename(&temporary, &directory, Path::new(name))?;
        Ok(())
    })();
    if let Err(error) = pre_rename {
        match directory.symlink_metadata(&temporary) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                directory.remove_file(&temporary)?;
                sync_cap_directory(&directory)?;
            }
            Ok(_) => anyhow::bail!("backup status temporary path changed type"),
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {}
            Err(cleanup) => return Err(cleanup.into()),
        }
        return Err(error);
    }
    if let Err(error) = sync_parent(&directory) {
        return Ok(StatusPublicationOutcome::PublishedButParentSyncFailed(
            error,
        ));
    }
    let reopened = match pin_directory_capability(parent) {
        Ok(reopened) => reopened,
        Err(error) => {
            return Ok(StatusPublicationOutcome::PublishedButIdentityUncertain(
                error,
            ));
        }
    };
    let final_check = (|| -> anyhow::Result<()> {
        let reopened_metadata = reopened.dir_metadata()?;
        anyhow::ensure!(
            metadata_identity(&reopened_metadata) == parent_identity,
            "backup status parent was replaced during publication"
        );
        #[cfg(unix)]
        {
            use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
            anyhow::ensure!(
                reopened_metadata.uid() == rustix::process::geteuid().as_raw()
                    && reopened_metadata.permissions().mode() & 0o777 == 0o700,
                "backup status parent owner or mode changed during publication"
            );
        }
        let mut published = open_cap_regular_nofollow(&reopened, Path::new(name))?;
        let metadata = published.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            anyhow::ensure!(
                metadata.nlink() == 1
                    && metadata.permissions().mode() & 0o777 == 0o400
                    && metadata.uid() == rustix::process::geteuid().as_raw(),
                "published backup status has the wrong owner, mode, or link count"
            );
        }
        let mut digest = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = published.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(u64::try_from(read)?)
                .ok_or_else(|| anyhow::anyhow!("published status length overflows"))?;
            digest.update(&buffer[..read]);
        }
        anyhow::ensure!(
            length == u64::try_from(bytes.len())?
                && hex::encode(digest.finalize()) == hex::encode(Sha256::digest(bytes)),
            "published backup status differs from the authenticated envelope"
        );
        Ok(())
    })();
    Ok(match final_check {
        Ok(()) => StatusPublicationOutcome::Published,
        Err(error) => StatusPublicationOutcome::PublishedButIdentityUncertain(error),
    })
}

#[cfg(test)]
async fn backup(
    config: &ServerConfig,
    campaign_root: &Path,
    max_campaign_bytes: u64,
    destination: &Path,
    created_at_unix_ms: u64,
    release_identity: &BackupReleaseIdentityV2,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    let config = config.clone();
    let campaign_root = campaign_root.to_owned();
    let destination = destination.to_owned();
    let release_identity = release_identity.clone();
    let restore_sources = restore_sources.clone();
    run_owned_backup(async move {
        backup_test_owned(
            &config,
            &campaign_root,
            max_campaign_bytes,
            &destination,
            created_at_unix_ms,
            &release_identity,
            &restore_sources,
        )
        .await
    })
    .await
}

#[cfg(test)]
async fn backup_test_owned(
    config: &ServerConfig,
    campaign_root: &Path,
    max_campaign_bytes: u64,
    destination: &Path,
    created_at_unix_ms: u64,
    release_identity: &BackupReleaseIdentityV2,
    restore_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        destination.is_absolute(),
        "backup destination must be absolute"
    );
    anyhow::ensure!(
        !tokio::fs::try_exists(destination).await?,
        "backup destination already exists"
    );
    let _operation_lock = acquire_backup_operation_lock(
        destination
            .parent()
            .ok_or_else(|| anyhow::anyhow!("test backup destination has no parent"))?,
    )?;
    let database = Database::connect(config).await?;
    let (lock, exclusive_fence) = acquire_backup_write_authority(&database).await?;
    let result = run_with_backup_lock_heartbeat(&database, &lock, async {
        wait_for_maintenance_writers(&database, &lock).await?;
        create_backup_directory(destination).await?;
        set_private_directory(destination).await?;
        let replay =
            ReplayStore::create(config.replay_directory.clone(), config.max_replay_bytes).await?;
        anyhow::ensure!(
            max_campaign_bytes > 0,
            "max_campaign_bytes must be positive"
        );
        let campaign = CampaignStore::create(campaign_root.to_owned(), max_campaign_bytes).await?;
        database.health_check().await?;
        replay.readiness_check().await?;
        campaign.readiness_check().await?;
        backup_locked(
            config,
            &database,
            &replay,
            &campaign,
            &lock,
            destination,
            created_at_unix_ms,
            release_identity,
            restore_sources,
        )
        .await?;
        let manifest_bytes = read_bounded_regular_nofollow(
            &destination.join("backup-manifest.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        )
        .await?;
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
        let backup_id = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("test backup destination has no UTF-8 identifier"))?;
        let envelope = BackupVerificationEnvelopeV2::new_authenticated(
            backup_id.to_owned(),
            &manifest,
            &[0x31; 32],
        )?;
        let envelope_path = destination.join("backup-verification-envelope.json");
        write_private_file(&envelope_path, &canonical_json_bytes(&envelope)?).await?;
        #[cfg(unix)]
        set_backup_permissions(&envelope_path, std::fs::Permissions::from_mode(0o400)).await?;
        sync_directory(destination).await
    })
    .await;
    let release = release_backup_gate_and_close_pool_under_exclusive_fence(
        &database,
        &lock,
        &exclusive_fence,
    )
    .await;
    match (result, release) {
        (Ok(value), Ok(true)) => Ok(value),
        (Ok(_), Ok(false)) => anyhow::bail!("backup lock was lost before release"),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(operation), Ok(true)) => Err(operation),
        (Err(operation), Ok(false)) => {
            Err(operation.context("backup failed and its writer gate disappeared"))
        }
        (Err(operation), Err(release)) => Err(operation.context(format!(
            "backup failed and releasing its writer gate also failed: {release}"
        ))),
    }
}

async fn backup_locked(
    config: &ServerConfig,
    database: &Database,
    replay: &ReplayStore,
    campaign: &CampaignStore,
    backup_lock: &str,
    destination: &Path,
    created_at_unix_ms: u64,
    release_identity: &BackupReleaseIdentityV2,
    readable_sources: &BTreeMap<PathBuf, PathBuf>,
) -> anyhow::Result<()> {
    refresh_backup_lock(database, backup_lock).await?;
    release_identity.validate()?;
    // Pin the entire restore-source closure before the first payload byte is
    // copied. Every later copy uses these exact descriptors, then rebinds the
    // source path and bytes; validation is never followed by a pathname reopen.
    let pinned_sources = pin_backup_restore_sources(config, readable_sources, release_identity)?;
    let database_path = destination.join("highscores.sqlite3");
    database.online_backup_to(&database_path).await?;
    scrub_transient_backup_state(&database_path, created_at_unix_ms).await?;
    #[cfg(unix)]
    set_backup_permissions(&database_path, std::fs::Permissions::from_mode(0o600)).await?;
    let mut files = vec![record_file(destination, &database_path).await?];
    let mut restore_sources = vec![RestoreSource {
        original_absolute_path: config.database_path.to_string_lossy().into_owned(),
        archive_relative_path: "highscores.sqlite3".to_owned(),
    }];
    let replay_destination = destination.join("replays");
    create_backup_directory(&replay_destination).await?;
    set_private_directory(&replay_destination).await?;
    let mut cursor = None;
    loop {
        let page = replay.inventory_page(cursor, 10_000).await?;
        if page.is_empty() {
            break;
        }
        refresh_backup_lock(database, backup_lock).await?;
        for entry in &page {
            let source = replay.open_verified(&entry.sha256, entry.bytes).await?;
            let relative = replay
                .path_for_digest(&entry.sha256)
                .strip_prefix(&config.replay_directory)?
                .to_owned();
            let target = replay_destination.join(relative);
            copy_open_file(source, &target).await?;
            files.push(record_file(destination, &target).await?);
        }
        cursor = page.last().map(|entry| entry.sha256);
        if page.len() < 10_000 {
            break;
        }
    }
    let campaign_destination = destination.join("campaigns");
    create_backup_directory(&campaign_destination).await?;
    set_private_directory(&campaign_destination).await?;
    for (index, entry) in campaign.inventory().await?.into_iter().enumerate() {
        if index % 1_000 == 0 {
            refresh_backup_lock(database, backup_lock).await?;
        }
        let relative = PathBuf::from(&hex::encode(entry.sha256)[..2])
            .join(format!("{}.campaign", hex::encode(entry.sha256)));
        let target = campaign_destination.join(relative);
        let source = campaign.open_verified(&entry.sha256).await?;
        copy_open_file(source, &target).await?;
        files.push(record_file(destination, &target).await?);
    }
    refresh_backup_lock(database, backup_lock).await?;
    restore_sources.push(RestoreSource {
        original_absolute_path: config.replay_directory.to_string_lossy().into_owned(),
        archive_relative_path: "replays".to_owned(),
    });
    restore_sources.push(RestoreSource {
        original_absolute_path: config
            .campaign_state_directory
            .to_string_lossy()
            .into_owned(),
        archive_relative_path: "campaigns".to_owned(),
    });

    let cursor_target = destination.join("restore/state/cursor-hmac.key");
    copy_pinned_restore_source(
        pinned_sources
            .get(&config.cursor_secret_path)
            .ok_or_else(|| anyhow::anyhow!("pinned cursor secret is missing"))?,
        &cursor_target,
    )
    .await?;
    files.push(record_file(destination, &cursor_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: config.cursor_secret_path.to_string_lossy().into_owned(),
        archive_relative_path: "restore/state/cursor-hmac.key".to_owned(),
    });
    let grant_target = destination.join("restore/state/competition-run-grant.key");
    copy_pinned_restore_source(
        pinned_sources
            .get(&config.competition_run_grant_secret_path)
            .ok_or_else(|| anyhow::anyhow!("pinned competition-grant secret is missing"))?,
        &grant_target,
    )
    .await?;
    files.push(record_file(destination, &grant_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: config
            .competition_run_grant_secret_path
            .to_string_lossy()
            .into_owned(),
        archive_relative_path: "restore/state/competition-run-grant.key".to_owned(),
    });
    let preflight_target = destination.join("restore/state/run-preflight-grant.key");
    copy_pinned_restore_source(
        pinned_sources
            .get(&config.run_preflight_grant_secret_path)
            .ok_or_else(|| anyhow::anyhow!("pinned run-preflight secret is missing"))?,
        &preflight_target,
    )
    .await?;
    files.push(record_file(destination, &preflight_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: config
            .run_preflight_grant_secret_path
            .to_string_lossy()
            .into_owned(),
        archive_relative_path: "restore/state/run-preflight-grant.key".to_owned(),
    });

    let moderation_original = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup requires the moderation bearer secret"))?;
    let moderation_target = destination.join("restore/state/moderation-bearer.token");
    copy_pinned_restore_source(
        pinned_sources
            .get(moderation_original)
            .ok_or_else(|| anyhow::anyhow!("pinned moderation secret is missing"))?,
        &moderation_target,
    )
    .await?;
    files.push(record_file(destination, &moderation_target).await?);
    restore_sources.push(RestoreSource {
        original_absolute_path: moderation_original.to_string_lossy().into_owned(),
        archive_relative_path: "restore/state/moderation-bearer.token".to_owned(),
    });

    for unit in SYSTEMD_UNIT_FILES {
        let original = Path::new(SYSTEMD_USER_ROOT).join(unit);
        let target = destination.join("restore/systemd/user").join(unit);
        copy_pinned_restore_source(
            pinned_sources
                .get(&original)
                .ok_or_else(|| anyhow::anyhow!("pinned installed unit is missing: {unit}"))?,
            &target,
        )
        .await?;
        files.push(record_file(destination, &target).await?);
        restore_sources.push(RestoreSource {
            original_absolute_path: original.to_string_lossy().into_owned(),
            archive_relative_path: format!("restore/systemd/user/{unit}"),
        });
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    anyhow::ensure!(
        files
            .windows(2)
            .all(|pair| pair[0].relative_path != pair[1].relative_path),
        "backup contains duplicate archive paths"
    );
    for source in pinned_sources.values() {
        source.revalidate("backup restore source at manifest boundary")?;
    }
    restore_sources
        .sort_by(|left, right| left.archive_relative_path.cmp(&right.archive_relative_path));
    let manifest = BackupManifest {
        schema_version: BACKUP_MANIFEST_SCHEMA_VERSION,
        created_at_unix_ms,
        database_schema_version: robin_highscores::db::CURRENT_SCHEMA_VERSION,
        release_identity: release_identity.clone(),
        root_unix_mode: 0o700,
        restore_sources,
        directories: canonical_backup_directories_v4(&files)?,
        files,
    };
    let manifest_path = destination.join("backup-manifest.json");
    manifest.validate()?;
    let bytes = canonical_json_bytes(&manifest)?;
    anyhow::ensure!(
        bytes.len() <= robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES,
        "canonical backup manifest exceeds the readiness envelope limit"
    );
    write_private_file(&manifest_path, &bytes).await?;
    sync_directory(destination).await?;
    Ok(())
}

async fn scrub_transient_backup_state(
    database_path: &Path,
    snapshot_at_unix_ms: u64,
) -> anyhow::Result<()> {
    scrub_transient_backup_state_with_hook(database_path, snapshot_at_unix_ms, || Ok(())).await
}

async fn scrub_transient_backup_state_with_hook<F>(
    database_path: &Path,
    snapshot_at_unix_ms: u64,
    after_updates: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    use futures_util::FutureExt as _;

    let snapshot_at_unix_ms = i64::try_from(snapshot_at_unix_ms)?;
    let options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Delete)
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    // This destination connection has its own SQLx worker, not the live pool.
    // Rollback and close must finish before partial cleanup or EX release, even
    // when a query fails or the body unwinds. The detached backup owner prevents
    // caller cancellation from dropping this closing future.
    let scrub = std::panic::AssertUnwindSafe(async {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut connection)
            .await?;
        anyhow::ensure!(
            journal_mode.eq_ignore_ascii_case("delete"),
            "backup snapshot did not enter DELETE journal mode before transient-state scrub"
        );
        let mut transaction = connection.begin().await?;
        sqlx::query(
            "UPDATE submissions \
         SET status = 'retry_pending', lease_owner = NULL, lease_expires_at_ms = NULL, \
             next_attempt_at_ms = MIN(next_attempt_at_ms, ?), \
             updated_at_ms = MAX(updated_at_ms, ?) \
         WHERE status = 'verifying'",
        )
        .bind(snapshot_at_unix_ms)
        .bind(snapshot_at_unix_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE submission_upload_reservations \
         SET state = 'abandoned', lease_token = NULL, lease_expires_at_ms = NULL, \
             abandoned_at_ms = MAX(updated_at_ms, reserved_at_ms, ?), \
             updated_at_ms = MAX(updated_at_ms, reserved_at_ms, ?) \
         WHERE state IN ('reserved', 'uploaded')",
        )
        .bind(snapshot_at_unix_ms)
        .bind(snapshot_at_unix_ms)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM maintenance_write_leases")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM maintenance_locks")
            .execute(&mut *transaction)
            .await?;
        after_updates()?;
        transaction.commit().await?;
        Ok::<_, anyhow::Error>(())
    })
    .catch_unwind()
    .await
    .unwrap_or_else(|_| Err(anyhow::anyhow!("backup transient-state scrub panicked")));
    let close = connection.close().await;
    match (scrub, close) {
        (Ok(()), Ok(())) => {}
        (Ok(()), Err(error)) => return Err(error.into()),
        (Err(error), Ok(())) => return Err(error),
        (Err(error), Err(close)) => {
            return Err(error.context(format!(
                "closing the backup destination SQLite worker also failed: {close}"
            )));
        }
    }
    for suffix in ["-wal", "-shm"] {
        anyhow::ensure!(
            !tokio::fs::try_exists(PathBuf::from(format!(
                "{}{}",
                database_path.display(),
                suffix
            )))
            .await?,
            "transient-state scrub left an unmanifested SQLite sidecar"
        );
    }
    std::fs::File::open(database_path)?.sync_all()?;
    Ok(())
}

fn validate_installed_unit_source(
    path: &Path,
    authority: &robin_highscores::backup::BackupReleaseUnitV2,
) -> anyhow::Result<()> {
    pin_restore_source(
        path,
        authority.unix_mode,
        Some(authority.artifact.byte_length),
        Some(authority.artifact.sha256.to_string().as_str()),
        "installed user unit",
    )?;
    Ok(())
}

fn validate_secret_source(path: &Path, exact_length: Option<u64>) -> anyhow::Result<()> {
    pin_restore_source(path, 0o400, exact_length, None, "backup secret source")?;
    Ok(())
}

#[derive(Debug)]
struct PinnedRestoreSource {
    path: PathBuf,
    file: std::fs::File,
    identity: FileIdentity,
    expected_mode: u32,
    bytes: Vec<u8>,
    sha256: String,
}

impl PinnedRestoreSource {
    fn revalidate(&self, label: &str) -> anyhow::Result<()> {
        revalidate_pinned_regular_path(&self.file, &self.path, self.expected_mode, None, label)?;
        anyhow::ensure!(
            metadata_identity_std(&self.file.metadata()?) == self.identity
                && read_pinned_source_bytes(&self.file, u64::try_from(self.bytes.len())?)?
                    == self.bytes,
            "{label} changed after it was pinned"
        );
        Ok(())
    }
}

fn read_pinned_source_bytes(file: &std::fs::File, maximum: u64) -> anyhow::Result<Vec<u8>> {
    use std::io::{Seek as _, SeekFrom};

    let mut duplicate = duplicate_pinned_file(file, false)?;
    duplicate.seek(SeekFrom::Start(0))?;
    read_bounded_pinned_file(duplicate, maximum)
}

fn pin_restore_source(
    path: &Path,
    expected_mode: u32,
    exact_length: Option<u64>,
    expected_sha256: Option<&str>,
    label: &str,
) -> anyhow::Result<PinnedRestoreSource> {
    anyhow::ensure!(path.is_absolute(), "{label} path must be absolute");
    anyhow::ensure!(
        std::fs::canonicalize(path)? == path,
        "{label} path must not traverse aliases or symlinks"
    );
    let file = open_regular_nofollow(path)?;
    validate_private_pinned_file(&file, expected_mode, label)?;
    let metadata = file.metadata()?;
    if let Some(exact_length) = exact_length {
        anyhow::ensure!(
            metadata.len() == exact_length,
            "{label} has the wrong length"
        );
    } else {
        anyhow::ensure!(
            (1..=64 * 1024).contains(&metadata.len()),
            "{label} is empty or oversized"
        );
    }
    let bytes = read_pinned_source_bytes(&file, metadata.len())?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? == metadata.len(),
        "{label} changed length while it was admitted"
    );
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if let Some(expected_sha256) = expected_sha256 {
        anyhow::ensure!(
            sha256 == expected_sha256,
            "{label} differs from its authenticated release artifact"
        );
    }
    let source = PinnedRestoreSource {
        path: path.to_owned(),
        identity: metadata_identity_std(&metadata),
        expected_mode,
        file,
        bytes,
        sha256,
    };
    source.revalidate(label)?;
    Ok(source)
}

fn pin_backup_restore_sources(
    config: &ServerConfig,
    readable_sources: &BTreeMap<PathBuf, PathBuf>,
    release_identity: &BackupReleaseIdentityV2,
) -> anyhow::Result<BTreeMap<PathBuf, PinnedRestoreSource>> {
    validate_backup_restore_source_contract(config, readable_sources)?;
    let mut pinned = BTreeMap::new();
    for (original, exact_length) in [
        (&config.cursor_secret_path, Some(32_u64)),
        (&config.competition_run_grant_secret_path, Some(32_u64)),
        (&config.run_preflight_grant_secret_path, Some(32_u64)),
    ] {
        let readable = readable_sources
            .get(original)
            .map(PathBuf::as_path)
            .unwrap_or(original);
        pinned.insert(
            original.clone(),
            pin_restore_source(readable, 0o400, exact_length, None, "backup secret source")?,
        );
    }
    let moderation = config
        .moderation_bearer_token_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("backup requires the moderation bearer secret"))?;
    let moderation_readable = readable_sources
        .get(moderation)
        .map(PathBuf::as_path)
        .unwrap_or(moderation);
    pinned.insert(
        moderation.clone(),
        pin_restore_source(
            moderation_readable,
            0o400,
            None,
            None,
            "backup moderation secret",
        )?,
    );
    for unit in SYSTEMD_UNIT_FILES {
        let original = Path::new(SYSTEMD_USER_ROOT).join(unit);
        let readable = readable_sources
            .get(&original)
            .cloned()
            .unwrap_or_else(|| original.clone());
        let authority = release_identity
            .installed_user_units
            .iter()
            .find(|authority| authority.release_relative_path == format!("systemd/user/{unit}"))
            .ok_or_else(|| {
                anyhow::anyhow!("active release omits user-unit authority for {unit}")
            })?;
        let expected_sha256 = authority.artifact.sha256.to_string();
        pinned.insert(
            original,
            pin_restore_source(
                &readable,
                authority.unix_mode,
                Some(authority.artifact.byte_length),
                Some(&expected_sha256),
                "installed user unit",
            )?,
        );
    }
    Ok(pinned)
}

async fn refresh_backup_lock(database: &Database, token: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        database.refresh_backup_lock(token, BACKUP_LOCK_TTL).await?,
        "backup lock expired or was lost"
    );
    Ok(())
}

async fn run_with_backup_lock_heartbeat<T, F>(
    database: &Database,
    token: &str,
    operation: F,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    run_with_backup_lock_heartbeat_using(operation, BACKUP_LOCK_TTL / 3, || {
        refresh_backup_lock(database, token)
    })
    .await
}

async fn run_with_backup_lock_heartbeat_using<T, F, R, RF>(
    operation: F,
    refresh_every: Duration,
    mut refresh: R,
) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
    R: FnMut() -> RF,
    RF: std::future::Future<Output = anyhow::Result<()>>,
{
    use futures_util::FutureExt as _;

    let operation = robin_highscores::physical_work::drain(operation);
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result,
            () = tokio::time::sleep(refresh_every) => {
                let refresh_result = std::panic::AssertUnwindSafe(async { refresh().await })
                    .catch_unwind().await;
                let original = match refresh_result {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => error,
                    Err(_) => anyhow::anyhow!("backup lock refresh panicked"),
                };
                // Keep the physical owner and outer EX/operation.lock alive.
                // Continuation still checks the exact token immediately before
                // authenticated installation and again before status publication.
                return Err(match (&mut operation).await {
                    Ok(_) => original,
                    Err(error) => original.context(format!(
                        "backup operation also failed while draining: {error:#}"
                    )),
                });
            }
        }
    }
}

async fn verify_backup_authenticated(
    directory: &Path,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<VerifiedBackup> {
    verify_backup_with_schema_policy(directory, true, backup_authority_key).await
}

#[cfg(test)]
async fn verify_backup(directory: &Path) -> anyhow::Result<VerifiedBackup> {
    verify_backup_authenticated(directory, &[0x31; 32]).await
}

async fn verify_backup_with_schema_policy(
    directory: &Path,
    require_current_schema: bool,
    backup_authority_key: &[u8; 32],
) -> anyhow::Result<VerifiedBackup> {
    let directory_name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("backup directory has no UTF-8 identifier"))?;
    let backup_id = if parse_backup_id(directory_name).is_some() {
        directory_name
    } else {
        directory_name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".partial"))
            .filter(|name| parse_backup_id(name).is_some())
            .ok_or_else(|| anyhow::anyhow!("backup directory has a noncanonical identifier"))?
    };
    anyhow::ensure!(
        parse_backup_id(backup_id).is_some(),
        "backup directory has a noncanonical identifier"
    );
    let parent_path = directory
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup directory has no parent"))?;
    anyhow::ensure!(
        std::fs::canonicalize(parent_path)? == parent_path,
        "backup parent is not its canonical real path"
    );
    let parent = pin_directory_capability(parent_path)?;
    let parent_guard = parent.try_clone()?.into_std_file();
    revalidate_pinned_root_directory(&parent_guard, parent_path, "backup parent")?;
    let parent_metadata = parent.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            parent_metadata.uid() == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o777 == 0o700,
            "backup parent has the wrong owner or mode"
        );
    }
    let name = directory
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("backup directory has no filename"))?;
    let pinned = open_cap_directory_nofollow(&parent, Path::new(name))?;
    let pinned_metadata = pinned.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            pinned_metadata.uid() == rustix::process::geteuid().as_raw()
                && pinned_metadata.dev() == parent_metadata.dev()
                && pinned_metadata.permissions().mode() & 0o777 == 0o700,
            "backup directory has the wrong owner, device, or mode"
        );
    }
    let admitted_identity = metadata_identity(&pinned_metadata);
    let verified = verify_backup_capability(
        &pinned,
        backup_id,
        backup_authority_key,
        require_current_schema,
    )
    .await?;
    revalidate_pinned_root_directory(&parent_guard, parent_path, "backup parent")?;
    let current = open_cap_directory_nofollow(&parent, Path::new(name))?;
    anyhow::ensure!(
        metadata_identity(&current.dir_metadata()?) == admitted_identity,
        "backup directory was substituted during verification"
    );
    Ok(verified)
}

async fn verify_backup_capability(
    root: &cap_std::fs::Dir,
    backup_id: &str,
    backup_authority_key: &[u8; 32],
    require_current_schema: bool,
) -> anyhow::Result<VerifiedBackup> {
    verify_backup_capability_with_compiled_schema(
        root,
        backup_id,
        backup_authority_key,
        robin_highscores::db::CURRENT_SCHEMA_VERSION,
        require_current_schema,
    )
    .await
}

async fn verify_backup_capability_with_compiled_schema(
    root: &cap_std::fs::Dir,
    backup_id: &str,
    backup_authority_key: &[u8; 32],
    compiled_schema_version: i64,
    require_current_schema: bool,
) -> anyhow::Result<VerifiedBackup> {
    anyhow::ensure!(
        compiled_schema_version >= 2,
        "compiled backup schema policy predates canonical V2 release authority"
    );
    let root_metadata = root.dir_metadata()?;
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.permissions().mode() & 0o777 == 0o700,
            "pinned backup root has the wrong owner or mode"
        );
    }
    let (manifest_bytes, manifest_identity) = read_cap_regular_bounded_with_mode_and_identity(
        root,
        Path::new("backup-manifest.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
        0o600,
    )?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&manifest)? == manifest_bytes,
        "pinned backup manifest is not canonical JSON"
    );
    manifest.validate()?;
    anyhow::ensure!(
        parse_backup_id(backup_id) == Some(manifest.created_at_unix_ms),
        "pinned backup directory identifier differs from its manifest timestamp"
    );
    let now = u64::try_from(robin_highscores::model::now_epoch_ms()?)?;
    anyhow::ensure!(
        manifest.created_at_unix_ms <= now,
        "pinned backup manifest is future-dated"
    );
    let (envelope_bytes, envelope_identity) = read_cap_regular_bounded_with_mode_and_identity(
        root,
        Path::new("backup-verification-envelope.json"),
        u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
        0o400,
    )?;
    let envelope: BackupVerificationEnvelopeV2 = serde_json::from_slice(&envelope_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&envelope)? == envelope_bytes,
        "pinned backup verification envelope is not canonical JSON"
    );
    envelope.verify_manifest(backup_authority_key, backup_id, &manifest)?;
    anyhow::ensure!(
        manifest.database_schema_version >= 2
            && manifest.database_schema_version <= compiled_schema_version
            && (!require_current_schema
                || manifest.database_schema_version == compiled_schema_version),
        "pinned backup schema is outside this verifier's authenticated policy"
    );

    let mut expected_paths = BTreeSet::from([
        "backup-manifest.json".to_owned(),
        "backup-verification-envelope.json".to_owned(),
    ]);
    let mut hashed_file_identities = BTreeMap::from([
        ("backup-manifest.json".to_owned(), manifest_identity),
        (
            "backup-verification-envelope.json".to_owned(),
            envelope_identity,
        ),
    ]);
    for expected in &manifest.files {
        let relative = Path::new(&expected.relative_path);
        anyhow::ensure!(
            expected_paths.insert(expected.relative_path.clone()),
            "pinned backup repeats a file path"
        );
        let (actual, identity) = record_cap_file_with_identity(root, relative)?;
        anyhow::ensure!(
            &actual == expected,
            "pinned backup object differs: {}",
            expected.relative_path
        );
        hashed_file_identities.insert(expected.relative_path.clone(), identity);
    }
    let actual_paths = backup_tree_paths_cap(root)?;
    anyhow::ensure!(
        actual_paths.files == expected_paths,
        "pinned backup contains missing or unexpected files"
    );
    anyhow::ensure!(
        actual_paths.directories
            == manifest
                .directories
                .iter()
                .map(|directory| directory.relative_path.clone())
                .collect(),
        "pinned backup contains missing or unexpected directories"
    );
    anyhow::ensure!(
        actual_paths.file_identities == hashed_file_identities,
        "pinned backup file inode closure changed between hashing and traversal"
    );
    let initial_tree = actual_paths;

    let database_file = open_cap_regular_nofollow(root, Path::new("highscores.sqlite3"))?;
    validate_private_pinned_file(&database_file, 0o600, "pinned backup database")?;
    anyhow::ensure!(
        initial_tree.file_identities.get("highscores.sqlite3")
            == Some(&metadata_identity_std(&database_file.metadata()?)),
        "pinned backup database inode differs from the hashed manifest object"
    );
    #[cfg(unix)]
    let database_url = {
        use std::os::fd::AsRawFd as _;
        format!(
            "sqlite:///proc/self/fd/{}?mode=ro&immutable=true",
            database_file.as_raw_fd()
        )
    };
    #[cfg(not(unix))]
    anyhow::bail!("pinned backup database verification requires a procfd-capable Unix host");
    #[cfg(unix)]
    let mut connection = sqlx::SqliteConnection::connect(&database_url).await?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    anyhow::ensure!(integrity == "ok", "pinned SQLite integrity check failed");
    let schema: i64 =
        sqlx::query("SELECT MAX(version) AS version FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&mut connection)
            .await?
            .try_get("version")?;
    anyhow::ensure!(
        schema == manifest.database_schema_version,
        "pinned backup database schema differs from its manifest"
    );
    if schema == 2 {
        // VpsReleaseManifestV2 begins at database schema 2. Keep these exact
        // relational/transient-state checks as the historical schema-2
        // verifier even after the running binary advances to a later schema.
        verify_database_object_inventory(&mut connection, &manifest.files).await?;
    } else if schema == robin_highscores::db::CURRENT_SCHEMA_VERSION {
        verify_database_object_inventory(&mut connection, &manifest.files).await?;
    } else {
        anyhow::bail!("pinned backup schema has no compiled semantic inventory verifier: {schema}");
    }
    let foreign_key_failures: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
            .fetch_one(&mut connection)
            .await?;
    anyhow::ensure!(
        foreign_key_failures == 0,
        "pinned backup database has foreign-key violations"
    );
    connection.close().await?;
    anyhow::ensure!(
        initial_tree.file_identities.get("highscores.sqlite3")
            == Some(&metadata_identity_std(&database_file.metadata()?)),
        "pinned backup database inode changed during SQLite verification"
    );

    for expected in &manifest.files {
        let (actual, identity) =
            record_cap_file_with_identity(root, Path::new(&expected.relative_path))?;
        anyhow::ensure!(
            &actual == expected
                && initial_tree.file_identities.get(&expected.relative_path) == Some(&identity),
            "pinned backup object changed before receipt: {}",
            expected.relative_path
        );
    }
    let (final_manifest_bytes, final_manifest_identity) =
        read_cap_regular_bounded_with_mode_and_identity(
            root,
            Path::new("backup-manifest.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_MANIFEST_BYTES)?,
            0o600,
        )?;
    let (final_envelope_bytes, final_envelope_identity) =
        read_cap_regular_bounded_with_mode_and_identity(
            root,
            Path::new("backup-verification-envelope.json"),
            u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES)?,
            0o400,
        )?;
    anyhow::ensure!(
        final_manifest_bytes == manifest_bytes
            && final_envelope_bytes == envelope_bytes
            && final_manifest_identity == manifest_identity
            && final_envelope_identity == envelope_identity,
        "pinned backup authority documents changed before receipt"
    );
    let final_tree = backup_tree_paths_cap(root)?;
    anyhow::ensure!(
        final_tree.files == initial_tree.files
            && final_tree.directories == initial_tree.directories
            && final_tree.file_identities == initial_tree.file_identities
            && final_tree.directory_identities == initial_tree.directory_identities
            && final_tree.root_identity == initial_tree.root_identity,
        "pinned backup topology changed before receipt"
    );
    let total_bytes = manifest.total_bytes()?;
    let directory_count = manifest.directory_count()?;
    Ok(VerifiedBackup {
        created_at_unix_ms: manifest.created_at_unix_ms,
        database_schema_version: manifest.database_schema_version,
        manifest_sha256: hex::encode(Sha256::digest(&manifest_bytes)),
        release_identity: manifest.release_identity,
        file_count: u64::try_from(manifest.files.len())?,
        directory_count,
        total_bytes,
        tree: final_tree,
    })
}

#[cfg(test)]
async fn verify_historical_backup_chain_with_compiled_schema(
    backup_root: &Path,
    backup_directory: &Path,
    backup_authority_key: &[u8; 32],
    compiled_schema_version: i64,
) -> anyhow::Result<VerifiedBackup> {
    anyhow::ensure!(
        backup_directory.parent() == Some(backup_root),
        "historical test backup is outside its authority root"
    );
    let root = pin_directory_capability(backup_root)?;
    let root_metadata = root.dir_metadata()?;
    let backup_id = backup_directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("historical backup has no canonical ID"))?;
    let child = open_cap_directory_nofollow(&root, Path::new(backup_id))?;
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as _;
        anyhow::ensure!(
            child.dir_metadata()?.dev() == root_metadata.dev(),
            "historical backup crosses the authority-root device"
        );
    }
    let verified = verify_backup_capability_with_compiled_schema(
        &child,
        backup_id,
        backup_authority_key,
        compiled_schema_version,
        false,
    )
    .await?;
    let preserved =
        load_preserved_release_authority(backup_root, &verified.release_identity).await?;
    anyhow::ensure!(
        preserved == verified.release_identity,
        "historical backup differs from its independent preserved release authority"
    );
    Ok(verified)
}

fn read_cap_regular_bounded(
    root: &cap_std::fs::Dir,
    relative: &Path,
    maximum: u64,
) -> anyhow::Result<Vec<u8>> {
    read_cap_regular_bounded_with_mode(root, relative, maximum, 0o600)
}

fn read_cap_regular_bounded_with_mode(
    root: &cap_std::fs::Dir,
    relative: &Path,
    maximum: u64,
    expected_mode: u32,
) -> anyhow::Result<Vec<u8>> {
    Ok(read_cap_regular_bounded_with_mode_and_identity(root, relative, maximum, expected_mode)?.0)
}

fn read_cap_regular_bounded_with_mode_and_identity(
    root: &cap_std::fs::Dir,
    relative: &Path,
    maximum: u64,
    expected_mode: u32,
) -> anyhow::Result<(Vec<u8>, FileIdentity)> {
    let file = open_cap_regular_nofollow(root, relative)?;
    validate_private_pinned_file(&file, expected_mode, "pinned backup file")?;
    let identity = metadata_identity_std(&file.metadata()?);
    let bytes = read_bounded_pinned_file(duplicate_pinned_file(&file, false)?, maximum)?;
    anyhow::ensure!(
        metadata_identity_std(&file.metadata()?) == identity,
        "pinned backup file changed identity while reading"
    );
    Ok((bytes, identity))
}

fn record_cap_file_with_identity(
    root: &cap_std::fs::Dir,
    relative: &Path,
) -> anyhow::Result<(BackupFile, FileIdentity)> {
    let mut file = open_cap_regular_nofollow(root, relative)?;
    validate_private_pinned_file(&file, 0o600, "pinned backup file")?;
    let metadata = file.metadata()?;
    let mut digest = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow::anyhow!("pinned backup file length overflows"))?;
        digest.update(&buffer[..read]);
    }
    anyhow::ensure!(
        byte_length == metadata.len() && file.metadata()?.len() == metadata.len(),
        "pinned backup file changed length while hashing"
    );
    let file_identity = metadata_identity_std(&metadata);
    anyhow::ensure!(
        metadata_identity_std(&file.metadata()?) == file_identity,
        "pinned backup file changed identity while hashing"
    );
    Ok((
        BackupFile {
            relative_path: relative
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("pinned backup path is not UTF-8"))?
                .replace('\\', "/"),
            byte_length,
            sha256: hex::encode(digest.finalize()),
        },
        file_identity,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BackupTreePaths {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
    file_identities: BTreeMap<String, FileIdentity>,
    directory_identities: BTreeMap<String, FileIdentity>,
    root_identity: FileIdentity,
}

fn backup_tree_paths_cap(root: &cap_std::fs::Dir) -> anyhow::Result<BackupTreePaths> {
    let root_metadata = root.dir_metadata()?;
    let root_identity = metadata_identity(&root_metadata);
    let mut pending = vec![(root.try_clone()?, PathBuf::new(), 0_usize)];
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut file_identities = BTreeMap::new();
    let mut directory_identities = BTreeMap::new();
    let mut visited = 0_usize;
    while let Some((directory, prefix, depth)) = pending.pop() {
        anyhow::ensure!(
            depth <= robin_highscores::backup::MAX_BACKUP_TREE_DEPTH,
            "pinned backup tree is too deep"
        );
        for entry in directory.entries()? {
            let entry = entry?;
            visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("pinned backup topology overflows"))?;
            anyhow::ensure!(
                visited <= robin_highscores::backup::MAX_BACKUP_MANIFEST_FILES + 2,
                "pinned backup topology exceeds its entry limit"
            );
            let name = entry.file_name();
            let name_text = name
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("pinned backup entry is not UTF-8"))?;
            anyhow::ensure!(
                name_text.len() <= robin_highscores::backup::MAX_BACKUP_COMPONENT_BYTES,
                "pinned backup component is too long"
            );
            let relative = prefix.join(name_text);
            anyhow::ensure!(
                relative.as_os_str().as_encoded_bytes().len()
                    <= robin_highscores::backup::MAX_BACKUP_RELATIVE_PATH_BYTES,
                "pinned backup path is too long"
            );
            let metadata = directory.symlink_metadata(&name)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
                "pinned backup tree contains a link or special node"
            );
            validate_managed_metadata(&metadata, &root_metadata, metadata.is_dir())?;
            if metadata.is_dir() {
                #[cfg(unix)]
                {
                    use cap_std::fs::PermissionsExt as _;
                    anyhow::ensure!(
                        metadata.permissions().mode() & 0o777 == 0o700,
                        "pinned backup directory has mode {:04o}, not 0700: {}",
                        metadata.permissions().mode() & 0o777,
                        relative.display(),
                    );
                }
                let child = open_cap_directory_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity(&child.dir_metadata()?) == metadata_identity(&metadata),
                    "pinned backup directory was substituted while opening"
                );
                let relative_text = relative.to_string_lossy().replace('\\', "/");
                directories.insert(relative_text.clone());
                directory_identities.insert(relative_text, metadata_identity(&metadata));
                pending.push((child, relative, depth + 1));
            } else {
                let child = open_cap_regular_nofollow(&directory, Path::new(&name))?;
                anyhow::ensure!(
                    metadata_identity_std(&child.metadata()?) == metadata_identity(&metadata),
                    "pinned backup file was substituted while opening"
                );
                let relative = relative
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("pinned backup path is not UTF-8"))?
                    .replace('\\', "/");
                #[cfg(unix)]
                {
                    use cap_std::fs::PermissionsExt as _;
                    let expected_mode = expected_backup_cleanup_file_mode(&relative)?;
                    anyhow::ensure!(
                        metadata.permissions().mode() & 0o777 == expected_mode,
                        "pinned backup file has a noncanonical mode"
                    );
                }
                anyhow::ensure!(
                    files.insert(relative.clone()),
                    "pinned backup repeats a path"
                );
                file_identities.insert(relative, metadata_identity(&metadata));
            }
        }
    }
    Ok(BackupTreePaths {
        files,
        directories,
        file_identities,
        directory_identities,
        root_identity,
    })
}

async fn verify_database_object_inventory(
    connection: &mut sqlx::SqliteConnection,
    files: &[BackupFile],
) -> anyhow::Result<()> {
    let transient_rows: i64 = sqlx::query_scalar(
        "SELECT \
           (SELECT COUNT(*) FROM maintenance_locks) + \
           (SELECT COUNT(*) FROM maintenance_write_leases) + \
           (SELECT COUNT(*) FROM submissions \
              WHERE status = 'verifying' OR lease_owner IS NOT NULL OR lease_expires_at_ms IS NOT NULL) + \
           (SELECT COUNT(*) FROM submission_upload_reservations \
              WHERE state IN ('reserved', 'uploaded') \
                 OR lease_token IS NOT NULL OR lease_expires_at_ms IS NOT NULL)",
    )
    .fetch_one(&mut *connection)
    .await?;
    anyhow::ensure!(
        transient_rows == 0,
        "backup database contains transient maintenance, worker, or upload leases"
    );
    let replay_files = indexed_object_files(files, "replays", ".rhrec", 2)?;
    let campaign_files = indexed_object_files(files, "campaigns", ".campaign", 1)?;
    verify_object_rows(connection, "replay_objects", &replay_files).await?;
    verify_object_rows(connection, "campaign_objects", &campaign_files).await?;

    let invalid_replay_references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM submissions submission \
         JOIN replay_objects object ON object.sha256 = submission.replay_sha256 \
         WHERE submission.tombstoned_at_ms IS NULL AND object.purge_state != 'live'",
    )
    .fetch_one(&mut *connection)
    .await?;
    anyhow::ensure!(
        invalid_replay_references == 0,
        "backup database has a non-live referenced replay"
    );
    let invalid_campaign_references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM campaign_object_submission_references reference \
         JOIN submissions submission ON submission.id = reference.submission_id \
         JOIN campaign_objects object ON object.sha256 = reference.sha256 \
         WHERE submission.tombstoned_at_ms IS NULL AND object.purge_state != 'live'",
    )
    .fetch_one(&mut *connection)
    .await?;
    anyhow::ensure!(
        invalid_campaign_references == 0,
        "backup database has a non-live referenced campaign object"
    );
    Ok(())
}

fn indexed_object_files<'a>(
    files: &'a [BackupFile],
    root: &str,
    suffix: &str,
    shard_count: usize,
) -> anyhow::Result<BTreeMap<[u8; 32], &'a BackupFile>> {
    let mut objects = BTreeMap::new();
    let prefix = format!("{root}/");
    for file in files
        .iter()
        .filter(|file| file.relative_path.starts_with(&prefix))
    {
        let components = Path::new(&file.relative_path)
            .components()
            .map(|component| match component {
                std::path::Component::Normal(value) => value
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("object path is not UTF-8")),
                _ => Err(anyhow::anyhow!("object path is unsafe")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        anyhow::ensure!(
            components.len() == shard_count + 2 && components[0] == root,
            "backup object path has the wrong shape"
        );
        let file_name = components
            .last()
            .expect("validated backup object path has a filename");
        let digest_hex = file_name
            .strip_suffix(suffix)
            .ok_or_else(|| anyhow::anyhow!("backup object has the wrong suffix"))?;
        anyhow::ensure!(
            digest_hex.len() == 64
                && digest_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "backup object name is not a canonical digest"
        );
        for shard in 0..shard_count {
            anyhow::ensure!(
                components[shard + 1] == &digest_hex[shard * 2..shard * 2 + 2],
                "backup object is in the wrong digest shard"
            );
        }
        anyhow::ensure!(
            file.sha256 == digest_hex,
            "backup content digest does not match its content address"
        );
        let digest: [u8; 32] = hex::decode(digest_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("backup object digest has the wrong decoded length"))?;
        anyhow::ensure!(
            objects.insert(digest, file).is_none(),
            "duplicate backup object digest"
        );
    }
    Ok(objects)
}

async fn verify_object_rows(
    connection: &mut sqlx::SqliteConnection,
    table: &str,
    objects: &BTreeMap<[u8; 32], &BackupFile>,
) -> anyhow::Result<()> {
    let query = match table {
        "replay_objects" => {
            "SELECT sha256, byte_length, purge_state FROM replay_objects ORDER BY sha256"
        }
        "campaign_objects" => {
            "SELECT sha256, byte_length, purge_state FROM campaign_objects ORDER BY sha256"
        }
        _ => anyhow::bail!("unsupported object inventory table"),
    };
    for row in sqlx::query(query).fetch_all(&mut *connection).await? {
        let digest_bytes: Vec<u8> = row.try_get("sha256")?;
        let digest: [u8; 32] = digest_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("database object digest has the wrong length"))?;
        let byte_length = u64::try_from(row.try_get::<i64, _>("byte_length")?)?;
        let purge_state: String = row.try_get("purge_state")?;
        match purge_state.as_str() {
            "live" => {
                let file = objects.get(&digest).ok_or_else(|| {
                    anyhow::anyhow!("live database object is missing from backup inventory")
                })?;
                anyhow::ensure!(
                    file.byte_length == byte_length,
                    "database and backup object byte lengths differ"
                );
            }
            "purging" => anyhow::bail!(
                "backup database contains a purging object despite the GC exclusion lock"
            ),
            "purged" => {
                // A post-snapshot upload may legitimately recreate these immutable
                // bytes. It is an unreferenced physical extra in this snapshot and
                // will be reconciled on restore; it is never treated as a live row.
            }
            _ => anyhow::bail!("database object has an unknown purge state"),
        }
    }
    Ok(())
}

async fn record_file(root: &Path, path: &Path) -> anyhow::Result<BackupFile> {
    use tokio::io::AsyncReadExt as _;

    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path).await?;
    let metadata = file.metadata().await?;
    anyhow::ensure!(metadata.is_file(), "backup source is not a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(metadata.nlink() == 1, "backup source is multiply linked");
    }
    let mut digest = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow::anyhow!("backup file length overflows"))?;
        digest.update(&buffer[..read]);
    }
    anyhow::ensure!(
        byte_length == metadata.len() && file.metadata().await?.len() == metadata.len(),
        "backup file changed length while hashing"
    );
    Ok(BackupFile {
        relative_path: path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/"),
        byte_length,
        sha256: hex::encode(digest.finalize()),
    })
}

async fn read_bounded_regular_nofollow(path: &Path, maximum: u64) -> anyhow::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt as _;

    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options.open(path).await?;
    let metadata = file.metadata().await?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= maximum,
        "bounded backup document is not a bounded regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            metadata.nlink() == 1,
            "bounded backup document is multiply linked"
        );
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(maximum + 1).read_to_end(&mut bytes).await?;
    anyhow::ensure!(
        u64::try_from(bytes.len())? <= maximum,
        "bounded backup document exceeds its byte limit"
    );
    Ok(bytes)
}

async fn copy_pinned_restore_source(
    source: &PinnedRestoreSource,
    destination: &Path,
) -> anyhow::Result<()> {
    copy_pinned_restore_source_with_hook(source, destination, || Ok(())).await
}

async fn copy_pinned_restore_source_with_hook<F>(
    source: &PinnedRestoreSource,
    destination: &Path,
    after_copy: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    use std::io::{Seek as _, SeekFrom};

    source.revalidate("backup restore source before copy")?;
    let mut duplicate = duplicate_pinned_file(&source.file, false)?;
    duplicate.seek(SeekFrom::Start(0))?;
    copy_open_file(tokio::fs::File::from_std(duplicate), destination).await?;
    after_copy()?;
    source.revalidate("backup restore source after copy")?;
    let target =
        read_bounded_regular_nofollow(destination, u64::try_from(source.bytes.len())?).await?;
    anyhow::ensure!(
        target == source.bytes && hex::encode(Sha256::digest(&target)) == source.sha256,
        "copied restore source differs from its pinned admitted bytes"
    );
    Ok(())
}

async fn copy_open_file(source: tokio::fs::File, destination: &Path) -> anyhow::Result<()> {
    let source = source.into_std().await;
    let destination = destination.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || {
        copy_open_file_sync(source, &destination)
    })
    .await
    .context("backup copy task failed")?
}

fn copy_open_file_sync(mut source: std::fs::File, destination: &Path) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        let mut missing = Vec::new();
        let mut cursor = parent;
        while !cursor.try_exists()? {
            missing.push(cursor.to_owned());
            cursor = cursor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("backup destination has no existing ancestor"))?;
        }
        std::fs::create_dir_all(parent)?;
        for directory in missing.into_iter().rev() {
            #[cfg(unix)]
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            #[cfg(not(unix))]
            let _ = directory;
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut target = options.open(destination)?;
    std::io::copy(&mut source, &mut target)?;
    target.sync_all()?;
    Ok(())
}

async fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let path = path.to_owned();
    let bytes = bytes.to_vec();
    robin_highscores::physical_work::spawn_blocking(move || {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()
    })
    .await
    .context("backup write task failed")??;
    Ok(())
}

async fn create_backup_directory(path: &Path) -> anyhow::Result<()> {
    let path = path.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || std::fs::create_dir(path))
        .await
        .context("backup directory creation task failed")??;
    Ok(())
}

#[cfg(unix)]
async fn set_backup_permissions(
    path: &Path,
    permissions: std::fs::Permissions,
) -> anyhow::Result<()> {
    let path = path.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || {
        std::fs::set_permissions(path, permissions)
    })
    .await
    .context("backup permission task failed")??;
    Ok(())
}

async fn set_private_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        set_backup_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}

async fn sync_directory(path: &Path) -> anyhow::Result<()> {
    let path = path.to_owned();
    robin_highscores::physical_work::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .context("backup directory sync task failed")??;
    Ok(())
}

fn install_verified_partial(
    partial: &Path,
    complete: &Path,
    backup_root: &Path,
) -> anyhow::Result<BackupInstallOutcome> {
    install_verified_partial_with(partial, complete, || {
        let directory = open_directory_nofollow(backup_root)?;
        directory.sync_all()?;
        Ok(())
    })
}

fn install_verified_partial_with<F>(
    partial: &Path,
    complete: &Path,
    sync_parent: F,
) -> anyhow::Result<BackupInstallOutcome>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    sync_private_tree_bottom_up(partial)?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        partial,
        rustix::fs::CWD,
        complete,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        anyhow::anyhow!("atomically install verified backup without replacement: {error}")
    })?;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        anyhow::ensure!(
            !complete.exists(),
            "completed backup destination already exists"
        );
        std::fs::rename(partial, complete)?;
    }
    Ok(match sync_parent() {
        Ok(()) => BackupInstallOutcome::Installed,
        Err(error) => BackupInstallOutcome::InstalledButParentSyncFailed(error),
    })
}

fn sync_private_tree_bottom_up(root: &Path) -> anyhow::Result<()> {
    let root_metadata = std::fs::symlink_metadata(root)?;
    anyhow::ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "verified backup staging root must be a real directory"
    );
    let mut pending = vec![(root.to_owned(), false, 0_usize)];
    let mut visited = 0_usize;
    while let Some((path, children_visited, depth)) = pending.pop() {
        anyhow::ensure!(
            depth <= 32,
            "verified backup tree is too deep to sync safely"
        );
        if children_visited {
            open_directory_nofollow(&path)?.sync_all()?;
            continue;
        }
        visited = visited
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("verified backup sync topology count overflows"))?;
        anyhow::ensure!(
            visited <= 1_000_000,
            "verified backup tree is too large to sync safely"
        );
        let metadata = std::fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "verified backup directory changed type before durability sync"
        );
        pending.push((path.clone(), true, depth));
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let child = entry.path();
            let metadata = std::fs::symlink_metadata(&child)?;
            anyhow::ensure!(
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
                "verified backup tree contains a link or special node during durability sync"
            );
            if metadata.is_dir() {
                pending.push((child, false, depth + 1));
            } else {
                open_regular_nofollow(&child)?.sync_all()?;
            }
        }
    }
    Ok(())
}

fn open_directory_nofollow(path: &Path) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        let descriptor = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )?;
        let file = std::fs::File::from(descriptor);
        anyhow::ensure!(file.metadata()?.is_dir(), "path is not a directory");
        return Ok(file);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
        let file = options.open(path)?;
        anyhow::ensure!(file.metadata()?.is_dir(), "path is not a directory");
        Ok(file)
    }
}

fn open_regular_nofollow(path: &Path) -> anyhow::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        let descriptor = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )?;
        let file = std::fs::File::from(descriptor);
        anyhow::ensure!(file.metadata()?.is_file(), "path is not a regular file");
        return Ok(file);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(metadata.is_file(), "path is not a regular file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            anyhow::ensure!(
                metadata.nlink() == 1,
                "verified backup file is multiply linked"
            );
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn assert_backup_completion_ownership(mode: &'static str) {
        use std::sync::Arc;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_owned();
        let mut config = ServerConfig::default();
        config.database_path = root.join("live.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        let runtime = database.runtime_fence().clone();
        let source = root.join("source");
        std::fs::write(&source, b"physical backup bytes").unwrap();
        let partial = root.join(".test.partial/payload");
        let published = root.join("published.marker");
        let operation_root = root.clone();
        let operation_path = partial.clone();
        let publication_path = published.clone();
        let operation_database = database.clone();
        let (registered, wait_registered) = tokio::sync::oneshot::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        let (release_queue, wait_release_queue) = std::sync::mpsc::channel();
        let refresh_seen = Arc::new(tokio::sync::Notify::new());
        let notify_refresh = Arc::clone(&refresh_seen);
        let replacement_ready = Arc::new(tokio::sync::Notify::new());
        let wait_replacement = Arc::clone(&replacement_ready);
        let mut caller = tokio::spawn(async move {
            run_owned_backup(async move {
                let _operation_lock = acquire_backup_operation_lock(&operation_root)?;
                let (token, fence) = acquire_backup_write_authority(&operation_database).await?;
                let operation_token = token.clone();
                let body_database = operation_database.clone();
                let refresh_database = operation_database.clone();
                let refresh_token = token.clone();
                let result = run_with_backup_lock_heartbeat_using(
                    async move {
                        // On the one-blocking-thread runtime this occupies its only
                        // thread before the registered physical copy is enqueued.
                        let blocker = if mode == "queued" {
                            let (started, wait_started) = tokio::sync::oneshot::channel();
                            let blocker = tokio::task::spawn_blocking(move || {
                                started.send(()).unwrap();
                                wait_release_queue
                                    .recv_timeout(Duration::from_secs(10))
                                    .unwrap();
                            });
                            wait_started.await?;
                            Some(blocker)
                        } else {
                            None
                        };
                        let job = robin_highscores::physical_work::spawn_blocking(move || {
                            wait_release.recv_timeout(Duration::from_secs(10)).unwrap();
                            copy_open_file_sync(std::fs::File::open(source)?, &operation_path)
                        });
                        registered.send(operation_token.clone()).unwrap();
                        if mode == "operation_error" || mode == "queued" {
                            drop(job);
                            drop(blocker);
                            anyhow::bail!("injected backup operation error");
                        }
                        if mode == "operation_panic" {
                            drop(job);
                            panic!("injected backup operation panic");
                        }
                        job.await??;
                        if mode == "heartbeat_then_panic" {
                            panic!("backup operation panicked after heartbeat loss");
                        }
                        // This is the same exact-token barrier retained immediately
                        // before real installation/status publication.
                        refresh_backup_lock(&body_database, &operation_token).await?;
                        std::fs::write(publication_path, b"authorized")?;
                        Ok(())
                    },
                    Duration::from_millis(10),
                    move || {
                        let notify = Arc::clone(&notify_refresh);
                        let wait_replacement = Arc::clone(&wait_replacement);
                        let database = refresh_database.clone();
                        let token = refresh_token.clone();
                        async move {
                            let result = match mode {
                                "heartbeat_error" | "heartbeat_then_panic" => {
                                    Err(anyhow::anyhow!("injected backup heartbeat error"))
                                }
                                "heartbeat_panic" => {
                                    notify.notify_one();
                                    panic!("injected backup heartbeat panic")
                                }
                                "replaced_token" => {
                                    wait_replacement.notified().await;
                                    refresh_backup_lock(&database, &token).await
                                }
                                _ => refresh_backup_lock(&database, &token).await,
                            };
                            notify.notify_one();
                            result
                        }
                    },
                )
                .await;
                let released = release_backup_gate_and_close_pool_under_exclusive_fence(
                    &operation_database,
                    &token,
                    &fence,
                )
                .await?;
                if mode != "replaced_token" {
                    anyhow::ensure!(released, "backup gate disappeared");
                }
                result
            })
            .await
        });
        let token = tokio::time::timeout(Duration::from_secs(5), wait_registered)
            .await
            .unwrap()
            .unwrap();
        if mode == "replaced_token" {
            assert!(database.release_backup_lock(&token).await.unwrap());
            database
                .acquire_backup_lock("replacement", BACKUP_LOCK_TTL)
                .await
                .unwrap();
            replacement_ready.notify_one();
        }
        if mode.starts_with("heartbeat_") || mode == "replaced_token" {
            tokio::time::timeout(Duration::from_secs(5), refresh_seen.notified())
                .await
                .unwrap();
        }
        if mode == "caller_cancelled" {
            caller.abort();
            assert!((&mut caller).await.unwrap_err().is_cancelled());
        } else {
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut caller)
                    .await
                    .is_err(),
                "backup owner returned before physical mutation completed"
            );
        }
        assert!(!partial.exists());
        assert!(runtime.try_lock_exclusive_quiescence().unwrap().is_none());
        assert!(runtime.try_lock_exclusive_admission().unwrap().is_none());
        assert!(
            acquire_backup_operation_lock(&root).is_err(),
            "another backup could clean the active partial"
        );
        assert!(database.backup_lock_active().await.unwrap());
        release.send(()).unwrap();
        if mode == "queued" {
            release_queue.send(()).unwrap();
        }
        if mode != "caller_cancelled" {
            let result = tokio::time::timeout(Duration::from_secs(5), caller)
                .await
                .unwrap()
                .unwrap();
            if mode == "success" {
                result.unwrap();
            } else {
                let error = format!("{:#}", result.unwrap_err());
                let expected = match mode {
                    "heartbeat_error" | "heartbeat_then_panic" => "injected backup heartbeat error",
                    "heartbeat_panic" => "backup lock refresh panicked",
                    "operation_error" | "queued" => "injected backup operation error",
                    "operation_panic" => "panicked after physical-work drain",
                    "replaced_token" => "backup lock expired or was lost",
                    _ => unreachable!(),
                };
                assert!(error.contains(expected), "{error}");
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if runtime.try_lock_exclusive_quiescence().unwrap().is_some()
                    && acquire_backup_operation_lock(&root).is_ok()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(partial).unwrap(), b"physical backup bytes");
        if mode == "replaced_token" {
            assert!(!published.exists());
        }
    }

    #[tokio::test]
    async fn backup_owner_drains_physical_work() {
        for mode in [
            "success",
            "heartbeat_error",
            "operation_error",
            "caller_cancelled",
            "replaced_token",
        ] {
            assert_backup_completion_ownership(mode).await;
        }
    }

    #[test]
    fn backup_owner_drains_queued_physical_work() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap()
            .block_on(assert_backup_completion_ownership("queued"));
    }

    #[tokio::test]
    #[ignore = "requires explicit LLVM backend for actual unwind/destructor execution"]
    async fn backup_owner_unwind_drains_physical_work() {
        for mode in ["operation_panic", "heartbeat_panic", "heartbeat_then_panic"] {
            assert_backup_completion_ownership(mode).await;
        }
    }

    async fn assert_scrub_connection_closes(panic: bool) {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("snapshot.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        database.close_fenced().await.unwrap();
        let error = scrub_transient_backup_state_with_hook(&config.database_path, 1, || {
            if panic {
                panic!("injected scrub panic with active transaction")
            }
            anyhow::bail!("injected scrub error with active transaction")
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains(if panic {
            "scrub panicked"
        } else {
            "injected scrub error"
        }));
        assert!(
            !config
                .database_path
                .with_extension("sqlite3-journal")
                .exists()
        );
        let options = SqliteConnectOptions::new()
            .filename(&config.database_path)
            .busy_timeout(Duration::ZERO);
        let mut reopened = sqlx::SqliteConnection::connect_with(&options)
            .await
            .unwrap();
        sqlx::query("BEGIN EXCLUSIVE")
            .execute(&mut reopened)
            .await
            .unwrap();
        sqlx::query("ROLLBACK")
            .execute(&mut reopened)
            .await
            .unwrap();
        reopened.close().await.unwrap();
    }

    #[tokio::test]
    async fn backup_owner_closes_destination_sqlite_on_error() {
        assert_scrub_connection_closes(false).await;
    }

    #[tokio::test]
    #[ignore = "requires explicit LLVM backend for actual unwind/destructor execution"]
    async fn backup_owner_unwind_closes_destination_sqlite() {
        assert_scrub_connection_closes(true).await;
    }

    #[cfg(target_os = "linux")]
    struct BackupAuthorityKeyHarness {
        _directory: tempfile::TempDir,
        opt_root: PathBuf,
        key_path: PathBuf,
        activation_lock: std::fs::File,
    }

    #[cfg(target_os = "linux")]
    impl BackupAuthorityKeyHarness {
        fn new() -> Self {
            use fs2::FileExt as _;
            use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

            let directory = tempfile::tempdir().unwrap();
            let opt_root = directory.path().join("opt/robin-highscores");
            let secret_parent = directory.path().join("state/api-secrets");
            std::fs::create_dir_all(&opt_root).unwrap();
            std::fs::create_dir_all(&secret_parent).unwrap();
            std::fs::set_permissions(&opt_root, std::fs::Permissions::from_mode(0o750)).unwrap();
            std::fs::set_permissions(&secret_parent, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            let lock_path = opt_root.join("activation.lock");
            let activation_lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&lock_path)
                .unwrap();
            std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600)).unwrap();
            activation_lock.sync_all().unwrap();
            std::fs::File::open(&opt_root).unwrap().sync_all().unwrap();
            activation_lock.lock_exclusive().unwrap();
            Self {
                _directory: directory,
                opt_root,
                key_path: secret_parent.join("backup-authority-hmac.key"),
                activation_lock,
            }
        }

        fn activation_lock_fd(&self) -> u32 {
            use std::os::fd::AsRawFd as _;
            u32::try_from(self.activation_lock.as_raw_fd()).unwrap()
        }

        fn initialize(&self, source_commit: &str) -> anyhow::Result<()> {
            initialize_backup_authority_key_v2_at(
                &self.opt_root,
                &self.key_path,
                rustix::process::geteuid().as_raw(),
                source_commit,
                self.activation_lock_fd(),
                |_| Ok(()),
            )
        }

        fn interrupt_initialize(
            &self,
            source_commit: &str,
            boundary: BackupAuthorityKeyBoundary,
        ) -> anyhow::Result<()> {
            let mut interrupted = false;
            initialize_backup_authority_key_v2_at(
                &self.opt_root,
                &self.key_path,
                rustix::process::geteuid().as_raw(),
                source_commit,
                self.activation_lock_fd(),
                |observed| {
                    if observed == boundary && !interrupted {
                        interrupted = true;
                        anyhow::bail!("injected interruption at {observed:?}");
                    }
                    Ok(())
                },
            )
        }

        fn complete<V>(&self, source_commit: &str, validate: V) -> anyhow::Result<()>
        where
            V: FnOnce() -> anyhow::Result<()>,
        {
            complete_backup_authority_key_v2_at(
                &self.opt_root,
                &self.key_path,
                rustix::process::geteuid().as_raw(),
                source_commit,
                self.activation_lock_fd(),
                validate,
                |_| Ok(()),
            )
        }

        fn interrupt_complete<V>(
            &self,
            source_commit: &str,
            validate: V,
            boundary: BackupAuthorityKeyBoundary,
        ) -> anyhow::Result<()>
        where
            V: FnOnce() -> anyhow::Result<()>,
        {
            let mut interrupted = false;
            complete_backup_authority_key_v2_at(
                &self.opt_root,
                &self.key_path,
                rustix::process::geteuid().as_raw(),
                source_commit,
                self.activation_lock_fd(),
                validate,
                |observed| {
                    if observed == boundary && !interrupted {
                        interrupted = true;
                        anyhow::bail!("injected interruption at {observed:?}");
                    }
                    Ok(())
                },
            )
        }

        fn intent_path(&self) -> PathBuf {
            self.key_path
                .parent()
                .unwrap()
                .join(BACKUP_AUTHORITY_INTENT_NAME)
        }

        fn temporary_intent_path(&self) -> PathBuf {
            self.key_path
                .parent()
                .unwrap()
                .join(BACKUP_AUTHORITY_INTENT_TEMP_NAME)
        }
    }

    #[cfg(target_os = "linux")]
    const TEST_SOURCE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[cfg(target_os = "linux")]
    fn test_anonymous_backup_authority_key(parent: &PinnedSecretParent) -> std::fs::File {
        use rustix::fs::{Mode, OFlags, fchmod, openat};
        use std::io::Write as _;
        use std::os::fd::AsFd as _;

        let anonymous = openat(
            parent.fd.as_fd(),
            ".",
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::TMPFILE,
            Mode::from_raw_mode(0o400),
        )
        .unwrap();
        fchmod(&anonymous, Mode::from_raw_mode(0o400)).unwrap();
        let mut anonymous = std::fs::File::from(anonymous);
        anonymous.write_all(&[0x5a; 32]).unwrap();
        anonymous.sync_all().unwrap();
        anonymous
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_publication_falls_back_from_empty_path_enoent() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let harness = BackupAuthorityKeyHarness::new();
        let parent =
            pin_secret_parent(&harness.key_path, rustix::process::geteuid().as_raw()).unwrap();
        let anonymous = test_anonymous_backup_authority_key(&parent);
        let retained = anonymous.metadata().unwrap();
        publish_anonymous_backup_authority_key_with(
            &anonymous,
            &parent,
            "backup-authority-hmac.key",
            Path::new("/proc/self/fd"),
            || Err(rustix::io::Errno::NOENT),
        )
        .unwrap();
        let published = std::fs::symlink_metadata(&harness.key_path).unwrap();
        assert_eq!(published.dev(), retained.dev());
        assert_eq!(published.ino(), retained.ino());
        assert_eq!(published.uid(), retained.uid());
        assert_eq!(published.nlink(), 1);
        assert_eq!(published.permissions().mode() & 0o777, 0o400);
        assert_eq!(published.len(), 32);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_proc_fallback_rejects_missing_and_substituted_descriptors() {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

        let missing = BackupAuthorityKeyHarness::new();
        let missing_parent =
            pin_secret_parent(&missing.key_path, rustix::process::geteuid().as_raw()).unwrap();
        let missing_anonymous = test_anonymous_backup_authority_key(&missing_parent);
        let missing_proc = tempfile::tempdir().unwrap();
        assert!(
            publish_anonymous_backup_authority_key_with(
                &missing_anonymous,
                &missing_parent,
                "backup-authority-hmac.key",
                missing_proc.path(),
                || Err(rustix::io::Errno::NOENT),
            )
            .is_err()
        );
        assert!(!missing.key_path.exists());

        let substituted = BackupAuthorityKeyHarness::new();
        let substituted_parent =
            pin_secret_parent(&substituted.key_path, rustix::process::geteuid().as_raw()).unwrap();
        let substituted_anonymous = test_anonymous_backup_authority_key(&substituted_parent);
        let fake_proc = tempfile::tempdir().unwrap();
        let other = fake_proc.path().join("other");
        std::fs::write(&other, [0x33; 32]).unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o400)).unwrap();
        let before = std::fs::metadata(&other).unwrap();
        symlink(
            &other,
            fake_proc
                .path()
                .join(substituted_anonymous.as_raw_fd().to_string()),
        )
        .unwrap();
        assert!(
            publish_anonymous_backup_authority_key_with(
                &substituted_anonymous,
                &substituted_parent,
                "backup-authority-hmac.key",
                fake_proc.path(),
                || Err(rustix::io::Errno::NOENT),
            )
            .is_err()
        );
        assert!(!substituted.key_path.exists());
        assert_eq!(std::fs::metadata(&other).unwrap().nlink(), before.nlink());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_proc_fallback_never_replaces_an_existing_destination() {
        use std::os::unix::fs::PermissionsExt as _;

        let harness = BackupAuthorityKeyHarness::new();
        std::fs::write(&harness.key_path, b"existing authority").unwrap();
        std::fs::set_permissions(&harness.key_path, std::fs::Permissions::from_mode(0o400))
            .unwrap();
        let parent =
            pin_secret_parent(&harness.key_path, rustix::process::geteuid().as_raw()).unwrap();
        let anonymous = test_anonymous_backup_authority_key(&parent);
        assert!(
            publish_anonymous_backup_authority_key_with(
                &anonymous,
                &parent,
                "backup-authority-hmac.key",
                Path::new("/proc/self/fd"),
                || Err(rustix::io::Errno::NOENT),
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(&harness.key_path).unwrap(),
            b"existing authority"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_proc_fallback_does_not_mask_policy_errors() {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::symlink;

        for direct_error in [rustix::io::Errno::PERM, rustix::io::Errno::ACCESS] {
            let harness = BackupAuthorityKeyHarness::new();
            let parent =
                pin_secret_parent(&harness.key_path, rustix::process::geteuid().as_raw()).unwrap();
            let anonymous = test_anonymous_backup_authority_key(&parent);
            let fake_proc = tempfile::tempdir().unwrap();
            symlink(
                format!("/proc/self/fd/{}", anonymous.as_raw_fd()),
                fake_proc.path().join(anonymous.as_raw_fd().to_string()),
            )
            .unwrap();

            let error = publish_anonymous_backup_authority_key_with(
                &anonymous,
                &parent,
                "backup-authority-hmac.key",
                fake_proc.path(),
                || Err(direct_error),
            )
            .unwrap_err();
            assert!(error.to_string().contains("publish anonymous"));
            assert!(!harness.key_path.exists());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_resumes_every_initialization_publication_boundary() {
        for boundary in [
            BackupAuthorityKeyBoundary::AnonymousKeySynced,
            BackupAuthorityKeyBoundary::IntentTemporaryCreated,
            BackupAuthorityKeyBoundary::IntentTemporaryWritten,
            BackupAuthorityKeyBoundary::IntentTemporarySynced,
            BackupAuthorityKeyBoundary::IntentPublished,
            BackupAuthorityKeyBoundary::KeyLinked,
            BackupAuthorityKeyBoundary::KeyDirectorySynced,
            BackupAuthorityKeyBoundary::KeyValidated,
        ] {
            let harness = BackupAuthorityKeyHarness::new();
            assert!(
                harness
                    .interrupt_initialize(TEST_SOURCE_COMMIT, boundary)
                    .is_err(),
                "boundary {boundary:?} was not reached"
            );
            harness.initialize(TEST_SOURCE_COMMIT).unwrap();
            let key = std::fs::read(&harness.key_path).unwrap();
            assert_eq!(key.len(), 32);
            assert!(key.iter().any(|byte| *byte != 0));
            assert!(harness.intent_path().is_file());
            assert!(!harness.temporary_intent_path().exists());
            harness.initialize(TEST_SOURCE_COMMIT).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_resumes_both_recovery_mutation_boundaries() {
        let temporary = BackupAuthorityKeyHarness::new();
        assert!(
            temporary
                .interrupt_initialize(
                    TEST_SOURCE_COMMIT,
                    BackupAuthorityKeyBoundary::IntentTemporarySynced,
                )
                .is_err()
        );
        assert!(
            temporary
                .interrupt_initialize(
                    TEST_SOURCE_COMMIT,
                    BackupAuthorityKeyBoundary::RecoveryTemporaryRemoved,
                )
                .is_err()
        );
        temporary.initialize(TEST_SOURCE_COMMIT).unwrap();

        let intent = BackupAuthorityKeyHarness::new();
        assert!(
            intent
                .interrupt_initialize(
                    TEST_SOURCE_COMMIT,
                    BackupAuthorityKeyBoundary::IntentPublished,
                )
                .is_err()
        );
        assert!(
            intent
                .interrupt_initialize(
                    TEST_SOURCE_COMMIT,
                    BackupAuthorityKeyBoundary::RecoveryIntentRemoved,
                )
                .is_err()
        );
        intent.initialize(TEST_SOURCE_COMMIT).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_completion_requires_outer_authority_and_is_resumable() {
        use std::cell::Cell;

        for boundary in [
            BackupAuthorityKeyBoundary::OuterAuthorityValidated,
            BackupAuthorityKeyBoundary::CompletionIntentRemoved,
            BackupAuthorityKeyBoundary::CompletionDirectorySynced,
        ] {
            let harness = BackupAuthorityKeyHarness::new();
            harness.initialize(TEST_SOURCE_COMMIT).unwrap();
            assert!(harness.intent_path().is_file());
            assert!(
                harness
                    .interrupt_complete(TEST_SOURCE_COMMIT, || Ok(()), boundary)
                    .is_err(),
                "completion boundary {boundary:?} was not reached"
            );
            let calls = Cell::new(0);
            harness
                .complete(TEST_SOURCE_COMMIT, || {
                    calls.set(calls.get() + 1);
                    Ok(())
                })
                .unwrap();
            assert_eq!(calls.get(), 1);
            assert!(!harness.intent_path().exists());
            assert_eq!(std::fs::read(&harness.key_path).unwrap().len(), 32);
            harness.complete(TEST_SOURCE_COMMIT, || Ok(())).unwrap();
        }

        let rejected = BackupAuthorityKeyHarness::new();
        rejected.initialize(TEST_SOURCE_COMMIT).unwrap();
        assert!(
            rejected
                .complete(TEST_SOURCE_COMMIT, || anyhow::bail!(
                    "outer authority absent"
                ))
                .is_err()
        );
        assert!(rejected.intent_path().is_file());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_requires_the_exact_held_lock_open_file_description() {
        use fs2::FileExt as _;
        use std::os::fd::AsRawFd as _;

        let harness = BackupAuthorityKeyHarness::new();
        let wrong = BackupAuthorityKeyHarness::new();
        assert!(
            initialize_backup_authority_key_v2_at(
                &harness.opt_root,
                &harness.key_path,
                rustix::process::geteuid().as_raw(),
                TEST_SOURCE_COMMIT,
                u32::try_from(wrong.activation_lock.as_raw_fd()).unwrap(),
                |_| Ok(()),
            )
            .is_err(),
            "a lock descriptor for another canonical root was accepted"
        );

        harness.activation_lock.unlock().unwrap();
        assert!(harness.initialize(TEST_SOURCE_COMMIT).is_err());
        harness.activation_lock.lock_exclusive().unwrap();

        let reopened = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(harness.opt_root.join("activation.lock"))
            .unwrap();
        assert!(
            initialize_backup_authority_key_v2_at(
                &harness.opt_root,
                &harness.key_path,
                rustix::process::geteuid().as_raw(),
                TEST_SOURCE_COMMIT,
                u32::try_from(reopened.as_raw_fd()).unwrap(),
                |_| Ok(()),
            )
            .is_err(),
            "a reopened canonical lock inode with a distinct OFD was accepted"
        );
        harness.initialize(TEST_SOURCE_COMMIT).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_rejects_stale_and_unjournaled_authority() {
        let stale = BackupAuthorityKeyHarness::new();
        assert!(
            stale
                .interrupt_initialize(
                    TEST_SOURCE_COMMIT,
                    BackupAuthorityKeyBoundary::IntentPublished,
                )
                .is_err()
        );
        assert!(
            stale
                .initialize("89abcdef0123456789abcdef0123456789abcdef")
                .is_err(),
            "a stale intent from another source transaction was discarded"
        );
        assert!(stale.intent_path().is_file());

        let unjournaled = BackupAuthorityKeyHarness::new();
        unjournaled.initialize(TEST_SOURCE_COMMIT).unwrap();
        std::fs::remove_file(unjournaled.intent_path()).unwrap();
        assert!(
            unjournaled.initialize(TEST_SOURCE_COMMIT).is_err(),
            "initialization silently adopted a final key without intent"
        );
        assert!(
            unjournaled
                .complete(TEST_SOURCE_COMMIT, || anyhow::bail!(
                    "runtime authority absent"
                ))
                .is_err()
        );
        unjournaled.complete(TEST_SOURCE_COMMIT, || Ok(())).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_rejects_key_symlink_hardlink_mode_and_content_substitution() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        for mutation in ["symlink", "hardlink", "mode", "content"] {
            let harness = BackupAuthorityKeyHarness::new();
            harness.initialize(TEST_SOURCE_COMMIT).unwrap();
            match mutation {
                "symlink" => {
                    let displaced = harness.key_path.with_extension("displaced");
                    std::fs::rename(&harness.key_path, &displaced).unwrap();
                    symlink(&displaced, &harness.key_path).unwrap();
                }
                "hardlink" => {
                    std::fs::hard_link(&harness.key_path, harness.key_path.with_extension("alias"))
                        .unwrap();
                }
                "mode" => {
                    std::fs::set_permissions(
                        &harness.key_path,
                        std::fs::Permissions::from_mode(0o600),
                    )
                    .unwrap();
                }
                "content" => {
                    std::fs::set_permissions(
                        &harness.key_path,
                        std::fs::Permissions::from_mode(0o600),
                    )
                    .unwrap();
                    std::fs::write(&harness.key_path, [0x5a; 32]).unwrap();
                    std::fs::set_permissions(
                        &harness.key_path,
                        std::fs::Permissions::from_mode(0o400),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                harness.initialize(TEST_SOURCE_COMMIT).is_err(),
                "{mutation} key substitution was accepted"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_key_v2_rejects_intent_and_owner_substitution() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        for mutation in ["symlink", "hardlink", "mode"] {
            let harness = BackupAuthorityKeyHarness::new();
            harness.initialize(TEST_SOURCE_COMMIT).unwrap();
            match mutation {
                "symlink" => {
                    let displaced = harness.intent_path().with_extension("displaced");
                    std::fs::rename(harness.intent_path(), &displaced).unwrap();
                    symlink(&displaced, harness.intent_path()).unwrap();
                }
                "hardlink" => {
                    std::fs::hard_link(
                        harness.intent_path(),
                        harness.intent_path().with_extension("alias"),
                    )
                    .unwrap();
                }
                "mode" => {
                    std::fs::set_permissions(
                        harness.intent_path(),
                        std::fs::Permissions::from_mode(0o600),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                harness.initialize(TEST_SOURCE_COMMIT).is_err(),
                "{mutation} intent substitution was accepted"
            );
        }

        let intent_content = BackupAuthorityKeyHarness::new();
        intent_content.initialize(TEST_SOURCE_COMMIT).unwrap();
        std::fs::set_permissions(
            intent_content.intent_path(),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut document: BackupAuthorityKeyIntentV1 =
            serde_json::from_slice(&std::fs::read(intent_content.intent_path()).unwrap()).unwrap();
        document.key_inode = document.key_inode.wrapping_add(1);
        std::fs::write(
            intent_content.intent_path(),
            canonical_json_bytes(&document).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(
            intent_content.intent_path(),
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        assert!(intent_content.initialize(TEST_SOURCE_COMMIT).is_err());

        let wrong_owner = BackupAuthorityKeyHarness::new();
        assert!(
            initialize_backup_authority_key_v2_at(
                &wrong_owner.opt_root,
                &wrong_owner.key_path,
                rustix::process::geteuid().as_raw().wrapping_add(1),
                TEST_SOURCE_COMMIT,
                wrong_owner.activation_lock_fd(),
                |_| Ok(()),
            )
            .is_err(),
            "a secret parent owned by a different expected identity was accepted"
        );
    }

    #[test]
    fn backup_authority_key_v2_cli_has_no_configurable_authority_paths() {
        let initialize = Arguments::try_parse_from([
            "admin",
            "initialize-backup-authority-key-v2",
            "--source-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--activation-lock-fd",
            "9",
        ])
        .unwrap();
        assert!(matches!(
            initialize.command,
            Command::InitializeBackupAuthorityKeyV2 {
                activation_lock_fd: 9,
                ..
            }
        ));
        assert!(
            Arguments::try_parse_from([
                "admin",
                "initialize-backup-authority-key-v2",
                "--source-commit",
                "0123456789abcdef0123456789abcdef01234567",
                "--activation-lock-fd",
                "9",
                "--key-path",
                "/tmp/injected",
            ])
            .is_err()
        );
        assert!(
            Arguments::try_parse_from([
                "admin",
                "complete-backup-authority-key-v2",
                "--source-commit",
                "0123456789abcdef0123456789abcdef01234567",
                "--activation-lock-fd",
                "9",
                "--candidate-release-root-fd",
                "10",
                "--expected-vps-release-manifest-sha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ])
            .is_ok()
        );
    }

    #[tokio::test]
    async fn uncertain_gate_release_keeps_exclusive_fence_through_pool_close() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        let token = database
            .acquire_backup_lock("release-error-test", Duration::from_secs(60))
            .await
            .unwrap();
        let exclusive = acquire_exclusive_backup_database_fence(&database, &token)
            .await
            .unwrap();
        let held_connection = database.pool().acquire().await.unwrap();
        let release_connection = std::sync::Arc::new(tokio::sync::Notify::new());
        let holder_release = release_connection.clone();
        let holder = tokio::spawn(async move {
            holder_release.notified().await;
            drop(held_connection);
        });
        let release_attempted = std::sync::Arc::new(tokio::sync::Notify::new());
        let cleanup_release_attempted = release_attempted.clone();
        let cleanup_database = database.clone();
        let cleanup_token = token.clone();
        let release_database = database.clone();
        let release_token = token.clone();
        let cleanup = tokio::spawn(async move {
            close_pool_under_exclusive_fence_after_release(
                &cleanup_database,
                &cleanup_token,
                &exclusive,
                async move {
                    assert!(release_database.release_backup_lock(&release_token).await?);
                    cleanup_release_attempted.notify_one();
                    Err(robin_highscores::db::DbError::Corrupt(
                        "injected outcome-uncertain post-delete failure".to_owned(),
                    ))
                },
            )
            .await
        });
        release_attempted.notified().await;
        assert!(
            database
                .runtime_fence()
                .try_lock_exclusive_quiescence()
                .unwrap()
                .is_none(),
            "cleanup dropped EX while a checked-out SQLx connection remained"
        );
        release_connection.notify_one();
        holder.await.unwrap();
        let error = cleanup.await.unwrap().unwrap_err();
        assert!(format!("{error:#}").contains("exact token is absent after reconciliation"));
        let runtime = database.runtime_fence().clone();
        let admission = runtime.try_lock_exclusive_admission().unwrap().unwrap();
        let quiescence = runtime.try_lock_exclusive_quiescence().unwrap().unwrap();
        runtime
            .validate_exclusive_pair(&admission, &quiescence)
            .unwrap();
    }

    #[tokio::test]
    async fn killed_worker_lease_drains_under_retained_exclusive_fence() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = tempfile::tempdir().unwrap();
        let mut config = ServerConfig::default();
        config.database_path = directory.path().join("highscores.sqlite3");
        let database = Database::migrate(&config).await.unwrap();
        let killed_worker_lease = database
            .acquire_maintenance_write_lease(
                robin_highscores::db::MaintenanceWriteClass::Worker,
                "killed-worker",
                Duration::from_millis(250),
            )
            .await
            .unwrap();
        let live_worker_operation = database
            .runtime_fence()
            .acquire_one_off_shared()
            .await
            .unwrap();
        let backup_lock = database
            .acquire_backup_lock("stale-lease-test", Duration::from_secs(1))
            .await
            .unwrap();
        let initial_backup_expiry: i64 = sqlx::query_scalar(
            "SELECT expires_at_ms FROM maintenance_locks WHERE name = 'backup' AND token = ?",
        )
        .bind(&backup_lock)
        .fetch_one(database.pool())
        .await
        .unwrap();

        let exclusive_acquisition =
            acquire_exclusive_backup_database_fence(&database, &backup_lock);
        tokio::pin!(exclusive_acquisition);
        tokio::select! {
            _ = &mut exclusive_acquisition => panic!("EX bypassed a live worker operation"),
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            1,
            "live worker lease vanished while its kernel fence was retained"
        );
        drop(live_worker_operation);
        let exclusive = exclusive_acquisition.await.unwrap();
        assert!(
            database
                .runtime_fence()
                .try_acquire_one_off_shared()
                .unwrap()
                .is_none(),
            "EX admission must reject every new database join during TTL recovery"
        );
        let snapshot_started = AtomicBool::new(false);
        let drain = run_with_backup_lock_heartbeat(&database, &backup_lock, async {
            wait_for_maintenance_writers_with_timing(
                &database,
                &backup_lock,
                Duration::from_secs(2),
                Duration::from_millis(10),
            )
            .await?;
            exclusive.revalidate()?;
            anyhow::ensure!(
                database.active_maintenance_write_lease_count().await? == 0,
                "stale worker lease remained active after its TTL drain"
            );
            snapshot_started.store(true, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        });
        tokio::pin!(drain);
        tokio::select! {
            result = &mut drain => panic!("snapshot crossed a live killed-worker lease: {result:?}"),
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
        assert!(
            !snapshot_started.load(Ordering::SeqCst),
            "snapshot began before the killed worker lease TTL elapsed"
        );
        let refreshed_backup_expiry: i64 = sqlx::query_scalar(
            "SELECT expires_at_ms FROM maintenance_locks WHERE name = 'backup' AND token = ?",
        )
        .bind(&backup_lock)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(
            refreshed_backup_expiry > initial_backup_expiry,
            "backup gate heartbeat did not advance during stale-lease recovery"
        );
        drain.await.unwrap();
        assert!(snapshot_started.load(Ordering::SeqCst));
        assert_eq!(
            database
                .active_maintenance_write_lease_count()
                .await
                .unwrap(),
            0
        );
        let stale_expiry: i64 = sqlx::query_scalar(
            "SELECT expires_at_ms FROM maintenance_write_leases WHERE token = ?",
        )
        .bind(&killed_worker_lease)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert!(stale_expiry <= robin_highscores::model::now_epoch_ms().unwrap());
        assert!(
            release_backup_gate_and_close_pool_under_exclusive_fence(
                &database,
                &backup_lock,
                &exclusive,
            )
            .await
            .unwrap()
        );
    }

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

    #[tokio::test]
    async fn backup_space_topology_densifies_sparse_and_tiny_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("objects");
        tokio::fs::create_dir(&root).await.unwrap();
        let allocation = 4096;
        let sparse = std::fs::File::create(root.join("sparse")).unwrap();
        sparse.set_len(allocation + 1).unwrap();
        drop(sparse);
        for index in 0..128 {
            tokio::fs::write(root.join(format!("tiny-{index:03}")), [0x5a])
                .await
                .unwrap();
        }

        let mut topology = BackupCopyTopology::default();
        add_regular_tree_capacity(&root, allocation, &mut topology)
            .await
            .unwrap();
        assert_eq!(topology.regular_files, 129);
        assert_eq!(topology.directories, 1);
        assert_eq!(
            topology.dense_file_bytes,
            2 * allocation + 128 * allocation,
            "logical apparent bytes must be rounded as dense destination allocations"
        );
        assert!(
            topology.dense_file_bytes > allocation + 1 + 128,
            "the strict estimate must not collapse to du --bytes semantics"
        );
        assert_eq!(round_up_to_allocation(0, allocation).unwrap(), 0);
        assert_eq!(round_up_to_allocation(1, allocation).unwrap(), allocation);
        assert_eq!(
            round_up_to_allocation(allocation + 1, allocation).unwrap(),
            2 * allocation
        );
        assert!(round_up_to_allocation(u64::MAX, allocation).is_err());
        let million_tiny_files = 1_000_000_u64;
        let million_dense_bytes = round_up_to_allocation(1, allocation)
            .unwrap()
            .checked_mul(million_tiny_files)
            .unwrap();
        let million_entry_bytes =
            conservative_entry_overhead(million_tiny_files, allocation).unwrap();
        assert_eq!(million_dense_bytes, 4_096_000_000);
        assert_eq!(million_entry_bytes, 4_096_000_000);
        assert!(
            conservative_entry_overhead(u64::MAX, allocation).is_err(),
            "a hostile topology must fail closed on arithmetic overflow"
        );
    }

    fn test_release_identity() -> BackupReleaseIdentityV2 {
        BackupReleaseIdentityV2 {
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            database_schema_version: robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
            vps_release_manifest_sha256: "12".repeat(32),
            publication_lock_sha256: "34".repeat(32),
            installed_user_units: test_release_units(),
        }
    }

    fn test_release_units() -> Vec<robin_highscores::backup::BackupReleaseUnitV2> {
        let mut units = SYSTEMD_UNIT_FILES
            .into_iter()
            .map(|unit| {
                let bytes = format!("fixture {unit}\n");
                robin_highscores::backup::BackupReleaseUnitV2 {
                    release_relative_path: format!("systemd/user/{unit}"),
                    artifact: ArtifactRefV1 {
                        sha256: Digest32::digest_bytes(bytes.as_bytes()),
                        byte_length: u64::try_from(bytes.len()).unwrap(),
                        media_type: "text/plain".to_owned(),
                    },
                    unix_mode: 0o440,
                }
            })
            .collect::<Vec<_>>();
        units.sort_by(|left, right| left.release_relative_path.cmp(&right.release_relative_path));
        units
    }

    async fn write_test_release_manifest(path: &Path) -> BackupReleaseIdentityV2 {
        let mut files = vec![serde_json::json!({
            "artifact": {
                "byte_length": 1,
                "media_type": "application/octet-stream",
                "sha256": "9a".repeat(32),
            },
            "path": "README.md",
            "unix_mode": 0o440,
        })];
        files.extend(test_release_units().into_iter().map(|unit| {
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
            "files": files,
            "publication_lock_sha256": "34".repeat(32),
            "publication_manifest_sha256": "56".repeat(32),
            "schema_version": 2,
            "source_commit": "0123456789abcdef0123456789abcdef01234567",
            "verifier_sha256": "78".repeat(32),
        });
        write_private_file(
            path,
            &robin_run_protocol::canonical_json_bytes(&document).unwrap(),
        )
        .await
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o440)).unwrap();
        load_backup_release_identity_oob(path).await.unwrap()
    }

    #[test]
    fn runtime_authority_probe_has_an_exact_config_free_cli_contract() {
        let command = Arguments::try_parse_from([
            "robin-highscores-admin",
            "--config",
            "/definitely/not/a/runtime-authority-input.toml",
            "probe-runtime-authority-v2",
            "--candidate-release-root-fd",
            "7",
            "--expected-vps-release-manifest-sha256",
            "abababababababababababababababababababababababababababababababab",
            "--backup-authority-state",
            "present",
        ])
        .unwrap()
        .command;
        assert!(matches!(
            command,
            Command::ProbeRuntimeAuthorityV2 {
                candidate_release_root_fd: 7,
                backup_authority_state: BackupAuthorityStateV2::Present,
                ..
            }
        ));
        assert!(
            Arguments::try_parse_from([
                "robin-highscores-admin",
                "probe-runtime-authority-v2",
                "--candidate-release-root-fd",
                "7",
                "--expected-vps-release-manifest-sha256",
                "abababababababababababababababababababababababababababababababab",
                "--backup-authority-state",
                "present",
                "--ambient-config-override",
                "/tmp/not-allowed",
            ])
            .is_err(),
            "the probe must not acquire an ambient override surface"
        );
    }

    #[test]
    fn transaction_and_offline_verifier_modes_have_distinct_typed_cli_contracts() {
        let identity = test_release_identity();
        let common = [
            "--backup-authority-key-fd",
            "6",
            "--expected-source-commit",
            identity.source_commit.as_str(),
            "--expected-vps-release-manifest-sha256",
            identity.vps_release_manifest_sha256.as_str(),
            "--expected-publication-lock-sha256",
            identity.publication_lock_sha256.as_str(),
        ];
        let mut offline = vec!["robin-highscores-admin", "verify-backup"];
        offline.extend(["--backup-root-fd", "4"]);
        offline.extend(["--backup-directory-fd", "3"]);
        offline.extend(common);
        offline.extend([
            "--expected-backup-manifest-sha256",
            "abababababababababababababababababababababababababababababababab",
        ]);
        assert!(matches!(
            Arguments::try_parse_from(offline).unwrap().command,
            Command::VerifyBackup { .. }
        ));

        let mut transaction = vec!["robin-highscores-admin", "verify-transaction-backup"];
        transaction.extend(["--backup-root-fd", "3"]);
        transaction.extend(["--status-envelope-fd", "4"]);
        transaction.extend(["--expected-release-manifest-fd", "5"]);
        transaction.extend(common);
        assert!(matches!(
            Arguments::try_parse_from(transaction.clone())
                .unwrap()
                .command,
            Command::VerifyTransactionBackup { .. }
        ));
        transaction.extend([
            "--expected-backup-manifest-sha256",
            "abababababababababababababababababababababababababababababababab",
        ]);
        assert!(Arguments::try_parse_from(transaction).is_err());

        let mut shell_selected_child = vec!["robin-highscores-admin", "verify-transaction-backup"];
        shell_selected_child.extend(["--backup-directory-fd", "3"]);
        shell_selected_child.extend(["--status-envelope-fd", "4"]);
        shell_selected_child.extend(["--expected-release-manifest-fd", "5"]);
        shell_selected_child.extend(common);
        assert!(
            Arguments::try_parse_from(shell_selected_child).is_err(),
            "transaction callers may pass only the canonical root FD, never a shell-selected child"
        );

        let mut missing_offline_digest = vec!["robin-highscores-admin", "verify-backup"];
        missing_offline_digest.extend(["--backup-root-fd", "4"]);
        missing_offline_digest.extend(["--backup-directory-fd", "3"]);
        missing_offline_digest.extend(common);
        assert!(Arguments::try_parse_from(missing_offline_digest).is_err());
    }

    #[test]
    fn live_schema_probe_has_one_config_free_typed_cli_contract() {
        let digest = "12".repeat(32);
        let arguments = Arguments::try_parse_from([
            "robin-highscores-admin",
            "verify-live-database-schema-v2",
            "--candidate-release-root-fd",
            "3",
            "--expected-vps-release-manifest-sha256",
            digest.as_str(),
        ])
        .unwrap();
        assert!(matches!(
            arguments.command,
            Command::VerifyLiveDatabaseSchemaV2 {
                candidate_release_root_fd: 3,
                expected_vps_release_manifest_sha256,
            } if expected_vps_release_manifest_sha256 == digest
        ));
        assert!(
            Arguments::try_parse_from([
                "robin-highscores-admin",
                "verify-live-database-schema-v2",
                "--candidate-release-root-fd",
                "3",
            ])
            .is_err()
        );
        assert!(
            Arguments::try_parse_from([
                "robin-highscores-admin",
                "verify-live-database-schema-v2",
                "--candidate-release-root-fd",
                "3",
                "--expected-vps-release-manifest-sha256",
                digest.as_str(),
                "--database-path",
                "/tmp/substituted.sqlite3",
            ])
            .is_err(),
            "production live-schema verification must not accept a path override"
        );
    }

    #[tokio::test]
    async fn pinned_restore_sources_reject_path_swaps_and_in_place_mutation() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

        let secret = directory.path().join("cursor-hmac.key");
        write_private_file(&secret, &[0x11; 32]).await.unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o400)).unwrap();
        let pinned =
            pin_restore_source(&secret, 0o400, Some(32), None, "test restore secret").unwrap();
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
            std::fs::set_permissions(&symlink_root, std::fs::Permissions::from_mode(0o700))
                .unwrap();
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

    #[tokio::test]
    async fn verified_backup_install_is_noreplace_and_reports_parent_sync_failure() {
        let directory = tempfile::tempdir().unwrap();
        let racing_partial = directory.path().join("partial-race");
        let racing_complete = directory.path().join("complete-race");
        tokio::fs::create_dir(&racing_partial).await.unwrap();
        write_private_file(&racing_partial.join("new"), b"new")
            .await
            .unwrap();
        tokio::fs::create_dir(&racing_complete).await.unwrap();
        write_private_file(&racing_complete.join("winner"), b"winner")
            .await
            .unwrap();
        assert!(
            install_verified_partial_with(&racing_partial, &racing_complete, || Ok(())).is_err(),
            "an independently installed completed backup must win the publication race"
        );
        assert!(racing_partial.join("new").is_file());
        assert_eq!(
            tokio::fs::read(racing_complete.join("winner"))
                .await
                .unwrap(),
            b"winner"
        );

        let partial = directory.path().join("partial-parent-sync");
        let complete = directory.path().join("complete-parent-sync");
        tokio::fs::create_dir_all(partial.join("nested"))
            .await
            .unwrap();
        write_private_file(&partial.join("nested/bytes"), b"durable bytes")
            .await
            .unwrap();
        let outcome = install_verified_partial_with(&partial, &complete, || {
            anyhow::bail!("injected parent fsync failure")
        })
        .unwrap();
        assert!(matches!(
            outcome,
            BackupInstallOutcome::InstalledButParentSyncFailed(_)
        ));
        assert!(!partial.exists());
        assert_eq!(
            tokio::fs::read(complete.join("nested/bytes"))
                .await
                .unwrap(),
            b"durable bytes"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn durability_sync_rejects_a_symlink_in_the_verified_tree() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let partial = directory.path().join("partial");
        let complete = directory.path().join("complete");
        tokio::fs::create_dir(&partial).await.unwrap();
        write_private_file(&directory.path().join("outside"), b"outside")
            .await
            .unwrap();
        symlink(
            directory.path().join("outside"),
            partial.join("substituted"),
        )
        .unwrap();
        assert!(install_verified_partial_with(&partial, &complete, || Ok(())).is_err());
        assert!(partial.exists());
        assert!(!complete.exists());
    }

    #[cfg(unix)]
    #[test]
    fn status_publication_is_one_atomic_owner_only_file_with_typed_failures() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let root = tempfile::tempdir().unwrap();
        let status_parent = root.path().join("status");
        std::fs::create_dir(&status_parent).unwrap();
        std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let status = status_parent.join("backup-status.json");
        std::fs::write(&status, b"old-envelope").unwrap();
        std::fs::set_permissions(&status, std::fs::Permissions::from_mode(0o400)).unwrap();

        assert!(
            publish_private_atomic_with_hooks(
                &status,
                b"pre-rename-failure",
                sync_cap_directory,
                || anyhow::bail!("injected pre-rename failure"),
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&status).unwrap(), b"old-envelope");
        assert!(
            std::fs::read_dir(&status_parent)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
        );

        let durability = publish_private_atomic_with(&status, b"new-envelope", |_| {
            anyhow::bail!("injected parent fsync failure")
        })
        .unwrap();
        assert!(matches!(
            durability,
            StatusPublicationOutcome::PublishedButParentSyncFailed(_)
        ));
        assert_eq!(std::fs::read(&status).unwrap(), b"new-envelope");

        let identity = publish_private_atomic_with(&status, b"authenticated-envelope", |_| {
            std::fs::remove_file(&status)?;
            std::fs::write(&status, b"substituted")?;
            std::fs::set_permissions(&status, std::fs::Permissions::from_mode(0o400))?;
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            identity,
            StatusPublicationOutcome::PublishedButIdentityUncertain(_)
        ));

        let parent_mode_race = publish_private_atomic_with(&status, b"mode-race", |_| {
            std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o777))?;
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            parent_mode_race,
            StatusPublicationOutcome::PublishedButIdentityUncertain(_)
        ));
        std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700)).unwrap();

        let published = publish_private_atomic(&status, b"canonical-envelope").unwrap();
        assert!(matches!(published, StatusPublicationOutcome::Published));
        assert_eq!(std::fs::read(&status).unwrap(), b"canonical-envelope");
        let metadata = std::fs::metadata(&status).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o400);
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
        std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert!(publish_private_atomic(&status, b"rejected").is_err());
        assert_eq!(std::fs::read(&status).unwrap(), b"canonical-envelope");
    }

    #[cfg(unix)]
    #[test]
    fn status_publication_detects_parent_replacement_after_install() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let status_parent = root.path().join("status");
        let detached_parent = root.path().join("detached-status");
        std::fs::create_dir(&status_parent).unwrap();
        std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let status = status_parent.join("backup-status.json");
        let outcome = publish_private_atomic_with(&status, b"envelope", |_| {
            std::fs::rename(&status_parent, &detached_parent)?;
            std::fs::create_dir(&status_parent)?;
            std::fs::set_permissions(&status_parent, std::fs::Permissions::from_mode(0o700))?;
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            outcome,
            StatusPublicationOutcome::PublishedButIdentityUncertain(_)
        ));
        assert!(!status.exists());
        assert_eq!(
            std::fs::read(detached_parent.join("backup-status.json")).unwrap(),
            b"envelope"
        );
    }

    #[cfg(unix)]
    #[test]
    fn operation_lock_rejects_symlink_hardlink_and_wrong_mode() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(acquire_backup_operation_lock(root.path()).unwrap());
        let lock = root.path().join(".backup-operation.lock");
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(acquire_backup_operation_lock(root.path()).is_err());
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::hard_link(&lock, root.path().join("lock-alias")).unwrap();
        assert!(acquire_backup_operation_lock(root.path()).is_err());
        std::fs::remove_file(root.path().join("lock-alias")).unwrap();
        std::fs::remove_file(&lock).unwrap();
        let outside = root.path().join("outside-lock");
        std::fs::write(&outside, b"").unwrap();
        symlink(&outside, &lock).unwrap();
        assert!(acquire_backup_operation_lock(root.path()).is_err());
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

    async fn test_restore_sources(
        config: &ServerConfig,
        root: &Path,
    ) -> BTreeMap<PathBuf, PathBuf> {
        let mut sources = BTreeMap::new();
        for secret in [
            &config.cursor_secret_path,
            &config.competition_run_grant_secret_path,
            &config.run_preflight_grant_secret_path,
            config.moderation_bearer_token_path.as_ref().unwrap(),
        ] {
            #[cfg(unix)]
            std::fs::set_permissions(secret, std::fs::Permissions::from_mode(0o400)).unwrap();
            sources.insert(secret.clone(), secret.clone());
        }
        let units = root.join("installed-user-units");
        tokio::fs::create_dir(&units).await.unwrap();
        for unit in SYSTEMD_UNIT_FILES {
            let source = units.join(unit);
            write_private_file(&source, format!("fixture {unit}\n").as_bytes())
                .await
                .unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o440)).unwrap();
            sources.insert(Path::new(SYSTEMD_USER_ROOT).join(unit), source);
        }
        sources
    }

    async fn refresh_database_manifest_entry(directory: &Path) {
        let manifest_path = directory.join("backup-manifest.json");
        let mut manifest: BackupManifest =
            serde_json::from_slice(&tokio::fs::read(&manifest_path).await.unwrap()).unwrap();
        let database = record_file(directory, &directory.join("highscores.sqlite3"))
            .await
            .unwrap();
        *manifest
            .files
            .iter_mut()
            .find(|entry| entry.relative_path == "highscores.sqlite3")
            .unwrap() = database;
        tokio::fs::write(&manifest_path, canonical_json_bytes(&manifest).unwrap())
            .await
            .unwrap();
        let backup_id = directory.file_name().unwrap().to_str().unwrap().to_owned();
        let envelope =
            BackupVerificationEnvelopeV2::new_authenticated(backup_id, &manifest, &[0x31; 32])
                .unwrap();
        let envelope_path = directory.join("backup-verification-envelope.json");
        #[cfg(unix)]
        std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        tokio::fs::write(&envelope_path, canonical_json_bytes(&envelope).unwrap())
            .await
            .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    }

    async fn remove_test_database_sidecars(database: &Path) {
        for suffix in ["-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", database.display()));
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("failed to remove test SQLite sidecar: {error}"),
            }
        }
    }

    #[test]
    fn all_secret_bootstraps_load_only_the_requested_path() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let cursor = directory.path().join("cursor.key");
        let competition = directory.path().join("competition.key");
        let preflight = directory.path().join("preflight.key");
        let config_path = directory.path().join("bootstrap.toml");
        std::fs::write(
            &config_path,
            format!(
                "cursor_secret_path = \"{}\"\ncompetition_run_grant_secret_path = \"{}\"\nrun_preflight_grant_secret_path = \"{}\"\ndatabase_path = \"relative-and-invalid-for-final-config\"\n",
                cursor.display(),
                competition.display(),
                preflight.display(),
            ),
        )
        .unwrap();
        let config = load_secret_bootstrap_config(&config_path, "cursor_secret_path").unwrap();
        config.load_or_create_cursor_key().unwrap();
        assert_eq!(std::fs::read(&cursor).unwrap().len(), 32);
        let config =
            load_secret_bootstrap_config(&config_path, "competition_run_grant_secret_path")
                .unwrap();
        assert_eq!(
            config
                .load_or_create_competition_run_grant_key()
                .unwrap()
                .len(),
            32
        );
        let config =
            load_secret_bootstrap_config(&config_path, "run_preflight_grant_secret_path").unwrap();
        assert_eq!(
            config
                .load_or_create_run_preflight_grant_key()
                .unwrap()
                .len(),
            32
        );
        assert_eq!(std::fs::read(competition).unwrap().len(), 32);
        assert_eq!(std::fs::read(preflight).unwrap().len(), 32);
        assert!(
            load_secret_bootstrap_config(&config_path, "backup_authority_hmac_secret_path")
                .is_err(),
            "the non-resumable legacy fifth-key bootstrap must not be reachable"
        );
        assert!(
            load_secret_bootstrap_config(&config_path, "moderation_bearer_token_path").is_err()
        );
    }

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
        config.run_preflight_grant_secret_path =
            directory.path().join("data/run-preflight-grant.key");
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
        let manifest: BackupManifest = serde_json::from_slice(
            &std::fs::read(destination.join("backup-manifest.json")).unwrap(),
        )
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
                    std::fs::set_permissions(
                        &final_tombstone,
                        std::fs::Permissions::from_mode(0o600),
                    )?;
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
            verify_backup_with_expected(
                &destination,
                &verified.manifest_sha256,
                &release_identity,
            )
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

    #[tokio::test]
    async fn publication_is_authenticated_atomic_and_keeps_a_complete_backup() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data");
        let configuration = directory.path().join("configuration");
        let release_manifest = directory.path().join("vps-release-manifest-v2.json");
        let backup_root = data.join("backups");
        let api_secrets = data.join("api-secrets");
        let status_root = data.join("status");
        let status_path = status_root.join("backup-status.json");
        tokio::fs::create_dir_all(&data).await.unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
        tokio::fs::create_dir(&status_root).await.unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        tokio::fs::create_dir(&api_secrets).await.unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&api_secrets, std::fs::Permissions::from_mode(0o700)).unwrap();
        tokio::fs::create_dir(&configuration).await.unwrap();
        write_private_file(&configuration.join("server.toml"), b"bind = 'loopback'\n")
            .await
            .unwrap();
        write_private_file(
            &configuration.join("api-moderation.token"),
            b"private-token",
        )
        .await
        .unwrap();
        let release_identity = write_test_release_manifest(&release_manifest).await;

        let mut config = ServerConfig::default();
        config.database_path = data.join("highscores.sqlite3");
        config.replay_directory = data.join("replays");
        config.campaign_state_directory = data.join("campaigns");
        config.cursor_secret_path = data.join("cursor-hmac.key");
        config.backup_authority_hmac_secret_path = api_secrets.join("backup-authority-hmac.key");
        config.competition_run_grant_secret_path = data.join("competition-run-grant.key");
        config.run_preflight_grant_secret_path = data.join("run-preflight-grant.key");
        config.moderation_bearer_token_path = Some(configuration.join("api-moderation.token"));
        config.backup_manifest_path = Some(status_path.clone());
        config.release_manifest_path = Some(release_manifest.clone());
        config.maximum_backup_age_hours = Some(32);
        config.minimum_storage_free_bytes = 64 * 1024 * 1024;
        write_private_file(&config.cursor_secret_path, &[0x31; 32])
            .await
            .unwrap();
        write_private_file(&config.backup_authority_hmac_secret_path, &[0x31; 32])
            .await
            .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(
            &config.backup_authority_hmac_secret_path,
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        write_private_file(&config.competition_run_grant_secret_path, &[0x32; 32])
            .await
            .unwrap();
        write_private_file(&config.run_preflight_grant_secret_path, &[0x33; 32])
            .await
            .unwrap();
        Database::migrate(&config).await.unwrap();
        let restore_sources = test_restore_sources(&config, directory.path()).await;
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
            )
            .await
            .is_err(),
            "backup authority must reject a missing production backup root"
        );
        assert!(
            !backup_root.exists(),
            "backup authority must not provision a missing production backup root"
        );
        tokio::fs::create_dir(&backup_root).await.unwrap();
        #[cfg(unix)]
        {
            std::fs::set_permissions(&backup_root, std::fs::Permissions::from_mode(0o750)).unwrap();
            assert!(
                backup_and_publish_status(
                    &config,
                    &release_manifest,
                    &release_identity,
                    &backup_root,
                    &status_path,
                    2,
                    &restore_sources,
                )
                .await
                .is_err(),
                "backup authority must reject a misprovisioned backup root"
            );
            assert_eq!(
                std::fs::metadata(&backup_root)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o750,
                "backup authority must not repair production root metadata"
            );
            std::fs::set_permissions(&backup_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let admission_partial =
            backup_root.join(format!(".backup-v4-3-{}.partial", "c".repeat(32)));
        tokio::fs::create_dir(&admission_partial).await.unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&admission_partial, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        write_private_file(&admission_partial.join("must-remain"), b"pre-admission")
            .await
            .unwrap();
        #[cfg(unix)]
        {
            std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o750)).unwrap();
            assert!(
                backup_and_publish_status(
                    &config,
                    &release_manifest,
                    &release_identity,
                    &backup_root,
                    &status_path,
                    2,
                    &restore_sources,
                )
                .await
                .is_err(),
                "a non-0700 status authority parent must fail before cleanup"
            );
            assert!(admission_partial.join("must-remain").is_file());
            std::fs::set_permissions(&status_root, std::fs::Permissions::from_mode(0o700)).unwrap();

            std::fs::write(&status_path, b"{}").unwrap();
            std::fs::set_permissions(&status_path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                backup_and_publish_status(
                    &config,
                    &release_manifest,
                    &release_identity,
                    &backup_root,
                    &status_path,
                    2,
                    &restore_sources,
                )
                .await
                .is_err(),
                "a non-0400 status authority must fail before cleanup"
            );
            assert!(admission_partial.join("must-remain").is_file());
            std::fs::remove_file(&status_path).unwrap();
        }
        let mutable_estimate = estimate_backup_space(
            &config,
            &release_identity,
            &backup_root,
            &status_path,
            &restore_sources,
        )
        .await
        .unwrap();
        let estimate_bytes = canonical_json_bytes(&mutable_estimate).unwrap();
        assert_eq!(
            serde_json::from_slice::<BackupSpaceEstimateV1>(&estimate_bytes).unwrap(),
            mutable_estimate,
            "deployment evidence must round-trip as one canonical typed document"
        );
        assert!(
            mutable_estimate.manifest_logical_upper_bound_bytes > 0
                && mutable_estimate.status_temp_logical_upper_bound_bytes
                    < mutable_estimate.manifest_logical_upper_bound_bytes
                && mutable_estimate.status_temp_logical_upper_bound_bytes
                    <= u64::try_from(robin_highscores::backup::MAX_BACKUP_STATUS_BYTES).unwrap(),
            "the estimator must size the full payload manifest and compact status independently"
        );
        assert_eq!(
            mutable_estimate.concurrent_database_margin_bytes,
            round_up_to_allocation(
                robin_highscores::storage_admission::maximum_capacity_demand_bytes(&config)
                    .unwrap()
                    .database,
                mutable_estimate.allocation_granularity_bytes,
            )
            .unwrap()
        );
        assert_eq!(
            mutable_estimate.concurrent_object_margin_bytes,
            u64::try_from(config.max_concurrent_uploads).unwrap()
                * round_up_to_allocation(
                    config.max_replay_bytes,
                    mutable_estimate.allocation_granularity_bytes,
                )
                .unwrap()
                + (u64::try_from(config.max_concurrent_uploads).unwrap() + 1)
                    * round_up_to_allocation(
                        config.max_campaign_bytes,
                        mutable_estimate.allocation_granularity_bytes,
                    )
                    .unwrap(),
            "every concurrently admitted replay/campaign pair must fit after the scan"
        );
        assert_eq!(
            mutable_estimate.restore_source_map_count,
            u64::try_from(restore_sources.len()).unwrap()
        );
        assert_eq!(
            mutable_estimate.required_scratch_bytes,
            mutable_estimate.dense_payload_bytes
                + mutable_estimate.directory_and_entry_overhead_bytes
                + mutable_estimate.manifest_allocation_upper_bound_bytes
                + mutable_estimate.status_temp_allocation_upper_bound_bytes
                + mutable_estimate.concurrent_object_margin_bytes
                + mutable_estimate.concurrent_database_margin_bytes,
            "one scratch generation must include exact documents plus bounded in-flight growth"
        );
        let mut insufficient = mutable_estimate.clone();
        insufficient.observed_available_bytes = insufficient.required_available_bytes - 1;
        assert!(insufficient.ensure_available().is_err());
        let mut inode_pressure = mutable_estimate.clone();
        inode_pressure.observed_available_inode_count = inode_pressure.required_inode_count - 1;
        assert!(inode_pressure.ensure_available().is_err());
        assert_eq!(
            std::fs::read_dir(&backup_root).unwrap().count(),
            1,
            "capacity rejection must not create anything beyond the pre-admission partial"
        );
        assert!(admission_partial.join("must-remain").is_file());
        let immutable_release_bytes = directory.path().join("immutable-release");
        tokio::fs::create_dir_all(immutable_release_bytes.join("static/datadir"))
            .await
            .unwrap();
        write_private_file(
            &immutable_release_bytes.join("static/datadir/not-a-backup-input"),
            &vec![0x5a; 128 * 1024],
        )
        .await
        .unwrap();
        config.manifest_directory = Some(immutable_release_bytes.join("manifests"));
        let estimate_with_immutable_release = estimate_backup_space(
            &config,
            &release_identity,
            &backup_root,
            &status_path,
            &restore_sources,
        )
        .await
        .unwrap();
        config.manifest_directory = None;
        let contemporaneous_without_immutable_release = estimate_backup_space(
            &config,
            &release_identity,
            &backup_root,
            &status_path,
            &restore_sources,
        )
        .await
        .unwrap();
        assert_eq!(
            estimate_with_immutable_release.required_scratch_bytes,
            contemporaneous_without_immutable_release.required_scratch_bytes,
            "release/static/datadir and manifest roots must not consume mutable backup capacity"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let unit_root = directory.path().join("installed-user-units");
            tokio::fs::create_dir(unit_root.join("default.target.wants"))
                .await
                .unwrap();
            tokio::fs::create_dir(unit_root.join("timers.target.wants"))
                .await
                .unwrap();
            symlink(
                unit_root.join("robin-highscores.target"),
                unit_root
                    .join("default.target.wants")
                    .join("robin-highscores.target"),
            )
            .unwrap();
            symlink(
                unit_root.join("robin-highscores-backup.timer"),
                unit_root
                    .join("timers.target.wants")
                    .join("robin-highscores-backup.timer"),
            )
            .unwrap();
            write_private_file(
                &unit_root.join("unrelated.service"),
                b"must not be archived",
            )
            .await
            .unwrap();
        }

        assert!(!status_path.exists());
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                0,
                &restore_sources,
            )
            .await
            .is_err()
        );
        assert!(!status_path.exists());

        let stale_partial = backup_root.join(format!(".backup-v4-1-{}.partial", "a".repeat(32)));
        tokio::fs::create_dir_all(stale_partial.join("restore/state"))
            .await
            .unwrap();
        write_private_file(&stale_partial.join("restore/state/interrupted"), b"sigkill")
            .await
            .unwrap();
        #[cfg(unix)]
        {
            std::fs::set_permissions(&stale_partial, std::fs::Permissions::from_mode(0o500))
                .unwrap();
            std::fs::set_permissions(
                stale_partial.join("restore"),
                std::fs::Permissions::from_mode(0o500),
            )
            .unwrap();
            std::fs::set_permissions(
                stale_partial.join("restore/state"),
                std::fs::Permissions::from_mode(0o500),
            )
            .unwrap();

            let key_path = config.backup_authority_hmac_secret_path.clone();
            let displaced_key = api_secrets.join("backup-authority-hmac.displaced");
            let swap_key_path = key_path.clone();
            let swap_displaced_key = displaced_key.clone();
            assert!(
                backup_and_publish_status_with_limit_and_publisher_and_hooks(
                    &config,
                    &release_manifest,
                    &release_identity,
                    &backup_root,
                    &status_path,
                    2,
                    &restore_sources,
                    robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                    publish_private_atomic,
                    move || {
                        std::fs::rename(&swap_key_path, &swap_displaced_key)?;
                        std::fs::write(&swap_key_path, [0x41; 32])?;
                        std::fs::set_permissions(
                            &swap_key_path,
                            std::fs::Permissions::from_mode(0o400),
                        )?;
                        Ok(())
                    },
                    || Ok(()),
                )
                .await
                .is_err(),
                "a pathname replacement of the pinned key must fail before backup installation"
            );
            assert!(!status_path.exists());
            assert!(
                std::fs::read_dir(&backup_root).unwrap().all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("backup-v4-")
                }),
                "a key swap before install must not leave a completed generation"
            );
            std::fs::remove_file(&key_path).unwrap();
            std::fs::rename(&displaced_key, &key_path).unwrap();

            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let key_mutator = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&key_path)
                .unwrap();
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o400)).unwrap();
            let mutation_handle = key_mutator.try_clone().unwrap();
            assert!(
                backup_and_publish_status_with_limit_and_publisher_and_hooks(
                    &config,
                    &release_manifest,
                    &release_identity,
                    &backup_root,
                    &status_path,
                    2,
                    &restore_sources,
                    robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                    publish_private_atomic,
                    || Ok(()),
                    move || {
                        use std::os::unix::fs::FileExt as _;
                        mutation_handle.write_all_at(&[0x42; 32], 0)?;
                        mutation_handle.sync_all()?;
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "an in-place key mutation must fail before status publication"
            );
            assert!(!status_path.exists());
            assert!(
                std::fs::read_dir(&backup_root).unwrap().all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("backup-v4-")
                }),
                "a key mutation before status publication must remove the unreferenced generation"
            );
            {
                use std::os::unix::fs::FileExt as _;
                key_mutator.write_all_at(&[0x31; 32], 0).unwrap();
                key_mutator.sync_all().unwrap();
            }
        }

        let first = backup_and_publish_status(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
        )
        .await
        .unwrap();
        assert!(!stale_partial.exists());
        assert!(!admission_partial.exists());
        assert!(first.is_dir());
        let status_bytes = tokio::fs::read(&status_path).await.unwrap();
        let status: BackupStatusV4 = serde_json::from_slice(&status_bytes).unwrap();
        assert_eq!(canonical_json_bytes(&status).unwrap(), status_bytes);
        status.verify(&[0x31; 32]).unwrap();
        assert_eq!(status.release_identity, release_identity);
        assert_eq!(Path::new(&status.backup_directory), first);
        assert!(!status_root.join("backup-manifest.json").exists());
        verify_backup(&first).await.unwrap();
        let first_manifest: BackupManifest = serde_json::from_slice(
            &tokio::fs::read(first.join("backup-manifest.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            status.backup_manifest_sha256,
            first_manifest.sha256().unwrap()
        );
        assert_eq!(
            status.database_schema_version,
            first_manifest.database_schema_version
        );
        assert_eq!(status.file_count, first_manifest.files.len() as u64);
        assert_eq!(status.total_bytes, first_manifest.total_bytes().unwrap());
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd as _;

            let backup_directory_file = std::fs::File::open(&first).unwrap();
            let status_file = std::fs::File::open(&status_path).unwrap();
            let release_file = std::fs::File::open(&release_manifest).unwrap();
            assert_eq!(
                load_backup_release_identity_oob_file(release_file)
                    .await
                    .unwrap(),
                release_identity
            );
            let release_hardlink = directory.path().join("release-manifest-hardlink.json");
            std::fs::hard_link(&release_manifest, &release_hardlink).unwrap();
            assert!(
                load_backup_release_identity_oob_file(
                    std::fs::File::open(&release_manifest).unwrap()
                )
                .await
                .is_err(),
                "a hard-linked release authority must be rejected"
            );
            std::fs::remove_file(release_hardlink).unwrap();
            std::fs::set_permissions(&release_manifest, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            assert!(
                load_backup_release_identity_oob_file(
                    std::fs::File::open(&release_manifest).unwrap()
                )
                .await
                .is_err(),
                "a release authority with the wrong mode must be rejected"
            );
            std::fs::set_permissions(&release_manifest, std::fs::Permissions::from_mode(0o440))
                .unwrap();
            let wrong_release_name = directory.path().join("not-the-release-manifest.json");
            std::fs::copy(&release_manifest, &wrong_release_name).unwrap();
            std::fs::set_permissions(&wrong_release_name, std::fs::Permissions::from_mode(0o440))
                .unwrap();
            assert!(
                require_pinned_file_name(
                    &std::fs::File::open(wrong_release_name).unwrap(),
                    "vps-release-manifest-v2.json",
                )
                .is_err(),
                "the out-of-band release descriptor must retain its canonical filename"
            );
            let verification = verify_backup_pinned_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                u32::try_from(status_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &backup_root,
                &status_path,
                false,
            )
            .await
            .unwrap();
            let receipt = &verification.receipt;
            receipt.validate().unwrap();
            assert_eq!(receipt.backup_id, status.backup_id);
            assert_eq!(receipt.backup_directory, status.backup_directory);
            let receipt_bytes = canonical_json_bytes(&receipt).unwrap();
            assert!(!receipt_bytes.ends_with(b"\n"));
            assert_eq!(
                canonical_json_bytes(
                    &serde_json::from_slice::<BackupVerificationReceiptV2>(&receipt_bytes).unwrap()
                )
                .unwrap(),
                receipt_bytes,
                "successful verifier stdout is an exact canonical receipt"
            );
            let explicit_receipt = receipt.clone();
            drop(verification);

            let historical = verify_backup_pinned_offline_with_expected(
                u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                &first_manifest.sha256().unwrap(),
                &release_identity,
                &[0x31; 32],
                &backup_root,
                false,
            )
            .await
            .unwrap();
            assert!(
                historical.receipt.current_status.is_none(),
                "historical verification must be independent of singleton latest status"
            );
            drop(historical);

            let backup_root_file = std::fs::File::open(&backup_root).unwrap();
            let (transaction_verification, transaction_root_guard) =
                verify_transaction_backup_from_root_pinned(
                    u32::try_from(backup_root_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &release_identity,
                    &[0x31; 32],
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .unwrap();
            assert_eq!(
                transaction_verification.receipt, explicit_receipt,
                "transaction mode derives exactly the authenticated status manifest digest"
            );
            drop(transaction_verification);
            drop(transaction_root_guard);

            let wrong_root = directory.path().join("wrong-parent/backups");
            std::fs::create_dir_all(&wrong_root).unwrap();
            std::fs::set_permissions(&wrong_root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let wrong_root_file = std::fs::File::open(&wrong_root).unwrap();
            assert!(
                pin_transaction_backup_from_status(
                    u32::try_from(wrong_root_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &backup_root,
                    &status_path,
                )
                .is_err(),
                "a same-basename backup root outside the canonical authority must be rejected"
            );

            let malicious_status_root = directory.path().join("malicious-path-status");
            std::fs::create_dir(&malicious_status_root).unwrap();
            let malicious_status_path = malicious_status_root.join("backup-status.json");
            let mut malicious_status = status.clone();
            malicious_status.backup_directory = directory
                .path()
                .join("outside")
                .join(&status.backup_id)
                .to_string_lossy()
                .into_owned();
            std::fs::write(
                &malicious_status_path,
                canonical_json_bytes(&malicious_status).unwrap(),
            )
            .unwrap();
            std::fs::set_permissions(
                &malicious_status_path,
                std::fs::Permissions::from_mode(0o400),
            )
            .unwrap();
            let malicious_status_file = std::fs::File::open(&malicious_status_path).unwrap();
            assert!(
                pin_transaction_backup_from_status(
                    u32::try_from(backup_root_file.as_raw_fd()).unwrap(),
                    u32::try_from(malicious_status_file.as_raw_fd()).unwrap(),
                    &backup_root,
                    &malicious_status_path,
                )
                .is_err(),
                "an embedded backup path outside canonical root/ID must never be opened"
            );

            let canonical_lock = backup_root.join(".backup-operation.lock");
            let displaced_lock = backup_root.join(".displaced-backup-operation.lock");
            let second_lock = std::sync::Mutex::new(None);
            assert!(
                verify_backup_pinned_with_expected_and_hook(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                    || {
                        std::fs::rename(&canonical_lock, &displaced_lock)?;
                        *second_lock.lock().unwrap() =
                            Some(acquire_backup_operation_lock(&backup_root)?);
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "a replaced lock domain must not yield a receipt"
            );
            let replacement_lock = second_lock.into_inner().unwrap();
            assert!(
                replacement_lock.is_some(),
                "the replacement inode demonstrates a second concurrently acquirable lock domain"
            );
            drop(replacement_lock);
            std::fs::remove_file(&canonical_lock).unwrap();
            std::fs::rename(&displaced_lock, &canonical_lock).unwrap();

            let displaced_status = status_root.join("displaced-backup-status.json");
            assert!(
                verify_backup_pinned_with_expected_and_hook(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                    || {
                        std::fs::rename(&status_path, &displaced_status)?;
                        std::fs::write(&status_path, &status_bytes)?;
                        std::fs::set_permissions(
                            &status_path,
                            std::fs::Permissions::from_mode(0o400),
                        )?;
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "a concurrently replaced status path must not yield a receipt"
            );
            std::fs::remove_file(&status_path).unwrap();
            std::fs::rename(&displaced_status, &status_path).unwrap();

            let displaced_backup = backup_root.join("displaced-complete-backup");
            assert!(
                verify_backup_pinned_with_expected_and_hook(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                    || {
                        std::fs::rename(&first, &displaced_backup)?;
                        std::fs::create_dir(&first)?;
                        std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o700))?;
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "a concurrently pruned or replaced backup path must not yield a receipt"
            );
            std::fs::remove_dir(&first).unwrap();
            std::fs::rename(&displaced_backup, &first).unwrap();

            for (relative, expected_mode, label) in [
                ("restore/state/cursor-hmac.key", 0o600, "payload"),
                ("backup-manifest.json", 0o600, "manifest"),
                (
                    "backup-verification-envelope.json",
                    0o400,
                    "verification envelope",
                ),
            ] {
                let original = first.join(relative);
                let displaced =
                    backup_root.join(format!("displaced-{}", relative.replace('/', "-")));
                let bytes = std::fs::read(&original).unwrap();
                assert!(
                    verify_backup_pinned_with_expected_and_hook(
                        u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                        u32::try_from(status_file.as_raw_fd()).unwrap(),
                        BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                        &release_identity,
                        &backup_root,
                        &status_path,
                        false,
                        || {
                            std::fs::rename(&original, &displaced)?;
                            std::fs::write(&original, &bytes)?;
                            std::fs::set_permissions(
                                &original,
                                std::fs::Permissions::from_mode(expected_mode),
                            )?;
                            Ok(())
                        },
                    )
                    .await
                    .is_err(),
                    "an identical-byte {label} inode substitution must not yield a receipt"
                );
                std::fs::remove_file(&original).unwrap();
                std::fs::rename(&displaced, &original).unwrap();
            }

            let empty_directory = first.join("replays");
            assert_eq!(std::fs::read_dir(&empty_directory).unwrap().count(), 0);
            let displaced_empty_directory = backup_root.join("displaced-empty-replays");
            assert!(
                verify_backup_pinned_with_expected_and_hook(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                    || {
                        std::fs::rename(&empty_directory, &displaced_empty_directory)?;
                        std::fs::create_dir(&empty_directory)?;
                        std::fs::set_permissions(
                            &empty_directory,
                            std::fs::Permissions::from_mode(0o700),
                        )?;
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "an identical empty-directory inode substitution must not yield a receipt"
            );
            std::fs::remove_dir(&empty_directory).unwrap();
            std::fs::rename(&displaced_empty_directory, &empty_directory).unwrap();

            let original_database = first.join("highscores.sqlite3");
            let displaced_database = backup_root.join("displaced-backup-database");
            let prepared_database = backup_root.join("prepared-backup-database");
            std::fs::copy(&original_database, &prepared_database).unwrap();
            std::fs::set_permissions(&prepared_database, std::fs::Permissions::from_mode(0o600))
                .unwrap();
            let prepared_url = format!("sqlite://{}", prepared_database.display());
            let mut prepared_connection = sqlx::SqliteConnection::connect(&prepared_url)
                .await
                .unwrap();
            sqlx::query("PRAGMA user_version = 17")
                .execute(&mut prepared_connection)
                .await
                .unwrap();
            prepared_connection.close().await.unwrap();
            remove_test_database_sidecars(&prepared_database).await;
            assert!(
                verify_backup_pinned_with_expected_and_hook(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    BackupManifestExpectation::Explicit(&first_manifest.sha256().unwrap()),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                    || {
                        std::fs::rename(&original_database, &displaced_database)?;
                        std::fs::rename(&prepared_database, &original_database)?;
                        Ok(())
                    },
                )
                .await
                .is_err(),
                "a different-byte valid-schema database substitution must not yield a receipt"
            );
            std::fs::remove_file(&original_database).unwrap();
            std::fs::rename(&displaced_database, &original_database).unwrap();

            let release_guard = std::fs::File::open(&release_manifest).unwrap();
            let displaced_release = directory.path().join("displaced-release-manifest.json");
            let release_bytes = std::fs::read(&release_manifest).unwrap();
            std::fs::rename(&release_manifest, &displaced_release).unwrap();
            std::fs::write(&release_manifest, &release_bytes).unwrap();
            std::fs::set_permissions(&release_manifest, std::fs::Permissions::from_mode(0o440))
                .unwrap();
            assert!(
                revalidate_pinned_regular_path(
                    &release_guard,
                    &release_manifest,
                    0o440,
                    None,
                    "out-of-band release manifest",
                )
                .is_err(),
                "a concurrently replaced release authority must not precede receipt stdout"
            );
            std::fs::remove_file(&release_manifest).unwrap();
            std::fs::rename(displaced_release, &release_manifest).unwrap();

            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "a regular file cannot substitute the pinned backup directory"
            );
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "a directory cannot substitute the pinned status envelope"
            );

            let wrong_name_path = status_root.join("not-backup-status.json");
            tokio::fs::write(&wrong_name_path, &status_bytes)
                .await
                .unwrap();
            std::fs::set_permissions(&wrong_name_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            let wrong_name_file = std::fs::File::open(&wrong_name_path).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(wrong_name_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &wrong_name_path,
                    false,
                )
                .await
                .is_err(),
                "a differently named status descriptor must be rejected"
            );

            let wrong_mode_root = directory.path().join("wrong-mode-status");
            tokio::fs::create_dir(&wrong_mode_root).await.unwrap();
            let wrong_mode_path = wrong_mode_root.join("backup-status.json");
            tokio::fs::write(&wrong_mode_path, &status_bytes)
                .await
                .unwrap();
            std::fs::set_permissions(&wrong_mode_path, std::fs::Permissions::from_mode(0o600))
                .unwrap();
            let wrong_mode_file = std::fs::File::open(&wrong_mode_path).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(wrong_mode_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &wrong_mode_path,
                    false,
                )
                .await
                .is_err(),
                "a non-0400 status descriptor must be rejected"
            );

            let noncanonical_root = directory.path().join("noncanonical-status");
            tokio::fs::create_dir(&noncanonical_root).await.unwrap();
            let noncanonical_path = noncanonical_root.join("backup-status.json");
            let mut noncanonical_bytes = status_bytes.clone();
            noncanonical_bytes.push(b'\n');
            tokio::fs::write(&noncanonical_path, noncanonical_bytes)
                .await
                .unwrap();
            std::fs::set_permissions(&noncanonical_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            let noncanonical_file = std::fs::File::open(&noncanonical_path).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(noncanonical_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &noncanonical_path,
                    false,
                )
                .await
                .is_err(),
                "a noncanonical status document must be rejected"
            );

            let same_name_root = directory.path().join("substituted-root");
            let same_name_directory = same_name_root.join(&status.backup_id);
            tokio::fs::create_dir_all(&same_name_directory)
                .await
                .unwrap();
            std::fs::set_permissions(&same_name_directory, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            let same_name_file = std::fs::File::open(&same_name_directory).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(same_name_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "matching basenames cannot substitute a different pinned backup root"
            );

            let deleted_status_root = directory.path().join("deleted-status");
            tokio::fs::create_dir(&deleted_status_root).await.unwrap();
            let deleted_status_path = deleted_status_root.join("backup-status.json");
            tokio::fs::write(&deleted_status_path, &status_bytes)
                .await
                .unwrap();
            std::fs::set_permissions(&deleted_status_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            let deleted_status_file = std::fs::File::open(&deleted_status_path).unwrap();
            tokio::fs::remove_file(&deleted_status_path).await.unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(deleted_status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &deleted_status_path,
                    false,
                )
                .await
                .is_err(),
                "an unlinked status-envelope descriptor has no durable path identity"
            );

            let status_hardlink = status_root.join("backup-status-hardlink");
            std::fs::hard_link(&status_path, &status_hardlink).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "a hard-linked status authority must be rejected"
            );
            std::fs::remove_file(status_hardlink).unwrap();

            let bad_status_root = directory.path().join("bad-status");
            tokio::fs::create_dir(&bad_status_root).await.unwrap();
            let bad_status_path = bad_status_root.join("backup-status.json");
            let bad_status = BackupStatusV4::new_authenticated(
                status.backup_id.clone(),
                status.backup_directory.clone(),
                first_manifest.clone(),
                &[0x99; 32],
            )
            .unwrap();
            tokio::fs::write(&bad_status_path, canonical_json_bytes(&bad_status).unwrap())
                .await
                .unwrap();
            std::fs::set_permissions(&bad_status_path, std::fs::Permissions::from_mode(0o400))
                .unwrap();
            let bad_status_file = std::fs::File::open(&bad_status_path).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(bad_status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "the status descriptor must come from the exact trusted parent"
            );
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(bad_status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &bad_status_path,
                    false,
                )
                .await
                .is_err(),
                "a status envelope authenticated by a different key must be rejected"
            );

            let mut wrong_schema_manifest = first_manifest.clone();
            wrong_schema_manifest.database_schema_version += 1;
            assert!(
                BackupStatusV4::new_authenticated(
                    status.backup_id.clone(),
                    status.backup_directory.clone(),
                    wrong_schema_manifest,
                    &[0x31; 32],
                )
                .is_err(),
                "status authoring must reject a database/release schema mismatch"
            );

            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &"ab".repeat(32),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "the pinned payload must match the independently expected manifest digest"
            );

            let mut wrong_release = release_identity.clone();
            wrong_release.source_commit = "abcdef0123456789abcdef0123456789abcdef01".to_owned();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &wrong_release,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "the pinned payload must match the out-of-band release identity"
            );

            std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o750)).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "a non-0700 pinned backup root must be rejected"
            );
            std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o700)).unwrap();

            let missing_path = first.join("restore/state/cursor-hmac.key");
            let hidden_path = first.join("restore/state/cursor-hmac.key.missing");
            std::fs::rename(&missing_path, &hidden_path).unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "a missing pinned payload object must be rejected"
            );
            std::fs::rename(hidden_path, missing_path).unwrap();

            let unexpected_path = first.join("restore/state/unexpected");
            write_private_file(&unexpected_path, b"unexpected")
                .await
                .unwrap();
            assert!(
                verify_backup_pinned_with_expected(
                    u32::try_from(backup_directory_file.as_raw_fd()).unwrap(),
                    u32::try_from(status_file.as_raw_fd()).unwrap(),
                    &first_manifest.sha256().unwrap(),
                    &release_identity,
                    &backup_root,
                    &status_path,
                    false,
                )
                .await
                .is_err(),
                "an unexpected pinned payload object must be rejected"
            );
            std::fs::remove_file(unexpected_path).unwrap();
        }
        let archived_units = first_manifest
            .files
            .iter()
            .filter(|file| file.relative_path.starts_with("restore/systemd/user/"))
            .map(|file| file.relative_path.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(archived_units.len(), SYSTEMD_UNIT_FILES.len());
        assert!(
            archived_units
                .iter()
                .all(|path| !path.contains("wants") && !path.contains("unrelated"))
        );
        assert!(first_manifest.files.iter().all(|file| {
            !file.relative_path.contains("release")
                && !file.relative_path.contains("static")
                && !file.relative_path.contains("datadir")
                && !file.relative_path.contains("manifest")
        }));

        let complete_names_before_publication_failure = std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| parse_backup_id(name).is_some())
            .collect::<BTreeSet<_>>();
        let status_before_publication_failure = tokio::fs::read(&status_path).await.unwrap();
        assert!(
            backup_and_publish_status_with_limit_and_publisher(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
                robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                |_, _| anyhow::bail!("injected definite pre-rename publication failure"),
            )
            .await
            .is_err()
        );
        assert_eq!(
            tokio::fs::read(&status_path).await.unwrap(),
            status_before_publication_failure,
            "a definite pre-rename error must leave the old envelope intact"
        );
        assert_eq!(
            std::fs::read_dir(&backup_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| parse_backup_id(name).is_some())
                .collect::<BTreeSet<_>>(),
            complete_names_before_publication_failure,
            "a definite status error must not accumulate an unreferenced complete backup"
        );

        let status_before_oversize = tokio::fs::read(&status_path).await.unwrap();
        assert!(
            backup_and_publish_status_with_limit(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
                1,
            )
            .await
            .is_err(),
            "an unpublishable envelope must fail before partial installation"
        );
        assert_eq!(
            tokio::fs::read(&status_path).await.unwrap(),
            status_before_oversize,
            "an oversized candidate must not replace the old readiness envelope"
        );
        assert!(
            std::fs::read_dir(&backup_root)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".partial")),
            "an oversized candidate must leave no partial backup"
        );

        let second = backup_and_publish_status(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
        )
        .await
        .unwrap();
        assert!(second.is_dir());
        assert_ne!(first, second);
        assert!(first.exists());
        verify_backup(&second).await.unwrap();
        let two_complete_names = std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| parse_backup_id(name).is_some())
            .collect::<BTreeSet<_>>();
        assert_eq!(two_complete_names.len(), 2);
        assert!(
            backup_and_publish_status_with_limit_and_publisher(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
                robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                |_, _| anyhow::bail!("injected failed replacement before status publication"),
            )
            .await
            .is_err()
        );
        assert_eq!(
            std::fs::read_dir(&backup_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| parse_backup_id(name).is_some())
                .collect::<BTreeSet<_>>(),
            two_complete_names,
            "a failed replacement must preserve both retained complete generations"
        );

        assert!(
            backup_and_publish_status_with_limit_and_publisher(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &status_path,
                2,
                &restore_sources,
                robin_highscores::backup::MAX_BACKUP_STATUS_BYTES,
                |path, bytes| match publish_private_atomic(path, bytes)? {
                    StatusPublicationOutcome::Published => {
                        Ok(StatusPublicationOutcome::PublishedButIdentityUncertain(
                            anyhow::anyhow!(
                                "injected crash after status publication and before retention"
                            ),
                        ))
                    }
                    uncertain => Ok(uncertain),
                },
            )
            .await
            .is_err(),
            "publication uncertainty must preserve the truthful newly published generation"
        );
        let crash_status: BackupStatusV4 =
            serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();
        crash_status.verify(&[0x31; 32]).unwrap();
        let crash_generation = PathBuf::from(&crash_status.backup_directory);
        assert!(crash_generation.is_dir());
        assert_eq!(
            std::fs::read_dir(&backup_root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| parse_backup_id(&entry.file_name().to_string_lossy()).is_some())
                .count(),
            3,
            "a crash after status publication may leave exactly retain+1 generations"
        );

        let third = backup_and_publish_status(
            &config,
            &release_manifest,
            &release_identity,
            &backup_root,
            &status_path,
            2,
            &restore_sources,
        )
        .await
        .unwrap();
        assert!(third.is_dir());
        assert!(!first.exists());
        assert!(!second.exists());
        assert!(crash_generation.exists());
        verify_backup(&third).await.unwrap();
        let managed_count = std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("backup-v4-")
            })
            .count();
        assert_eq!(managed_count, 2);
        assert!(
            std::fs::read_dir(&status_root)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    !name.starts_with(".backup-status.json-")
                })
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = directory.path().join("outside-partial-target");
            tokio::fs::create_dir(&outside).await.unwrap();
            write_private_file(&outside.join("preserved"), b"outside")
                .await
                .unwrap();
            let hostile_partial =
                backup_root.join(format!(".backup-v4-2-{}.partial", "b".repeat(32)));
            symlink(&outside, &hostile_partial).unwrap();
            assert!(
                backup_and_publish_status(
                    &config,
                    &release_manifest,
                    &release_identity,
                    &backup_root,
                    &status_path,
                    2,
                    &restore_sources,
                )
                .await
                .is_err()
            );
            assert_eq!(
                tokio::fs::read(outside.join("preserved")).await.unwrap(),
                b"outside"
            );
            tokio::fs::remove_file(hostile_partial).await.unwrap();
        }

        let nested_status = backup_root.join("backup-status.json");
        config.backup_manifest_path = Some(nested_status.clone());
        assert!(
            backup_and_publish_status(
                &config,
                &release_manifest,
                &release_identity,
                &backup_root,
                &nested_status,
                2,
                &restore_sources,
            )
            .await
            .is_err(),
            "backup payload roots must never double as the API-readable status authority"
        );
    }
}
