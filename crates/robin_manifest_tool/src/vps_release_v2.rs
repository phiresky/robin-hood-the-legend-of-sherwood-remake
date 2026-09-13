//! Deterministic, non-deploying VPS release bundle assembly.
//!
//! A VPS bundle is a deliberately smaller closure than a publication. It
//! carries only API/verifier inputs and reviewed user deployment configuration. Browser
//! static roots, secrets, databases, object stores, and copyrighted raw game
//! installations are never copied. Raw Demo/Full roots are validated as two
//! distinct read-only user-owned trees and recorded only as path declarations.

use std::collections::{BTreeMap, BTreeSet};
mod host_policy;
use host_policy::{DEPLOY_BOOTSTRAP_FILES, ROOT_ONCE_KIT_FILES, validate_host_template_bytes};
mod activation_lock;
pub use activation_lock::{
    PinnedVpsActivationLockV2, acquire_vps_activation_lock_v2, pin_inherited_vps_activation_lock_v2,
};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use robin_run_protocol::{
    ArtifactRefV1, BuildManifestV2, CanonicalDocument as _, Digest32,
    HIGHSCORES_DATABASE_SCHEMA_VERSION, OfficialContentEditionV1, OfficialSourceTreeManifestV2,
    Validate as _, VerifierJobConfigCatalogV1, canonical_json_bytes,
};
use serde::{Deserialize, Serialize};

use crate::publication_v3::{
    BackendPublicationV3, PublicationLockV3, PublicationManifestV3, ValidatedPublicationV3,
};
use crate::{
    MAX_DOCUMENT_BYTES, artifact_from_file, config_parent, ensure_absent_output, path_to_manifest,
    read_regular_file_bounded, resolve_path, staging_directory, strict_json_from_slice,
    validate_mount_root, validate_regular_file, walk_regular_files, write_new_file_bytes,
};

const PLAN_SCHEMA_VERSION: u32 = 2;
const MANIFEST_SCHEMA_VERSION: u32 = 2;
const MIN_SUPPORTED_DATABASE_SCHEMA_VERSION: i64 = 2;
const RAW_ROOTS_SCHEMA_VERSION: u32 = 2;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const SOURCE_COMMIT_FILE: &str = "SOURCE_COMMIT";
const SHA256SUMS_FILE: &str = "SHA256SUMS";
const MODE_INVENTORY_FILE: &str = "MODE_INVENTORY";
const RELEASE_MANIFEST_FILE: &str = "vps-release-manifest-v2.json";
const RAW_ROOT_DECLARATIONS_FILE: &str = "private/raw-root-declarations-v2.json";
const DEPLOY_BOOTSTRAP_SHA256SUMS_FILE: &str = "deploy/DEPLOY_BOOTSTRAP_SHA256SUMS";
const ROOT_ONCE_SHA256SUMS_FILE: &str = "deploy/ROOT_ONCE_SHA256SUMS";
const DEPLOYMENT_USER: &str = "robinhood";
const DEPLOYMENT_HOME: &str = "/home/robinhood";
const INSTALL_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores";
const STATE_ROOT: &str = "/home/robinhood/.local/share/robin-highscores";
const BACKUP_ROOT: &str = "/home/robinhood/.local/share/robin-highscores/backups";
const BACKUP_STATUS_ROOT: &str = "/home/robinhood/.local/share/robin-highscores/status";
const BACKUP_STATUS_PATH: &str =
    "/home/robinhood/.local/share/robin-highscores/status/backup-status.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VpsBinaryRoleV2 {
    Admin,
    ManifestTool,
    Server,
    Worker,
    ReplayVerifier,
}

