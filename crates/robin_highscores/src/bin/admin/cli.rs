//! Typed command parsing and dispatch. Authority stays with the called owner.

use super::capacity::estimate_backup_space;
use super::execution::backup_and_publish_status;
use super::filesystem::duplicate_inherited_fd;
use super::key_activation::complete_backup_authority_key_v2;
use super::key_activation::initialize_backup_authority_key_v2;
use super::policy::DEFAULT_BACKUP_ROOT;
use super::policy::DEFAULT_BACKUP_STATUS;
use super::sources::validate_backup_restore_source_contract;
use super::verification::BackupVerificationMode;
use super::verification::run_pinned_verifier_command;
use clap::Parser;
use clap::Subcommand;
use robin_highscores::Database;
use robin_highscores::ServerConfig;
use robin_highscores::backup::load_backup_release_identity;
use robin_highscores::live_schema::LiveDatabaseSchemaProbeV2;
use robin_highscores::live_schema::verify_live_database_schema_v2;
use robin_highscores::runtime_authority::BackupAuthorityStateV2;
use robin_highscores::runtime_authority::CandidateSelfRoleV2;
use robin_highscores::runtime_authority::attest_candidate_release_root_v2;
use robin_highscores::runtime_authority::probe_runtime_authority_v2;
use robin_run_protocol::canonical_json_bytes;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

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

pub(super) async fn run() -> anyhow::Result<()> {
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
    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
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

#[cfg(test)]
mod tests;