impl VpsBinaryRoleV2 {
    fn output_name(self) -> &'static str {
        match self {
            Self::Admin => "robin-highscores-admin",
            Self::ManifestTool => "robin-highscores-manifestctl",
            Self::Server => "robin-highscores-server",
            Self::Worker => "robin-highscores-worker",
            Self::ReplayVerifier => "robin-replay-verifier",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsBinarySourceV2 {
    pub role: VpsBinaryRoleV2,
    pub source: PathBuf,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VpsConfigRoleV2 {
    Server,
    Worker,
    ApiEnvironment,
    WorkerEnvironment,
}

impl VpsConfigRoleV2 {
    fn output_name(self) -> &'static str {
        match self {
            Self::Server => "highscores-server.toml",
            Self::Worker => "highscores-worker.toml",
            Self::ApiEnvironment => "api.env",
            Self::WorkerEnvironment => "worker.env",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsConfigSourceV2 {
    pub role: VpsConfigRoleV2,
    pub source: PathBuf,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VpsHostFileRoleV2 {
    UserTarget,
    ApiService,
    WorkerService,
    BackupService,
    BackupTimer,
    DeployReleaseScript,
    RollbackReleaseScript,
    ValidateReleaseScript,
    RealRuntimeFenceReleaseGate,
    RealRuntimeFenceHarness,
    RealRuntimeFenceSelftest,
    RootOnceScript,
    NginxChallenge,
    NginxCloudflareOnly,
    NginxApiLocations,
    NginxVhost,
    DeploymentReadme,
    OperatorRunbook,
    BackupRunbook,
}

impl VpsHostFileRoleV2 {
    fn output_path(self) -> &'static str {
        match self {
            Self::UserTarget => "systemd/user/robin-highscores.target",
            Self::ApiService => "systemd/user/robin-highscores-api.service",
            Self::WorkerService => "systemd/user/robin-highscores-worker.service",
            Self::BackupService => "systemd/user/robin-highscores-backup.service",
            Self::BackupTimer => "systemd/user/robin-highscores-backup.timer",
            Self::DeployReleaseScript => "deploy/deploy-release.sh",
            Self::RollbackReleaseScript => "deploy/rollback-release.sh",
            Self::ValidateReleaseScript => "deploy/validate-release-bundle.sh",
            Self::RealRuntimeFenceReleaseGate => "deploy/tests/real-runtime-fence-release-gate.sh",
            Self::RealRuntimeFenceHarness => "deploy/tests/real-runtime-fence-e2e.py",
            Self::RealRuntimeFenceSelftest => "deploy/tests/real-runtime-fence-e2e-selftest.py",
            Self::RootOnceScript => "deploy/root-once.sh",
            Self::NginxChallenge => "deploy/nginx-robinhood-api.challenge.conf",
            Self::NginxCloudflareOnly => "deploy/nginx-robinhood-cloudflare-only.conf",
            Self::NginxApiLocations => "deploy/nginx-robinhood-api.locations.conf",
            Self::NginxVhost => "deploy/nginx-robinhood-api.vhost.conf",
            Self::DeploymentReadme => "deploy/README.md",
            Self::OperatorRunbook => "deploy/VPS_RELEASE_INSTALL.md",
            Self::BackupRunbook => "deploy/BACKUP_RESTORE.md",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsHostFileSourceV2 {
    pub role: VpsHostFileRoleV2,
    pub source: PathBuf,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateRawRootV2 {
    pub edition: OfficialContentEditionV1,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateRawRootDeclarationsV2 {
    pub schema_version: u32,
    pub roots: Vec<PrivateRawRootV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsReleasePlanV2 {
    pub schema_version: u32,
    pub source_commit: String,
    pub publication_v3: PathBuf,
    pub binaries: Vec<VpsBinarySourceV2>,
    pub configs: Vec<VpsConfigSourceV2>,
    pub host_files: Vec<VpsHostFileSourceV2>,
    pub private_raw_roots: Vec<PrivateRawRootV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsReleaseFileV2 {
    pub path: String,
    pub artifact: ArtifactRefV1,
    pub unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsUserDeploymentV2 {
    pub user: String,
    pub home: PathBuf,
    pub install_root: PathBuf,
    pub persistent_state_root: PathBuf,
    pub current_link: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpsReleaseManifestV2 {
    pub schema_version: u32,
    pub source_commit: String,
    pub database_schema_version: i64,
    pub deployment: VpsUserDeploymentV2,
    pub publication_lock_sha256: Digest32,
    pub publication_manifest_sha256: Digest32,
    pub verifier_sha256: Digest32,
    /// Complete payload inventory. This excludes the four self-describing
    /// top-level files: this manifest, SOURCE_COMMIT, MODE_INVENTORY and
    /// SHA256SUMS.
    pub files: Vec<VpsReleaseFileV2>,
}

impl robin_run_protocol::Validate for VpsUserDeploymentV2 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if *self != canonical_user_deployment() {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "vps_user_deployment_v2",
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for PrivateRawRootDeclarationsV2 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        let identities = self
            .roots
            .iter()
            .map(|root| root.edition)
            .collect::<Vec<_>>();
        let expected = [
            OfficialContentEditionV1::Demo,
            OfficialContentEditionV1::Full,
        ];
        let expected_paths = [
            Path::new("/home/robinhood/.local/share/robin-highscores/raw-content/demo"),
            Path::new("/home/robinhood/.local/share/robin-highscores/raw-content/full"),
        ];
        if self.schema_version != RAW_ROOTS_SCHEMA_VERSION
            || identities != expected
            || self
                .roots
                .iter()
                .any(|root| !normalized_absolute(&root.root))
            || !self
                .roots
                .iter()
                .zip(expected_paths)
                .all(|(root, expected)| root.root == expected)
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "private_raw_root_declarations_v2",
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for VpsReleaseManifestV2 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION
            || !valid_source_commit(&self.source_commit)
            || !(MIN_SUPPORTED_DATABASE_SCHEMA_VERSION..=HIGHSCORES_DATABASE_SCHEMA_VERSION)
                .contains(&self.database_schema_version)
            || self.deployment.validate().is_err()
            || self.publication_lock_sha256.is_zero()
            || self.publication_manifest_sha256.is_zero()
            || self.verifier_sha256.is_zero()
            || self.files.is_empty()
            || !self
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            || self.files.iter().any(|file| {
                !valid_relative_manifest_path(&file.path)
                    || file.artifact.validate().is_err()
                    || file.unix_mode != canonical_file_mode(&file.path)
            })
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "vps_release_manifest_v2",
            });
        }
        Ok(())
    }
}

impl VpsReleaseManifestV2 {
    /// Admit a newly assembled or promoted candidate for this exact runtime.
    ///
    /// Structural V2 validation deliberately also authenticates supported
    /// historical database schemas so a future release can verify its source
    /// before an explicit forward migration. A candidate becoming current is
    /// stricter: it must already name the database schema compiled into this
    /// release.
    fn validate_current_candidate(
        &self,
    ) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        self.validate()?;
        if self.database_schema_version != HIGHSCORES_DATABASE_SCHEMA_VERSION {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "vps_release_manifest_v2_candidate_database_schema_version",
            });
        }
        Ok(())
    }
}

impl VpsReleasePlanV2 {
    fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut plan: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse canonical VPS release plan {}", path.display()))?;
        ensure!(
            canonical_json_bytes(&plan)? == bytes,
            "VPS release plan is not byte-for-byte canonical JSON"
        );
        ensure!(
            plan.schema_version == PLAN_SCHEMA_VERSION,
            "unsupported VPS release plan schema"
        );
        validate_plan_shape(&plan)?;
        let base = fs::canonicalize(config_parent(path)?)?;
        resolve_path(&base, &mut plan.publication_v3);
        for binary in &mut plan.binaries {
            resolve_path(&base, &mut binary.source);
        }
        for config in &mut plan.configs {
            resolve_path(&base, &mut config.source);
        }
        for file in &mut plan.host_files {
            resolve_path(&base, &mut file.source);
        }
        for declaration in &mut plan.private_raw_roots {
            resolve_path(&base, &mut declaration.root);
        }
        Ok(plan)
    }

    fn load_pinned_absolute_bytes(bytes: &[u8], path: &Path) -> Result<Self> {
        let plan: Self = strict_json_from_slice(bytes)
            .with_context(|| format!("parse canonical VPS release plan {}", path.display()))?;
        ensure!(
            canonical_json_bytes(&plan)? == bytes,
            "VPS release plan is not byte-for-byte canonical JSON"
        );
        ensure!(
            plan.schema_version == PLAN_SCHEMA_VERSION,
            "unsupported VPS release plan schema"
        );
        validate_plan_shape(&plan)?;
        ensure!(
            normalized_absolute(&plan.publication_v3)
                && plan
                    .binaries
                    .iter()
                    .all(|entry| normalized_absolute(&entry.source))
                && plan
                    .configs
                    .iter()
                    .all(|entry| normalized_absolute(&entry.source))
                && plan
                    .host_files
                    .iter()
                    .all(|entry| normalized_absolute(&entry.source))
                && plan
                    .private_raw_roots
                    .iter()
                    .all(|entry| normalized_absolute(&entry.root)),
            "descriptor-based VPS operation requires every plan input path to be normalized and absolute"
        );
        Ok(plan)
    }
}
#[cfg(test)]
mod tests;

mod bundle;

mod fd_policy;
use fd_policy::InheritedFd;
mod activation;

mod sources;

mod runtime_fence;

mod plan;

mod config_policy;

mod staging;

pub use bundle::{assemble_vps_release_v2, promote_vps_release_v2, validate_vps_release_v2};
use bundle::{
    canonical_file_mode, canonical_user_deployment, load_canonical, normalized_absolute,
    paths_overlap, reject_hardlink, reject_mounts_at_or_below, valid_relative_manifest_path,
    valid_source_commit, validate_immutable_raw_root, validate_pinned_current_vps_release_root,
};

use activation::load_pinned_vps_plan;
pub use activation::{
    exec_vps_deploy_activation_v2, exec_vps_rollback_activation_v2, project_vps_publication_lock_v2,
};

pub use sources::{consume_vps_sources_v2, promote_inherited_vps_release_v2};
use sources::{pin_vps_candidate_parents_at, pinned_entry_exists};

pub use runtime_fence::initialize_vps_runtime_fence_v1;

use plan::{
    validate_assembly_inputs, validate_linux_elf, validate_pinned_file, validate_plan_shape,
};

use config_policy::{
    reject_placeholders, validate_backup_sandbox_contract, validate_final_config,
    validate_final_host_file, validate_worker_raw_authority,
};

pub use staging::VpsReleaseInstalledButParentSyncFailed;
use staging::{
    MAX_FAILED_VPS_STAGING_DEPTH, MAX_FAILED_VPS_STAGING_ENTRIES, VpsPersistenceOutcome,
    discard_failed_vps_staging, persist_vps_staging, vps_installed_durability_error,
};
