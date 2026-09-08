//! Deterministic, non-deploying VPS release bundle assembly.
//!
//! A VPS bundle is a deliberately smaller closure than a publication. It
//! carries only API/verifier inputs and reviewed user deployment configuration. Browser
//! static roots, secrets, databases, object stores, and copyrighted raw game
//! installations are never copied. Raw Demo/Full roots are validated as two
//! distinct read-only user-owned trees and recorded only as path declarations.

use std::collections::{BTreeMap, BTreeSet};
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
    validate_mount_root, validate_regular_file, walk_regular_files, write_bytes,
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
const SYSTEM_UNIT_ROOT: &str = "/etc/systemd/system";
const CANONICAL_VALIDATE_RELEASE_SCRIPT: &str =
    include_str!("../../robin_highscores/deploy/validate-release-bundle.sh");
const CANONICAL_DEPLOY_RELEASE_SCRIPT: &str =
    include_str!("../../robin_highscores/deploy/deploy-release.sh");
const CANONICAL_ROLLBACK_RELEASE_SCRIPT: &str =
    include_str!("../../robin_highscores/deploy/rollback-release.sh");
const CANONICAL_REAL_FENCE_RELEASE_GATE: &str =
    include_str!("../../robin_highscores/deploy/tests/real-runtime-fence-release-gate.sh");
const CANONICAL_REAL_FENCE_HARNESS: &str =
    include_str!("../../robin_highscores/deploy/tests/real-runtime-fence-e2e.py");
const CANONICAL_REAL_FENCE_SELFTEST: &str =
    include_str!("../../robin_highscores/deploy/tests/real-runtime-fence-e2e-selftest.py");
const CANONICAL_ROOT_ONCE_SCRIPT: &str = include_str!("../../robin_highscores/deploy/root-once.sh");
const CANONICAL_NGINX_CHALLENGE: &str =
    include_str!("../../robin_highscores/deploy/nginx-robinhood-api.challenge.conf");
const CANONICAL_NGINX_CLOUDFLARE_ONLY: &str =
    include_str!("../../robin_highscores/deploy/nginx-robinhood-cloudflare-only.conf");
const CANONICAL_NGINX_API_LOCATIONS: &str =
    include_str!("../../robin_highscores/deploy/nginx-robinhood-api.locations.conf");
const CANONICAL_NGINX_VHOST: &str =
    include_str!("../../robin_highscores/deploy/nginx-robinhood-api.vhost.conf");
const CANONICAL_USER_TARGET: &str =
    include_str!("../../robin_highscores/deploy/robin-highscores.target");
const CANONICAL_API_SERVICE: &str =
    include_str!("../../robin_highscores/deploy/robin-highscores-api.service");
const CANONICAL_WORKER_SERVICE: &str =
    include_str!("../../robin_highscores/deploy/robin-highscores-worker.service");
const CANONICAL_BACKUP_SERVICE: &str =
    include_str!("../../robin_highscores/deploy/robin-highscores-backup.service");
const CANONICAL_BACKUP_TIMER: &str =
    include_str!("../../robin_highscores/deploy/robin-highscores-backup.timer");
const VALIDATOR_SYSTEM_UNIT_DENYLIST_BLOCK: &str = concat!(
    "    unit_path=$bundle/systemd/user/$unit\n",
    "    grep -Fq \"$release_root\" \"$unit_path\" || fail \"$unit is not pinned to the exact release\"\n",
    "    if grep -Eq '^(User|Group|SupplementaryGroups)=|WantedBy=multi-user.target|/etc/systemd/system|verifier-broker|sudo|polkit' \"$unit_path\"; then\n",
    "        fail \"$unit contains a root/system-service assumption\"\n",
    "    fi\n",
);
const ROOT_ONCE_KIT_FILES: [&str; 5] = [
    "nginx-robinhood-api.challenge.conf",
    "nginx-robinhood-cloudflare-only.conf",
    "nginx-robinhood-api.locations.conf",
    "nginx-robinhood-api.vhost.conf",
    "root-once.sh",
];
const DEPLOY_BOOTSTRAP_FILES: [&str; 3] = [
    "deploy-release.sh",
    "rollback-release.sh",
    "validate-release-bundle.sh",
];

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

/// Assemble one immutable, absent-output-only VPS release bundle.
pub fn assemble_vps_release_v2(plan_path: &Path, output: &Path) -> Result<Digest32> {
    ensure_absent_output(output)?;
    ensure!(output.is_absolute(), "VPS release output must be absolute");
    ensure!(
        normalized_absolute(output),
        "VPS release output is not normalized"
    );
    reject_hardlink(plan_path)?;
    let plan = VpsReleasePlanV2::load(plan_path)?;
    validate_vps_release_assembly_output(output, &plan.source_commit)?;
    let mut publication =
        crate::publication_v3::validate_publication_v3_authority(&plan.publication_v3)?;
    let publication_lock_sha256 = publication.lock_sha256();
    let publication_manifest: PublicationManifestV3 =
        publication.load_document("publication-manifest-v3.json")?;
    let publication_manifest_sha256 = publication_manifest.canonical_digest()?;
    let backend: BackendPublicationV3 = publication.load_document("backend/publication-v3.json")?;
    validate_assembly_inputs(
        &plan,
        output,
        backend.verifier_program.sha256,
        &backend.verifier_operator_config,
        &backend
            .campaign_states
            .iter()
            .map(|state| state.artifact.sha256)
            .collect(),
    )?;
    let build_path = format!(
        "backend/manifests/builds/{}.json",
        backend.build_manifest_sha256
    );
    let build: BuildManifestV2 = publication.load_document(&build_path)?;
    ensure!(
        build.source_commit == plan.source_commit,
        "release source commit differs from the admitted BuildManifestV2"
    );
    let verifier = plan
        .binaries
        .iter()
        .find(|binary| binary.role == VpsBinaryRoleV2::ReplayVerifier)
        .context("release plan omits replay verifier")?;
    ensure!(
        verifier.artifact == backend.verifier_program,
        "release verifier differs from publication-v3"
    );
    let catalog_path = format!(
        "private/verifier/operator-config/{}",
        backend.verifier_operator_config.sha256
    );
    let _: VerifierJobConfigCatalogV1 = publication.load_document(&catalog_path)?;
    ensure!(
        publication.artifact(&catalog_path, &backend.verifier_operator_config.media_type)?
            == backend.verifier_operator_config,
        "publication job catalog differs from its pin"
    );
    publication.ensure_live()?;

    let staging = staging_directory(output)?;
    let assembled = (|| {
        materialize_bundle(staging.path(), &plan, &mut publication)?;
        publication.ensure_live()?;
        let files = payload_inventory(staging.path())?;
        let manifest = VpsReleaseManifestV2 {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source_commit: plan.source_commit.clone(),
            database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
            deployment: canonical_user_deployment(),
            publication_lock_sha256,
            publication_manifest_sha256,
            verifier_sha256: verifier.artifact.sha256,
            files,
        };
        manifest.validate_current_candidate()?;
        write_bytes(
            &staging.path().join(RELEASE_MANIFEST_FILE),
            &canonical_json_bytes(&manifest)?,
        )?;
        write_bytes(
            &staging.path().join(SOURCE_COMMIT_FILE),
            format!("{}\n", plan.source_commit).as_bytes(),
        )?;
        write_mode_inventory(staging.path())?;
        write_sha256sums(staging.path())?;
        make_bundle_read_only(staging.path())?;
        validate_vps_release_root(staging.path(), false)
    })();
    match assembled {
        Ok(manifest_sha256) => match persist_vps_staging(&staging, output) {
            Ok(VpsPersistenceOutcome::Installed) => {
                let _installed_path = staging.keep();
                Ok(manifest_sha256)
            }
            Ok(VpsPersistenceOutcome::InstalledButParentSyncFailed(sync_error)) => {
                let _installed_path = staging.keep();
                Err(vps_installed_durability_error(
                    output,
                    manifest_sha256,
                    &plan.source_commit,
                    sync_error,
                ))
            }
            Err(persist_error) => match discard_failed_vps_staging(staging) {
                Ok(()) => Err(persist_error),
                Err(cleanup_error) => Err(persist_error.context(format!(
                    "VPS persistence also failed to securely remove staging: {cleanup_error:#}"
                ))),
            },
        },
        Err(assembly_error) => match discard_failed_vps_staging(staging) {
            Ok(()) => Err(assembly_error),
            Err(cleanup_error) => Err(assembly_error.context(format!(
                "VPS assembly also failed to securely remove staging: {cleanup_error:#}"
            ))),
        },
    }
}

fn validate_vps_release_assembly_output(output: &Path, source_commit: &str) -> Result<()> {
    ensure!(
        output
            == Path::new(INSTALL_ROOT)
                .join("releases")
                .join(format!("{source_commit}.partial")),
        "VPS release assembly output must be exact INSTALL_ROOT/releases/SOURCE_COMMIT.partial"
    );
    Ok(())
}

/// Project the publication-lock digest from one inherited, immutable
/// `VpsReleaseManifestV2` descriptor. The caller supplies the manifest digest
/// independently; no release path or sidecar participates in this scalar
/// authority boundary.
pub fn project_vps_publication_lock_v2(
    release_manifest_fd: std::os::fd::RawFd,
    expected_vps_release_manifest_sha256: &str,
) -> Result<Digest32> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::FileType;
        use std::io::Read as _;

        ensure!(
            release_manifest_fd >= 3,
            "release-manifest descriptor must be at least 3"
        );
        let expected = expected_vps_release_manifest_sha256
            .parse::<Digest32>()
            .context(
                "expected VPS release manifest digest is not canonical lowercase hexadecimal",
            )?;
        ensure!(
            !expected.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        let duplicate = NixOwnedFdV2(nix_legacy::unistd::dup(release_manifest_fd)?);
        let initial = nix_legacy::sys::stat::fstat(duplicate.0)?;
        ensure!(
            FileType::from_raw_mode(initial.st_mode).is_file()
                && initial.st_uid == rustix::process::geteuid().as_raw()
                && initial.st_nlink == 1
                && initial.st_mode & 0o777 == 0o440
                && initial.st_size > 0
                && u64::try_from(initial.st_size)? <= MAX_DOCUMENT_BYTES,
            "release-manifest descriptor has unsafe type, owner, links, mode, or size"
        );
        let descriptor = PathBuf::from(format!("/proc/self/fd/{}", duplicate.0));
        let mut reader = File::open(&descriptor)?;
        let reader_initial = rustix::fs::fstat(&reader)?;
        ensure!(
            reader_initial.st_dev == initial.st_dev
                && reader_initial.st_ino == initial.st_ino
                && reader_initial.st_mode == initial.st_mode,
            "release-manifest procfs duplicate names another inode"
        );
        let mut bytes = Vec::with_capacity(usize::try_from(initial.st_size)?);
        std::io::Read::by_ref(&mut reader)
            .take(MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == u64::try_from(initial.st_size)?,
            "release-manifest descriptor length changed while it was read"
        );
        let reader_observed = rustix::fs::fstat(&reader)?;
        ensure!(
            reader_observed.st_dev == reader_initial.st_dev
                && reader_observed.st_ino == reader_initial.st_ino
                && reader_observed.st_uid == reader_initial.st_uid
                && reader_observed.st_gid == reader_initial.st_gid
                && reader_observed.st_mode == reader_initial.st_mode
                && reader_observed.st_nlink == reader_initial.st_nlink
                && reader_observed.st_size == reader_initial.st_size
                && reader_observed.st_mtime == reader_initial.st_mtime
                && reader_observed.st_mtime_nsec == reader_initial.st_mtime_nsec
                && reader_observed.st_ctime == reader_initial.st_ctime
                && reader_observed.st_ctime_nsec == reader_initial.st_ctime_nsec,
            "release-manifest procfs duplicate changed while it was read"
        );
        let observed = nix_legacy::sys::stat::fstat(duplicate.0)?;
        ensure!(
            observed.st_dev == initial.st_dev
                && observed.st_ino == initial.st_ino
                && observed.st_uid == initial.st_uid
                && observed.st_gid == initial.st_gid
                && observed.st_mode == initial.st_mode
                && observed.st_nlink == initial.st_nlink
                && observed.st_size == initial.st_size
                && observed.st_mtime == initial.st_mtime
                && observed.st_mtime_nsec == initial.st_mtime_nsec
                && observed.st_ctime == initial.st_ctime
                && observed.st_ctime_nsec == initial.st_ctime_nsec,
            "release-manifest descriptor changed while it was read"
        );
        ensure!(
            Digest32::digest_bytes(&bytes) == expected,
            "release-manifest descriptor differs from its out-of-band digest"
        );
        let manifest: VpsReleaseManifestV2 = strict_json_from_slice(&bytes)
            .context("parse descriptor-pinned canonical VpsReleaseManifestV2")?;
        manifest.validate()?;
        ensure!(
            canonical_json_bytes(&manifest)? == bytes,
            "descriptor-pinned VpsReleaseManifestV2 is not canonical JSON"
        );
        ensure!(
            nix_legacy::sys::stat::fstat(duplicate.0)? == observed,
            "release-manifest descriptor changed after canonical validation"
        );
        Ok(manifest.publication_lock_sha256)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (release_manifest_fd, expected_vps_release_manifest_sha256);
        anyhow::bail!("descriptor-pinned VPS projection requires Linux procfs")
    }
}

#[cfg(unix)]
fn load_pinned_vps_plan(
    plan_fd: &Path,
    expected_plan_sha256: &str,
) -> Result<(VpsReleasePlanV2, Vec<u8>, Digest32)> {
    use rustix::fs::FileType;

    let descriptor = canonical_proc_descriptor(plan_fd, "VPS release plan")?;
    let metadata = nix_legacy::sys::stat::fstat(descriptor)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_file()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_nlink == 1
            && metadata.st_mode & 0o777 == 0o400,
        "VPS release plan descriptor must be an owner-only regular nlink-1 file"
    );
    let expected_plan_sha256 = expected_plan_sha256
        .parse::<Digest32>()
        .context("expected VPS release plan digest is not canonical lowercase hexadecimal")?;
    let plan_bytes =
        read_pinned_descriptor_bounded(descriptor, MAX_DOCUMENT_BYTES, "VPS release plan")?;
    ensure!(
        Digest32::digest_bytes(&plan_bytes) == expected_plan_sha256,
        "VPS release plan descriptor differs from its out-of-band digest"
    );
    let plan = VpsReleasePlanV2::load_pinned_absolute_bytes(&plan_bytes, plan_fd)?;
    Ok((plan, plan_bytes, expected_plan_sha256))
}

/// The exact canonical activation lock held across deploy, rollback, and
/// destructive source consumption.
#[cfg(target_os = "linux")]
#[derive(Debug)]
struct NixOwnedFdV2(std::os::fd::RawFd);

#[cfg(target_os = "linux")]
impl Drop for NixOwnedFdV2 {
    fn drop(&mut self) {
        let _ = nix_legacy::unistd::close(self.0);
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
enum PinnedVpsActivationLockDescriptorV2 {
    Owned(std::os::fd::OwnedFd),
    InheritedDuplicate(NixOwnedFdV2),
}

#[cfg(target_os = "linux")]
impl PinnedVpsActivationLockDescriptorV2 {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd as _;

        match self {
            Self::Owned(fd) => fd.as_raw_fd(),
            Self::InheritedDuplicate(fd) => fd.0,
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct PinnedVpsActivationLockV2 {
    lock_fd: PinnedVpsActivationLockDescriptorV2,
    opt_fd: std::os::fd::OwnedFd,
    opt_root: PathBuf,
    opt_device: u64,
    opt_inode: u64,
    lock_device: u64,
    lock_inode: u64,
}

#[cfg(target_os = "linux")]
impl PinnedVpsActivationLockV2 {
    pub fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.lock_fd.as_raw_fd()
    }

    pub fn clear_close_on_exec(&self) -> Result<()> {
        nix_legacy::fcntl::fcntl(
            self.lock_fd.as_raw_fd(),
            nix_legacy::fcntl::FcntlArg::F_SETFD(nix_legacy::fcntl::FdFlag::empty()),
        )?;
        Ok(())
    }

    pub fn ensure_canonical(&self) -> Result<()> {
        use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let path_metadata = fs::symlink_metadata(&self.opt_root)?;
        ensure!(
            path_metadata.is_dir()
                && !path_metadata.file_type().is_symlink()
                && fs::canonicalize(&self.opt_root)? == self.opt_root
                && path_metadata.uid() == rustix::process::geteuid().as_raw()
                && path_metadata.permissions().mode() & 0o777 == 0o750
                && path_metadata.dev() == self.opt_device
                && path_metadata.ino() == self.opt_inode,
            "canonical activation opt root changed while its lock was held"
        );
        let current_opt = openat2(
            rustix::fs::CWD,
            &self.opt_root,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let current_opt_metadata = rustix::fs::fstat(&current_opt)?;
        let pinned_opt_metadata = rustix::fs::fstat(&self.opt_fd)?;
        ensure!(
            current_opt_metadata.st_dev == self.opt_device
                && current_opt_metadata.st_ino == self.opt_inode
                && pinned_opt_metadata.st_dev == self.opt_device
                && pinned_opt_metadata.st_ino == self.opt_inode,
            "canonical activation opt root differs from its held descriptor"
        );
        let current_lock = openat2(
            current_opt.as_fd(),
            "activation.lock",
            OFlags::RDWR | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let current_lock_metadata = rustix::fs::fstat(&current_lock)?;
        let held_lock_metadata = nix_legacy::sys::stat::fstat(self.lock_fd.as_raw_fd())?;
        ensure!(
            FileType::from_raw_mode(current_lock_metadata.st_mode).is_file()
                && current_lock_metadata.st_uid == rustix::process::geteuid().as_raw()
                && current_lock_metadata.st_nlink == 1
                && current_lock_metadata.st_mode & 0o777 == 0o600
                && current_lock_metadata.st_dev == self.lock_device
                && current_lock_metadata.st_ino == self.lock_inode
                && held_lock_metadata.st_dev == self.lock_device
                && held_lock_metadata.st_ino == self.lock_inode
                && held_lock_metadata.st_nlink == 1
                && held_lock_metadata.st_mode & 0o777 == 0o600,
            "canonical activation lock changed while its descriptor was held"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub fn acquire_vps_activation_lock_v2() -> Result<PinnedVpsActivationLockV2> {
    acquire_vps_activation_lock_at(Path::new(INSTALL_ROOT))
}

/// Validate every inherited deploy authority before acquiring the canonical
/// activation lock, then replace this process with the reviewed deploy script.
/// The same lock open-file-description and plan descriptor remain inherited by
/// the script and every destructive source-consumption child.
pub fn exec_vps_deploy_activation_v2(
    script_fd: &Path,
    bootstrap_manifest_fd: &Path,
    validator_fd: &Path,
    manifest_tool_fd: &Path,
    plan_fd: &Path,
    expected_plan_sha256: &str,
    expected_vps_release_manifest_sha256: &str,
    business_arguments: &[std::ffi::OsString],
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::process::CommandExt as _;

        let (candidate, expected_commit, expected_bootstrap_sha256) =
            parse_deploy_business_arguments(business_arguments)?;
        let authorities = pin_vps_activation_exec_authorities(
            VpsActivationExecOperationV2::Deploy,
            script_fd,
            bootstrap_manifest_fd,
            validator_fd,
            manifest_tool_fd,
            expected_bootstrap_sha256,
            Some(plan_fd),
        )?;
        let (plan, _, _) = load_pinned_vps_plan(plan_fd, expected_plan_sha256)?;
        ensure!(
            plan.source_commit == expected_commit,
            "VPS activation plan source commit differs from the requested deploy"
        );
        validate_deploy_plan_source_paths(&plan, &expected_commit)?;
        let expected_vps_release_manifest_sha256 = expected_vps_release_manifest_sha256
            .parse::<Digest32>()
            .context("expected VPS release manifest is not canonical lowercase hexadecimal")?;
        ensure!(
            !expected_vps_release_manifest_sha256.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        let candidate_fd = pin_vps_activation_candidate(&candidate)?;
        let candidate_path = PathBuf::from(format!("/proc/self/fd/{}/.", candidate_fd.as_raw_fd()));
        let actual_manifest = validate_pinned_current_vps_release_root(&candidate_path)?;
        ensure!(
            actual_manifest == expected_vps_release_manifest_sha256,
            "candidate differs from the out-of-band VPS release manifest digest"
        );
        let manifest: VpsReleaseManifestV2 =
            load_canonical(&candidate_path.join(RELEASE_MANIFEST_FILE))?;
        ensure!(
            manifest.source_commit == expected_commit,
            "candidate VPS release source commit differs from the requested deploy"
        );

        let activation_lock = acquire_vps_activation_lock_v2()?;
        activation_lock.ensure_canonical()?;
        let plan_descriptor = plan_fd_descriptor(plan_fd)?;
        for descriptor in authorities.iter().chain(std::iter::once(&plan_descriptor)) {
            clear_vps_close_on_exec(*descriptor)?;
        }
        clear_vps_close_on_exec(candidate_fd.as_raw_fd())?;
        activation_lock.clear_close_on_exec()?;
        let candidate_fd_path =
            PathBuf::from(format!("/proc/self/fd/{}", candidate_fd.as_raw_fd()));
        let lock_path = PathBuf::from(format!("/proc/self/fd/{}", activation_lock.as_raw_fd()));
        let expected_vps_release_manifest_sha256 = expected_vps_release_manifest_sha256.to_string();
        let mut command = std::process::Command::new(script_fd);
        command.args(business_arguments).args([
            bootstrap_manifest_fd.as_os_str(),
            validator_fd.as_os_str(),
            manifest_tool_fd.as_os_str(),
            plan_fd.as_os_str(),
            candidate_fd_path.as_os_str(),
            lock_path.as_os_str(),
            std::ffi::OsStr::new(expected_plan_sha256),
            std::ffi::OsStr::new(&expected_vps_release_manifest_sha256),
        ]);
        let error = command.exec();
        Err(error).context("exec descriptor-pinned VPS deploy transaction")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            script_fd,
            bootstrap_manifest_fd,
            validator_fd,
            manifest_tool_fd,
            plan_fd,
            expected_plan_sha256,
            expected_vps_release_manifest_sha256,
            business_arguments,
        );
        anyhow::bail!("VPS activation execution requires Linux descriptor semantics")
    }
}

/// Rollback is intentionally disjoint from uploader source authority: it
/// admits no plan descriptor and can never invoke source consumption.
pub fn exec_vps_rollback_activation_v2(
    script_fd: &Path,
    bootstrap_manifest_fd: &Path,
    validator_fd: &Path,
    manifest_tool_fd: &Path,
    business_arguments: &[std::ffi::OsString],
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;

        let expected_bootstrap_sha256 = parse_rollback_business_arguments(business_arguments)?;
        let authorities = pin_vps_activation_exec_authorities(
            VpsActivationExecOperationV2::Rollback,
            script_fd,
            bootstrap_manifest_fd,
            validator_fd,
            manifest_tool_fd,
            expected_bootstrap_sha256,
            None,
        )?;
        let activation_lock = acquire_vps_activation_lock_v2()?;
        activation_lock.ensure_canonical()?;
        for descriptor in &authorities {
            clear_vps_close_on_exec(*descriptor)?;
        }
        activation_lock.clear_close_on_exec()?;
        let lock_path = PathBuf::from(format!("/proc/self/fd/{}", activation_lock.as_raw_fd()));
        let mut command = std::process::Command::new(script_fd);
        command.args(business_arguments).args([
            bootstrap_manifest_fd.as_os_str(),
            validator_fd.as_os_str(),
            manifest_tool_fd.as_os_str(),
            lock_path.as_os_str(),
        ]);
        let error = command.exec();
        Err(error).context("exec descriptor-pinned VPS rollback transaction")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            script_fd,
            bootstrap_manifest_fd,
            validator_fd,
            manifest_tool_fd,
            business_arguments,
        );
        anyhow::bail!("VPS activation execution requires Linux descriptor semantics")
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum VpsActivationExecOperationV2 {
    Deploy,
    Rollback,
}

#[cfg(target_os = "linux")]
fn os_argument(arguments: &[std::ffi::OsString], index: usize, label: &str) -> Result<String> {
    arguments
        .get(index)
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .with_context(|| format!("VPS activation {label} is missing or not UTF-8"))
}

#[cfg(target_os = "linux")]
fn parse_deploy_business_arguments(
    arguments: &[std::ffi::OsString],
) -> Result<(PathBuf, String, &str)> {
    let offset = if arguments.first().and_then(|value| value.to_str()) == Some("--resume-installed")
    {
        ensure!(arguments.len() == 5, "invalid deploy resume argument count");
        1
    } else {
        ensure!(arguments.len() == 4, "invalid deploy argument count");
        0
    };
    let candidate = PathBuf::from(os_argument(arguments, offset, "candidate path")?);
    let commit = os_argument(arguments, offset + 1, "source commit")?;
    ensure!(valid_source_commit(&commit), "invalid deploy source commit");
    let expected_candidate = if offset == 0 {
        Path::new(INSTALL_ROOT)
            .join("releases")
            .join(format!("{commit}.partial"))
    } else {
        Path::new(INSTALL_ROOT).join("releases").join(&commit)
    };
    ensure!(
        candidate == expected_candidate,
        "deploy candidate path is not the exact canonical transaction path"
    );
    let sums = os_argument(arguments, offset + 2, "SHA256SUMS digest")?;
    sums.parse::<Digest32>()
        .context("deploy SHA256SUMS digest is not canonical")?;
    let bootstrap = arguments[offset + 3]
        .to_str()
        .context("deploy bootstrap digest is not UTF-8")?;
    bootstrap
        .parse::<Digest32>()
        .context("deploy bootstrap digest is not canonical")?;
    Ok((candidate, commit, bootstrap))
}

#[cfg(target_os = "linux")]
fn parse_rollback_business_arguments(arguments: &[std::ffi::OsString]) -> Result<&str> {
    let offset = if arguments.first().and_then(|value| value.to_str()) == Some("--resume-target") {
        ensure!(
            arguments.len() == 4,
            "invalid rollback resume argument count"
        );
        1
    } else {
        ensure!(arguments.len() == 3, "invalid rollback argument count");
        0
    };
    let commit = os_argument(arguments, offset, "rollback source commit")?;
    ensure!(
        valid_source_commit(&commit),
        "invalid rollback source commit"
    );
    os_argument(arguments, offset + 1, "rollback SHA256SUMS digest")?
        .parse::<Digest32>()
        .context("rollback SHA256SUMS digest is not canonical")?;
    let bootstrap = arguments[offset + 2]
        .to_str()
        .context("rollback bootstrap digest is not UTF-8")?;
    bootstrap
        .parse::<Digest32>()
        .context("rollback bootstrap digest is not canonical")?;
    Ok(bootstrap)
}

#[cfg(target_os = "linux")]
fn plan_fd_descriptor(path: &Path) -> Result<std::os::fd::RawFd> {
    canonical_proc_descriptor(path, "VPS release plan")
}

#[cfg(target_os = "linux")]
fn canonical_proc_descriptor(path: &Path, label: &str) -> Result<std::os::fd::RawFd> {
    let descriptor = path
        .to_str()
        .and_then(|path| path.strip_prefix("/proc/self/fd/"))
        .with_context(|| format!("{label} must be an explicit /proc/self/fd descriptor"))?;
    ensure!(
        !descriptor.is_empty() && descriptor.bytes().all(|byte| byte.is_ascii_digit()),
        "{label} descriptor is not canonical"
    );
    descriptor
        .parse::<std::os::fd::RawFd>()
        .with_context(|| format!("{label} descriptor is out of range"))
}

#[cfg(target_os = "linux")]
fn read_pinned_descriptor_bounded(
    descriptor: std::os::fd::RawFd,
    maximum_bytes: u64,
    label: &str,
) -> Result<Vec<u8>> {
    use rustix::fs::FileType;
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::FileExt as _;

    ensure!(descriptor >= 3, "{label} descriptor must be at least 3");
    let initial = nix_legacy::sys::stat::fstat(descriptor)?;
    ensure!(
        FileType::from_raw_mode(initial.st_mode).is_file()
            && initial.st_uid == rustix::process::geteuid().as_raw()
            && initial.st_nlink == 1
            && initial.st_size >= 0
            && u64::try_from(initial.st_size)? <= maximum_bytes,
        "{label} descriptor has unsafe type, owner, links, or size"
    );
    // Reading can legitimately update atime on relatime/strictatime mounts.
    // Continue checking inode, permissions, size, mtime and ctime for mutation.
    let unchanged = |mut observed: nix_legacy::sys::stat::FileStat| {
        observed.st_atime = initial.st_atime;
        observed.st_atime_nsec = initial.st_atime_nsec;
        observed == initial
    };
    let duplicate = NixOwnedFdV2(nix_legacy::unistd::dup(descriptor)?);
    ensure!(
        unchanged(nix_legacy::sys::stat::fstat(duplicate.0)?),
        "{label} descriptor changed before its duplicate was read"
    );
    let duplicate_path = PathBuf::from(format!("/proc/self/fd/{}", duplicate.0));
    let file = File::open(duplicate_path)?;
    ensure!(
        unchanged(nix_legacy::sys::stat::fstat(file.as_raw_fd())?),
        "{label} procfs duplicate names another inode"
    );
    let expected_length = usize::try_from(initial.st_size)?;
    let mut bytes = vec![0_u8; expected_length];
    let mut offset = 0;
    while offset < expected_length {
        let read = file.read_at(&mut bytes[offset..], u64::try_from(offset)?)?;
        ensure!(read != 0, "{label} descriptor shortened while it was read");
        offset += read;
    }
    let mut extra = [0_u8; 1];
    ensure!(
        file.read_at(&mut extra, u64::try_from(expected_length)?)? == 0,
        "{label} descriptor grew while it was read"
    );
    ensure!(
        unchanged(nix_legacy::sys::stat::fstat(file.as_raw_fd())?)
            && unchanged(nix_legacy::sys::stat::fstat(descriptor)?),
        "{label} descriptor changed while it was read"
    );
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn pin_vps_activation_exec_authorities(
    operation: VpsActivationExecOperationV2,
    script_fd: &Path,
    bootstrap_manifest_fd: &Path,
    validator_fd: &Path,
    manifest_tool_fd: &Path,
    expected_bootstrap_sha256: &str,
    plan_fd: Option<&Path>,
) -> Result<Vec<std::os::fd::RawFd>> {
    use rustix::fs::FileType;

    let descriptors = [
        (script_fd, 0o500, "activation script"),
        (bootstrap_manifest_fd, 0o400, "bootstrap manifest"),
        (validator_fd, 0o500, "release validator"),
        (manifest_tool_fd, 0o550, "manifest tool"),
    ];
    let mut raw = Vec::with_capacity(5);
    for (path, mode, label) in descriptors {
        let descriptor = canonical_proc_descriptor(path, label)?;
        ensure!(descriptor >= 3, "{label} descriptor must be at least 3");
        let metadata = nix_legacy::sys::stat::fstat(descriptor)?;
        ensure!(
            FileType::from_raw_mode(metadata.st_mode).is_file()
                && metadata.st_uid == rustix::process::geteuid().as_raw()
                && metadata.st_nlink == 1
                && metadata.st_mode & 0o777 == mode,
            "{label} descriptor has unsafe type, owner, links, or mode"
        );
        raw.push(descriptor);
    }
    if let Some(plan_fd) = plan_fd {
        let descriptor = canonical_proc_descriptor(plan_fd, "VPS release plan")?;
        let metadata = nix_legacy::sys::stat::fstat(descriptor)?;
        ensure!(
            FileType::from_raw_mode(metadata.st_mode).is_file()
                && metadata.st_uid == rustix::process::geteuid().as_raw()
                && metadata.st_nlink == 1
                && metadata.st_mode & 0o777 == 0o400,
            "VPS release plan descriptor has unsafe type, owner, links, or mode"
        );
        raw.push(descriptor);
    }
    ensure!(
        raw.iter().copied().collect::<BTreeSet<_>>().len() == raw.len(),
        "VPS activation authority descriptors must be distinct"
    );

    let executing = fs::metadata("/proc/self/exe")?;
    let manifest_tool = nix_legacy::sys::stat::fstat(raw[3])?;
    use std::os::unix::fs::MetadataExt as _;
    ensure!(
        executing.dev() == manifest_tool.st_dev && executing.ino() == manifest_tool.st_ino,
        "manifest-tool descriptor is not the currently executing inode"
    );
    let expected_bootstrap = expected_bootstrap_sha256
        .parse::<Digest32>()
        .context("bootstrap manifest digest is not canonical")?;
    let bootstrap_bytes =
        read_pinned_descriptor_bounded(raw[1], MAX_DOCUMENT_BYTES, "bootstrap manifest")?;
    ensure!(
        Digest32::digest_bytes(&bootstrap_bytes) == expected_bootstrap,
        "bootstrap manifest differs from its out-of-band digest"
    );
    let bootstrap_text = std::str::from_utf8(&bootstrap_bytes)?;
    let mut entries = BTreeMap::new();
    for line in bootstrap_text.lines() {
        let (digest, name) = line
            .split_once("  ")
            .context("bootstrap manifest line is not canonical")?;
        let digest = digest
            .parse::<Digest32>()
            .context("bootstrap manifest entry digest is not canonical")?;
        ensure!(
            matches!(
                name,
                "deploy-release.sh" | "rollback-release.sh" | "validate-release-bundle.sh"
            ) && entries.insert(name, digest).is_none(),
            "bootstrap manifest has an unknown or duplicate entry"
        );
    }
    ensure!(
        entries.len() == 3,
        "bootstrap manifest inventory is incomplete"
    );
    let expected_script_name = match operation {
        VpsActivationExecOperationV2::Deploy => "deploy-release.sh",
        VpsActivationExecOperationV2::Rollback => "rollback-release.sh",
    };
    let descriptor_digest = |descriptor, label| -> Result<Digest32> {
        Ok(Digest32::digest_bytes(&read_pinned_descriptor_bounded(
            descriptor,
            MAX_DOCUMENT_BYTES,
            label,
        )?))
    };
    ensure!(
        descriptor_digest(raw[0], "activation script")? == entries[expected_script_name],
        "activation script differs from the pinned bootstrap manifest"
    );
    ensure!(
        descriptor_digest(raw[2], "release validator")? == entries["validate-release-bundle.sh"],
        "release validator differs from the pinned bootstrap manifest"
    );
    Ok(raw)
}

#[cfg(target_os = "linux")]
fn clear_vps_close_on_exec(descriptor: std::os::fd::RawFd) -> Result<()> {
    let flags = nix_legacy::fcntl::fcntl(descriptor, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
    let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
    flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
    nix_legacy::fcntl::fcntl(descriptor, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_deploy_plan_source_paths(plan: &VpsReleasePlanV2, commit: &str) -> Result<()> {
    let source = Path::new(INSTALL_ROOT)
        .join("incoming")
        .join(format!(".sources-{commit}"));
    ensure!(
        plan.publication_v3 == source.join("publication-v3")
            && plan
                .binaries
                .iter()
                .all(|entry| entry.source == source.join("bin").join(entry.role.output_name()))
            && plan
                .configs
                .iter()
                .all(|entry| entry.source == source.join("config").join(entry.role.output_name()))
            && plan.host_files.iter().all(|entry| {
                entry.source == source.join("host").join(entry.role.output_path())
            }),
        "VPS deploy plan does not bind the exact uploader source closure"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn pin_vps_activation_candidate(path: &Path) -> Result<std::os::fd::OwnedFd> {
    pin_vps_activation_candidate_at(
        path,
        &Path::new(INSTALL_ROOT).join("incoming"),
        &Path::new(INSTALL_ROOT).join("releases"),
    )
}

#[cfg(target_os = "linux")]
fn pin_vps_activation_candidate_at(
    path: &Path,
    incoming_root: &Path,
    releases_root: &Path,
) -> Result<std::os::fd::OwnedFd> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::os::fd::AsFd as _;

    ensure!(
        normalized_absolute(path) && path.parent() == Some(releases_root),
        "VPS activation candidate is outside the exact releases root"
    );
    let basename = path
        .file_name()
        .context("VPS activation candidate has no basename")?;
    let parents = pin_vps_candidate_parents_at(incoming_root, releases_root)?;
    let path_metadata = statat(
        parents.installed_parent_fd.as_fd(),
        basename,
        AtFlags::SYMLINK_NOFOLLOW,
    )?;
    ensure!(
        FileType::from_raw_mode(path_metadata.st_mode).is_dir()
            && path_metadata.st_uid == rustix::process::geteuid().as_raw()
            && path_metadata.st_mode & 0o777 == 0o550,
        "VPS activation candidate has unsafe type, owner, or mode"
    );
    let fd = openat2(
        parents.installed_parent_fd.as_fd(),
        basename,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let pinned = rustix::fs::fstat(&fd)?;
    ensure!(
        FileType::from_raw_mode(pinned.st_mode).is_dir()
            && pinned.st_dev == path_metadata.st_dev
            && pinned.st_ino == path_metadata.st_ino
            && pinned.st_uid == path_metadata.st_uid
            && pinned.st_mode & 0o777 == 0o550,
        "VPS activation candidate changed while it was pinned"
    );
    Ok(fd)
}

#[cfg(target_os = "linux")]
pub fn pin_inherited_vps_activation_lock_v2(
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<PinnedVpsActivationLockV2> {
    pin_inherited_vps_activation_lock_at(Path::new(INSTALL_ROOT), activation_lock_fd)
}

#[cfg(target_os = "linux")]
fn pin_inherited_vps_activation_lock_at(
    opt_root: &Path,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<PinnedVpsActivationLockV2> {
    use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
    use rustix::io::Errno;
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    ensure!(
        activation_lock_fd >= 3,
        "inherited activation lock descriptor must be at least 3"
    );
    let lock_fd = NixOwnedFdV2(nix_legacy::unistd::dup(activation_lock_fd)?);
    nix_legacy::fcntl::fcntl(
        lock_fd.0,
        nix_legacy::fcntl::FcntlArg::F_SETFD(nix_legacy::fcntl::FdFlag::FD_CLOEXEC),
    )?;
    let opt_metadata = fs::symlink_metadata(opt_root)?;
    ensure!(
        opt_metadata.is_dir()
            && !opt_metadata.file_type().is_symlink()
            && fs::canonicalize(opt_root)? == opt_root
            && opt_metadata.uid() == rustix::process::geteuid().as_raw()
            && opt_metadata.permissions().mode() & 0o777 == 0o750,
        "activation opt root must be canonical, EUID-owned, and mode 0750"
    );
    let opt_fd = openat2(
        rustix::fs::CWD,
        opt_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_opt = rustix::fs::fstat(&opt_fd)?;
    ensure!(
        pinned_opt.st_dev == opt_metadata.dev() && pinned_opt.st_ino == opt_metadata.ino(),
        "activation opt root changed while inherited lock was pinned"
    );
    let inherited_metadata = nix_legacy::sys::stat::fstat(lock_fd.0)?;
    let canonical_lock = openat2(
        opt_fd.as_fd(),
        "activation.lock",
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let canonical_metadata = rustix::fs::fstat(&canonical_lock)?;
    ensure!(
        rustix::fs::FileType::from_raw_mode(inherited_metadata.st_mode).is_file()
            && inherited_metadata.st_uid == rustix::process::geteuid().as_raw()
            && inherited_metadata.st_nlink == 1
            && inherited_metadata.st_mode & 0o777 == 0o600
            && inherited_metadata.st_dev == pinned_opt.st_dev
            && inherited_metadata.st_dev == canonical_metadata.st_dev
            && inherited_metadata.st_ino == canonical_metadata.st_ino,
        "inherited activation lock is not the canonical owner-only lock inode"
    );
    match flock(&canonical_lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {
            flock(&canonical_lock, FlockOperation::Unlock)?;
            anyhow::bail!(
                "inherited activation lock descriptor does not already own the exclusive lock"
            );
        }
        Err(Errno::WOULDBLOCK) => {}
        Err(error) => return Err(error.into()),
    }
    #[allow(deprecated)]
    nix_legacy::fcntl::flock(
        lock_fd.0,
        nix_legacy::fcntl::FlockArg::LockExclusiveNonblock,
    )
    .context("inherited activation lock is not the open file description holding exclusion")?;
    let pinned = PinnedVpsActivationLockV2 {
        lock_fd: PinnedVpsActivationLockDescriptorV2::InheritedDuplicate(lock_fd),
        opt_fd,
        opt_root: opt_root.to_path_buf(),
        opt_device: pinned_opt.st_dev,
        opt_inode: pinned_opt.st_ino,
        lock_device: inherited_metadata.st_dev,
        lock_inode: inherited_metadata.st_ino,
    };
    pinned.ensure_canonical()?;
    Ok(pinned)
}

#[cfg(target_os = "linux")]
fn acquire_vps_activation_lock_at(opt_root: &Path) -> Result<PinnedVpsActivationLockV2> {
    use rustix::fs::{FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
    use rustix::io::Errno;
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let opt_metadata = fs::symlink_metadata(opt_root)?;
    ensure!(
        opt_metadata.is_dir()
            && !opt_metadata.file_type().is_symlink()
            && fs::canonicalize(opt_root)? == opt_root
            && opt_metadata.uid() == rustix::process::geteuid().as_raw()
            && opt_metadata.permissions().mode() & 0o777 == 0o750,
        "activation opt root must be canonical, EUID-owned, and mode 0750"
    );
    let opt_fd = openat2(
        rustix::fs::CWD,
        opt_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_opt = rustix::fs::fstat(&opt_fd)?;
    ensure!(
        pinned_opt.st_dev == opt_metadata.dev() && pinned_opt.st_ino == opt_metadata.ino(),
        "activation opt root changed while being pinned"
    );
    let lock_name = Path::new("activation.lock");
    let (lock_fd, created) = match openat2(
        opt_fd.as_fd(),
        lock_name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => (fd, true),
        Err(Errno::EXIST) => (
            openat2(
                opt_fd.as_fd(),
                lock_name,
                OFlags::RDWR | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )?,
            false,
        ),
        Err(error) => return Err(error.into()),
    };
    let lock_metadata = rustix::fs::fstat(&lock_fd)?;
    let lock_std_metadata = fs::metadata(format!("/proc/self/fd/{}", lock_fd.as_raw_fd()))?;
    ensure!(
        lock_std_metadata.is_file()
            && lock_metadata.st_uid == rustix::process::geteuid().as_raw()
            && lock_std_metadata.nlink() == 1
            && lock_std_metadata.permissions().mode() & 0o777 == 0o600
            && lock_metadata.st_dev == pinned_opt.st_dev,
        "activation lock has unsafe owner, type, links, mode, or device"
    );
    let observed_lock = openat2(
        opt_fd.as_fd(),
        lock_name,
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let observed = rustix::fs::fstat(&observed_lock)?;
    ensure!(
        observed.st_dev == lock_metadata.st_dev && observed.st_ino == lock_metadata.st_ino,
        "activation lock path changed after pinning"
    );
    if created {
        rustix::fs::fsync(&opt_fd)?;
    }
    flock(&lock_fd, FlockOperation::NonBlockingLockExclusive)
        .context("another VPS activation holds the shared lock")?;
    let final_observed = rustix::fs::fstat(&openat2(
        opt_fd.as_fd(),
        lock_name,
        OFlags::RDWR | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?)?;
    ensure!(
        final_observed.st_dev == lock_metadata.st_dev
            && final_observed.st_ino == lock_metadata.st_ino,
        "activation lock path changed after locking"
    );
    Ok(PinnedVpsActivationLockV2 {
        lock_fd: PinnedVpsActivationLockDescriptorV2::Owned(lock_fd),
        opt_fd,
        opt_root: opt_root.to_path_buf(),
        opt_device: pinned_opt.st_dev,
        opt_inode: pinned_opt.st_ino,
        lock_device: lock_metadata.st_dev,
        lock_inode: lock_metadata.st_ino,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VpsSourceEntryKindV1 {
    Directory,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VpsSourceConsumePhaseV1 {
    Prepared,
    RootUnlinked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VpsSourceConsumeEntryV1 {
    path: String,
    kind: VpsSourceEntryKindV1,
    unix_mode: u32,
    sha256: Option<Digest32>,
    byte_length: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VpsSourceConsumeJournalV1 {
    schema_version: u32,
    phase: VpsSourceConsumePhaseV1,
    source_commit: String,
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    source_device: u64,
    source_inode: u64,
    candidate_device: u64,
    candidate_inode: u64,
    entries: Vec<VpsSourceConsumeEntryV1>,
}

/// Consume the exact uploader-owned source closure after its assembled
/// candidate has been independently authenticated.
///
/// The plan descriptor is read once and binds all derived paths. A durable
/// inventory journal is published before the source root is renamed to its
/// deterministic consuming name, so interruption during deletion can resume
/// only from an exact remaining subset of the original closure.
pub fn consume_vps_sources_v2(
    plan_fd: &Path,
    expected_plan_sha256: &str,
    expected_release_manifest_sha256: &str,
    candidate_root_fd: std::os::fd::RawFd,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<Digest32> {
    #[cfg(target_os = "linux")]
    {
        let activation_lock = pin_inherited_vps_activation_lock_v2(activation_lock_fd)?;
        let (plan, plan_bytes, plan_sha256) = load_pinned_vps_plan(plan_fd, expected_plan_sha256)?;
        let release_manifest_sha256 = expected_release_manifest_sha256
            .parse::<Digest32>()
            .context(
                "expected VPS release manifest digest is not canonical lowercase hexadecimal",
            )?;
        ensure!(
            !release_manifest_sha256.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        let candidate_root = pin_inherited_vps_candidate_root_v2(
            &plan.source_commit,
            candidate_root_fd,
            release_manifest_sha256,
        )?;
        consume_vps_sources_in_with(
            &plan,
            &plan_bytes,
            plan_sha256,
            release_manifest_sha256,
            &Path::new(INSTALL_ROOT).join("incoming"),
            &Path::new(INSTALL_ROOT)
                .join("incoming")
                .join(format!(".sources-{}", plan.source_commit)),
            &candidate_root,
            |candidate| {
                let digest = validate_pinned_current_vps_release_root(candidate)?;
                let manifest: VpsReleaseManifestV2 =
                    load_canonical(&candidate.join(RELEASE_MANIFEST_FILE))?;
                ensure!(
                    manifest.canonical_digest()? == digest,
                    "pinned candidate manifest identity changed"
                );
                Ok((digest, manifest.publication_lock_sha256))
            },
            validate_vps_source_publication_v3,
            || activation_lock.ensure_canonical(),
            |_| Ok(()),
            |_| Ok(()),
        )?;
        activation_lock.ensure_canonical()?;
        Ok(release_manifest_sha256)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            plan_fd,
            expected_plan_sha256,
            expected_release_manifest_sha256,
            candidate_root_fd,
        );
        anyhow::bail!("VPS source consumption requires Linux openat2 and renameat2")
    }
}

/// Atomically promote the exact candidate directory retained by the outer
/// transaction. No release copy is made: the inherited inode moves from its
/// canonical partial basename to its canonical installed basename.
pub fn promote_inherited_vps_release_v2(
    expected_release_manifest_sha256: &str,
    candidate_root_fd: std::os::fd::RawFd,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<Digest32> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{AtFlags, FileType, RenameFlags, renameat_with, statat};
        use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};

        let expected = expected_release_manifest_sha256
            .parse::<Digest32>()
            .context("expected VPS release manifest is not canonical lowercase hexadecimal")?;
        ensure!(
            !expected.is_zero(),
            "expected VPS release manifest digest is zero"
        );
        ensure!(
            candidate_root_fd >= 3,
            "candidate-root descriptor must be at least 3"
        );
        let descriptor = PathBuf::from(format!("/proc/self/fd/{candidate_root_fd}"));
        let duplicate = OwnedFd::from(File::open(&descriptor)?);
        let root = PathBuf::from(format!("/proc/self/fd/{}/.", duplicate.as_raw_fd()));
        ensure!(
            validate_pinned_current_vps_release_root(&root)? == expected,
            "retained candidate differs from the out-of-band V2 manifest digest"
        );
        let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
        let candidate = pin_inherited_vps_candidate_root_v2(
            &manifest.source_commit,
            candidate_root_fd,
            expected,
        )?;
        let activation_lock = pin_inherited_vps_activation_lock_v2(activation_lock_fd)?;
        activation_lock.ensure_canonical()?;

        if candidate.canonical_path()? == candidate.installed_path {
            candidate.ensure_canonical()?;
            return Ok(expected);
        }
        ensure!(
            candidate.canonical_path()? == candidate.partial_path,
            "retained candidate is neither the partial nor installed release root"
        );
        let release_parent = &candidate.parents.installed_parent_fd;
        let partial_parent_metadata = rustix::fs::fstat(release_parent)?;
        let release_metadata = rustix::fs::fstat(release_parent)?;
        ensure!(
            FileType::from_raw_mode(partial_parent_metadata.st_mode).is_dir()
                && FileType::from_raw_mode(release_metadata.st_mode).is_dir()
                && partial_parent_metadata.st_uid == rustix::process::geteuid().as_raw()
                && release_metadata.st_uid == rustix::process::geteuid().as_raw()
                && partial_parent_metadata.st_mode & 0o777 == 0o750
                && release_metadata.st_mode & 0o777 == 0o750
                && partial_parent_metadata.st_dev == release_metadata.st_dev
                && partial_parent_metadata.st_dev == candidate.device,
            "VPS promotion parents have unsafe identity, owner, mode, or device"
        );
        let partial_name = candidate
            .partial_path
            .file_name()
            .context("partial candidate has no basename")?;
        let installed_name = candidate
            .installed_path
            .file_name()
            .context("installed candidate has no basename")?;
        let named = statat(
            release_parent.as_fd(),
            partial_name,
            AtFlags::SYMLINK_NOFOLLOW,
        )?;
        ensure!(
            named.st_dev == candidate.device && named.st_ino == candidate.inode,
            "partial candidate basename differs from the retained descriptor"
        );
        ensure!(
            statat(
                release_parent.as_fd(),
                installed_name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .is_err_and(|error| error == rustix::io::Errno::NOENT),
            "installed VPS release already exists"
        );
        activation_lock.ensure_canonical()?;
        let rename = renameat_with(
            release_parent.as_fd(),
            partial_name,
            release_parent.as_fd(),
            installed_name,
            RenameFlags::NOREPLACE,
        );
        if let Err(error) = rename {
            if candidate.canonical_path()? != candidate.installed_path {
                return Err(error.into());
            }
        }
        candidate.ensure_canonical()?;
        rustix::fs::fsync(&release_parent)?;
        activation_lock.ensure_canonical()?;
        candidate.ensure_canonical()?;
        Ok(expected)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            expected_release_manifest_sha256,
            candidate_root_fd,
            activation_lock_fd,
        );
        anyhow::bail!("VPS inherited candidate promotion requires Linux")
    }
}

#[cfg(target_os = "linux")]
const RUNTIME_FENCE_INTENT_NAME: &str = ".runtime-fence-init-v1.json";
#[cfg(target_os = "linux")]
const RUNTIME_FENCE_INTENT_TEMPORARY_NAME: &str = ".runtime-fence-init-v1.json.new";
#[cfg(target_os = "linux")]
const RUNTIME_FENCE_INTENT_WRITING_NAME: &str = ".runtime-fence-init-v1.json.writing";

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeFenceInitPhaseV1 {
    Authorized,
    StagingBound,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeFenceInitIntentV1 {
    schema_version: u32,
    phase: RuntimeFenceInitPhaseV1,
    source_commit: String,
    staging_name: String,
    staging_device: Option<u64>,
    staging_inode: Option<u64>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeFenceInitBoundaryV1 {
    IntentWritingCreated,
    IntentWritingSynced,
    IntentNewPublished,
    AuthorizedIntentPublished,
    StagingCreated,
    BoundIntentWritingCreated,
    BoundIntentWritingSynced,
    BoundIntentNewPublished,
    BoundIntentExchanged,
    StagingBoundIntentPublished,
    AdmissionLeafSynced,
    QuiescenceLeafSynced,
    StagingSealed,
    FinalPublished,
    IntentRemoved,
}

#[cfg(target_os = "linux")]
struct PinnedRuntimeFenceIntentV1 {
    file: File,
    metadata: rustix::fs::Stat,
    document: RuntimeFenceInitIntentV1,
    bytes: Vec<u8>,
}

#[cfg(target_os = "linux")]
fn pin_runtime_fence_intent(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
    allowed_modes: &[u32],
) -> Result<PinnedRuntimeFenceIntentV1> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat};
    use std::io::Read as _;
    use std::os::fd::AsFd as _;

    let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    let state_metadata = rustix::fs::fstat(state)?;
    ensure!(
        FileType::from_raw_mode(named.st_mode).is_file()
            && named.st_uid == rustix::process::geteuid().as_raw()
            && named.st_dev == state_metadata.st_dev
            && named.st_nlink == 1
            && allowed_modes.contains(&(named.st_mode & 0o777))
            && named.st_size > 0
            && named.st_size as u64 <= MAX_DOCUMENT_BYTES,
        "runtime-fence initializer intent has unsafe metadata"
    );
    let fd = openat2(
        state.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let metadata = rustix::fs::fstat(&fd)?;
    ensure!(
        metadata.st_dev == named.st_dev
            && metadata.st_ino == named.st_ino
            && metadata.st_uid == named.st_uid
            && metadata.st_mode == named.st_mode
            && metadata.st_nlink == named.st_nlink
            && metadata.st_size == named.st_size,
        "runtime-fence initializer intent changed while it was pinned"
    );
    let mut file = File::from(fd);
    let mut bytes = Vec::with_capacity(metadata.st_size as usize);
    file.read_to_end(&mut bytes)?;
    let after_read = rustix::fs::fstat(&file)?;
    ensure!(
        after_read.st_dev == metadata.st_dev
            && after_read.st_ino == metadata.st_ino
            && after_read.st_uid == metadata.st_uid
            && after_read.st_mode == metadata.st_mode
            && after_read.st_nlink == metadata.st_nlink
            && after_read.st_size == metadata.st_size
            && after_read.st_mtime == metadata.st_mtime
            && after_read.st_mtime_nsec == metadata.st_mtime_nsec
            && after_read.st_ctime == metadata.st_ctime
            && after_read.st_ctime_nsec == metadata.st_ctime_nsec,
        "runtime-fence initializer intent changed while it was read"
    );
    let document: RuntimeFenceInitIntentV1 = strict_json_from_slice(&bytes)?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "runtime-fence initializer intent is not canonical JSON"
    );
    Ok(PinnedRuntimeFenceIntentV1 {
        file,
        metadata,
        document,
        bytes,
    })
}

#[cfg(target_os = "linux")]
fn validate_runtime_fence_intent(
    intent: &RuntimeFenceInitIntentV1,
    source_commit: &str,
    staging_name: &str,
    state_device: u64,
) -> Result<()> {
    ensure!(
        intent.schema_version == 1
            && intent.source_commit == source_commit
            && intent.staging_name == staging_name
            && match intent.phase {
                RuntimeFenceInitPhaseV1::Authorized => {
                    intent.staging_device.is_none() && intent.staging_inode.is_none()
                }
                RuntimeFenceInitPhaseV1::StagingBound => {
                    intent.staging_device == Some(state_device)
                        && intent.staging_inode.is_some_and(|inode| inode != 0)
                }
            },
        "runtime-fence initializer intent does not bind this exact initialization"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn runtime_fence_bound_identity(intent: &RuntimeFenceInitIntentV1) -> Result<(u64, u64)> {
    ensure!(
        intent.phase == RuntimeFenceInitPhaseV1::StagingBound,
        "runtime-fence initializer intent has not bound a staging inode"
    );
    Ok((
        intent
            .staging_device
            .context("runtime-fence bound intent omitted its device")?,
        intent
            .staging_inode
            .context("runtime-fence bound intent omitted its inode")?,
    ))
}

#[cfg(target_os = "linux")]
fn runtime_fence_named_identity(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<Option<(u64, u64)>> {
    use rustix::fs::{AtFlags, FileType, statat};
    use std::os::fd::AsFd as _;

    match statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => {
            ensure!(
                FileType::from_raw_mode(metadata.st_mode).is_dir(),
                "runtime-fence initializer authority name is not a directory"
            );
            Ok(Some((metadata.st_dev, metadata.st_ino)))
        }
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
fn remove_exact_runtime_fence_intent(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
    pinned: &PinnedRuntimeFenceIntentV1,
) -> Result<()> {
    use rustix::fs::{AtFlags, statat, unlinkat};
    use std::os::fd::AsFd as _;

    let observed = pin_runtime_fence_intent(state, name, &[0o400])?;
    ensure!(
        observed.metadata.st_dev == pinned.metadata.st_dev
            && observed.metadata.st_ino == pinned.metadata.st_ino
            && observed.metadata.st_mode == pinned.metadata.st_mode
            && observed.metadata.st_nlink == pinned.metadata.st_nlink
            && observed.metadata.st_size == pinned.metadata.st_size
            && observed.bytes == pinned.bytes,
        "runtime-fence initializer intent was substituted before removal"
    );
    let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    ensure!(
        named.st_dev == observed.metadata.st_dev
            && named.st_ino == observed.metadata.st_ino
            && named.st_mode == observed.metadata.st_mode
            && named.st_nlink == 1,
        "runtime-fence initializer intent basename changed before removal"
    );
    unlinkat(state.as_fd(), name, AtFlags::empty())?;
    ensure!(
        rustix::fs::fstat(&observed.file)?.st_nlink == 0,
        "runtime-fence initializer intent remains linked after removal"
    );
    rustix::fs::fsync(state)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_runtime_fence_writing_scratch(
    state: &std::os::fd::OwnedFd,
    name: &std::ffi::OsStr,
) -> Result<()> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, ResolveFlags, openat2, statat, unlinkat};
    use std::os::fd::AsFd as _;

    let state_metadata = rustix::fs::fstat(state)?;
    let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
    ensure!(
        FileType::from_raw_mode(named.st_mode).is_file()
            && named.st_uid == rustix::process::geteuid().as_raw()
            && named.st_dev == state_metadata.st_dev
            && named.st_nlink == 1
            && matches!(named.st_mode & 0o777, 0o400 | 0o600)
            && named.st_size >= 0
            && named.st_size as u64 <= MAX_DOCUMENT_BYTES,
        "runtime-fence intent writing scratch has unsafe metadata"
    );
    let scratch = openat2(
        state.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let pinned = rustix::fs::fstat(&scratch)?;
    ensure!(
        pinned.st_dev == named.st_dev
            && pinned.st_ino == named.st_ino
            && pinned.st_uid == named.st_uid
            && pinned.st_mode == named.st_mode
            && pinned.st_nlink == named.st_nlink
            && pinned.st_size == named.st_size,
        "runtime-fence intent writing scratch changed while it was pinned"
    );
    unlinkat(state.as_fd(), name, AtFlags::empty())?;
    ensure!(
        rustix::fs::fstat(&scratch)?.st_nlink == 0,
        "runtime-fence intent writing scratch remains linked after removal"
    );
    rustix::fs::fsync(state)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_runtime_fence_intent_new<F>(
    state: &std::os::fd::OwnedFd,
    writing_name: &std::ffi::OsStr,
    new_name: &std::ffi::OsStr,
    document: &RuntimeFenceInitIntentV1,
    activation_lock: &PinnedVpsActivationLockV2,
    boundaries: (
        RuntimeFenceInitBoundaryV1,
        RuntimeFenceInitBoundaryV1,
        RuntimeFenceInitBoundaryV1,
    ),
    after_boundary: &mut F,
) -> Result<PinnedRuntimeFenceIntentV1>
where
    F: FnMut(RuntimeFenceInitBoundaryV1) -> Result<()>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, fchmod, openat2, renameat_with};
    use std::os::fd::AsFd as _;

    ensure!(
        !pinned_entry_exists(state, &writing_name.to_os_string())?
            && !pinned_entry_exists(state, &new_name.to_os_string())?,
        "runtime-fence initializer intent scratch is not clean"
    );
    let bytes = canonical_json_bytes(document)?;
    let writing_fd = openat2(
        state.as_fd(),
        writing_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut writing = File::from(writing_fd);
    after_boundary(boundaries.0)?;
    writing.write_all(&bytes)?;
    writing.sync_all()?;
    rustix::fs::fsync(state)?;
    after_boundary(boundaries.1)?;
    fchmod(&writing, Mode::from_raw_mode(0o400))?;
    writing.sync_all()?;
    let pinned = pin_runtime_fence_intent(state, writing_name, &[0o400])?;
    ensure!(
        pinned.document == *document && pinned.bytes == bytes,
        "runtime-fence initializer intent writing scratch changed before publication"
    );
    activation_lock.ensure_canonical()?;
    renameat_with(
        state.as_fd(),
        writing_name,
        state.as_fd(),
        new_name,
        RenameFlags::NOREPLACE,
    )?;
    rustix::fs::fsync(state)?;
    after_boundary(boundaries.2)?;
    activation_lock.ensure_canonical()?;
    let published = pin_runtime_fence_intent(state, new_name, &[0o400])?;
    ensure!(
        published.document == *document && published.bytes == bytes,
        "runtime-fence initializer intent .new differs from complete writing scratch"
    );
    Ok(published)
}

/// Create the two permanent runtime lock inodes through a private,
/// crash-resumable staging directory, then atomically publish that exact
/// directory. Runtime processes only ever adopt the sealed final topology.
pub fn initialize_vps_runtime_fence_v1(
    source_commit: &str,
    activation_lock_fd: std::os::fd::RawFd,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let activation_lock = pin_inherited_vps_activation_lock_v2(activation_lock_fd)?;
        initialize_vps_runtime_fence_v1_at(
            source_commit,
            &activation_lock,
            Path::new(STATE_ROOT),
            |_| Ok(()),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source_commit, activation_lock_fd);
        anyhow::bail!("VPS runtime-fence initialization requires Linux")
    }
}

#[cfg(target_os = "linux")]
fn initialize_vps_runtime_fence_v1_at<F>(
    source_commit: &str,
    activation_lock: &PinnedVpsActivationLockV2,
    state_path: &Path,
    mut after_boundary: F,
) -> Result<()>
where
    F: FnMut(RuntimeFenceInitBoundaryV1) -> Result<()>,
{
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, RenameFlags, ResolveFlags, fchmod, mkdirat,
        openat2, renameat_with, statat,
    };
    use std::ffi::{OsStr, OsString};
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStringExt as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    ensure!(
        valid_source_commit(source_commit),
        "invalid runtime-fence source commit"
    );
    activation_lock.ensure_canonical()?;
    let state_metadata = fs::symlink_metadata(state_path)?;
    ensure!(
        state_metadata.is_dir()
            && !state_metadata.file_type().is_symlink()
            && fs::canonicalize(state_path)? == state_path
            && state_metadata.uid() == rustix::process::geteuid().as_raw()
            && state_metadata.permissions().mode() & 0o777 == 0o700,
        "runtime-fence state root is not canonical owner-only mode 0700"
    );
    let state = openat2(
        rustix::fs::CWD,
        state_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let state_pinned = rustix::fs::fstat(&state)?;
    ensure!(
        state_pinned.st_dev == state_metadata.dev() && state_pinned.st_ino == state_metadata.ino(),
        "runtime-fence state root changed while it was pinned"
    );
    let final_name = OsString::from("runtime-fence");
    let staging_name = OsString::from(format!(".runtime-fence-{source_commit}.partial"));
    let staging_name_text = staging_name
        .to_str()
        .context("runtime-fence staging name is not UTF-8")?;
    let intent_name = OsString::from(RUNTIME_FENCE_INTENT_NAME);
    let intent_temporary_name = OsString::from(RUNTIME_FENCE_INTENT_TEMPORARY_NAME);
    let intent_writing_name = OsString::from(RUNTIME_FENCE_INTENT_WRITING_NAME);

    let scan = openat2(
        state.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut buffer = Vec::with_capacity(4096);
    let mut directory = RawDir::new(&scan, buffer.spare_capacity_mut());
    while let Some(entry) = directory.next() {
        let entry = entry?;
        let bytes = entry.file_name().to_bytes();
        if bytes.starts_with(b".runtime-fence-") && bytes.ends_with(b".partial") {
            ensure!(
                bytes == staging_name.as_encoded_bytes(),
                "foreign runtime-fence staging evidence exists"
            );
        }
    }

    let validate_fence =
        |root: &std::os::fd::OwnedFd, mode: u32, allow_subset: bool| -> Result<()> {
            let root_metadata = rustix::fs::fstat(root)?;
            ensure!(
                FileType::from_raw_mode(root_metadata.st_mode).is_dir()
                    && root_metadata.st_uid == rustix::process::geteuid().as_raw()
                    && root_metadata.st_dev == state_pinned.st_dev
                    && root_metadata.st_mode & 0o777 == mode,
                "runtime-fence directory has unsafe identity, owner, device, or mode"
            );
            let scan = openat2(
                root.as_fd(),
                ".",
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )?;
            let mut buffer = Vec::with_capacity(4096);
            let mut directory = RawDir::new(&scan, buffer.spare_capacity_mut());
            let mut names = BTreeSet::new();
            while let Some(entry) = directory.next() {
                let entry = entry?;
                let bytes = entry.file_name().to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                let name = OsString::from_vec(bytes.to_vec());
                ensure!(
                    name == "db-admission.lock" || name == "db-quiescence.lock",
                    "runtime-fence contains an unexpected entry"
                );
                ensure!(
                    names.insert(name.clone()),
                    "runtime-fence entry is duplicated"
                );
                let named = statat(root.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
                ensure!(
                    FileType::from_raw_mode(named.st_mode).is_file()
                        && named.st_uid == rustix::process::geteuid().as_raw()
                        && named.st_dev == state_pinned.st_dev
                        && named.st_nlink == 1
                        && named.st_size == 0
                        && named.st_mode & 0o777 == 0o400,
                    "runtime-fence leaf has unsafe type, owner, device, links, size, or mode"
                );
                let leaf = openat2(
                    root.as_fd(),
                    &name,
                    OFlags::RDONLY | OFlags::CLOEXEC,
                    Mode::empty(),
                    ResolveFlags::BENEATH
                        | ResolveFlags::NO_SYMLINKS
                        | ResolveFlags::NO_MAGICLINKS
                        | ResolveFlags::NO_XDEV,
                )?;
                let pinned = rustix::fs::fstat(&leaf)?;
                ensure!(
                    pinned.st_dev == named.st_dev
                        && pinned.st_ino == named.st_ino
                        && pinned.st_uid == named.st_uid
                        && pinned.st_mode == named.st_mode
                        && pinned.st_nlink == named.st_nlink
                        && pinned.st_size == named.st_size,
                    "runtime-fence leaf changed while it was pinned"
                );
            }
            ensure!(
                allow_subset
                    || names
                        == BTreeSet::from([
                            OsString::from("db-admission.lock"),
                            OsString::from("db-quiescence.lock"),
                        ]),
                "runtime-fence final inventory is incomplete"
            );
            Ok(())
        };

    let open_fence = |name: &OsStr| -> Result<std::os::fd::OwnedFd> {
        use rustix::fs::StatxFlags;

        let named = statat(state.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)?;
        let state_mount = rustix::fs::statx(
            state.as_fd(),
            ".",
            AtFlags::NO_AUTOMOUNT,
            StatxFlags::MNT_ID,
        )?;
        let named_mount = rustix::fs::statx(
            state.as_fd(),
            name,
            AtFlags::NO_AUTOMOUNT | AtFlags::SYMLINK_NOFOLLOW,
            StatxFlags::MNT_ID,
        )?;
        ensure!(
            state_mount.stx_mask & StatxFlags::MNT_ID.bits() != 0
                && named_mount.stx_mask & StatxFlags::MNT_ID.bits() != 0
                && state_mount.stx_mnt_id == named_mount.stx_mnt_id,
            "runtime-fence staging is a nested mount"
        );
        let root = openat2(
            state.as_fd(),
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let pinned = rustix::fs::fstat(&root)?;
        ensure!(
            pinned.st_dev == named.st_dev
                && pinned.st_ino == named.st_ino
                && pinned.st_uid == named.st_uid
                && pinned.st_mode == named.st_mode,
            "runtime-fence directory changed while it was pinned"
        );
        Ok(root)
    };

    if pinned_entry_exists(&state, &final_name)? {
        ensure!(
            !pinned_entry_exists(&state, &staging_name)?
                && !pinned_entry_exists(&state, &intent_temporary_name)?
                && !pinned_entry_exists(&state, &intent_writing_name)?,
            "sealed runtime-fence coexists with non-terminal initializer evidence"
        );
        let final_root = open_fence(&final_name)?;
        validate_fence(&final_root, 0o500, false)?;
        if pinned_entry_exists(&state, &intent_name)? {
            let pinned_intent = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
            validate_runtime_fence_intent(
                &pinned_intent.document,
                source_commit,
                staging_name_text,
                state_pinned.st_dev,
            )?;
            let final_metadata = rustix::fs::fstat(&final_root)?;
            ensure!(
                (final_metadata.st_dev, final_metadata.st_ino)
                    == runtime_fence_bound_identity(&pinned_intent.document)?,
                "sealed runtime-fence differs from its durable initializer intent"
            );
            if pinned_entry_exists(&state, &intent_temporary_name)? {
                let old = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
                validate_runtime_fence_intent(
                    &old.document,
                    source_commit,
                    staging_name_text,
                    state_pinned.st_dev,
                )?;
                ensure!(
                    old.document.phase == RuntimeFenceInitPhaseV1::Authorized,
                    "sealed runtime-fence retained a non-authorized predecessor intent"
                );
                activation_lock.ensure_canonical()?;
                remove_exact_runtime_fence_intent(&state, &intent_temporary_name, &old)?;
            }
            activation_lock.ensure_canonical()?;
            remove_exact_runtime_fence_intent(&state, &intent_name, &pinned_intent)?;
            after_boundary(RuntimeFenceInitBoundaryV1::IntentRemoved)?;
        } else {
            ensure!(
                !pinned_entry_exists(&state, &intent_temporary_name)?,
                "sealed runtime-fence lacks its primary intent but retains .new"
            );
        }
        activation_lock.ensure_canonical()?;
        return Ok(());
    }

    let mut intent = if pinned_entry_exists(&state, &intent_name)? {
        let current = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
        validate_runtime_fence_intent(
            &current.document,
            source_commit,
            staging_name_text,
            state_pinned.st_dev,
        )?;
        Some(current)
    } else {
        None
    };

    if pinned_entry_exists(&state, &intent_writing_name)? {
        ensure!(
            intent.is_some()
                || (!pinned_entry_exists(&state, &staging_name)?
                    && !pinned_entry_exists(&state, &intent_temporary_name)?),
            "unauthorized runtime-fence intent scratch coexists with mutated state"
        );
        activation_lock.ensure_canonical()?;
        remove_runtime_fence_writing_scratch(&state, &intent_writing_name)?;
        activation_lock.ensure_canonical()?;
    }

    if intent.is_none() {
        ensure!(
            !pinned_entry_exists(&state, &staging_name)?,
            "runtime-fence staging exists before a durable authorization intent"
        );
        if pinned_entry_exists(&state, &intent_temporary_name)? {
            let authorized = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
            validate_runtime_fence_intent(
                &authorized.document,
                source_commit,
                staging_name_text,
                state_pinned.st_dev,
            )?;
            ensure!(
                authorized.document.phase == RuntimeFenceInitPhaseV1::Authorized,
                "initial runtime-fence .new is not an authorization intent"
            );
        } else {
            let authorized = RuntimeFenceInitIntentV1 {
                schema_version: 1,
                phase: RuntimeFenceInitPhaseV1::Authorized,
                source_commit: source_commit.to_owned(),
                staging_name: staging_name_text.to_owned(),
                staging_device: None,
                staging_inode: None,
            };
            write_runtime_fence_intent_new(
                &state,
                &intent_writing_name,
                &intent_temporary_name,
                &authorized,
                activation_lock,
                (
                    RuntimeFenceInitBoundaryV1::IntentWritingCreated,
                    RuntimeFenceInitBoundaryV1::IntentWritingSynced,
                    RuntimeFenceInitBoundaryV1::IntentNewPublished,
                ),
                &mut after_boundary,
            )?;
        }
        activation_lock.ensure_canonical()?;
        renameat_with(
            state.as_fd(),
            &intent_temporary_name,
            state.as_fd(),
            &intent_name,
            RenameFlags::NOREPLACE,
        )?;
        rustix::fs::fsync(&state)?;
        after_boundary(RuntimeFenceInitBoundaryV1::AuthorizedIntentPublished)?;
        intent = Some(pin_runtime_fence_intent(&state, &intent_name, &[0o400])?);
    }

    let mut intent = intent.context("runtime-fence initialization lacks a durable intent")?;
    if pinned_entry_exists(&state, &intent_temporary_name)? {
        let adjacent = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
        validate_runtime_fence_intent(
            &adjacent.document,
            source_commit,
            staging_name_text,
            state_pinned.st_dev,
        )?;
        match (intent.document.phase, adjacent.document.phase) {
            (RuntimeFenceInitPhaseV1::Authorized, RuntimeFenceInitPhaseV1::StagingBound) => {
                ensure!(
                    runtime_fence_named_identity(&state, &staging_name)?
                        == Some(runtime_fence_bound_identity(&adjacent.document)?),
                    "bound .new intent differs from the retained staging inode"
                );
                activation_lock.ensure_canonical()?;
                renameat_with(
                    state.as_fd(),
                    &intent_name,
                    state.as_fd(),
                    &intent_temporary_name,
                    RenameFlags::EXCHANGE,
                )?;
                rustix::fs::fsync(&state)?;
                after_boundary(RuntimeFenceInitBoundaryV1::BoundIntentExchanged)?;
                intent = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
            }
            (RuntimeFenceInitPhaseV1::StagingBound, RuntimeFenceInitPhaseV1::Authorized) => {}
            _ => anyhow::bail!("runtime-fence intents are not an exact adjacent transition"),
        }
        let predecessor = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
        ensure!(
            predecessor.document.phase == RuntimeFenceInitPhaseV1::Authorized,
            "runtime-fence .new does not contain the exact authorized predecessor"
        );
        activation_lock.ensure_canonical()?;
        remove_exact_runtime_fence_intent(&state, &intent_temporary_name, &predecessor)?;
        after_boundary(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)?;
    }

    if intent.document.phase == RuntimeFenceInitPhaseV1::Authorized {
        if !pinned_entry_exists(&state, &staging_name)? {
            activation_lock.ensure_canonical()?;
            mkdirat(state.as_fd(), &staging_name, Mode::from_raw_mode(0o700))?;
            rustix::fs::fsync(&state)?;
            after_boundary(RuntimeFenceInitBoundaryV1::StagingCreated)?;
        }
        let staging = open_fence(&staging_name)?;
        validate_fence(&staging, 0o700, true)?;
        let staging_metadata = rustix::fs::fstat(&staging)?;
        let bound = RuntimeFenceInitIntentV1 {
            schema_version: 1,
            phase: RuntimeFenceInitPhaseV1::StagingBound,
            source_commit: source_commit.to_owned(),
            staging_name: staging_name_text.to_owned(),
            staging_device: Some(staging_metadata.st_dev),
            staging_inode: Some(staging_metadata.st_ino),
        };
        write_runtime_fence_intent_new(
            &state,
            &intent_writing_name,
            &intent_temporary_name,
            &bound,
            activation_lock,
            (
                RuntimeFenceInitBoundaryV1::BoundIntentWritingCreated,
                RuntimeFenceInitBoundaryV1::BoundIntentWritingSynced,
                RuntimeFenceInitBoundaryV1::BoundIntentNewPublished,
            ),
            &mut after_boundary,
        )?;
        activation_lock.ensure_canonical()?;
        renameat_with(
            state.as_fd(),
            &intent_name,
            state.as_fd(),
            &intent_temporary_name,
            RenameFlags::EXCHANGE,
        )?;
        rustix::fs::fsync(&state)?;
        after_boundary(RuntimeFenceInitBoundaryV1::BoundIntentExchanged)?;
        let predecessor = pin_runtime_fence_intent(&state, &intent_temporary_name, &[0o400])?;
        ensure!(
            predecessor.document == intent.document,
            "runtime-fence bound-intent exchange did not retain its exact predecessor"
        );
        intent = pin_runtime_fence_intent(&state, &intent_name, &[0o400])?;
        ensure!(
            intent.document == bound,
            "runtime-fence bound-intent exchange did not publish its exact successor"
        );
        activation_lock.ensure_canonical()?;
        remove_exact_runtime_fence_intent(&state, &intent_temporary_name, &predecessor)?;
        after_boundary(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)?;
    }

    let staging_identity = runtime_fence_named_identity(&state, &staging_name)?;
    ensure!(
        staging_identity == Some(runtime_fence_bound_identity(&intent.document)?),
        "runtime-fence staging differs from its durable initializer intent"
    );
    let staging = open_fence(&staging_name)?;
    let staging_mode = rustix::fs::fstat(&staging)?.st_mode & 0o777;
    ensure!(
        staging_mode == 0o700 || staging_mode == 0o500,
        "runtime-fence staging has an unsafe mode"
    );
    validate_fence(&staging, staging_mode, true)?;
    activation_lock.ensure_canonical()?;
    fchmod(&staging, Mode::from_raw_mode(0o700))?;
    for (leaf_name, boundary) in [
        (
            "db-admission.lock",
            RuntimeFenceInitBoundaryV1::AdmissionLeafSynced,
        ),
        (
            "db-quiescence.lock",
            RuntimeFenceInitBoundaryV1::QuiescenceLeafSynced,
        ),
    ] {
        let leaf = match openat2(
            staging.as_fd(),
            leaf_name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o400),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        ) {
            Ok(leaf) => leaf,
            Err(rustix::io::Errno::EXIST) => openat2(
                staging.as_fd(),
                leaf_name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?,
            Err(error) => return Err(error.into()),
        };
        let metadata = rustix::fs::fstat(&leaf)?;
        ensure!(
            FileType::from_raw_mode(metadata.st_mode).is_file()
                && metadata.st_uid == rustix::process::geteuid().as_raw()
                && metadata.st_dev == state_pinned.st_dev
                && metadata.st_nlink == 1
                && metadata.st_size == 0
                && metadata.st_mode & 0o777 == 0o400,
            "runtime-fence leaf creation did not produce exact authority"
        );
        rustix::fs::fsync(&leaf)?;
        rustix::fs::fsync(&staging)?;
        after_boundary(boundary)?;
    }
    activation_lock.ensure_canonical()?;
    fchmod(&staging, Mode::from_raw_mode(0o500))?;
    rustix::fs::fsync(&staging)?;
    validate_fence(&staging, 0o500, false)?;
    after_boundary(RuntimeFenceInitBoundaryV1::StagingSealed)?;
    let staging_metadata = rustix::fs::fstat(&staging)?;
    activation_lock.ensure_canonical()?;
    let rename = renameat_with(
        state.as_fd(),
        &staging_name,
        state.as_fd(),
        &final_name,
        RenameFlags::NOREPLACE,
    );
    if let Err(error) = rename {
        let final_metadata = statat(state.as_fd(), &final_name, AtFlags::SYMLINK_NOFOLLOW);
        if !final_metadata.as_ref().is_ok_and(|metadata| {
            metadata.st_dev == staging_metadata.st_dev && metadata.st_ino == staging_metadata.st_ino
        }) {
            return Err(error.into());
        }
    }
    rustix::fs::fsync(&state)?;
    after_boundary(RuntimeFenceInitBoundaryV1::FinalPublished)?;
    let final_root = open_fence(&final_name)?;
    let final_metadata = rustix::fs::fstat(&final_root)?;
    ensure!(
        (final_metadata.st_dev, final_metadata.st_ino)
            == runtime_fence_bound_identity(&intent.document)?,
        "runtime-fence final differs from its durable initializer intent"
    );
    validate_fence(&final_root, 0o500, false)?;
    ensure!(
        !pinned_entry_exists(&state, &staging_name)?,
        "runtime-fence staging remains after publication"
    );
    activation_lock.ensure_canonical()?;
    remove_exact_runtime_fence_intent(&state, &intent_name, &intent)?;
    after_boundary(RuntimeFenceInitBoundaryV1::IntentRemoved)?;
    activation_lock.ensure_canonical()?;
    Ok(())
}

#[cfg(target_os = "linux")]
struct PinnedVpsSourceRoot {
    name: std::ffi::OsString,
    fd: std::os::fd::OwnedFd,
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
struct PinnedVpsCandidateParentsV2 {
    install_root_fd: std::os::fd::OwnedFd,
    incoming_parent_fd: std::os::fd::OwnedFd,
    installed_parent_fd: std::os::fd::OwnedFd,
    install_root_path: PathBuf,
    incoming_parent_path: PathBuf,
    installed_parent_path: PathBuf,
    install_root_device: u64,
    install_root_inode: u64,
    incoming_parent_device: u64,
    incoming_parent_inode: u64,
    installed_parent_device: u64,
    installed_parent_inode: u64,
}

#[cfg(target_os = "linux")]
struct PinnedInheritedVpsCandidateRootV2 {
    fd: std::os::fd::OwnedFd,
    parents: PinnedVpsCandidateParentsV2,
    partial_path: PathBuf,
    installed_path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
impl PinnedInheritedVpsCandidateRootV2 {
    fn canonical_path(&self) -> Result<PathBuf> {
        use rustix::fs::{AtFlags, FileType, statat};
        use std::os::fd::AsFd as _;

        let matches = [
            (&self.parents.installed_parent_fd, &self.partial_path),
            (&self.parents.installed_parent_fd, &self.installed_path),
        ]
        .into_iter()
        .filter(|(parent, path)| {
            path.file_name().is_some_and(|basename| {
                statat(parent.as_fd(), basename, AtFlags::SYMLINK_NOFOLLOW).is_ok_and(|metadata| {
                    FileType::from_raw_mode(metadata.st_mode).is_dir()
                        && metadata.st_dev == self.device
                        && metadata.st_ino == self.inode
                })
            })
        })
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "inherited VPS candidate must name exactly one canonical partial or installed root"
        );
        Ok(matches.into_iter().next().expect("length checked"))
    }

    fn ensure_canonical(&self) -> Result<()> {
        use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let pinned = rustix::fs::fstat(&self.fd)?;
        ensure!(
            FileType::from_raw_mode(pinned.st_mode).is_dir()
                && pinned.st_dev == self.device
                && pinned.st_ino == self.inode
                && pinned.st_uid == rustix::process::geteuid().as_raw()
                && pinned.st_mode & 0o777 == 0o550,
            "inherited VPS candidate descriptor changed"
        );
        let validate_named_parent = |path: &Path,
                                     retained: &std::os::fd::OwnedFd,
                                     expected_device: u64,
                                     expected_inode: u64|
         -> Result<()> {
            let named = fs::symlink_metadata(path)?;
            let held = rustix::fs::fstat(retained)?;
            ensure!(
                named.is_dir()
                    && !named.file_type().is_symlink()
                    && fs::canonicalize(path)? == path
                    && named.uid() == rustix::process::geteuid().as_raw()
                    && named.permissions().mode() & 0o777 == 0o750
                    && named.dev() == expected_device
                    && named.ino() == expected_inode
                    && held.st_uid == rustix::process::geteuid().as_raw()
                    && held.st_mode & 0o777 == 0o750
                    && held.st_dev == expected_device
                    && held.st_ino == expected_inode,
                "retained VPS candidate parent authority changed"
            );
            Ok(())
        };
        validate_named_parent(
            &self.parents.install_root_path,
            &self.parents.install_root_fd,
            self.parents.install_root_device,
            self.parents.install_root_inode,
        )?;
        let rebound_install_root = openat2(
            rustix::fs::CWD,
            &self.parents.install_root_path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let rebound_install_root = rustix::fs::fstat(&rebound_install_root)?;
        ensure!(
            rebound_install_root.st_dev == self.parents.install_root_device
                && rebound_install_root.st_ino == self.parents.install_root_inode,
            "retained VPS install root pathname changed"
        );
        for (name, path, retained, expected_device, expected_inode) in [
            (
                "incoming",
                &self.parents.incoming_parent_path,
                &self.parents.incoming_parent_fd,
                self.parents.incoming_parent_device,
                self.parents.incoming_parent_inode,
            ),
            (
                "releases",
                &self.parents.installed_parent_path,
                &self.parents.installed_parent_fd,
                self.parents.installed_parent_device,
                self.parents.installed_parent_inode,
            ),
        ] {
            validate_named_parent(path, retained, expected_device, expected_inode)?;
            let rebound = openat2(
                self.parents.install_root_fd.as_fd(),
                name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let rebound = rustix::fs::fstat(&rebound)?;
            ensure!(
                rebound.st_dev == expected_device && rebound.st_ino == expected_inode,
                "retained VPS candidate parent pathname changed"
            );
        }
        let canonical_path = self.canonical_path()?;
        let named = fs::symlink_metadata(&canonical_path)?;
        ensure!(
            named.is_dir()
                && !named.file_type().is_symlink()
                && fs::canonicalize(&canonical_path)? == canonical_path
                && named.dev() == self.device
                && named.ino() == self.inode
                && named.uid() == rustix::process::geteuid().as_raw()
                && named.permissions().mode() & 0o777 == 0o550,
            "inherited VPS candidate canonical pathname changed"
        );
        let basename = canonical_path
            .file_name()
            .context("inherited VPS candidate has no basename")?;
        let parent = &self.parents.installed_parent_fd;
        let rebound = openat2(
            parent.as_fd(),
            basename,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let rebound = rustix::fs::fstat(&rebound)?;
        ensure!(
            rebound.st_dev == self.device && rebound.st_ino == self.inode,
            "inherited VPS candidate path does not name the retained descriptor"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn pin_inherited_vps_candidate_root_v2(
    source_commit: &str,
    candidate_root_fd: std::os::fd::RawFd,
    expected_manifest_sha256: Digest32,
) -> Result<PinnedInheritedVpsCandidateRootV2> {
    pin_inherited_vps_candidate_root_at(
        source_commit,
        candidate_root_fd,
        expected_manifest_sha256,
        &Path::new(INSTALL_ROOT).join("incoming"),
        &Path::new(INSTALL_ROOT).join("releases"),
    )
}

#[cfg(target_os = "linux")]
fn pin_inherited_vps_candidate_root_at(
    source_commit: &str,
    candidate_root_fd: std::os::fd::RawFd,
    expected_manifest_sha256: Digest32,
    incoming_root: &Path,
    releases_root: &Path,
) -> Result<PinnedInheritedVpsCandidateRootV2> {
    pin_inherited_vps_candidate_root_at_with(
        source_commit,
        candidate_root_fd,
        expected_manifest_sha256,
        incoming_root,
        releases_root,
        |root| {
            let digest = validate_pinned_current_vps_release_root(root)?;
            let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
            Ok((digest, manifest.source_commit))
        },
    )
}

#[cfg(target_os = "linux")]
fn pin_vps_candidate_parents_at(
    incoming_root: &Path,
    releases_root: &Path,
) -> Result<PinnedVpsCandidateParentsV2> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let install_root = incoming_root
        .parent()
        .context("VPS incoming root has no install parent")?;
    ensure!(
        incoming_root
            .file_name()
            .is_some_and(|name| name == "incoming")
            && releases_root
                .file_name()
                .is_some_and(|name| name == "releases")
            && releases_root.parent() == Some(install_root)
            && normalized_absolute(install_root)
            && normalized_absolute(incoming_root)
            && normalized_absolute(releases_root),
        "VPS candidate parents are not the exact normalized incoming/releases siblings"
    );
    let install_named = fs::symlink_metadata(install_root)?;
    ensure!(
        install_named.is_dir()
            && !install_named.file_type().is_symlink()
            && fs::canonicalize(install_root)? == install_root
            && install_named.uid() == rustix::process::geteuid().as_raw()
            && install_named.permissions().mode() & 0o777 == 0o750,
        "VPS candidate install root has unsafe path, owner, or mode"
    );
    let install_root_fd = openat2(
        rustix::fs::CWD,
        install_root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let install_pinned = rustix::fs::fstat(&install_root_fd)?;
    ensure!(
        install_pinned.st_dev == install_named.dev()
            && install_pinned.st_ino == install_named.ino(),
        "VPS candidate install root changed while it was pinned"
    );
    let pin_child = |name: &str, path: &Path| -> Result<std::os::fd::OwnedFd> {
        let named = fs::symlink_metadata(path)?;
        ensure!(
            named.is_dir()
                && !named.file_type().is_symlink()
                && fs::canonicalize(path)? == path
                && named.uid() == rustix::process::geteuid().as_raw()
                && named.permissions().mode() & 0o777 == 0o750
                && named.dev() == install_pinned.st_dev,
            "VPS candidate parent has unsafe path, owner, mode, or device"
        );
        let child = openat2(
            install_root_fd.as_fd(),
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let pinned = rustix::fs::fstat(&child)?;
        ensure!(
            pinned.st_dev == named.dev()
                && pinned.st_ino == named.ino()
                && pinned.st_uid == named.uid()
                && pinned.st_mode & 0o777 == 0o750,
            "VPS candidate parent changed while it was pinned"
        );
        Ok(child)
    };
    let incoming_parent_fd = pin_child("incoming", incoming_root)?;
    let installed_parent_fd = pin_child("releases", releases_root)?;
    let incoming_parent = rustix::fs::fstat(&incoming_parent_fd)?;
    let installed_parent = rustix::fs::fstat(&installed_parent_fd)?;
    Ok(PinnedVpsCandidateParentsV2 {
        install_root_fd,
        incoming_parent_fd,
        installed_parent_fd,
        install_root_path: install_root.to_path_buf(),
        incoming_parent_path: incoming_root.to_path_buf(),
        installed_parent_path: releases_root.to_path_buf(),
        install_root_device: install_pinned.st_dev,
        install_root_inode: install_pinned.st_ino,
        incoming_parent_device: incoming_parent.st_dev,
        incoming_parent_inode: incoming_parent.st_ino,
        installed_parent_device: installed_parent.st_dev,
        installed_parent_inode: installed_parent.st_ino,
    })
}

#[cfg(target_os = "linux")]
fn pin_inherited_vps_candidate_root_at_with<V>(
    source_commit: &str,
    candidate_root_fd: std::os::fd::RawFd,
    expected_manifest_sha256: Digest32,
    incoming_root: &Path,
    releases_root: &Path,
    validate_candidate: V,
) -> Result<PinnedInheritedVpsCandidateRootV2>
where
    V: Fn(&Path) -> Result<(Digest32, String)>,
{
    use rustix::fs::FileType;
    use std::os::fd::{AsRawFd as _, OwnedFd};

    let parents = pin_vps_candidate_parents_at(incoming_root, releases_root)?;

    ensure!(
        candidate_root_fd >= 3,
        "inherited VPS candidate descriptor must be at least 3"
    );
    let inherited = nix_legacy::sys::stat::fstat(candidate_root_fd)?;
    ensure!(
        FileType::from_raw_mode(inherited.st_mode).is_dir()
            && inherited.st_uid == rustix::process::geteuid().as_raw()
            && inherited.st_mode & 0o777 == 0o550,
        "inherited VPS candidate descriptor has unsafe type, owner, or mode"
    );
    let descriptor_path = PathBuf::from(format!("/proc/self/fd/{candidate_root_fd}"));
    let fd = OwnedFd::from(File::open(&descriptor_path)?);
    let pinned = rustix::fs::fstat(&fd)?;
    ensure!(
        pinned.st_dev == inherited.st_dev && pinned.st_ino == inherited.st_ino,
        "inherited VPS candidate descriptor changed while it was duplicated"
    );
    let partial = releases_root.join(format!("{source_commit}.partial"));
    let installed = releases_root.join(source_commit);
    let candidate = PinnedInheritedVpsCandidateRootV2 {
        fd,
        parents,
        partial_path: partial,
        installed_path: installed,
        device: pinned.st_dev,
        inode: pinned.st_ino,
    };
    candidate.ensure_canonical()?;
    let root = PathBuf::from(format!("/proc/self/fd/{}/.", candidate.fd.as_raw_fd()));
    let (manifest_sha256, manifest_source_commit) = validate_candidate(&root)?;
    ensure!(
        manifest_sha256 == expected_manifest_sha256,
        "inherited VPS candidate differs from the out-of-band V2 manifest digest"
    );
    ensure!(
        manifest_source_commit == source_commit,
        "inherited VPS candidate source commit differs from the plan"
    );
    candidate.ensure_canonical()?;
    Ok(candidate)
}

#[cfg(target_os = "linux")]
fn consume_vps_sources_in_with<CV, PV, EL, AR, AU>(
    plan: &VpsReleasePlanV2,
    plan_bytes: &[u8],
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    incoming_parent: &Path,
    logical_source: &Path,
    inherited_candidate: &PinnedInheritedVpsCandidateRootV2,
    validate_candidate: CV,
    validate_publication: PV,
    ensure_lock: EL,
    after_rename: AR,
    after_root_unlink: AU,
) -> Result<()>
where
    CV: Fn(&Path) -> Result<(Digest32, Digest32)>,
    PV: Fn(&PinnedVpsSourceRoot, &Path) -> Result<Digest32>,
    EL: Fn() -> Result<()>,
    AR: Fn(&Path) -> Result<()>,
    AU: Fn(&Path) -> Result<()>,
{
    use rustix::fs::{
        AtFlags, Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with, statat, unlinkat,
    };
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let expected_incoming = Path::new(INSTALL_ROOT).join("incoming");
    if incoming_parent == expected_incoming {
        ensure!(
            fs::canonicalize(incoming_parent)? == incoming_parent,
            "VPS incoming parent is not canonical"
        );
        reject_mounts_at_or_below(incoming_parent)?;
    }
    let incoming_metadata = fs::symlink_metadata(incoming_parent)?;
    ensure!(
        incoming_metadata.is_dir()
            && !incoming_metadata.file_type().is_symlink()
            && incoming_metadata.uid() == rustix::process::geteuid().as_raw()
            && incoming_metadata.permissions().mode() & 0o777 == 0o750,
        "VPS incoming parent must be EUID-owned mode 0750"
    );
    let parent_fd = openat2(
        rustix::fs::CWD,
        incoming_parent,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let parent = rustix::fs::fstat(&parent_fd)?;
    ensure!(
        parent.st_dev == incoming_metadata.dev()
            && parent.st_ino == incoming_metadata.ino()
            && parent.st_uid == rustix::process::geteuid().as_raw(),
        "VPS incoming parent changed while it was pinned"
    );

    let source_name = std::ffi::OsString::from(format!(".sources-{}", plan.source_commit));
    let consuming_name =
        std::ffi::OsString::from(format!(".sources-{}.consuming", plan.source_commit));
    let journal_name =
        std::ffi::OsString::from(format!(".sources-{}.consume-v1.json", plan.source_commit));
    let journal_temporary_name = std::ffi::OsString::from(format!(
        ".sources-{}.consume-v1.json.new",
        plan.source_commit
    ));
    let terminal_journal_name = std::ffi::OsString::from(format!(
        ".sources-{}.consume-v1.complete.json",
        plan.source_commit
    ));
    let terminal_journal_temporary_name = std::ffi::OsString::from(format!(
        ".sources-{}.consume-v1.complete.json.new",
        plan.source_commit
    ));

    let candidate_fd = &inherited_candidate.fd;
    let candidate = rustix::fs::fstat(&candidate_fd)?;
    ensure!(
        candidate.st_uid == rustix::process::geteuid().as_raw()
            && candidate.st_dev == parent.st_dev
            && candidate.st_mode & 0o777 == 0o550,
        "VPS candidate root identity, device, owner, or mode is unsafe"
    );
    let candidate_path = PathBuf::from(format!("/proc/self/fd/{}/.", candidate_fd.as_raw_fd()));
    let (validated_candidate_manifest, candidate_publication_lock) =
        validate_candidate(&candidate_path)?;
    ensure!(
        validated_candidate_manifest == release_manifest_sha256,
        "pinned VPS candidate differs from the expected release manifest digest"
    );
    ensure!(
        fs::read(candidate_path.join(SOURCE_COMMIT_FILE))?
            == format!("{}\n", plan.source_commit).as_bytes(),
        "pinned VPS candidate source commit differs from the plan"
    );
    let ensure_authorities = || -> Result<()> {
        ensure_lock()?;
        ensure_named_vps_incoming_parent(incoming_parent, &parent)?;
        inherited_candidate.ensure_canonical()
    };
    ensure_authorities()?;

    // Recovery names are transaction evidence. Authenticate the exact
    // candidate authority before moving any of them back into place.
    for name in [
        &journal_name,
        &journal_temporary_name,
        &terminal_journal_name,
        &terminal_journal_temporary_name,
    ] {
        restore_vps_source_document_removing(&parent_fd, name, &ensure_authorities)?;
    }

    reconcile_vps_source_journal_temporary(
        &parent_fd,
        &journal_name,
        &journal_temporary_name,
        &source_name,
        &consuming_name,
        &plan.source_commit,
        VpsSourceConsumePhaseV1::Prepared,
        &ensure_authorities,
    )?;
    reconcile_vps_source_journal_temporary(
        &parent_fd,
        &terminal_journal_name,
        &terminal_journal_temporary_name,
        &source_name,
        &consuming_name,
        &plan.source_commit,
        VpsSourceConsumePhaseV1::RootUnlinked,
        &ensure_authorities,
    )?;
    let source_exists = pinned_entry_exists(&parent_fd, &source_name)?;
    let consuming_exists = pinned_entry_exists(&parent_fd, &consuming_name)?;
    ensure!(
        !(source_exists && consuming_exists),
        "both VPS source and consuming roots exist; preserve ambiguous evidence"
    );

    if pinned_entry_exists(&parent_fd, &terminal_journal_name)? {
        let terminal = load_vps_source_consume_journal(&parent_fd, &terminal_journal_name)?;
        validate_vps_source_consume_journal(
            &terminal,
            plan,
            plan_sha256,
            release_manifest_sha256,
            candidate.st_dev,
            candidate.st_ino,
        )?;
        ensure!(
            terminal.phase == VpsSourceConsumePhaseV1::RootUnlinked
                && !source_exists
                && !consuming_exists,
            "terminal VPS source journal coexists with a source root"
        );
        rustix::fs::fsync(&parent_fd)?;
        ensure_authorities()?;
        ensure_lock()?;
        ensure_named_vps_incoming_parent(incoming_parent, &parent)?;
        if pinned_entry_exists(&parent_fd, &journal_name)? {
            let mut expected_prepared = terminal.clone();
            expected_prepared.phase = VpsSourceConsumePhaseV1::Prepared;
            remove_exact_vps_source_journal(
                &parent_fd,
                &journal_name,
                &expected_prepared,
                &ensure_authorities,
            )?;
        }
        remove_exact_vps_source_journal(
            &parent_fd,
            &terminal_journal_name,
            &terminal,
            &ensure_authorities,
        )?;
        ensure_authorities()?;
        return Ok(());
    }

    let mut journal = if pinned_entry_exists(&parent_fd, &journal_name)? {
        load_vps_source_consume_journal(&parent_fd, &journal_name)?
    } else if source_exists {
        let source = open_vps_source_root(&parent_fd, &parent, source_name.clone(), 0o700)?;
        let journal = build_vps_source_consume_journal(
            plan,
            plan_bytes,
            plan_sha256,
            release_manifest_sha256,
            &source,
            candidate.st_dev,
            candidate.st_ino,
            &validate_publication,
            logical_source,
            candidate_publication_lock,
        )?;
        ensure_authorities()?;
        publish_vps_source_consume_journal(
            &parent_fd,
            &journal_name,
            &journal_temporary_name,
            &journal,
            &ensure_authorities,
        )?;
        journal
    } else if consuming_exists {
        anyhow::bail!("VPS consuming source root exists without its durable inventory journal");
    } else {
        // A completed retry is idempotent after the exact candidate is still
        // authenticated above.
        rustix::fs::fsync(&parent_fd)?;
        ensure_authorities()?;
        return Ok(());
    };
    validate_vps_source_consume_journal(
        &journal,
        plan,
        plan_sha256,
        release_manifest_sha256,
        candidate.st_dev,
        candidate.st_ino,
    )?;
    ensure_authorities()?;
    ensure!(
        journal.phase == VpsSourceConsumePhaseV1::Prepared,
        "the primary VPS source journal is not in prepared phase"
    );
    if !source_exists && !consuming_exists {
        rustix::fs::fsync(&parent_fd)?;
        ensure_authorities()?;
        let terminal = publish_vps_source_terminal_journal(
            &parent_fd,
            &terminal_journal_name,
            &terminal_journal_temporary_name,
            &journal,
            &ensure_authorities,
        )?;
        ensure_authorities()?;
        remove_exact_vps_source_journal(&parent_fd, &journal_name, &journal, &ensure_authorities)?;
        ensure_authorities()?;
        remove_exact_vps_source_journal(
            &parent_fd,
            &terminal_journal_name,
            &terminal,
            &ensure_authorities,
        )?;
        ensure_authorities()?;
        return Ok(());
    }

    let consuming = if consuming_exists {
        let root = open_vps_source_root(&parent_fd, &parent, consuming_name.clone(), 0o700)?;
        ensure!(
            root.device == journal.source_device && root.inode == journal.source_inode,
            "VPS consuming source root differs from its durable journal"
        );
        root
    } else {
        ensure!(
            source_exists,
            "VPS source root disappeared before consumption"
        );
        let source = open_vps_source_root(&parent_fd, &parent, source_name.clone(), 0o700)?;
        ensure!(
            source.device == journal.source_device && source.inode == journal.source_inode,
            "VPS source root differs from its durable journal"
        );
        ensure_authorities()?;
        renameat_with(
            parent_fd.as_fd(),
            &source_name,
            parent_fd.as_fd(),
            &consuming_name,
            RenameFlags::NOREPLACE,
        )
        .or_else(|rename_error| {
            let observed = statat(
                parent_fd.as_fd(),
                &consuming_name,
                AtFlags::SYMLINK_NOFOLLOW,
            );
            if observed.as_ref().is_ok_and(|metadata| {
                metadata.st_dev == source.device && metadata.st_ino == source.inode
            }) && statat(parent_fd.as_fd(), &source_name, AtFlags::SYMLINK_NOFOLLOW)
                .is_err_and(|error| error == rustix::io::Errno::NOENT)
            {
                Ok(())
            } else {
                Err(rename_error)
            }
        })?;
        rustix::fs::fsync(&parent_fd)?;
        after_rename(&incoming_parent.join(&consuming_name))?;
        PinnedVpsSourceRoot {
            name: consuming_name.clone(),
            ..source
        }
    };

    validate_vps_source_inventory_subset(&consuming, &journal.entries)?;
    ensure_authorities()?;
    let mut seen = 1;
    clear_pinned_vps_directory(
        &consuming.fd,
        Path::new(""),
        rustix::process::geteuid().as_raw(),
        consuming.device,
        0,
        &mut seen,
        &journal.entries,
        &ensure_authorities,
    )?;
    let named = statat(
        parent_fd.as_fd(),
        &consuming.name,
        AtFlags::SYMLINK_NOFOLLOW,
    )?;
    ensure!(
        named.st_dev == consuming.device && named.st_ino == consuming.inode,
        "VPS consuming source basename was substituted before root unlink"
    );
    ensure_authorities()?;
    unlinkat(parent_fd.as_fd(), &consuming.name, AtFlags::REMOVEDIR)?;
    ensure!(
        rustix::fs::fstat(&consuming.fd)?.st_nlink == 0,
        "VPS consuming source inode remains linked after root unlink"
    );
    rustix::fs::fsync(&parent_fd)?;
    ensure_authorities()?;
    after_root_unlink(incoming_parent)?;
    ensure_authorities()?;
    let terminal = publish_vps_source_terminal_journal(
        &parent_fd,
        &terminal_journal_name,
        &terminal_journal_temporary_name,
        &journal,
        &ensure_authorities,
    )?;
    ensure_authorities()?;
    remove_exact_vps_source_journal(&parent_fd, &journal_name, &journal, &ensure_authorities)?;
    ensure_authorities()?;
    remove_exact_vps_source_journal(
        &parent_fd,
        &terminal_journal_name,
        &terminal,
        &ensure_authorities,
    )?;
    ensure_authorities()?;
    journal.entries.clear();
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_named_vps_incoming_parent(
    incoming_parent: &Path,
    expected: &rustix::fs::Stat,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let observed = fs::symlink_metadata(incoming_parent)?;
    ensure!(
        observed.is_dir()
            && !observed.file_type().is_symlink()
            && fs::canonicalize(incoming_parent)? == incoming_parent
            && observed.uid() == rustix::process::geteuid().as_raw()
            && observed.permissions().mode() & 0o777 == 0o750
            && observed.dev() == expected.st_dev
            && observed.ino() == expected.st_ino,
        "canonical VPS incoming parent changed during source consumption"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_vps_source_root(
    parent_fd: &std::os::fd::OwnedFd,
    parent: &rustix::fs::Stat,
    name: std::ffi::OsString,
    expected_mode: u32,
) -> Result<PinnedVpsSourceRoot> {
    use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let fd = openat2(
        parent_fd.as_fd(),
        &name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let metadata = rustix::fs::fstat(&fd)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_dir()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == parent.st_dev
            && metadata.st_mode & 0o777 == expected_mode,
        "VPS source root has unsafe identity, device, owner, or mode"
    );
    Ok(PinnedVpsSourceRoot {
        name,
        fd,
        device: metadata.st_dev,
        inode: metadata.st_ino,
    })
}

#[cfg(target_os = "linux")]
fn open_named_vps_source_root(
    source: &PinnedVpsSourceRoot,
    logical_source: &Path,
) -> Result<std::os::fd::OwnedFd> {
    use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};

    let rebound = openat2(
        rustix::fs::CWD,
        logical_source,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let metadata = rustix::fs::fstat(&rebound)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_dir()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == source.device
            && metadata.st_ino == source.inode
            && metadata.st_mode & 0o777 == 0o700,
        "named VPS source root differs from its pinned descriptor"
    );
    Ok(rebound)
}

#[cfg(target_os = "linux")]
fn open_pinned_vps_source_publication(
    source: &PinnedVpsSourceRoot,
) -> Result<std::os::fd::OwnedFd> {
    use rustix::fs::{FileType, Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let publication = openat2(
        source.fd.as_fd(),
        Path::new("publication-v3"),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let metadata = rustix::fs::fstat(&publication)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode).is_dir()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == source.device
            && metadata.st_mode & 0o777 == 0o700,
        "VPS source PublicationV3 root has unsafe identity, device, owner, or mode"
    );
    Ok(publication)
}

#[cfg(target_os = "linux")]
fn same_stable_vps_source_node(left: &rustix::fs::Stat, right: &rustix::fs::Stat) -> bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_uid == right.st_uid
        && left.st_gid == right.st_gid
        && left.st_nlink == right.st_nlink
        && left.st_mode == right.st_mode
        && left.st_size == right.st_size
        && left.st_mtime == right.st_mtime
        && left.st_mtime_nsec == right.st_mtime_nsec
        && left.st_ctime == right.st_ctime
        && left.st_ctime_nsec == right.st_ctime_nsec
}

#[cfg(target_os = "linux")]
fn with_pinned_vps_source_publication<T, F>(
    source: &PinnedVpsSourceRoot,
    logical_source: &Path,
    use_publication: F,
) -> Result<T>
where
    F: FnOnce(&Path, &File) -> Result<T>,
{
    let named_source = open_named_vps_source_root(source, logical_source)?;
    let publication = open_pinned_vps_source_publication(source)?;
    let publication_identity = rustix::fs::fstat(&publication)?;
    let named_publication = {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        openat2(
            named_source.as_fd(),
            Path::new("publication-v3"),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?
    };
    ensure!(
        same_stable_vps_source_node(
            &publication_identity,
            &rustix::fs::fstat(&named_publication)?,
        ),
        "VPS source PublicationV3 differs between its retained and named parents"
    );

    let publication_file = File::from(publication);
    let logical_publication = logical_source.join("publication-v3");
    let result = use_publication(&logical_publication, &publication_file)?;

    open_named_vps_source_root(source, logical_source)?;
    let rebound_publication = open_pinned_vps_source_publication(source)?;
    ensure!(
        same_stable_vps_source_node(
            &publication_identity,
            &rustix::fs::fstat(&rebound_publication)?,
        ),
        "pinned VPS source PublicationV3 changed during validation"
    );
    Ok(result)
}

#[cfg(target_os = "linux")]
fn validate_vps_source_publication_v3(
    source: &PinnedVpsSourceRoot,
    logical_source: &Path,
) -> Result<Digest32> {
    with_pinned_vps_source_publication(
        source,
        logical_source,
        |logical_publication, publication| {
            let validated = crate::publication_v3::validate_pinned_publication_v3(
                logical_publication,
                publication,
            )?;
            validated.ensure_live()?;
            Ok(validated.lock_sha256())
        },
    )
}

#[cfg(target_os = "linux")]
fn build_vps_source_consume_journal<PV>(
    plan: &VpsReleasePlanV2,
    plan_bytes: &[u8],
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    source: &PinnedVpsSourceRoot,
    candidate_device: u64,
    candidate_inode: u64,
    validate_publication: &PV,
    logical_source: &Path,
    candidate_publication_lock: Digest32,
) -> Result<VpsSourceConsumeJournalV1>
where
    PV: Fn(&PinnedVpsSourceRoot, &Path) -> Result<Digest32>,
{
    ensure!(
        plan.publication_v3 == logical_source.join("publication-v3"),
        "VPS publication source must be exact .sources-COMMIT/publication-v3"
    );
    for binary in &plan.binaries {
        ensure!(
            binary.source == logical_source.join("bin").join(binary.role.output_name()),
            "VPS binary source is outside the exact source closure"
        );
    }
    for config in &plan.configs {
        ensure!(
            config.source
                == logical_source
                    .join("config")
                    .join(config.role.output_name()),
            "VPS config source is outside the exact source closure"
        );
    }
    for host in &plan.host_files {
        ensure!(
            host.source == logical_source.join("host").join(host.role.output_path()),
            "VPS host input is outside the exact source closure"
        );
    }

    let entries = inventory_pinned_vps_source_root(source)?;
    let by_path = entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut expected = BTreeSet::from([".".to_owned()]);
    let mut insert_file = |path: &str| -> Result<()> {
        ensure!(valid_relative_manifest_path(path), "unsafe VPS source path");
        expected.insert(path.to_owned());
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
        Ok(())
    };
    insert_file("vps-release-plan-v2.json")?;
    for binary in &plan.binaries {
        insert_file(&format!("bin/{}", binary.role.output_name()))?;
    }
    for config in &plan.configs {
        insert_file(&format!("config/{}", config.role.output_name()))?;
    }
    for host in &plan.host_files {
        insert_file(&format!("host/{}", host.role.output_path()))?;
    }

    ensure!(
        validate_publication(source, logical_source)? == candidate_publication_lock,
        "source PublicationV3 lock differs from the candidate-bound publication lock"
    );
    for entry in &entries {
        if entry.path == "publication-v3" || entry.path.starts_with("publication-v3/") {
            expected.insert(entry.path.clone());
        }
    }
    ensure!(
        entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>()
            == expected,
        "VPS source closure has missing or extra entries"
    );

    validate_source_entry(&by_path, ".", VpsSourceEntryKindV1::Directory, 0o700, None)?;
    for directory in [
        "bin",
        "config",
        "host",
        "host/systemd",
        "host/systemd/user",
        "host/deploy",
        "host/deploy/tests",
    ] {
        validate_source_entry(
            &by_path,
            directory,
            VpsSourceEntryKindV1::Directory,
            0o700,
            None,
        )?;
    }
    validate_source_entry(
        &by_path,
        "vps-release-plan-v2.json",
        VpsSourceEntryKindV1::File,
        0o400,
        Some(&ArtifactRefV1 {
            sha256: plan_sha256,
            byte_length: plan_bytes.len() as u64,
            media_type: "application/json".to_owned(),
        }),
    )?;
    for binary in &plan.binaries {
        validate_source_entry(
            &by_path,
            &format!("bin/{}", binary.role.output_name()),
            VpsSourceEntryKindV1::File,
            0o550,
            Some(&binary.artifact),
        )?;
    }
    for config in &plan.configs {
        validate_source_entry(
            &by_path,
            &format!("config/{}", config.role.output_name()),
            VpsSourceEntryKindV1::File,
            0o440,
            Some(&config.artifact),
        )?;
    }
    for host in &plan.host_files {
        validate_source_entry(
            &by_path,
            &format!("host/{}", host.role.output_path()),
            VpsSourceEntryKindV1::File,
            canonical_file_mode(host.role.output_path()),
            Some(&host.artifact),
        )?;
    }

    Ok(VpsSourceConsumeJournalV1 {
        schema_version: 1,
        phase: VpsSourceConsumePhaseV1::Prepared,
        source_commit: plan.source_commit.clone(),
        plan_sha256,
        release_manifest_sha256,
        source_device: source.device,
        source_inode: source.inode,
        candidate_device,
        candidate_inode,
        entries,
    })
}

fn validate_source_entry(
    entries: &BTreeMap<&str, &VpsSourceConsumeEntryV1>,
    path: &str,
    kind: VpsSourceEntryKindV1,
    unix_mode: u32,
    artifact: Option<&ArtifactRefV1>,
) -> Result<()> {
    let entry = entries
        .get(path)
        .with_context(|| format!("VPS source closure is missing {path}"))?;
    ensure!(
        entry.kind == kind && entry.unix_mode == unix_mode,
        "VPS source entry {path} has the wrong kind or mode"
    );
    match artifact {
        Some(artifact) => ensure!(
            entry.sha256 == Some(artifact.sha256)
                && entry.byte_length == Some(artifact.byte_length),
            "VPS source entry {path} differs from its plan artifact"
        ),
        None => ensure!(
            entry.sha256.is_none() && entry.byte_length.is_none(),
            "VPS source directory {path} carries file identity"
        ),
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn inventory_pinned_vps_source_root(
    source: &PinnedVpsSourceRoot,
) -> Result<Vec<VpsSourceConsumeEntryV1>> {
    let root = rustix::fs::fstat(&source.fd)?;
    let mut entries = vec![VpsSourceConsumeEntryV1 {
        path: ".".to_owned(),
        kind: VpsSourceEntryKindV1::Directory,
        unix_mode: root.st_mode & 0o777,
        sha256: None,
        byte_length: None,
    }];
    let mut seen = 1;
    inventory_pinned_vps_source_directory(
        &source.fd,
        Path::new(""),
        source.device,
        0,
        &mut seen,
        &mut entries,
    )?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

#[cfg(target_os = "linux")]
fn inventory_pinned_vps_source_directory(
    directory_fd: &std::os::fd::OwnedFd,
    relative_root: &Path,
    expected_device: u64,
    depth: usize,
    seen: &mut usize,
    entries: &mut Vec<VpsSourceConsumeEntryV1>,
) -> Result<()> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, RawDir, ResolveFlags, openat2, statat};
    use std::ffi::OsString;
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStringExt as _;

    ensure!(
        depth <= MAX_FAILED_VPS_STAGING_DEPTH,
        "VPS source closure exceeds traversal depth bound"
    );
    let scan_fd = openat2(
        directory_fd.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut buffer = Vec::with_capacity(65_536);
    let mut names = Vec::new();
    let mut directory = RawDir::new(&scan_fd, buffer.spare_capacity_mut());
    while let Some(entry) = directory.next() {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    names.sort();
    for name in names {
        *seen = seen.checked_add(1).context("VPS source entry overflow")?;
        ensure!(
            *seen <= MAX_FAILED_VPS_STAGING_ENTRIES,
            "VPS source closure exceeds entry bound"
        );
        let metadata = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
        ensure!(
            metadata.st_uid == rustix::process::geteuid().as_raw()
                && metadata.st_dev == expected_device,
            "VPS source closure contains mixed ownership or devices"
        );
        let relative = relative_root.join(&name);
        let path = path_to_manifest(&relative)?;
        let file_type = FileType::from_raw_mode(metadata.st_mode);
        if file_type.is_dir() {
            let child_fd = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&child_fd)?;
            ensure!(
                pinned.st_dev == metadata.st_dev
                    && pinned.st_ino == metadata.st_ino
                    && pinned.st_uid == metadata.st_uid
                    && pinned.st_mode == metadata.st_mode,
                "VPS source directory changed while it was pinned"
            );
            entries.push(VpsSourceConsumeEntryV1 {
                path,
                kind: VpsSourceEntryKindV1::Directory,
                unix_mode: metadata.st_mode & 0o777,
                sha256: None,
                byte_length: None,
            });
            inventory_pinned_vps_source_directory(
                &child_fd,
                &relative,
                expected_device,
                depth + 1,
                seen,
                entries,
            )?;
        } else if file_type.is_file() {
            ensure!(
                metadata.st_nlink == 1,
                "VPS source contains a hard-linked file"
            );
            let file_fd = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&file_fd)?;
            ensure!(
                pinned.st_dev == metadata.st_dev
                    && pinned.st_ino == metadata.st_ino
                    && pinned.st_uid == metadata.st_uid
                    && pinned.st_mode == metadata.st_mode
                    && pinned.st_nlink == metadata.st_nlink
                    && pinned.st_size == metadata.st_size,
                "VPS source file changed while it was pinned"
            );
            let mut pinned_file = File::from(file_fd);
            let sha256 = Digest32::digest_reader(&mut pinned_file)?;
            entries.push(VpsSourceConsumeEntryV1 {
                path,
                kind: VpsSourceEntryKindV1::File,
                unix_mode: metadata.st_mode & 0o777,
                sha256: Some(sha256),
                byte_length: Some(metadata.st_size as u64),
            });
        } else {
            anyhow::bail!("VPS source closure contains a link or special node");
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn clear_pinned_vps_directory<F>(
    directory_fd: &std::os::fd::OwnedFd,
    relative_root: &Path,
    expected_uid: u32,
    expected_device: u64,
    depth: usize,
    seen: &mut usize,
    expected_entries: &[VpsSourceConsumeEntryV1],
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, ResolveFlags, fchmod, openat2, statat, unlinkat,
    };
    use std::ffi::OsString;
    use std::io::{Seek as _, SeekFrom};
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStringExt as _;

    ensure!(
        depth <= MAX_FAILED_VPS_STAGING_DEPTH,
        "VPS source cleanup exceeds traversal depth bound"
    );
    let directory = rustix::fs::fstat(directory_fd)?;
    ensure!(
        FileType::from_raw_mode(directory.st_mode).is_dir()
            && directory.st_uid == expected_uid
            && directory.st_dev == expected_device,
        "pinned VPS source cleanup directory has unsafe identity"
    );

    ensure_authority()?;
    fchmod(directory_fd, Mode::from_raw_mode(0o700))?;
    ensure_authority()?;
    let writable = rustix::fs::fstat(directory_fd)?;
    ensure!(
        writable.st_dev == directory.st_dev
            && writable.st_ino == directory.st_ino
            && writable.st_uid == expected_uid
            && writable.st_mode & 0o777 == 0o700,
        "pinned VPS source cleanup directory changed during permission transition"
    );

    let scan_fd = openat2(
        directory_fd.as_fd(),
        ".",
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut buffer = Vec::with_capacity(65_536);
    let mut names = Vec::new();
    let mut entries = RawDir::new(&scan_fd, buffer.spare_capacity_mut());
    while let Some(entry) = entries.next() {
        let entry = entry?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    names.sort();

    for name in names {
        *seen = seen
            .checked_add(1)
            .context("VPS source cleanup entry count overflow")?;
        ensure!(
            *seen <= MAX_FAILED_VPS_STAGING_ENTRIES,
            "VPS source cleanup exceeds entry bound"
        );
        let named = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
        ensure!(
            named.st_uid == expected_uid && named.st_dev == expected_device,
            "VPS source cleanup entry has mixed ownership or devices"
        );
        let relative = relative_root.join(&name);
        let expected_path = path_to_manifest(&relative)?;
        let expected_entry = expected_entries
            .iter()
            .find(|entry| entry.path == expected_path)
            .with_context(|| {
                format!("VPS source cleanup found unjournaled entry {expected_path}")
            })?;
        let kind = FileType::from_raw_mode(named.st_mode);
        if kind.is_dir() {
            ensure!(
                expected_entry.kind == VpsSourceEntryKindV1::Directory
                    && expected_entry.sha256.is_none()
                    && expected_entry.byte_length.is_none(),
                "VPS source cleanup directory differs from its durable journal"
            );
            let child = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&child)?;
            ensure!(
                pinned.st_dev == named.st_dev
                    && pinned.st_ino == named.st_ino
                    && pinned.st_uid == named.st_uid
                    && pinned.st_mode == named.st_mode,
                "VPS source cleanup directory changed while it was pinned"
            );
            clear_pinned_vps_directory(
                &child,
                &relative,
                expected_uid,
                expected_device,
                depth + 1,
                seen,
                expected_entries,
                ensure_authority,
            )?;
            let rebound = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            ensure!(
                rebound.st_dev == pinned.st_dev
                    && rebound.st_ino == pinned.st_ino
                    && rebound.st_uid == pinned.st_uid
                    && FileType::from_raw_mode(rebound.st_mode).is_dir()
                    && rebound.st_mode & 0o777 == 0o700,
                "VPS source cleanup directory basename was substituted"
            );
            ensure_authority()?;
            unlinkat(directory_fd.as_fd(), &name, AtFlags::REMOVEDIR)?;
            ensure!(
                rustix::fs::fstat(&child)?.st_nlink == 0,
                "VPS source cleanup directory remains linked after unlink"
            );
        } else if kind.is_file() {
            ensure!(named.st_nlink == 1, "VPS source cleanup found a hard link");
            let child = openat2(
                directory_fd.as_fd(),
                &name,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let pinned = rustix::fs::fstat(&child)?;
            ensure!(
                pinned.st_dev == named.st_dev
                    && pinned.st_ino == named.st_ino
                    && pinned.st_uid == named.st_uid
                    && pinned.st_mode == named.st_mode
                    && pinned.st_nlink == 1
                    && pinned.st_size == named.st_size,
                "VPS source cleanup file changed while it was pinned"
            );
            ensure!(
                expected_entry.kind == VpsSourceEntryKindV1::File
                    && expected_entry.byte_length == Some(pinned.st_size as u64),
                "VPS source cleanup file length differs from its durable journal"
            );
            let mut child = File::from(child);
            let first_digest = Digest32::digest_reader(&mut child)?;
            ensure!(
                expected_entry.sha256 == Some(first_digest),
                "VPS source cleanup file bytes differ from its durable journal"
            );
            let after_first_hash = rustix::fs::fstat(&child)?;
            ensure!(
                after_first_hash.st_dev == pinned.st_dev
                    && after_first_hash.st_ino == pinned.st_ino
                    && after_first_hash.st_uid == pinned.st_uid
                    && after_first_hash.st_mode == pinned.st_mode
                    && after_first_hash.st_nlink == pinned.st_nlink
                    && after_first_hash.st_size == pinned.st_size
                    && after_first_hash.st_mtime == pinned.st_mtime
                    && after_first_hash.st_mtime_nsec == pinned.st_mtime_nsec
                    && after_first_hash.st_ctime == pinned.st_ctime
                    && after_first_hash.st_ctime_nsec == pinned.st_ctime_nsec,
                "VPS source cleanup file changed while it was hashed"
            );
            let rebound = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            ensure!(
                rebound.st_dev == pinned.st_dev
                    && rebound.st_ino == pinned.st_ino
                    && rebound.st_uid == pinned.st_uid
                    && rebound.st_mode == pinned.st_mode
                    && rebound.st_nlink == 1
                    && rebound.st_size == pinned.st_size
                    && rebound.st_mtime == pinned.st_mtime
                    && rebound.st_mtime_nsec == pinned.st_mtime_nsec
                    && rebound.st_ctime == pinned.st_ctime
                    && rebound.st_ctime_nsec == pinned.st_ctime_nsec,
                "VPS source cleanup file basename was substituted"
            );
            ensure_authority()?;
            child.seek(SeekFrom::Start(0))?;
            ensure!(
                Digest32::digest_reader(&mut child)? == first_digest,
                "VPS source cleanup file changed at its unlink boundary"
            );
            let final_pinned = rustix::fs::fstat(&child)?;
            let final_named = statat(directory_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW)?;
            ensure!(
                final_pinned.st_dev == pinned.st_dev
                    && final_pinned.st_ino == pinned.st_ino
                    && final_pinned.st_uid == pinned.st_uid
                    && final_pinned.st_mode == pinned.st_mode
                    && final_pinned.st_nlink == 1
                    && final_pinned.st_size == pinned.st_size
                    && final_pinned.st_mtime == pinned.st_mtime
                    && final_pinned.st_mtime_nsec == pinned.st_mtime_nsec
                    && final_pinned.st_ctime == pinned.st_ctime
                    && final_pinned.st_ctime_nsec == pinned.st_ctime_nsec
                    && final_named.st_dev == final_pinned.st_dev
                    && final_named.st_ino == final_pinned.st_ino
                    && final_named.st_mode == final_pinned.st_mode
                    && final_named.st_nlink == final_pinned.st_nlink
                    && final_named.st_size == final_pinned.st_size
                    && final_named.st_mtime == final_pinned.st_mtime
                    && final_named.st_mtime_nsec == final_pinned.st_mtime_nsec
                    && final_named.st_ctime == final_pinned.st_ctime
                    && final_named.st_ctime_nsec == final_pinned.st_ctime_nsec,
                "VPS source cleanup file identity changed at its unlink boundary"
            );
            unlinkat(directory_fd.as_fd(), &name, AtFlags::empty())?;
            ensure!(
                rustix::fs::fstat(&child)?.st_nlink == 0,
                "VPS source cleanup file remains linked after unlink"
            );
        } else {
            anyhow::bail!("VPS source cleanup found a symlink or special node");
        }
        rustix::fs::fsync(directory_fd)?;
        ensure_authority()?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_vps_source_consume_journal(
    journal: &VpsSourceConsumeJournalV1,
    plan: &VpsReleasePlanV2,
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    candidate_device: u64,
    candidate_inode: u64,
) -> Result<()> {
    ensure!(
        journal.schema_version == 1
            && journal.source_commit == plan.source_commit
            && journal.plan_sha256 == plan_sha256
            && journal.release_manifest_sha256 == release_manifest_sha256
            && journal.candidate_device == candidate_device
            && journal.candidate_inode == candidate_inode
            && journal.source_device != 0
            && journal.source_inode != 0
            && !journal.entries.is_empty()
            && journal
                .entries
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path),
        "VPS source consume journal identity or ordering is invalid"
    );
    for entry in &journal.entries {
        ensure!(
            entry.path == "." || valid_relative_manifest_path(&entry.path),
            "VPS source consume journal has an unsafe path"
        );
        match entry.kind {
            VpsSourceEntryKindV1::Directory => ensure!(
                entry.sha256.is_none() && entry.byte_length.is_none(),
                "VPS source consume journal directory has file identity"
            ),
            VpsSourceEntryKindV1::File => ensure!(
                entry.sha256.is_some() && entry.byte_length.is_some(),
                "VPS source consume journal file lacks identity"
            ),
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn load_vps_source_consume_journal(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
) -> Result<VpsSourceConsumeJournalV1> {
    let bytes = read_pinned_vps_source_file(parent_fd, name, MAX_DOCUMENT_BYTES)?;
    let journal: VpsSourceConsumeJournalV1 = strict_json_from_slice(&bytes)?;
    ensure!(
        canonical_json_bytes(&journal)? == bytes,
        "VPS source consume journal is not byte-for-byte canonical JSON"
    );
    Ok(journal)
}

#[cfg(target_os = "linux")]
struct PinnedVpsSourceDocument {
    file: File,
    metadata: rustix::fs::Stat,
    bytes: Vec<u8>,
}

#[cfg(target_os = "linux")]
fn read_pinned_vps_source_file(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    limit: u64,
) -> Result<Vec<u8>> {
    Ok(pin_vps_source_document(parent_fd, name, limit, &[0o400])?.bytes)
}

#[cfg(target_os = "linux")]
fn pin_vps_source_document(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    limit: u64,
    allowed_modes: &[u32],
) -> Result<PinnedVpsSourceDocument> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::Read as _;
    use std::os::fd::AsFd as _;

    let fd = openat2(
        parent_fd.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    let parent = rustix::fs::fstat(parent_fd)?;
    let metadata = rustix::fs::fstat(&fd)?;
    ensure!(
        rustix::fs::FileType::from_raw_mode(metadata.st_mode).is_file()
            && metadata.st_uid == rustix::process::geteuid().as_raw()
            && metadata.st_dev == parent.st_dev
            && metadata.st_nlink == 1
            && allowed_modes.contains(&(metadata.st_mode & 0o777))
            && metadata.st_size >= 0
            && metadata.st_size as u64 <= limit,
        "VPS source consume journal metadata is unsafe"
    );
    let mut file = File::from(fd);
    let mut bytes = Vec::with_capacity(metadata.st_size as usize);
    file.read_to_end(&mut bytes)?;
    let observed = rustix::fs::fstat(&file)?;
    ensure!(
        bytes.len() as u64 == metadata.st_size as u64
            && observed.st_dev == metadata.st_dev
            && observed.st_ino == metadata.st_ino
            && observed.st_uid == metadata.st_uid
            && observed.st_nlink == metadata.st_nlink
            && observed.st_mode == metadata.st_mode
            && observed.st_size == metadata.st_size,
        "VPS source consume journal changed while read"
    );
    Ok(PinnedVpsSourceDocument {
        file,
        metadata,
        bytes,
    })
}

#[cfg(target_os = "linux")]
fn same_vps_source_document(
    left: &PinnedVpsSourceDocument,
    right: &PinnedVpsSourceDocument,
) -> bool {
    left.metadata.st_dev == right.metadata.st_dev
        && left.metadata.st_ino == right.metadata.st_ino
        && left.metadata.st_uid == right.metadata.st_uid
        && left.metadata.st_nlink == right.metadata.st_nlink
        && left.metadata.st_mode == right.metadata.st_mode
        && left.metadata.st_size == right.metadata.st_size
        && left.bytes == right.bytes
}

#[cfg(target_os = "linux")]
fn remove_pinned_vps_source_document<F>(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    expected_bytes: Option<&[u8]>,
    allowed_modes: &[u32],
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{AtFlags, RenameFlags, renameat_with, unlinkat};
    use std::os::fd::AsFd as _;

    let removing_name = std::ffi::OsString::from(format!("{}.removing", name.to_string_lossy()));
    let pinned = if pinned_entry_exists(parent_fd, &removing_name)? {
        ensure!(
            !pinned_entry_exists(parent_fd, name)?,
            "VPS source document and its removing name both exist"
        );
        pin_vps_source_document(parent_fd, &removing_name, MAX_DOCUMENT_BYTES, allowed_modes)?
    } else {
        let pinned = pin_vps_source_document(parent_fd, name, MAX_DOCUMENT_BYTES, allowed_modes)?;
        if let Some(expected) = expected_bytes {
            ensure!(
                pinned.bytes == expected,
                "VPS source document changed before guarded removal"
            );
        }
        ensure_authority()?;
        renameat_with(
            parent_fd.as_fd(),
            name,
            parent_fd.as_fd(),
            &removing_name,
            RenameFlags::NOREPLACE,
        )
        .or_else(|rename_error| -> Result<()> {
            let observed = pin_vps_source_document(
                parent_fd,
                &removing_name,
                MAX_DOCUMENT_BYTES,
                allowed_modes,
            );
            if observed
                .as_ref()
                .is_ok_and(|observed| same_vps_source_document(&pinned, observed))
                && !pinned_entry_exists(parent_fd, name)?
            {
                Ok(())
            } else {
                Err(rename_error.into())
            }
        })?;
        rustix::fs::fsync(parent_fd)?;
        let observed =
            pin_vps_source_document(parent_fd, &removing_name, MAX_DOCUMENT_BYTES, allowed_modes)?;
        ensure!(
            same_vps_source_document(&pinned, &observed),
            "VPS source document changed during guarded removal"
        );
        pinned
    };
    if let Some(expected) = expected_bytes {
        ensure!(
            pinned.bytes == expected,
            "VPS source document removing name has unexpected bytes"
        );
    }
    let observed =
        pin_vps_source_document(parent_fd, &removing_name, MAX_DOCUMENT_BYTES, allowed_modes)?;
    ensure!(
        same_vps_source_document(&pinned, &observed),
        "VPS source document removing name was substituted"
    );
    ensure_authority()?;
    unlinkat(parent_fd.as_fd(), &removing_name, AtFlags::empty())?;
    ensure!(
        rustix::fs::fstat(&pinned.file)?.st_nlink == 0,
        "VPS source document inode remains linked after guarded removal"
    );
    rustix::fs::fsync(parent_fd)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn restore_vps_source_document_removing<F>(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{RenameFlags, renameat_with};
    use std::os::fd::AsFd as _;

    let removing_name = std::ffi::OsString::from(format!("{}.removing", name.to_string_lossy()));
    if !pinned_entry_exists(parent_fd, &removing_name)? {
        return Ok(());
    }
    ensure!(
        !pinned_entry_exists(parent_fd, name)?,
        "VPS source document and its removing name both exist"
    );
    let pinned = pin_vps_source_document(
        parent_fd,
        &removing_name,
        MAX_DOCUMENT_BYTES,
        &[0o400, 0o600],
    )?;
    ensure_authority()?;
    renameat_with(
        parent_fd.as_fd(),
        &removing_name,
        parent_fd.as_fd(),
        name,
        RenameFlags::NOREPLACE,
    )
    .or_else(|rename_error| -> Result<()> {
        let observed =
            pin_vps_source_document(parent_fd, name, MAX_DOCUMENT_BYTES, &[0o400, 0o600]);
        if observed
            .as_ref()
            .is_ok_and(|observed| same_vps_source_document(&pinned, observed))
            && !pinned_entry_exists(parent_fd, &removing_name)?
        {
            Ok(())
        } else {
            Err(rename_error.into())
        }
    })?;
    rustix::fs::fsync(parent_fd)?;
    let observed = pin_vps_source_document(parent_fd, name, MAX_DOCUMENT_BYTES, &[0o400, 0o600])?;
    ensure!(
        same_vps_source_document(&pinned, &observed),
        "VPS source document changed while recovering guarded removal"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn pinned_entry_exists(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
) -> Result<bool> {
    use rustix::fs::{AtFlags, statat};
    use std::os::fd::AsFd as _;

    match statat(parent_fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
fn reconcile_vps_source_journal_temporary<F>(
    parent_fd: &std::os::fd::OwnedFd,
    journal_name: &std::ffi::OsString,
    temporary_name: &std::ffi::OsString,
    source_name: &std::ffi::OsString,
    consuming_name: &std::ffi::OsString,
    expected_commit: &str,
    expected_phase: VpsSourceConsumePhaseV1,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{AtFlags, RenameFlags, renameat_with, statat};
    use std::os::fd::AsFd as _;

    let temporary = match statat(parent_fd.as_fd(), temporary_name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        rustix::fs::FileType::from_raw_mode(temporary.st_mode).is_file()
            && temporary.st_uid == rustix::process::geteuid().as_raw()
            && temporary.st_nlink == 1,
        "VPS source consume journal temporary is unsafe"
    );
    let temporary_journal = load_vps_source_consume_journal(parent_fd, temporary_name);
    if let Ok(temporary_journal) = temporary_journal {
        ensure!(
            temporary_journal.source_commit == expected_commit
                && temporary_journal.phase == expected_phase,
            "VPS source consume journal temporary has the wrong commit or phase"
        );
        if pinned_entry_exists(parent_fd, journal_name)? {
            let current = load_vps_source_consume_journal(parent_fd, journal_name)?;
            ensure!(
                current == temporary_journal,
                "VPS source consume journal and temporary disagree"
            );
            remove_exact_vps_source_journal(
                parent_fd,
                temporary_name,
                &temporary_journal,
                ensure_authority,
            )?;
        } else {
            ensure_authority()?;
            renameat_with(
                parent_fd.as_fd(),
                temporary_name,
                parent_fd.as_fd(),
                journal_name,
                RenameFlags::NOREPLACE,
            )
            .or_else(|rename_error| {
                let installed = load_vps_source_consume_journal(parent_fd, journal_name);
                let temporary_absent = !pinned_entry_exists(parent_fd, temporary_name)?;
                if installed
                    .as_ref()
                    .is_ok_and(|journal| *journal == temporary_journal)
                    && temporary_absent
                {
                    Ok::<(), anyhow::Error>(())
                } else {
                    Err(rename_error.into())
                }
            })?;
        }
        rustix::fs::fsync(parent_fd)?;
        return Ok(());
    }
    let source_exists = pinned_entry_exists(parent_fd, source_name)?;
    let consuming_exists = pinned_entry_exists(parent_fd, consuming_name)?;
    let journal_exists = pinned_entry_exists(parent_fd, journal_name)?;
    let safe_invalid_prepared = expected_phase == VpsSourceConsumePhaseV1::Prepared
        && source_exists
        && !consuming_exists
        && !journal_exists;
    let safe_invalid_terminal = expected_phase == VpsSourceConsumePhaseV1::RootUnlinked
        && !source_exists
        && !consuming_exists
        && !journal_exists;
    ensure!(
        safe_invalid_prepared || safe_invalid_terminal,
        "invalid VPS source journal temporary coexists with mutated consumption state"
    );
    remove_pinned_vps_source_document(
        parent_fd,
        temporary_name,
        None,
        &[0o600, 0o400],
        ensure_authority,
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn publish_vps_source_terminal_journal<F>(
    parent_fd: &std::os::fd::OwnedFd,
    terminal_name: &std::ffi::OsString,
    temporary_name: &std::ffi::OsString,
    prepared: &VpsSourceConsumeJournalV1,
    ensure_authority: &F,
) -> Result<VpsSourceConsumeJournalV1>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, fchmod, openat2, renameat_with};
    use std::io::Write as _;
    use std::os::fd::AsFd as _;

    ensure!(
        prepared.phase == VpsSourceConsumePhaseV1::Prepared,
        "VPS source journal is not in prepared phase"
    );
    let mut journal = prepared.clone();
    journal.phase = VpsSourceConsumePhaseV1::RootUnlinked;
    let bytes = canonical_json_bytes(&journal)?;
    ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_BYTES,
        "terminal VPS source consume journal exceeds the reader bound"
    );
    let temporary_fd = openat2(
        parent_fd.as_fd(),
        temporary_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut temporary = File::from(temporary_fd);
    temporary.write_all(&bytes)?;
    temporary.flush()?;
    fchmod(&temporary, Mode::from_raw_mode(0o400))?;
    temporary.sync_all()?;
    ensure_authority()?;
    renameat_with(
        parent_fd.as_fd(),
        temporary_name,
        parent_fd.as_fd(),
        terminal_name,
        RenameFlags::NOREPLACE,
    )
    .or_else(|rename_error| {
        let installed = load_vps_source_consume_journal(parent_fd, terminal_name);
        let temporary_absent = !pinned_entry_exists(parent_fd, temporary_name)?;
        if installed
            .as_ref()
            .is_ok_and(|observed| *observed == journal)
            && temporary_absent
        {
            Ok::<(), anyhow::Error>(())
        } else {
            Err(rename_error.into())
        }
    })?;
    rustix::fs::fsync(parent_fd)?;
    ensure!(
        load_vps_source_consume_journal(parent_fd, terminal_name)? == journal,
        "terminal VPS source journal changed after publication"
    );
    Ok(journal)
}

#[cfg(target_os = "linux")]
fn publish_vps_source_consume_journal<F>(
    parent_fd: &std::os::fd::OwnedFd,
    journal_name: &std::ffi::OsString,
    temporary_name: &std::ffi::OsString,
    journal: &VpsSourceConsumeJournalV1,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, fchmod, openat2, renameat_with};
    use std::io::Write as _;
    use std::os::fd::AsFd as _;

    let bytes = canonical_json_bytes(journal)?;
    ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_BYTES,
        "VPS source consume journal exceeds the reader bound"
    );
    let temporary_fd = openat2(
        parent_fd.as_fd(),
        temporary_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut temporary = File::from(temporary_fd);
    temporary.write_all(&bytes)?;
    temporary.flush()?;
    fchmod(&temporary, Mode::from_raw_mode(0o400))?;
    temporary.sync_all()?;
    ensure_authority()?;
    renameat_with(
        parent_fd.as_fd(),
        temporary_name,
        parent_fd.as_fd(),
        journal_name,
        RenameFlags::NOREPLACE,
    )?;
    rustix::fs::fsync(parent_fd)?;
    ensure!(
        load_vps_source_consume_journal(parent_fd, journal_name)? == *journal,
        "published VPS source consume journal changed"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_vps_source_inventory_subset(
    source: &PinnedVpsSourceRoot,
    expected: &[VpsSourceConsumeEntryV1],
) -> Result<()> {
    let expected = expected
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let actual = inventory_pinned_vps_source_root(source)?;
    for entry in actual {
        let original = expected
            .get(entry.path.as_str())
            .with_context(|| format!("VPS consuming source has extra entry {}", entry.path))?;
        let cleanup_mode = entry.kind == VpsSourceEntryKindV1::Directory
            && entry.unix_mode == 0o700
            && original.kind == VpsSourceEntryKindV1::Directory;
        ensure!(
            entry.kind == original.kind
                && (entry.unix_mode == original.unix_mode || cleanup_mode)
                && entry.sha256 == original.sha256
                && entry.byte_length == original.byte_length,
            "VPS consuming source entry {} differs from its durable inventory",
            entry.path
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_exact_vps_source_journal<F>(
    parent_fd: &std::os::fd::OwnedFd,
    name: &std::ffi::OsString,
    expected: &VpsSourceConsumeJournalV1,
    ensure_authority: &F,
) -> Result<()>
where
    F: Fn() -> Result<()>,
{
    let bytes = canonical_json_bytes(expected)?;
    ensure!(
        bytes.len() as u64 <= MAX_DOCUMENT_BYTES
            && load_vps_source_consume_journal(parent_fd, name)? == *expected,
        "VPS source consume journal changed before removal"
    );
    remove_pinned_vps_source_document(parent_fd, name, Some(&bytes), &[0o400], ensure_authority)
}

/// Validate one release solely from its canonical, self-contained evidence.
pub fn validate_vps_release_v2(root: &Path) -> Result<Digest32> {
    validate_vps_release_root(root, true)
}

/// Validate the exact inode named by `partial` through a pinned directory
/// descriptor and atomically promote it to `output` without replacement.
///
/// This is deliberately Linux-only. Deployment must not validate one pathname
/// and later rename whatever a same-UID process substituted at that pathname.
pub fn promote_vps_release_v2(
    partial: &Path,
    output: &Path,
    expected_sha256sums_sha256: &str,
) -> Result<Digest32> {
    #[cfg(target_os = "linux")]
    {
        ensure!(
            normalized_absolute(partial) && normalized_absolute(output),
            "VPS promotion paths must be normalized absolute paths"
        );
        let release_parent = Path::new(INSTALL_ROOT).join("releases");
        ensure!(
            partial.parent() == Some(release_parent.as_path())
                && output.parent() == Some(release_parent.as_path()),
            "VPS promotion must stay in the exact releases directory"
        );
        let partial_name = partial
            .file_name()
            .context("VPS partial path has no basename")?;
        let output_name = output
            .file_name()
            .context("VPS output path has no basename")?;
        let partial_name = partial_name
            .to_str()
            .context("VPS partial basename is not UTF-8")?;
        let output_name = output_name
            .to_str()
            .context("VPS output basename is not UTF-8")?;
        let source_commit = partial_name
            .strip_suffix(".partial")
            .context("VPS partial basename lacks the exact .partial suffix")?;
        ensure!(
            valid_source_commit(source_commit) && output_name == source_commit,
            "VPS partial and output names do not bind one exact source commit"
        );
        let expected_sha256sums_sha256 = expected_sha256sums_sha256
            .parse::<Digest32>()
            .context("expected SHA256SUMS digest is not canonical lowercase hexadecimal")?;
        promote_pinned_vps_release_with(
            partial,
            output,
            &release_parent,
            source_commit,
            expected_sha256sums_sha256,
            validate_pinned_current_vps_release_root,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (partial, output, expected_sha256sums_sha256);
        anyhow::bail!("pinned VPS release promotion requires Linux openat2 and renameat2")
    }
}

#[cfg(target_os = "linux")]
fn promote_pinned_vps_release_with<F>(
    partial: &Path,
    output: &Path,
    release_parent: &Path,
    source_commit: &str,
    expected_sha256sums_sha256: Digest32,
    validate_pinned: F,
) -> Result<Digest32>
where
    F: FnOnce(&Path) -> Result<Digest32>,
{
    use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with};
    use std::os::fd::{AsFd as _, AsRawFd as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    ensure!(
        fs::canonicalize(release_parent)? == release_parent,
        "VPS releases parent is not canonical"
    );
    let parent_fd = openat2(
        rustix::fs::CWD,
        release_parent,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let parent_metadata = fs::metadata(release_parent)?;
    let pinned_parent = rustix::fs::fstat(&parent_fd)?;
    ensure!(
        parent_metadata.dev() == pinned_parent.st_dev
            && parent_metadata.ino() == pinned_parent.st_ino
            && parent_metadata.uid() == rustix::process::geteuid().as_raw()
            && parent_metadata.permissions().mode() & 0o777 == 0o750,
        "VPS releases parent identity, owner, or mode is unsafe"
    );

    let partial_name = partial
        .file_name()
        .context("VPS partial path has no basename")?;
    let output_name = output
        .file_name()
        .context("VPS output path has no basename")?;
    let partial_fd = openat2(
        parent_fd.as_fd(),
        partial_name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let pinned_partial = rustix::fs::fstat(&partial_fd)?;
    ensure!(
        pinned_partial.st_uid == rustix::process::geteuid().as_raw()
            && pinned_partial.st_dev == pinned_parent.st_dev
            && pinned_partial.st_mode & 0o777 == 0o550,
        "VPS partial root identity, owner, device, or mode is unsafe"
    );
    // The trailing `/.` makes the final component the pinned directory,
    // rather than the procfs magic-link entry itself.
    let pinned_path = PathBuf::from(format!("/proc/self/fd/{}/.", partial_fd.as_raw_fd()));
    let manifest_sha256 = validate_pinned(&pinned_path)?;
    ensure!(
        fs::read(pinned_path.join(SOURCE_COMMIT_FILE))? == format!("{source_commit}\n").as_bytes(),
        "pinned VPS partial source commit differs from its basename"
    );
    ensure!(
        Digest32::digest_bytes(&fs::read(pinned_path.join(SHA256SUMS_FILE))?)
            == expected_sha256sums_sha256,
        "pinned VPS partial SHA256SUMS differs from the out-of-band digest"
    );

    let observed_parent = fs::metadata(release_parent)?;
    let observed_partial = fs::symlink_metadata(partial)?;
    ensure!(
        observed_parent.dev() == pinned_parent.st_dev
            && observed_parent.ino() == pinned_parent.st_ino
            && observed_partial.dev() == pinned_partial.st_dev
            && observed_partial.ino() == pinned_partial.st_ino,
        "VPS releases parent or validated partial was substituted"
    );
    renameat_with(
        parent_fd.as_fd(),
        partial_name,
        parent_fd.as_fd(),
        output_name,
        RenameFlags::NOREPLACE,
    )
    .context("atomically promote pinned VPS release without replacement")?;
    Ok(manifest_sha256)
}

fn validate_vps_release_root(root: &Path, enforce_directory_name: bool) -> Result<Digest32> {
    validate_mount_root(root)?;
    validate_vps_release_contents(root, enforce_directory_name)
}

fn validate_pinned_current_vps_release_root(root: &Path) -> Result<Digest32> {
    let metadata = fs::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir(),
        "pinned VPS release root is not a directory"
    );
    let manifest_sha256 = validate_vps_release_contents(root, false)?;
    let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
    manifest.validate_current_candidate()?;
    Ok(manifest_sha256)
}

fn validate_vps_release_contents(root: &Path, enforce_directory_name: bool) -> Result<Digest32> {
    #[cfg(unix)]
    reject_mounts_strictly_below(root)?;
    #[cfg(not(unix))]
    anyhow::bail!("VPS releases require fail-closed Unix mount validation");
    reject_links_and_special_nodes(root, true)?;
    validate_single_owner_tree(root)?;
    validate_single_device_tree(root)?;
    let manifest: VpsReleaseManifestV2 = load_canonical(&root.join(RELEASE_MANIFEST_FILE))?;
    if enforce_directory_name {
        ensure!(
            root.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| valid_release_directory_name(name, &manifest.source_commit)),
            "VPS release directory is neither SOURCE_COMMIT nor its exact .partial candidate"
        );
        if root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| candidate_release_directory_name(name, &manifest.source_commit))
        {
            manifest.validate_current_candidate()?;
        }
    }
    ensure!(
        fs::read(root.join(SOURCE_COMMIT_FILE))?
            == format!("{}\n", manifest.source_commit).as_bytes(),
        "SOURCE_COMMIT differs from release manifest"
    );
    ensure!(
        payload_inventory(root)? == manifest.files,
        "VPS payload inventory differs from its canonical manifest"
    );
    ensure!(
        fs::read(root.join(MODE_INVENTORY_FILE))? == expected_mode_inventory(root)?,
        "MODE_INVENTORY is missing, reordered, or substituted"
    );
    ensure!(
        fs::read(root.join(SHA256SUMS_FILE))? == expected_sha256sums(root)?,
        "SHA256SUMS is missing, reordered, or substituted"
    );
    validate_bundle_shape(root, &manifest)?;
    validate_embedded_publication(root, &manifest)?;
    Ok(manifest.canonical_digest()?)
}

fn validate_plan_shape(plan: &VpsReleasePlanV2) -> Result<()> {
    ensure!(
        valid_source_commit(&plan.source_commit),
        "invalid source commit"
    );
    ensure_strict_roles(plan.binaries.iter().map(|entry| entry.role), "binary roles")?;
    ensure_strict_roles(plan.configs.iter().map(|entry| entry.role), "config roles")?;
    ensure_strict_roles(
        plan.host_files.iter().map(|entry| entry.role),
        "host-file roles",
    )?;
    let required_binaries = vec![
        VpsBinaryRoleV2::Admin,
        VpsBinaryRoleV2::ManifestTool,
        VpsBinaryRoleV2::Server,
        VpsBinaryRoleV2::Worker,
        VpsBinaryRoleV2::ReplayVerifier,
    ];
    ensure!(
        plan.binaries
            .iter()
            .map(|entry| entry.role)
            .collect::<Vec<_>>()
            == required_binaries,
        "release binary inventory is incomplete or noncanonical"
    );
    ensure!(
        plan.binaries
            .iter()
            .map(|binary| binary.artifact.sha256)
            .collect::<BTreeSet<_>>()
            .len()
            == plan.binaries.len(),
        "one binary was substituted for another release role"
    );
    let required_configs = vec![
        VpsConfigRoleV2::Server,
        VpsConfigRoleV2::Worker,
        VpsConfigRoleV2::ApiEnvironment,
        VpsConfigRoleV2::WorkerEnvironment,
    ];
    ensure!(
        plan.configs
            .iter()
            .map(|entry| entry.role)
            .collect::<Vec<_>>()
            == required_configs,
        "release config inventory is incomplete or noncanonical"
    );
    let required_host_files = vec![
        VpsHostFileRoleV2::UserTarget,
        VpsHostFileRoleV2::ApiService,
        VpsHostFileRoleV2::WorkerService,
        VpsHostFileRoleV2::BackupService,
        VpsHostFileRoleV2::BackupTimer,
        VpsHostFileRoleV2::DeployReleaseScript,
        VpsHostFileRoleV2::RollbackReleaseScript,
        VpsHostFileRoleV2::ValidateReleaseScript,
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
        VpsHostFileRoleV2::RealRuntimeFenceHarness,
        VpsHostFileRoleV2::RealRuntimeFenceSelftest,
        VpsHostFileRoleV2::RootOnceScript,
        VpsHostFileRoleV2::NginxChallenge,
        VpsHostFileRoleV2::NginxCloudflareOnly,
        VpsHostFileRoleV2::NginxApiLocations,
        VpsHostFileRoleV2::NginxVhost,
        VpsHostFileRoleV2::DeploymentReadme,
        VpsHostFileRoleV2::OperatorRunbook,
        VpsHostFileRoleV2::BackupRunbook,
    ];
    ensure!(
        plan.host_files
            .iter()
            .map(|entry| entry.role)
            .collect::<Vec<_>>()
            == required_host_files,
        "release host-file inventory is incomplete or noncanonical"
    );
    let declarations = PrivateRawRootDeclarationsV2 {
        schema_version: RAW_ROOTS_SCHEMA_VERSION,
        roots: plan.private_raw_roots.clone(),
    };
    declarations.validate()?;
    for artifact in plan
        .binaries
        .iter()
        .map(|entry| &entry.artifact)
        .chain(plan.configs.iter().map(|entry| &entry.artifact))
        .chain(plan.host_files.iter().map(|entry| &entry.artifact))
    {
        artifact.validate()?;
        ensure!(
            !artifact.sha256.is_zero(),
            "release plan contains a zero digest"
        );
    }
    Ok(())
}

fn ensure_strict_roles<T: Ord + Copy>(roles: impl Iterator<Item = T>, label: &str) -> Result<()> {
    let roles = roles.collect::<Vec<_>>();
    ensure!(
        roles.windows(2).all(|pair| pair[0] < pair[1]),
        "{label} repeat or are not in canonical order"
    );
    Ok(())
}

fn validate_assembly_inputs(
    plan: &VpsReleasePlanV2,
    output: &Path,
    verifier_sha256: Digest32,
    job_catalog: &ArtifactRefV1,
    campaign_states: &BTreeSet<Digest32>,
) -> Result<()> {
    let output_parent = fs::canonicalize(
        output
            .parent()
            .context("VPS release output has no parent directory")?,
    )?;
    ensure!(
        output.parent() == Some(output_parent.as_path()),
        "VPS release output parent must be normalized and non-symlink"
    );
    let output_normalized = output_parent.join(
        output
            .file_name()
            .context("VPS release output has no final component")?,
    );
    ensure!(
        normalized_absolute(&plan.publication_v3),
        "PublicationV3 input path must be normalized and absolute"
    );
    let publication = plan.publication_v3.clone();
    let mut roots = vec![publication.clone(), output_normalized.clone()];
    for declaration in &plan.private_raw_roots {
        validate_immutable_raw_root(&declaration.root)?;
        roots.push(fs::canonicalize(&declaration.root)?);
    }
    for left in 0..roots.len() {
        for right in left + 1..roots.len() {
            ensure!(
                !paths_overlap(&roots[left], &roots[right]),
                "publication, output, and private raw roots must be distinct and non-overlapping"
            );
        }
    }
    for binary in &plan.binaries {
        validate_pinned_file(&binary.source, &binary.artifact)?;
        validate_linux_executable_source(&binary.source)?;
    }
    for config in &plan.configs {
        validate_pinned_file(&config.source, &config.artifact)?;
        validate_final_config(
            config.role,
            &config.source,
            verifier_sha256,
            job_catalog,
            &plan.source_commit,
            campaign_states,
        )?;
    }
    let worker_config = plan
        .configs
        .iter()
        .find(|config| config.role == VpsConfigRoleV2::Worker)
        .context("release plan omits worker config")?;
    validate_worker_raw_authority(
        &worker_config.source,
        &plan.source_commit,
        &plan
            .publication_v3
            .join("private/official-content-authority/private/source-tree-manifests-v2"),
        &plan.private_raw_roots,
    )?;
    for file in &plan.host_files {
        validate_pinned_file(&file.source, &file.artifact)?;
        validate_final_host_file(file.role, &file.source, &plan.source_commit)?;
    }
    let server_config = plan
        .configs
        .iter()
        .find(|config| config.role == VpsConfigRoleV2::Server)
        .context("release plan omits server config")?;
    let host_file = |role| {
        plan.host_files
            .iter()
            .find(|file| file.role == role)
            .map(|file| file.source.as_path())
            .context("release plan omits backup sandbox unit")
    };
    validate_backup_sandbox_contract(
        &server_config.source,
        host_file(VpsHostFileRoleV2::ApiService)?,
        host_file(VpsHostFileRoleV2::WorkerService)?,
        host_file(VpsHostFileRoleV2::BackupService)?,
        host_file(VpsHostFileRoleV2::BackupTimer)?,
        &plan.source_commit,
    )?;
    Ok(())
}

fn validate_pinned_file(path: &Path, expected: &ArtifactRefV1) -> Result<()> {
    reject_hardlink(path)?;
    ensure!(
        artifact_from_file(path, &expected.media_type)? == *expected,
        "pinned release source {} differs from its artifact identity",
        path.display()
    );
    Ok(())
}

fn validate_linux_executable_source(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        ensure!(
            fs::metadata(path)?.permissions().mode() & 0o111 != 0,
            "release binary is not executable: {}",
            path.display()
        );
    }
    validate_linux_elf(path)
}

fn validate_linux_elf(path: &Path) -> Result<()> {
    let bytes = read_regular_file_bounded(path, 1024 * 1024 * 1024)?;
    let elf = goblin::elf::Elf::parse(&bytes)
        .with_context(|| format!("release binary is not ELF: {}", path.display()))?;
    ensure!(
        elf.header.e_machine == goblin::elf::header::EM_X86_64
            && matches!(
                elf.header.e_type,
                goblin::elf::header::ET_EXEC | goblin::elf::header::ET_DYN
            ),
        "release binary is not an x86-64 Linux executable"
    );
    Ok(())
}

fn validate_final_config(
    role: VpsConfigRoleV2,
    path: &Path,
    verifier_sha256: Digest32,
    job_catalog: &ArtifactRefV1,
    source_commit: &str,
    campaign_states: &BTreeSet<Digest32>,
) -> Result<()> {
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    reject_placeholders(&bytes, "release config")?;
    match role {
        VpsConfigRoleV2::ApiEnvironment | VpsConfigRoleV2::WorkerEnvironment => {
            ensure!(
                bytes == b"RUST_LOG=info\n",
                "release environment files may contain only RUST_LOG=info"
            );
        }
        VpsConfigRoleV2::Server => {
            let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
            reject_obsolete_toml(&value)?;
            for (field, expected) in [
                ("bind", "127.0.0.1:8787"),
                (
                    "database_path",
                    "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3",
                ),
                (
                    "replay_directory",
                    "/home/robinhood/.local/share/robin-highscores/replays",
                ),
                (
                    "campaign_state_directory",
                    "/home/robinhood/.local/share/robin-highscores/campaign-states",
                ),
                (
                    "cursor_secret_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key",
                ),
                (
                    "competition_run_grant_secret_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/competition-run-grant.key",
                ),
                (
                    "run_preflight_grant_secret_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/run-preflight-grant.key",
                ),
                (
                    "moderation_bearer_token_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token",
                ),
                ("backup_manifest_path", BACKUP_STATUS_PATH),
            ] {
                ensure!(
                    value.get(field).and_then(toml::Value::as_str) == Some(expected),
                    "server config {field} is not the final private path"
                );
            }
            let expected_release_manifest =
                format!("{INSTALL_ROOT}/releases/{source_commit}/{RELEASE_MANIFEST_FILE}");
            ensure!(
                value
                    .get("release_manifest_path")
                    .and_then(toml::Value::as_str)
                    == Some(expected_release_manifest.as_str()),
                "server config must bind the exact installed VPS release manifest"
            );
            ensure!(
                value
                    .get("allowed_origins")
                    .and_then(toml::Value::as_array)
                    .is_some_and(Vec::is_empty),
                "production server config must not enable CORS origins"
            );
            ensure!(
                value
                    .get("maximum_backup_age_hours")
                    .and_then(toml::Value::as_integer)
                    == Some(32),
                "server config maximum backup age must cover daily schedule, jitter, timeout, and margin"
            );
            ensure!(
                toml_integer_at_least(&value, "minimum_storage_free_bytes", 1 << 30),
                "server config must reserve at least 1 GiB of storage"
            );
            for field in [
                "max_replay_bytes",
                "max_campaign_bytes",
                "max_metadata_bytes",
                "max_concurrent_uploads",
            ] {
                ensure!(
                    positive_toml_integer(&value, field),
                    "server config {field} must be positive"
                );
            }
            let expected_manifest_root =
                format!("{INSTALL_ROOT}/releases/{source_commit}/config/manifests");
            ensure!(
                value
                    .get("manifest_directory")
                    .and_then(toml::Value::as_str)
                    == Some(expected_manifest_root.as_str()),
                "server config must use the installed immutable manifest root"
            );
            let profiles = value
                .get("admission_profiles")
                .and_then(toml::Value::as_array)
                .filter(|profiles| !profiles.is_empty())
                .context("server config has no final admission profiles")?;
            let expected_campaign_root =
                format!("{INSTALL_ROOT}/releases/{source_commit}/private/campaign-states");
            let referenced_campaigns = profiles
                .iter()
                .map(|profile| {
                    let path = profile
                        .get("canonical_campaign_state_path")
                        .and_then(toml::Value::as_str)
                        .context("admission profile omits canonical campaign state path")?;
                    let path = Path::new(path);
                    ensure!(
                        path.parent() == Some(Path::new(&expected_campaign_root)),
                        "admission profile campaign template escapes the immutable release"
                    );
                    let digest = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .context("admission profile campaign template has no digest filename")?
                        .parse::<Digest32>()?;
                    Ok(digest)
                })
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                referenced_campaigns == *campaign_states,
                "server profiles do not bind the exact publication campaign templates"
            );
        }
        VpsConfigRoleV2::Worker => {
            let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
            reject_obsolete_toml(&value)?;
            let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
            let expected_server_config = format!("{release_root}/config/highscores-server.toml");
            ensure!(
                value.get("server_config").and_then(toml::Value::as_str)
                    == Some(expected_server_config.as_str()),
                "worker config does not use the exact commit-named server config"
            );
            ensure!(
                value
                    .get("campaign_state_directory")
                    .and_then(toml::Value::as_str)
                    == Some("/home/robinhood/.local/share/robin-highscores/campaign-states"),
                "worker config does not use the persistent campaign store"
            );
            let expected_catalog_path = format!(
                "{release_root}/private/verifier/operator-config/{}",
                job_catalog.sha256,
            );
            let expected_catalog_sha256 = job_catalog.sha256.to_string();
            ensure!(
                value
                    .get("verifier_job_config_catalog")
                    .and_then(toml::Value::as_str)
                    == Some(expected_catalog_path.as_str())
                    && value
                        .get("verifier_job_config_catalog_sha256")
                        .and_then(toml::Value::as_str)
                        == Some(expected_catalog_sha256.as_str()),
                "worker config does not bind the publication job catalog"
            );
            let launcher = value
                .get("verifier_launcher")
                .and_then(toml::Value::as_table)
                .context("worker config omits [verifier_launcher]")?;
            let expected_verifier_path = format!("{release_root}/bin/robin-replay-verifier");
            let expected_verifier_sha256 = verifier_sha256.to_string();
            ensure!(
                launcher.get("bwrap_program").and_then(toml::Value::as_str)
                    == Some("/usr/bin/bwrap")
                    && launcher
                        .get("prlimit_program")
                        .and_then(toml::Value::as_str)
                        == Some("/usr/bin/prlimit")
                    && launcher
                        .get("verifier_program")
                        .and_then(toml::Value::as_str)
                        == Some(expected_verifier_path.as_str()),
                "worker verifier launcher does not use the fixed host tools and exact release verifier"
            );
            ensure!(
                nonzero_lower_hex_table(launcher, "bwrap_sha256")
                    && nonzero_lower_hex_table(launcher, "prlimit_sha256")
                    && launcher
                        .get("verifier_sha256")
                        .and_then(toml::Value::as_str)
                        == Some(expected_verifier_sha256.as_str()),
                "worker verifier launcher has an absent, zero, or substituted digest"
            );
            for (field, expected) in [
                ("wall_timeout_seconds", 120),
                ("cpu_limit_seconds", 120),
                ("address_space_limit_bytes", 1_073_741_824),
                ("process_limit", 32),
                ("open_files_limit", 128),
                ("file_size_limit_bytes", 134_217_728),
                ("max_request_bytes", 1_048_576),
            ] {
                ensure!(
                    launcher.get(field).and_then(toml::Value::as_integer) == Some(expected),
                    "worker verifier launcher {field} differs from the canonical resource envelope"
                );
            }
            let limits = value
                .get("limits")
                .and_then(toml::Value::as_table)
                .context("worker config omits [limits]")?;
            let max_campaign_bytes = limits
                .get("max_campaign_bytes")
                .and_then(toml::Value::as_integer)
                .context("worker limits omit max_campaign_bytes")?;
            ensure!(
                max_campaign_bytes > 0 && max_campaign_bytes <= 134_217_728,
                "worker campaign limit exceeds the direct launch file-size limit"
            );
            let _ = worker_source_manifest_selections(&value, source_commit)?;
            for forbidden in [
                "broker_socket",
                "broker_response_timeout_seconds",
                "verifier_sha256",
                "sandbox_launcher",
                "sandbox_launcher_sha256",
            ] {
                ensure!(
                    value.get(forbidden).is_none(),
                    "worker config retains obsolete root-level field {forbidden}"
                );
            }
        }
    }
    Ok(())
}

fn reject_obsolete_toml(value: &toml::Value) -> Result<()> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let key = key.to_ascii_lowercase();
                ensure!(
                    !key.contains("broker")
                        && !key.contains("polkit")
                        && !matches!(
                            key.as_str(),
                            "worker_uid"
                                | "sandbox_launcher"
                                | "sandbox_launcher_sha256"
                                | "runtime_max_seconds"
                                | "memory_max_bytes"
                                | "tasks_max"
                                | "nofile_max"
                                | "file_size_max_bytes"
                        ),
                    "release config retains obsolete privileged field {key}"
                );
                reject_obsolete_toml(value)?;
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                reject_obsolete_toml(value)?;
            }
        }
        toml::Value::String(value) => {
            ensure!(
                !value.contains("verifier-broker")
                    && !value.contains("polkit")
                    && value != "/usr/bin/systemd-run"
                    && !value.starts_with("/opt/robin-highscores")
                    && !value.starts_with("/var/lib/robin-highscores")
                    && !value.starts_with("/srv/robin-highscores")
                    && !value.starts_with("/etc/robin-highscores"),
                "release config retains an obsolete privileged path or authority"
            );
        }
        _ => {}
    }
    Ok(())
}

fn positive_toml_integer(value: &toml::Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(toml::Value::as_integer)
        .is_some_and(|value| value > 0)
}

fn toml_integer_at_least(value: &toml::Value, field: &str, minimum: i64) -> bool {
    value
        .get(field)
        .and_then(toml::Value::as_integer)
        .is_some_and(|value| value >= minimum)
}

fn worker_source_manifest_selections(
    value: &toml::Value,
    source_commit: &str,
) -> Result<[(OfficialContentEditionV1, Digest32); 2]> {
    let expected_root =
        format!("{INSTALL_ROOT}/releases/{source_commit}/private/source-tree-manifests-v2");
    let mut selections = Vec::new();
    for (field, edition) in [
        ("demo_raw_content_manifest", OfficialContentEditionV1::Demo),
        ("full_raw_content_manifest", OfficialContentEditionV1::Full),
    ] {
        let configured = value
            .get(field)
            .and_then(toml::Value::as_str)
            .with_context(|| format!("worker config omits {field}"))?;
        let path = Path::new(configured);
        ensure!(
            normalized_absolute(path) && path.parent() == Some(Path::new(&expected_root)),
            "worker config {field} escapes the immutable release manifest root"
        );
        let digest = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
            .context("worker raw-content manifest is not a digest-named JSON file")?
            .parse::<Digest32>()?;
        ensure!(
            !digest.is_zero(),
            "worker raw-content manifest uses a zero digest"
        );
        selections.push((edition, digest));
    }
    ensure!(
        selections[0].1 != selections[1].1,
        "Demo and Full raw roots cannot share one source manifest"
    );
    Ok([selections[0], selections[1]])
}

fn validate_worker_raw_authority(
    worker_config: &Path,
    source_commit: &str,
    local_manifest_root: &Path,
    raw_roots: &[PrivateRawRootV2],
) -> Result<()> {
    let bytes = read_regular_file_bounded(worker_config, MAX_CONFIG_BYTES)?;
    let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    let selections = worker_source_manifest_selections(&value, source_commit)?;
    for (edition, digest) in selections {
        let path = local_manifest_root.join(format!("{digest}.json"));
        let manifest: OfficialSourceTreeManifestV2 = load_canonical(&path)?;
        ensure!(
            manifest.canonical_digest()? == digest && manifest.edition == edition,
            "selected raw-content source manifest has the wrong digest or edition"
        );
        let raw_root = raw_roots
            .iter()
            .find(|root| root.edition == edition)
            .context("raw-content declaration omits selected edition")?;
        validate_raw_content_against_manifest(&raw_root.root, &manifest)
            .with_context(|| format!("validate {edition:?} raw content against {digest}"))?;
    }
    Ok(())
}

fn validate_raw_content_against_manifest(
    raw_root: &Path,
    manifest: &OfficialSourceTreeManifestV2,
) -> Result<()> {
    for expected in &manifest.files {
        ensure!(
            valid_relative_manifest_path(&expected.path),
            "source manifest contains an unsafe raw path"
        );
        let path = raw_root.join(&expected.path);
        let actual = artifact_from_file(&path, "application/octet-stream")?;
        ensure!(
            actual.sha256 == expected.sha256 && actual.byte_length == expected.byte_length,
            "raw content differs from selected source manifest at {}",
            expected.path
        );
    }
    Ok(())
}

fn nonzero_lower_hex_table(value: &toml::map::Map<String, toml::Value>, field: &str) -> bool {
    value
        .get(field)
        .and_then(toml::Value::as_str)
        .is_some_and(|value| {
            value.len() == 64
                && value.bytes().any(|byte| byte != b'0')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

fn validate_final_text(path: &Path, label: &str) -> Result<()> {
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    std::str::from_utf8(&bytes).with_context(|| format!("{label} is not UTF-8"))?;
    reject_placeholders(&bytes, label)
}

fn validate_final_host_file(
    role: VpsHostFileRoleV2,
    path: &Path,
    source_commit: &str,
) -> Result<()> {
    validate_final_text(path, "host deployment file")?;
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    let text = std::str::from_utf8(&bytes)?;
    let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
    validate_system_unit_root_authority(role, text)?;
    let canonical_bytes = match role {
        VpsHostFileRoleV2::UserTarget => Some(CANONICAL_USER_TARGET.to_owned()),
        VpsHostFileRoleV2::ApiService => {
            Some(CANONICAL_API_SERVICE.replace("@SOURCE_COMMIT@", source_commit))
        }
        VpsHostFileRoleV2::WorkerService => {
            Some(CANONICAL_WORKER_SERVICE.replace("@SOURCE_COMMIT@", source_commit))
        }
        VpsHostFileRoleV2::BackupService => {
            Some(CANONICAL_BACKUP_SERVICE.replace("@SOURCE_COMMIT@", source_commit))
        }
        VpsHostFileRoleV2::BackupTimer => Some(CANONICAL_BACKUP_TIMER.to_owned()),
        VpsHostFileRoleV2::DeployReleaseScript => Some(CANONICAL_DEPLOY_RELEASE_SCRIPT.to_owned()),
        VpsHostFileRoleV2::RollbackReleaseScript => {
            Some(CANONICAL_ROLLBACK_RELEASE_SCRIPT.to_owned())
        }
        VpsHostFileRoleV2::ValidateReleaseScript => {
            Some(CANONICAL_VALIDATE_RELEASE_SCRIPT.to_owned())
        }
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate => {
            Some(CANONICAL_REAL_FENCE_RELEASE_GATE.to_owned())
        }
        VpsHostFileRoleV2::RealRuntimeFenceHarness => Some(CANONICAL_REAL_FENCE_HARNESS.to_owned()),
        VpsHostFileRoleV2::RealRuntimeFenceSelftest => {
            Some(CANONICAL_REAL_FENCE_SELFTEST.to_owned())
        }
        VpsHostFileRoleV2::RootOnceScript => Some(CANONICAL_ROOT_ONCE_SCRIPT.to_owned()),
        VpsHostFileRoleV2::NginxChallenge => Some(CANONICAL_NGINX_CHALLENGE.to_owned()),
        VpsHostFileRoleV2::NginxCloudflareOnly => Some(CANONICAL_NGINX_CLOUDFLARE_ONLY.to_owned()),
        VpsHostFileRoleV2::NginxApiLocations => Some(CANONICAL_NGINX_API_LOCATIONS.to_owned()),
        VpsHostFileRoleV2::NginxVhost => Some(CANONICAL_NGINX_VHOST.to_owned()),
        _ => None,
    };
    if let Some(canonical) = canonical_bytes {
        ensure!(
            text == canonical,
            "security-sensitive host file differs from its exact canonical repository template"
        );
    } else {
        reject_placeholders(&bytes, "host deployment file")?;
    }
    match role {
        VpsHostFileRoleV2::UserTarget => {
            ensure!(
                text.contains("robin-highscores-api.service")
                    && text.contains("robin-highscores-worker.service")
                    && text.contains("WantedBy=default.target"),
                "user target does not own the API/worker boot lifecycle"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::ApiService => {
            ensure!(
                text.contains(&format!(
                    "ExecStart={release_root}/bin/robin-highscores-server --config {release_root}/config/highscores-server.toml"
                )),
                "API user unit is not pinned to the exact release"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::WorkerService => {
            ensure!(
                text.contains(&format!(
                    "ExecStart={release_root}/bin/robin-highscores-worker --config {release_root}/config/highscores-worker.toml"
                )),
                "worker user unit is not pinned to the exact release"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::BackupService => {
            ensure!(
                text.contains(&format!(
                    "ExecStart={release_root}/bin/robin-highscores-admin"
                )) && text.contains(&format!(
                    "--config {release_root}/config/highscores-server.toml"
                )),
                "backup user unit is not pinned to the exact release"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::BackupTimer => {
            ensure!(
                text.contains("Unit=robin-highscores-backup.service")
                    && text.contains("WantedBy=timers.target"),
                "backup timer does not activate the user backup service"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::DeployReleaseScript | VpsHostFileRoleV2::RollbackReleaseScript => {
            validate_literal_install_root_assignment(text)?;
            ensure!(
                text.starts_with("#!/bin/sh\n")
                    && text.contains(INSTALL_ROOT)
                    && text.contains("systemctl --user"),
                "user release script does not use the canonical install root and user manager"
            );
            ensure!(
                !text.contains("sudo ") && !text.contains("/etc/systemd/system"),
                "user release script retains privileged activation authority"
            );
            if role == VpsHostFileRoleV2::DeployReleaseScript {
                ensure!(
                    text.contains(
                        "managed state directory is not exact and will not be repaired"
                    ) && text.contains(
                        "backup state directory is missing on upgrade and will not be repaired"
                    ) && text.contains("if [ \"$receipt_source_commit\" = none ]; then")
                        && text.contains(
                            "for initialized_directory in \"$backup_root\" \"$state_root/status\"; do"
                        )
                        && text.contains("mkdir -m 0700 -- \"$initialized_directory\"")
                        && text.contains(
                            "could not durably bind the initialized runtime authority"
                        )
                        && text.contains(
                            "deploy/tests/real-runtime-fence-release-gate.sh"
                        )
                        && text.contains("ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD")
                        && text.contains(
                            "mandatory authentic runtime-fence release gate failed before activation mutation"
                        )
                        && !text.contains("chmod 0700 -- \"$state_root\"")
                        && !text.contains("chmod 0700 -- \"$managed_directory\""),
                    "deploy script must validate all upgrade state without repair and create clean-first backup state only after durable runtime authority"
                );
            }
        }
        VpsHostFileRoleV2::ValidateReleaseScript => {
            ensure!(
                text.starts_with("#!/bin/sh\n")
                    && text.contains("robin-highscores-manifestctl")
                    && text.contains("SHA256SUMS")
                    && text.contains("MODE_INVENTORY"),
                "release validator does not check the typed bundle and both inventories"
            );
            ensure!(
                !text.contains("sudo "),
                "release validator retains privileged activation authority"
            );
        }
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate
        | VpsHostFileRoleV2::RealRuntimeFenceHarness
        | VpsHostFileRoleV2::RealRuntimeFenceSelftest => {
            ensure!(
                text.starts_with("#!/bin/sh\n") || text.starts_with("#!/usr/bin/env python3\n"),
                "real runtime-fence gate authority has no exact interpreter"
            );
            ensure!(
                !text.contains("sudo ") && !text.contains("/etc/systemd/system"),
                "real runtime-fence gate retains privileged mutation authority"
            );
        }
        VpsHostFileRoleV2::RootOnceScript => {
            ensure!(
                text.starts_with("#!/bin/sh\n") && text.contains("nginx"),
                "root-once script is not the reviewed nginx setup"
            );
            ensure!(
                !text.contains("/etc/systemd/system") && !text.contains("useradd"),
                "root-once nginx script retains service/principal bootstrap authority"
            );
        }
        VpsHostFileRoleV2::NginxChallenge => {
            ensure!(
                text.contains(".well-known/acme-challenge"),
                "nginx challenge vhost omits the ACME challenge route"
            );
        }
        VpsHostFileRoleV2::NginxCloudflareOnly => {
            ensure!(
                text.contains("allow ") && text.contains("deny all"),
                "nginx Cloudflare include is not a fail-closed allowlist"
            );
        }
        VpsHostFileRoleV2::NginxApiLocations => {
            ensure!(
                text.contains("127.0.0.1:8787") && text.contains("/api"),
                "nginx include does not route the loopback leaderboard API"
            );
        }
        VpsHostFileRoleV2::NginxVhost => {
            ensure!(
                text.contains("robinhood.phiresky.xyz")
                    && text.contains("robinhood-api.locations.conf"),
                "nginx vhost does not bind the production domain and API include"
            );
        }
        VpsHostFileRoleV2::DeploymentReadme
        | VpsHostFileRoleV2::OperatorRunbook
        | VpsHostFileRoleV2::BackupRunbook => {}
    }
    Ok(())
}

fn validate_system_unit_root_authority(role: VpsHostFileRoleV2, text: &str) -> Result<()> {
    if role == VpsHostFileRoleV2::ValidateReleaseScript {
        ensure!(
            text == CANONICAL_VALIDATE_RELEASE_SCRIPT
                && text.matches(SYSTEM_UNIT_ROOT).count() == 1
                && text.matches(VALIDATOR_SYSTEM_UNIT_DENYLIST_BLOCK).count() == 1,
            "release validator must be the exact canonical script with one system-unit-root denylist block"
        );
    } else {
        ensure!(
            !text.contains(SYSTEM_UNIT_ROOT),
            "host deployment file retains root system-service authority"
        );
    }
    Ok(())
}

fn validate_literal_install_root_assignment(text: &str) -> Result<()> {
    let expected = format!("opt_root={INSTALL_ROOT}");
    let mut assignments = text
        .lines()
        .filter(|line| line.trim_start().starts_with("opt_root="));
    ensure!(
        assignments.next() == Some(expected.as_str()) && assignments.next().is_none(),
        "user release script must contain exactly one literal {expected} assignment"
    );
    Ok(())
}

fn validate_user_unit(text: &str) -> Result<()> {
    for forbidden in [
        "User=",
        "Group=",
        "SupplementaryGroups=",
        "WantedBy=multi-user.target",
        "/etc/systemd/system",
        "/var/lib/robin-highscores",
        "/srv/robin-highscores",
        "verifier-broker",
        "systemd-run",
        "polkit",
    ] {
        ensure!(
            !text.contains(forbidden),
            "user unit retains obsolete root deployment field {forbidden}"
        );
    }
    ensure!(
        !text
            .split(|character: char| character.is_ascii_whitespace() || character == '=')
            .any(|token| token.starts_with("/opt/robin-highscores")),
        "user unit retains the obsolete root-owned /opt release path"
    );
    Ok(())
}

fn validate_backup_sandbox_contract(
    server_config: &Path,
    api_unit: &Path,
    worker_unit: &Path,
    backup_unit: &Path,
    backup_timer: &Path,
    source_commit: &str,
) -> Result<()> {
    let config_bytes = read_regular_file_bounded(server_config, MAX_CONFIG_BYTES)?;
    let config: toml::Value = toml::from_str(std::str::from_utf8(&config_bytes)?)?;
    ensure!(
        config
            .get("maximum_backup_age_hours")
            .and_then(toml::Value::as_integer)
            == Some(32),
        "backup readiness maximum age must be exactly 32 hours"
    );
    let status_path = config
        .get("backup_manifest_path")
        .and_then(toml::Value::as_str)
        .context("server config omits backup_manifest_path")?;
    ensure!(
        status_path == BACKUP_STATUS_PATH,
        "server backup_manifest_path must name the isolated status authority"
    );
    let status_parent = Path::new(status_path)
        .parent()
        .context("server backup_manifest_path has no parent")?;
    ensure!(
        status_parent == Path::new(BACKUP_STATUS_ROOT)
            && !Path::new(status_path).starts_with(BACKUP_ROOT),
        "server backup_manifest_path must remain outside the backup payload root"
    );
    let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
    let release_manifest = format!("{release_root}/{RELEASE_MANIFEST_FILE}");
    ensure!(
        config
            .get("release_manifest_path")
            .and_then(toml::Value::as_str)
            == Some(release_manifest.as_str()),
        "server release_manifest_path must name the exact installed release manifest"
    );

    let api = unit_text(api_unit)?;
    ensure!(
        has_exact_unit_path(&api, "ReadOnlyPaths", BACKUP_STATUS_ROOT),
        "API unit must receive read-only access to the exact backup status directory"
    );
    ensure!(
        has_exact_unit_path(&api, "InaccessiblePaths", BACKUP_ROOT),
        "API unit must keep backup payloads inaccessible"
    );
    ensure!(
        !has_exact_unit_path(&api, "ReadWritePaths", BACKUP_STATUS_ROOT)
            && !unit_grants_path(&api, BACKUP_ROOT),
        "API unit grants forbidden write/status or backup-payload access"
    );

    let backup = unit_text(backup_unit)?;
    for writable in [BACKUP_ROOT, BACKUP_STATUS_ROOT] {
        ensure!(
            has_exact_unit_path(&backup, "ReadWritePaths", writable),
            "backup unit must write the exact payload and status directories"
        );
    }
    ensure!(
        backup.lines().any(|line| {
            line.starts_with("ExecStart=")
                && line.contains(&format!(" --release-manifest-path {release_manifest} "))
                && line.contains(&format!(" --backup-root {BACKUP_ROOT} "))
                && line.contains(&format!(" --status-path {BACKUP_STATUS_PATH} "))
        }),
        "backup unit command does not publish to the server's exact status authority"
    );
    let exec = backup
        .lines()
        .find(|line| line.starts_with("ExecStart="))
        .context("backup unit omits ExecStart")?;
    ensure!(
        !exec.contains("--configuration-root")
            && !exec.contains(&format!("--restore-source-map {release_root}="))
            && !exec.contains(&format!(
                "--restore-source-map {DEPLOYMENT_HOME}/.config/systemd/user="
            )),
        "backup unit must not copy immutable release/config trees or the symlink-bearing user-unit tree"
    );
    let tokens = exec.split_ascii_whitespace().collect::<Vec<_>>();
    let actual_maps = tokens
        .windows(2)
        .filter(|pair| pair[0] == "--restore-source-map")
        .map(|pair| pair[1].to_owned())
        .collect::<BTreeSet<_>>();
    let secret_root = format!("{STATE_ROOT}/api-secrets");
    let user_unit_root = format!("{DEPLOYMENT_HOME}/.config/systemd/user");
    let expected_maps = [
        format!("{secret_root}/cursor-hmac.key={secret_root}/cursor-hmac.key"),
        format!("{secret_root}/competition-run-grant.key={secret_root}/competition-run-grant.key"),
        format!("{secret_root}/run-preflight-grant.key={secret_root}/run-preflight-grant.key"),
        format!("{secret_root}/moderation-bearer.token={secret_root}/moderation-bearer.token"),
        format!("{user_unit_root}/robin-highscores.target={user_unit_root}/robin-highscores.target"),
        format!("{user_unit_root}/robin-highscores-api.service={user_unit_root}/robin-highscores-api.service"),
        format!("{user_unit_root}/robin-highscores-worker.service={user_unit_root}/robin-highscores-worker.service"),
        format!("{user_unit_root}/robin-highscores-backup.service={user_unit_root}/robin-highscores-backup.service"),
        format!("{user_unit_root}/robin-highscores-backup.timer={user_unit_root}/robin-highscores-backup.timer"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    ensure!(
        actual_maps == expected_maps,
        "backup unit restore maps must name exactly four secrets and five regular user units"
    );
    ensure!(
        !has_exact_unit_path(&backup, "ReadOnlyPaths", &user_unit_root),
        "backup unit must not expose the whole symlink-bearing user-unit tree"
    );
    for unit in [
        "robin-highscores.target",
        "robin-highscores-api.service",
        "robin-highscores-worker.service",
        "robin-highscores-backup.service",
        "robin-highscores-backup.timer",
    ] {
        ensure!(
            has_exact_unit_path(
                &backup,
                "ReadOnlyPaths",
                &format!("{user_unit_root}/{unit}"),
            ),
            "backup sandbox omits exact user-unit source {unit}"
        );
    }
    ensure!(
        backup
            .lines()
            .any(|line| line.trim() == "TimeoutStartSec=6h"),
        "backup timeout differs from the readiness timing contract"
    );
    let timer = unit_text(backup_timer)?;
    ensure!(
        timer
            .lines()
            .any(|line| line.trim() == "OnCalendar=*-*-* 02:15:00")
            && timer
                .lines()
                .any(|line| line.trim() == "RandomizedDelaySec=45m")
            && 32 * 60 > 24 * 60 + 45 + 6 * 60 + 60,
        "backup age does not cover daily interval, jitter, timeout, and one-hour margin"
    );

    let worker = unit_text(worker_unit)?;
    for inaccessible in [BACKUP_ROOT, BACKUP_STATUS_ROOT] {
        ensure!(
            has_exact_unit_path(&worker, "InaccessiblePaths", inaccessible),
            "worker unit must not gain access to backup payload or status authority"
        );
        ensure!(
            !unit_grants_path(&worker, inaccessible),
            "worker unit grants forbidden backup/status access"
        );
    }
    Ok(())
}

fn unit_text(path: &Path) -> Result<String> {
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    Ok(std::str::from_utf8(&bytes)?.to_owned())
}

fn has_exact_unit_path(text: &str, directive: &str, path: &str) -> bool {
    let expected = format!("{directive}={path}");
    text.lines().any(|line| line.trim() == expected)
}

fn unit_grants_path(text: &str, path: &str) -> bool {
    text.lines().any(|line| {
        ["ReadOnlyPaths", "ReadWritePaths"].iter().any(|directive| {
            line.trim()
                .strip_prefix(&format!("{directive}="))
                .is_some_and(|paths| paths.split_ascii_whitespace().any(|entry| entry == path))
        })
    })
}

fn reject_placeholders(bytes: &[u8], label: &str) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    let lowercase = text.to_ascii_lowercase();
    ensure!(
        !lowercase.contains(&"0".repeat(64))
            && !lowercase.contains("changeme")
            && !lowercase.contains("placeholder")
            && !lowercase.contains("example.invalid"),
        "{label} contains a zero/example/placeholder value"
    );
    Ok(())
}

fn materialize_bundle(
    root: &Path,
    plan: &VpsReleasePlanV2,
    publication: &mut ValidatedPublicationV3,
) -> Result<()> {
    for binary in &plan.binaries {
        copy_exact(
            &binary.source,
            &root.join("bin").join(binary.role.output_name()),
            &binary.artifact,
        )?;
    }
    for config in &plan.configs {
        copy_exact(
            &config.source,
            &root.join("config").join(config.role.output_name()),
            &config.artifact,
        )?;
    }
    for file in &plan.host_files {
        copy_exact(
            &file.source,
            &root.join(file.role.output_path()),
            &file.artifact,
        )?;
    }
    write_bytes(
        &root.join(ROOT_ONCE_SHA256SUMS_FILE),
        &expected_root_once_sha256sums(root)?,
    )?;
    write_bytes(
        &root.join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
        &expected_deploy_bootstrap_sha256sums(root)?,
    )?;

    copy_publication_tree_exact(
        publication,
        "backend/manifests",
        &root.join("config/manifests"),
    )?;
    for (source, destination) in [
        (
            "private/official-content-authority/verifier-bundles",
            "private/verifier-bundles",
        ),
        ("private/campaign-states", "private/campaign-states"),
        (
            "private/verifier/operator-config",
            "private/verifier/operator-config",
        ),
        (
            "private/official-content-authority/private/source-tree-manifests-v2",
            "private/source-tree-manifests-v2",
        ),
    ] {
        copy_publication_tree_exact(publication, source, &root.join(destination))?;
    }
    for (source, destination) in [
        (
            "backend/publication-v3.json",
            "publication/backend-publication-v3.json",
        ),
        (
            "publication-manifest-v3.json",
            "publication/publication-manifest-v3.json",
        ),
        (
            "publication-manifest-v3.sha256",
            "publication/publication-manifest-v3.sha256",
        ),
        (
            "publication-lock-v3.json",
            "publication/publication-lock-v3.json",
        ),
        (
            "publication-lock-v3.sha256",
            "publication/publication-lock-v3.sha256",
        ),
    ] {
        copy_publication_file_exact(publication, source, &root.join(destination))?;
    }
    publication.ensure_live()?;
    let declarations = PrivateRawRootDeclarationsV2 {
        schema_version: RAW_ROOTS_SCHEMA_VERSION,
        roots: plan.private_raw_roots.clone(),
    };
    declarations.validate()?;
    write_bytes(
        &root.join(RAW_ROOT_DECLARATIONS_FILE),
        &canonical_json_bytes(&declarations)?,
    )?;
    Ok(())
}

fn copy_publication_tree_exact(
    publication: &mut ValidatedPublicationV3,
    source_prefix: &str,
    destination: &Path,
) -> Result<()> {
    let directories = publication.relative_directories(source_prefix);
    ensure!(
        directories.first().is_some_and(|path| path == "."),
        "validated PublicationV3 omits source directory {source_prefix}"
    );
    ensure!(!destination.exists(), "bundle destination already exists");
    fs::create_dir_all(destination)?;
    for relative in directories.into_iter().filter(|path| path != ".") {
        fs::create_dir(destination.join(relative))?;
    }
    for relative in publication.relative_files(source_prefix) {
        copy_publication_file_exact(
            publication,
            &format!("{source_prefix}/{relative}"),
            &destination.join(relative),
        )?;
    }
    publication.ensure_live()
}

fn copy_publication_file_exact(
    publication: &mut ValidatedPublicationV3,
    source: &str,
    destination: &Path,
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(destination)?;
    let copied = publication.copy_file_to(source, &mut output)?;
    ensure!(
        artifact_from_file(destination, &copied.media_type)? == copied,
        "VPS bundle copy changed retained PublicationV3 source {source}"
    );
    Ok(())
}

fn copy_exact(source: &Path, destination: &Path, expected: &ArtifactRefV1) -> Result<()> {
    validate_pinned_file(source, expected)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut reader = BufReader::new(File::open(source)?);
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?,
    );
    std::io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    ensure!(
        artifact_from_file(destination, &expected.media_type)? == *expected,
        "bundle copy changed {}",
        destination.display()
    );
    Ok(())
}

fn payload_inventory(root: &Path) -> Result<Vec<VpsReleaseFileV2>> {
    let mut files = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, absolute)| Ok((path_to_manifest(&relative)?, absolute)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|(path, _)| {
            !matches!(
                path.as_str(),
                SOURCE_COMMIT_FILE | SHA256SUMS_FILE | MODE_INVENTORY_FILE | RELEASE_MANIFEST_FILE
            )
        })
        .map(|(path, absolute)| {
            Ok(VpsReleaseFileV2 {
                unix_mode: canonical_file_mode(&path),
                path,
                artifact: artifact_from_file(&absolute, "application/octet-stream")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // `PathBuf::Ord` compares path components, while the canonical manifest
    // contract compares the serialized UTF-8 paths. Those orders differ for
    // prefix siblings such as `private/verifier/` and
    // `private/verifier-bundles/` (`'-' < '/'` in the manifest strings).
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn write_mode_inventory(root: &Path) -> Result<()> {
    let bytes = expected_mode_inventory_with_future_metadata(root)?;
    write_bytes(&root.join(MODE_INVENTORY_FILE), &bytes)
}

fn expected_mode_inventory_with_future_metadata(root: &Path) -> Result<Vec<u8>> {
    let mut entries = tree_modes(root, true)?;
    entries.insert(MODE_INVENTORY_FILE.into(), ('f', 0o440));
    entries.insert(SHA256SUMS_FILE.into(), ('f', 0o440));
    mode_inventory_bytes(&entries)
}

fn expected_mode_inventory(root: &Path) -> Result<Vec<u8>> {
    let entries = tree_modes(root, false)?;
    mode_inventory_bytes(&entries)
}

fn tree_modes(root: &Path, expected: bool) -> Result<BTreeMap<String, (char, u32)>> {
    let root_mode = if expected { 0o550 } else { actual_mode(root)? };
    let mut entries = BTreeMap::from([(".".to_owned(), ('d', root_mode))]);
    let mut pending = vec![(PathBuf::new(), root.to_path_buf())];
    while let Some((relative_root, absolute_root)) = pending.pop() {
        let mut children = fs::read_dir(&absolute_root)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let metadata = fs::symlink_metadata(child.path())?;
            let relative = relative_root.join(child.file_name());
            let path = path_to_manifest(&relative)?;
            if metadata.is_dir() {
                let mode = if expected {
                    0o550
                } else {
                    actual_mode(&child.path())?
                };
                entries.insert(format!("{path}/"), ('d', mode));
                pending.push((relative, child.path()));
            } else {
                ensure!(metadata.is_file(), "mode inventory found a special node");
                let mode = if expected {
                    canonical_file_mode(&path)
                } else {
                    actual_mode(&child.path())?
                };
                entries.insert(path, ('f', mode));
            }
        }
    }
    Ok(entries)
}

fn mode_inventory_bytes(entries: &BTreeMap<String, (char, u32)>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for (path, (kind, mode)) in entries {
        ensure!(
            matches!((*kind, *mode), ('f', 0o440) | ('f', 0o550) | ('d', 0o550)),
            "bundle contains mutable mode"
        );
        writeln!(&mut bytes, "{kind} {mode:04o}  {path}")?;
    }
    Ok(bytes)
}

fn write_sha256sums(root: &Path) -> Result<()> {
    write_bytes(&root.join(SHA256SUMS_FILE), &expected_sha256sums(root)?)
}

fn expected_sha256sums(root: &Path) -> Result<Vec<u8>> {
    let mut files = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, absolute)| Ok((path_to_manifest(&relative)?, absolute)))
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut bytes = Vec::new();
    for (path, absolute) in files {
        if path == SHA256SUMS_FILE {
            continue;
        }
        let artifact = artifact_from_file(&absolute, "application/octet-stream")?;
        writeln!(&mut bytes, "{}  {path}", artifact.sha256)?;
    }
    Ok(bytes)
}

fn expected_root_once_sha256sums(root: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for name in ROOT_ONCE_KIT_FILES {
        let artifact =
            artifact_from_file(&root.join("deploy").join(name), "application/octet-stream")?;
        writeln!(&mut bytes, "{}  {name}", artifact.sha256)?;
    }
    Ok(bytes)
}

fn validate_root_once_sha256sums(root: &Path) -> Result<()> {
    ensure!(
        fs::read(root.join(ROOT_ONCE_SHA256SUMS_FILE))? == expected_root_once_sha256sums(root)?,
        "ROOT_ONCE_SHA256SUMS is missing, reordered, substituted, or has extra entries"
    );
    Ok(())
}

fn expected_deploy_bootstrap_sha256sums(root: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for name in DEPLOY_BOOTSTRAP_FILES {
        let artifact =
            artifact_from_file(&root.join("deploy").join(name), "application/octet-stream")?;
        writeln!(&mut bytes, "{}  {name}", artifact.sha256)?;
    }
    Ok(bytes)
}

fn validate_deploy_bootstrap_sha256sums(root: &Path) -> Result<()> {
    ensure!(
        fs::read(root.join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE))?
            == expected_deploy_bootstrap_sha256sums(root)?,
        "DEPLOY_BOOTSTRAP_SHA256SUMS is missing, reordered, substituted, or has extra entries"
    );
    Ok(())
}

fn make_bundle_read_only(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for (relative, path) in walk_regular_files(root)? {
            let relative = path_to_manifest(&relative)?;
            fs::set_permissions(
                path,
                fs::Permissions::from_mode(canonical_file_mode(&relative)),
            )?;
        }
        let mut directories = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            directories.push(directory.clone());
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o550))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("VPS releases require Unix permission semantics")
}

const MAX_FAILED_VPS_STAGING_ENTRIES: usize = 262_144;
const MAX_FAILED_VPS_STAGING_DEPTH: usize = 128;

#[derive(Debug)]
pub struct VpsReleaseInstalledButParentSyncFailed {
    pub output: PathBuf,
    pub release_manifest_sha256: Digest32,
    pub source_commit: String,
    pub source: anyhow::Error,
}

impl std::fmt::Display for VpsReleaseInstalledButParentSyncFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "VPS release {} for source {} was atomically installed with manifest {} but parent-directory durability sync failed; validate the immutable final and treat it as installed",
            self.output.display(),
            self.source_commit,
            self.release_manifest_sha256,
        )
    }
}

impl std::error::Error for VpsReleaseInstalledButParentSyncFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

fn vps_installed_durability_error(
    output: &Path,
    release_manifest_sha256: Digest32,
    source_commit: &str,
    source: anyhow::Error,
) -> anyhow::Error {
    VpsReleaseInstalledButParentSyncFailed {
        output: output.to_path_buf(),
        release_manifest_sha256,
        source_commit: source_commit.to_owned(),
        source,
    }
    .into()
}

#[derive(Debug)]
enum VpsPersistenceOutcome {
    Installed,
    InstalledButParentSyncFailed(anyhow::Error),
}

fn persist_vps_staging(
    staging: &tempfile::TempDir,
    output: &Path,
) -> Result<VpsPersistenceOutcome> {
    persist_vps_staging_with(staging, output, |parent| {
        File::open(parent)?.sync_all()?;
        Ok(())
    })
}

fn persist_vps_staging_with<F>(
    staging: &tempfile::TempDir,
    output: &Path,
    sync_parent: F,
) -> Result<VpsPersistenceOutcome>
where
    F: FnOnce(&Path) -> Result<()>,
{
    crate::sync_directory_tree(staging.path())?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging.path(),
        rustix::fs::CWD,
        output,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .with_context(|| {
        format!(
            "atomically install {} as {}",
            staging.path().display(),
            output.display()
        )
    })?;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (staging, output);
        anyhow::bail!("VPS release installation requires atomic rename NOREPLACE support");
    }
    let parent = output.parent().context("VPS output has no parent")?;
    Ok(match sync_parent(parent) {
        Ok(()) => VpsPersistenceOutcome::Installed,
        Err(error) => VpsPersistenceOutcome::InstalledButParentSyncFailed(error),
    })
}

fn discard_failed_vps_staging(staging: tempfile::TempDir) -> Result<()> {
    let staging_path = staging.path().to_path_buf();
    if let Err(cleanup_guard_error) = make_failed_vps_staging_removable(&staging_path) {
        let preserved = staging.keep();
        return Err(cleanup_guard_error.context(format!(
            "unsafe failed VPS staging was preserved at {}",
            preserved.display()
        )));
    }
    staging
        .close()
        .with_context(|| format!("remove failed VPS staging {}", staging_path.display()))?;
    ensure!(
        fs::symlink_metadata(&staging_path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "failed VPS staging still exists after cleanup: {}",
        staging_path.display()
    );
    Ok(())
}

fn make_failed_vps_staging_removable(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let metadata = fs::symlink_metadata(root)
            .with_context(|| format!("inspect failed VPS staging {}", root.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "failed VPS staging root is not a real directory"
        );
        let expected_uid = metadata.uid();
        let expected_device = metadata.dev();
        ensure!(
            expected_uid == rustix::process::geteuid().as_raw(),
            "failed VPS staging root is not owned by the current user"
        );
        reject_mounts_at_or_below(root)?;
        let mut seen = 1_usize;
        let mut pending = vec![(root.to_path_buf(), 0_usize)];
        let mut directories = Vec::new();
        while let Some((directory, depth)) = pending.pop() {
            ensure!(
                depth <= MAX_FAILED_VPS_STAGING_DEPTH,
                "failed VPS staging exceeds cleanup depth bound"
            );
            let metadata = fs::symlink_metadata(&directory)?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "failed VPS staging directory was substituted"
            );
            ensure!(
                metadata.uid() == expected_uid && metadata.dev() == expected_device,
                "failed VPS staging contains mixed ownership or devices"
            );
            directories.push(directory.clone());
            let mut entries = fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                seen = seen
                    .checked_add(1)
                    .context("failed VPS staging entry count overflow")?;
                ensure!(
                    seen <= MAX_FAILED_VPS_STAGING_ENTRIES,
                    "failed VPS staging exceeds cleanup entry bound"
                );
                let metadata = fs::symlink_metadata(entry.path())?;
                ensure!(
                    metadata.uid() == expected_uid && metadata.dev() == expected_device,
                    "failed VPS staging contains mixed ownership or devices"
                );
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_file() {
                    ensure!(
                        metadata.nlink() == 1,
                        "failed VPS staging contains a hard-linked file"
                    );
                    continue;
                }
                ensure!(
                    metadata.is_dir(),
                    "failed VPS staging contains a special node"
                );
                pending.push((entry.path(), depth + 1));
            }
        }
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        anyhow::bail!("VPS staging cleanup requires Unix permission semantics");
    }
    Ok(())
}

#[cfg(unix)]
fn reject_mounts_at_or_below(root: &Path) -> Result<()> {
    reject_mounts(root, true)
}

#[cfg(unix)]
fn reject_mounts_strictly_below(root: &Path) -> Result<()> {
    reject_mounts(root, false)
}

#[cfg(unix)]
fn reject_mounts(root: &Path, reject_root_itself: bool) -> Result<()> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let canonical_root = fs::canonicalize(root)?;
    let mountinfo = fs::read("/proc/self/mountinfo").context("read mount inventory")?;
    for line in mountinfo
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let mount_field = line
            .split(|byte| *byte == b' ')
            .nth(4)
            .context("malformed /proc/self/mountinfo line")?;
        let mut decoded = Vec::with_capacity(mount_field.len());
        let mut index = 0;
        while index < mount_field.len() {
            if mount_field[index] == b'\\'
                && index + 3 < mount_field.len()
                && mount_field[index + 1..index + 4]
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'7'))
            {
                decoded.push(
                    (mount_field[index + 1] - b'0') * 64
                        + (mount_field[index + 2] - b'0') * 8
                        + (mount_field[index + 3] - b'0'),
                );
                index += 4;
            } else {
                decoded.push(mount_field[index]);
                index += 1;
            }
        }
        let mount_path = PathBuf::from(OsString::from_vec(decoded));
        ensure!(
            !mount_path.starts_with(&canonical_root)
                || (!reject_root_itself && mount_path == canonical_root),
            "failed VPS staging contains a mount at {}",
            mount_path.display()
        );
    }
    Ok(())
}

fn validate_bundle_shape(root: &Path, manifest: &VpsReleaseManifestV2) -> Result<()> {
    let paths = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    for required in [
        "bin/robin-highscores-admin",
        "bin/robin-highscores-manifestctl",
        "bin/robin-highscores-server",
        "bin/robin-highscores-worker",
        "bin/robin-replay-verifier",
        "config/highscores-server.toml",
        "config/highscores-worker.toml",
        "config/api.env",
        "config/worker.env",
        "systemd/user/robin-highscores.target",
        "systemd/user/robin-highscores-api.service",
        "systemd/user/robin-highscores-worker.service",
        "systemd/user/robin-highscores-backup.service",
        "systemd/user/robin-highscores-backup.timer",
        "deploy/deploy-release.sh",
        "deploy/rollback-release.sh",
        "deploy/validate-release-bundle.sh",
        "deploy/tests/real-runtime-fence-release-gate.sh",
        "deploy/tests/real-runtime-fence-e2e.py",
        "deploy/tests/real-runtime-fence-e2e-selftest.py",
        DEPLOY_BOOTSTRAP_SHA256SUMS_FILE,
        ROOT_ONCE_SHA256SUMS_FILE,
        "deploy/root-once.sh",
        "deploy/nginx-robinhood-api.challenge.conf",
        "deploy/nginx-robinhood-cloudflare-only.conf",
        "deploy/nginx-robinhood-api.locations.conf",
        "deploy/nginx-robinhood-api.vhost.conf",
        "deploy/README.md",
        "deploy/VPS_RELEASE_INSTALL.md",
        "deploy/BACKUP_RESTORE.md",
        RAW_ROOT_DECLARATIONS_FILE,
        "publication/backend-publication-v3.json",
        "publication/publication-manifest-v3.json",
        "publication/publication-manifest-v3.sha256",
        "publication/publication-lock-v3.json",
        "publication/publication-lock-v3.sha256",
    ] {
        ensure!(paths.contains(required), "VPS bundle omits {required}");
    }
    ensure!(
        paths.iter().all(|path| !forbidden_release_path(path)),
        "release retained broker, polkit, socket, or root system-service authority"
    );
    ensure!(
        paths.iter().all(|path| {
            !path.starts_with("cloudflare-public/")
                && !path.starts_with("cloudflare-identity-signer/")
                && !path.starts_with("static/")
                && !path.starts_with("datadirs/")
                && !path.starts_with("raw/")
        }),
        "VPS release contains public/private static or copyrighted raw files"
    );
    let declarations: PrivateRawRootDeclarationsV2 =
        load_canonical(&root.join(RAW_ROOT_DECLARATIONS_FILE))?;
    declarations.validate()?;
    for declaration in &declarations.roots {
        validate_immutable_raw_root(&declaration.root).with_context(|| {
            format!(
                "validate separately installed {:?} raw root",
                declaration.edition
            )
        })?;
    }
    let mut canonical_roots = declarations
        .roots
        .iter()
        .map(|declaration| fs::canonicalize(&declaration.root))
        .collect::<std::io::Result<Vec<_>>>()?;
    canonical_roots.push(fs::canonicalize(root)?);
    for left in 0..canonical_roots.len() {
        for right in left + 1..canonical_roots.len() {
            ensure!(
                !paths_overlap(&canonical_roots[left], &canonical_roots[right]),
                "installed raw roots overlap each other or the release"
            );
        }
    }
    for binary in [
        "robin-highscores-admin",
        "robin-highscores-manifestctl",
        "robin-highscores-server",
        "robin-highscores-worker",
        "robin-replay-verifier",
    ] {
        validate_linux_elf(&root.join("bin").join(binary))?;
    }
    let backend: BackendPublicationV3 =
        load_canonical(&root.join("publication/backend-publication-v3.json"))?;
    for (role, relative) in [
        (VpsConfigRoleV2::Server, "config/highscores-server.toml"),
        (VpsConfigRoleV2::Worker, "config/highscores-worker.toml"),
        (VpsConfigRoleV2::ApiEnvironment, "config/api.env"),
        (VpsConfigRoleV2::WorkerEnvironment, "config/worker.env"),
    ] {
        validate_final_config(
            role,
            &root.join(relative),
            manifest.verifier_sha256,
            &backend.verifier_operator_config,
            &manifest.source_commit,
            &backend
                .campaign_states
                .iter()
                .map(|state| state.artifact.sha256)
                .collect(),
        )?;
    }
    validate_worker_raw_authority(
        &root.join("config/highscores-worker.toml"),
        &manifest.source_commit,
        &root.join("private/source-tree-manifests-v2"),
        &declarations.roots,
    )?;
    validate_root_once_sha256sums(root)?;
    validate_deploy_bootstrap_sha256sums(root)?;
    for (role, relative) in [
        (
            VpsHostFileRoleV2::UserTarget,
            "systemd/user/robin-highscores.target",
        ),
        (
            VpsHostFileRoleV2::ApiService,
            "systemd/user/robin-highscores-api.service",
        ),
        (
            VpsHostFileRoleV2::WorkerService,
            "systemd/user/robin-highscores-worker.service",
        ),
        (
            VpsHostFileRoleV2::BackupService,
            "systemd/user/robin-highscores-backup.service",
        ),
        (
            VpsHostFileRoleV2::BackupTimer,
            "systemd/user/robin-highscores-backup.timer",
        ),
        (
            VpsHostFileRoleV2::DeployReleaseScript,
            "deploy/deploy-release.sh",
        ),
        (
            VpsHostFileRoleV2::RollbackReleaseScript,
            "deploy/rollback-release.sh",
        ),
        (
            VpsHostFileRoleV2::ValidateReleaseScript,
            "deploy/validate-release-bundle.sh",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            "deploy/tests/real-runtime-fence-release-gate.sh",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            "deploy/tests/real-runtime-fence-e2e.py",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            "deploy/tests/real-runtime-fence-e2e-selftest.py",
        ),
        (VpsHostFileRoleV2::RootOnceScript, "deploy/root-once.sh"),
        (
            VpsHostFileRoleV2::NginxChallenge,
            "deploy/nginx-robinhood-api.challenge.conf",
        ),
        (
            VpsHostFileRoleV2::NginxCloudflareOnly,
            "deploy/nginx-robinhood-cloudflare-only.conf",
        ),
        (
            VpsHostFileRoleV2::NginxApiLocations,
            "deploy/nginx-robinhood-api.locations.conf",
        ),
        (
            VpsHostFileRoleV2::NginxVhost,
            "deploy/nginx-robinhood-api.vhost.conf",
        ),
        (VpsHostFileRoleV2::DeploymentReadme, "deploy/README.md"),
        (
            VpsHostFileRoleV2::OperatorRunbook,
            "deploy/VPS_RELEASE_INSTALL.md",
        ),
        (VpsHostFileRoleV2::BackupRunbook, "deploy/BACKUP_RESTORE.md"),
    ] {
        validate_final_host_file(role, &root.join(relative), &manifest.source_commit)?;
    }
    validate_backup_sandbox_contract(
        &root.join("config/highscores-server.toml"),
        &root.join("systemd/user/robin-highscores-api.service"),
        &root.join("systemd/user/robin-highscores-worker.service"),
        &root.join("systemd/user/robin-highscores-backup.service"),
        &root.join("systemd/user/robin-highscores-backup.timer"),
        &manifest.source_commit,
    )?;
    Ok(())
}

fn validate_embedded_publication(root: &Path, manifest: &VpsReleaseManifestV2) -> Result<()> {
    let publication_root = root.join("publication");
    let publication_manifest: PublicationManifestV3 =
        load_canonical(&publication_root.join("publication-manifest-v3.json"))?;
    ensure!(
        publication_manifest.canonical_digest()? == manifest.publication_manifest_sha256,
        "embedded publication manifest differs from release identity"
    );
    ensure!(
        fs::read(publication_root.join("publication-manifest-v3.sha256"))?
            == manifest.publication_manifest_sha256.to_string().as_bytes(),
        "embedded publication manifest sidecar mismatch"
    );
    let lock: PublicationLockV3 =
        load_canonical(&publication_root.join("publication-lock-v3.json"))?;
    ensure!(
        lock.publication_manifest_sha256 == manifest.publication_manifest_sha256
            && lock.canonical_digest()? == manifest.publication_lock_sha256,
        "embedded publication lock does not bind the release publication"
    );
    ensure!(
        fs::read(publication_root.join("publication-lock-v3.sha256"))?
            == manifest.publication_lock_sha256.to_string().as_bytes(),
        "embedded publication lock sidecar mismatch"
    );
    let locked_files = lock
        .files
        .iter()
        .map(|file| (file.path.as_str(), &file.artifact))
        .collect::<BTreeMap<_, _>>();
    for (source, bundled) in [
        (
            "backend/publication-v3.json",
            "publication/backend-publication-v3.json",
        ),
        (
            "publication-manifest-v3.json",
            "publication/publication-manifest-v3.json",
        ),
        (
            "publication-manifest-v3.sha256",
            "publication/publication-manifest-v3.sha256",
        ),
    ] {
        let expected = locked_files
            .get(source)
            .with_context(|| format!("publication lock omits {source}"))?;
        let actual = artifact_from_file(&root.join(bundled), "application/octet-stream")?;
        ensure!(
            actual.sha256 == expected.sha256 && actual.byte_length == expected.byte_length,
            "embedded publication evidence differs from lock at {source}"
        );
    }
    let backend: BackendPublicationV3 =
        load_canonical(&publication_root.join("backend-publication-v3.json"))?;
    ensure!(
        backend.build_manifest_sha256 == publication_manifest.build_manifest_sha256
            && backend.verifier_program.sha256 == manifest.verifier_sha256,
        "embedded backend publication is substituted"
    );
    let catalog_path = root
        .join("private/verifier/operator-config")
        .join(backend.verifier_operator_config.sha256.to_string());
    let _: VerifierJobConfigCatalogV1 = load_canonical(&catalog_path)?;
    ensure!(
        artifact_from_file(&catalog_path, &backend.verifier_operator_config.media_type)?
            == backend.verifier_operator_config,
        "bundled verifier job catalog differs from publication"
    );
    let build: BuildManifestV2 = load_canonical(
        &root
            .join("config/manifests/builds")
            .join(format!("{}.json", backend.build_manifest_sha256)),
    )?;
    ensure!(
        build.source_commit == manifest.source_commit,
        "bundle source commit differs from its BuildManifestV2"
    );
    validate_publication_subset(root, &lock, &backend, manifest)?;
    Ok(())
}

fn validate_publication_subset(
    root: &Path,
    lock: &PublicationLockV3,
    backend: &BackendPublicationV3,
    manifest: &VpsReleaseManifestV2,
) -> Result<()> {
    let source_files = lock
        .files
        .iter()
        .map(|file| (file.path.as_str(), &file.artifact))
        .collect::<BTreeMap<_, _>>();
    let mut expected = BTreeMap::<String, &ArtifactRefV1>::new();
    for (source, artifact) in &source_files {
        let destination = publication_file_to_vps_path(source);
        if let Some(destination) = destination {
            ensure!(
                expected.insert(destination, artifact).is_none(),
                "publication subset destination collision"
            );
        }
    }
    ensure!(
        expected
            .keys()
            .any(|path| path.starts_with("private/verifier-bundles/"))
            && expected
                .keys()
                .any(|path| path.starts_with("private/campaign-states/"))
            && expected
                .keys()
                .any(|path| path.starts_with("private/source-tree-manifests-v2/")),
        "publication lock omits verifier bundles, campaign templates, or source manifests"
    );
    for (path, artifact) in expected {
        let actual = artifact_from_file(&root.join(&path), "application/octet-stream")?;
        ensure!(
            actual.sha256 == artifact.sha256 && actual.byte_length == artifact.byte_length,
            "publication file omitted or substituted at {path}"
        );
    }
    let mut exact_paths = source_files
        .keys()
        .filter_map(|source| publication_file_to_vps_path(source))
        .collect::<BTreeSet<_>>();
    exact_paths.extend(
        [
            "bin/robin-highscores-admin",
            "bin/robin-highscores-manifestctl",
            "bin/robin-highscores-server",
            "bin/robin-highscores-worker",
            "bin/robin-replay-verifier",
            "config/highscores-server.toml",
            "config/highscores-worker.toml",
            "config/api.env",
            "config/worker.env",
            "systemd/user/robin-highscores.target",
            "systemd/user/robin-highscores-api.service",
            "systemd/user/robin-highscores-worker.service",
            "systemd/user/robin-highscores-backup.service",
            "systemd/user/robin-highscores-backup.timer",
            "deploy/deploy-release.sh",
            "deploy/rollback-release.sh",
            "deploy/validate-release-bundle.sh",
            DEPLOY_BOOTSTRAP_SHA256SUMS_FILE,
            ROOT_ONCE_SHA256SUMS_FILE,
            "deploy/root-once.sh",
            "deploy/nginx-robinhood-api.challenge.conf",
            "deploy/nginx-robinhood-cloudflare-only.conf",
            "deploy/nginx-robinhood-api.locations.conf",
            "deploy/nginx-robinhood-api.vhost.conf",
            "deploy/README.md",
            "deploy/VPS_RELEASE_INSTALL.md",
            "deploy/BACKUP_RESTORE.md",
            RAW_ROOT_DECLARATIONS_FILE,
            "publication/backend-publication-v3.json",
            "publication/publication-manifest-v3.json",
            "publication/publication-manifest-v3.sha256",
            "publication/publication-lock-v3.json",
            "publication/publication-lock-v3.sha256",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    extend_real_runtime_fence_payload_paths(&mut exact_paths);
    let actual_paths = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    ensure!(
        actual_paths == exact_paths,
        "VPS release contains an omitted or extra payload path"
    );
    let verifier_path = root.join("bin/robin-replay-verifier");
    let verifier = artifact_from_file(&verifier_path, &backend.verifier_program.media_type)?;
    ensure!(
        verifier == backend.verifier_program,
        "named verifier binary differs from publication"
    );
    let expected_campaigns = backend
        .campaign_states
        .iter()
        .map(|state| state.artifact.sha256.to_string())
        .collect::<BTreeSet<_>>();
    let actual_campaigns = fs::read_dir(root.join("private/campaign-states"))?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| std::io::Error::other("campaign filename is not UTF-8"))
        })
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    ensure!(
        actual_campaigns == expected_campaigns,
        "campaign template set differs from publication"
    );
    Ok(())
}

fn extend_real_runtime_fence_payload_paths(paths: &mut BTreeSet<String>) {
    paths.extend(
        [
            "deploy/tests/real-runtime-fence-release-gate.sh",
            "deploy/tests/real-runtime-fence-e2e.py",
            "deploy/tests/real-runtime-fence-e2e-selftest.py",
        ]
        .into_iter()
        .map(str::to_owned),
    );
}

fn publication_file_to_vps_path(source: &str) -> Option<String> {
    if let Some(relative) = source.strip_prefix("backend/manifests/") {
        Some(format!("config/manifests/{relative}"))
    } else if let Some(relative) =
        source.strip_prefix("private/official-content-authority/verifier-bundles/")
    {
        Some(format!("private/verifier-bundles/{relative}"))
    } else if let Some(relative) =
        source.strip_prefix("private/official-content-authority/private/source-tree-manifests-v2/")
    {
        Some(format!("private/source-tree-manifests-v2/{relative}"))
    } else if source.starts_with("private/campaign-states/")
        || source.starts_with("private/verifier/operator-config/")
    {
        Some(source.to_owned())
    } else {
        None
    }
}

fn load_canonical<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical document {}", path.display()))?;
    document.validate()?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "{} is not canonical JSON",
        path.display()
    );
    Ok(document)
}

fn validate_immutable_raw_root(root: &Path) -> Result<()> {
    validate_mount_root(root)?;
    reject_links_and_special_nodes(root, true)?;
    validate_single_owner_tree(root)?;
    for (_, file) in walk_regular_files(root)? {
        ensure!(
            actual_mode(&file)? == 0o440,
            "private raw root files must use mode 0440"
        );
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        ensure!(
            actual_mode(&directory)? == 0o550,
            "private raw root directories must use mode 0550"
        );
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

fn reject_links_and_special_nodes(root: &Path, reject_writable: bool) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let metadata = fs::symlink_metadata(&directory)?;
        ensure!(metadata.is_dir(), "tree contains a non-directory root");
        if reject_writable {
            ensure!(
                actual_mode(&directory)? & 0o222 == 0,
                "tree contains a mutable directory"
            );
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "tree contains forbidden symlink {}",
                entry.path().display()
            );
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                ensure!(metadata.is_file(), "tree contains a special node");
                reject_hardlink(&entry.path())?;
                if reject_writable {
                    ensure!(
                        actual_mode(&entry.path())? & 0o222 == 0,
                        "tree contains a mutable file"
                    );
                }
            }
        }
    }
    Ok(())
}

fn reject_hardlink(path: &Path) -> Result<()> {
    validate_regular_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        ensure!(
            fs::symlink_metadata(path)?.nlink() == 1,
            "hard-linked release input is forbidden: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn validate_single_owner_tree(root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let expected_uid = fs::symlink_metadata(root)?.uid();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.uid() == expected_uid,
            "immutable tree contains mixed ownership at {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_single_owner_tree(_root: &Path) -> Result<()> {
    anyhow::bail!("VPS releases require Unix ownership semantics")
}

#[cfg(unix)]
fn validate_single_device_tree(root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let expected_device = fs::symlink_metadata(root)?.dev();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.dev() == expected_device,
            "immutable tree crosses devices at {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_single_device_tree(_root: &Path) -> Result<()> {
    anyhow::bail!("VPS releases require Unix device identity semantics")
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

fn valid_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_release_directory_name(name: &str, source_commit: &str) -> bool {
    name == source_commit || candidate_release_directory_name(name, source_commit)
}

fn candidate_release_directory_name(name: &str, source_commit: &str) -> bool {
    name == format!("{source_commit}.partial")
}

fn forbidden_release_path(path: &str) -> bool {
    path.starts_with("polkit/")
        || path.starts_with("systemd/system/")
        || (path.starts_with("systemd/") && !path.starts_with("systemd/user/"))
        || path.contains("verifier-broker")
        || path.ends_with(".socket")
}

fn valid_relative_manifest_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && !value
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
}

#[cfg(unix)]
fn actual_mode(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn actual_mode(_path: &Path) -> Result<u32> {
    anyhow::bail!("VPS releases require Unix permission semantics")
}

fn canonical_user_deployment() -> VpsUserDeploymentV2 {
    VpsUserDeploymentV2 {
        user: DEPLOYMENT_USER.to_owned(),
        home: DEPLOYMENT_HOME.into(),
        install_root: INSTALL_ROOT.into(),
        persistent_state_root: STATE_ROOT.into(),
        current_link: format!("{INSTALL_ROOT}/current").into(),
    }
}

fn canonical_file_mode(path: &str) -> u32 {
    if matches!(
        path,
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
    ) {
        0o550
    } else {
        0o440
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(bytes: &[u8]) -> ArtifactRefV1 {
        ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: bytes.len() as u64,
            media_type: "application/octet-stream".into(),
        }
    }

    #[cfg(target_os = "linux")]
    struct SourceConsumeFixture {
        sandbox: tempfile::TempDir,
        incoming: PathBuf,
        releases: PathBuf,
        logical_source: PathBuf,
        candidate: PathBuf,
        plan: VpsReleasePlanV2,
        plan_bytes: Vec<u8>,
        plan_sha256: Digest32,
        release_manifest_sha256: Digest32,
        publication_lock_sha256: Digest32,
        retained_admin_writer: File,
    }

    #[cfg(target_os = "linux")]
    impl SourceConsumeFixture {
        fn consuming(&self) -> PathBuf {
            self.incoming
                .join(format!(".sources-{}.consuming", self.plan.source_commit))
        }

        fn journal(&self) -> PathBuf {
            self.incoming.join(format!(
                ".sources-{}.consume-v1.json",
                self.plan.source_commit
            ))
        }

        fn journal_temporary(&self) -> PathBuf {
            self.incoming.join(format!(
                ".sources-{}.consume-v1.json.new",
                self.plan.source_commit
            ))
        }

        fn terminal_journal(&self) -> PathBuf {
            self.incoming.join(format!(
                ".sources-{}.consume-v1.complete.json",
                self.plan.source_commit
            ))
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for SourceConsumeFixture {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;

            fn make_tree_removable(path: &Path) {
                let Ok(metadata) = fs::symlink_metadata(path) else {
                    return;
                };
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return;
                }
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                let Ok(entries) = fs::read_dir(path) else {
                    return;
                };
                for entry in entries.flatten() {
                    make_tree_removable(&entry.path());
                }
            }

            make_tree_removable(self.sandbox.path());
        }
    }

    #[cfg(target_os = "linux")]
    fn set_mode(path: &Path, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    struct RuntimeFenceFixture {
        sandbox: tempfile::TempDir,
        opt_root: PathBuf,
        state_root: PathBuf,
        source_commit: String,
        activation_lock: PinnedVpsActivationLockV2,
    }

    #[cfg(target_os = "linux")]
    impl RuntimeFenceFixture {
        fn new() -> Result<Self> {
            let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
            set_mode(sandbox.path(), 0o700)?;
            let opt_root = sandbox.path().join("opt");
            fs::create_dir(&opt_root)?;
            set_mode(&opt_root, 0o750)?;
            let state_root = sandbox.path().join("state");
            fs::create_dir(&state_root)?;
            set_mode(&state_root, 0o700)?;
            let activation_lock = acquire_vps_activation_lock_at(&opt_root)?;
            Ok(Self {
                sandbox,
                opt_root,
                state_root,
                source_commit: "a".repeat(40),
                activation_lock,
            })
        }

        fn staging(&self) -> PathBuf {
            self.state_root
                .join(format!(".runtime-fence-{}.partial", self.source_commit))
        }

        fn foreign_staging(&self) -> PathBuf {
            self.state_root
                .join(format!(".runtime-fence-{}.partial", "b".repeat(40)))
        }

        fn intent(&self) -> PathBuf {
            self.state_root.join(RUNTIME_FENCE_INTENT_NAME)
        }

        fn intent_temporary(&self) -> PathBuf {
            self.state_root.join(RUNTIME_FENCE_INTENT_TEMPORARY_NAME)
        }

        fn final_root(&self) -> PathBuf {
            self.state_root.join("runtime-fence")
        }

        fn run(&self) -> Result<()> {
            initialize_vps_runtime_fence_v1_at(
                &self.source_commit,
                &self.activation_lock,
                &self.state_root,
                |_| Ok(()),
            )
        }

        fn fail_after(&self, target: RuntimeFenceInitBoundaryV1) -> Result<()> {
            initialize_vps_runtime_fence_v1_at(
                &self.source_commit,
                &self.activation_lock,
                &self.state_root,
                |boundary| {
                    ensure!(boundary != target, "injected runtime-fence crash boundary");
                    Ok(())
                },
            )
        }

        fn assert_sealed(&self) -> Result<()> {
            use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

            ensure!(
                self.final_root().is_dir(),
                "runtime-fence was not published"
            );
            ensure!(
                fs::symlink_metadata(self.final_root())?
                    .permissions()
                    .mode()
                    & 0o777
                    == 0o500,
                "runtime-fence final mode is not 0500"
            );
            for name in ["db-admission.lock", "db-quiescence.lock"] {
                let metadata = fs::symlink_metadata(self.final_root().join(name))?;
                ensure!(
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.permissions().mode() & 0o777 == 0o400
                        && metadata.nlink() == 1
                        && metadata.len() == 0,
                    "runtime-fence final leaf is not exact"
                );
            }
            ensure!(
                !self.staging().exists()
                    && !self.intent().exists()
                    && !self.intent_temporary().exists(),
                "runtime-fence terminal initialization retained recovery evidence"
            );
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for RuntimeFenceFixture {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;

            fn make_removable(path: &Path) {
                let Ok(metadata) = fs::symlink_metadata(path) else {
                    return;
                };
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return;
                }
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        make_removable(&entry.path());
                    }
                }
            }

            make_removable(self.sandbox.path());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_cleanly_publishes_exact_permanent_inodes() -> Result<()> {
        let fixture = RuntimeFenceFixture::new()?;
        fixture.run()?;
        fixture.assert_sealed()?;
        fixture.run()?;
        fixture.assert_sealed()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_reconciles_every_authenticated_crash_boundary() -> Result<()> {
        for boundary in [
            RuntimeFenceInitBoundaryV1::IntentWritingCreated,
            RuntimeFenceInitBoundaryV1::IntentWritingSynced,
            RuntimeFenceInitBoundaryV1::IntentNewPublished,
            RuntimeFenceInitBoundaryV1::AuthorizedIntentPublished,
            RuntimeFenceInitBoundaryV1::StagingCreated,
            RuntimeFenceInitBoundaryV1::BoundIntentWritingCreated,
            RuntimeFenceInitBoundaryV1::BoundIntentWritingSynced,
            RuntimeFenceInitBoundaryV1::BoundIntentNewPublished,
            RuntimeFenceInitBoundaryV1::BoundIntentExchanged,
            RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished,
            RuntimeFenceInitBoundaryV1::AdmissionLeafSynced,
            RuntimeFenceInitBoundaryV1::QuiescenceLeafSynced,
            RuntimeFenceInitBoundaryV1::StagingSealed,
            RuntimeFenceInitBoundaryV1::FinalPublished,
            RuntimeFenceInitBoundaryV1::IntentRemoved,
        ] {
            let fixture = RuntimeFenceFixture::new()?;
            ensure!(
                fixture.fail_after(boundary).is_err(),
                "crash injection did not stop at {boundary:?}"
            );
            fixture.run()?;
            fixture.assert_sealed()?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_converges_after_real_sigkill_at_every_mutation_boundary()
    -> Result<()> {
        use std::os::unix::process::ExitStatusExt as _;
        use std::process::Command;

        const CHILD: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_CHILD";
        const OPT_ROOT: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_OPT";
        const STATE_ROOT_ENV: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_STATE";
        const COMMIT: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_COMMIT";
        const BOUNDARY: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_BOUNDARY";
        const BOUNDARIES: [RuntimeFenceInitBoundaryV1; 15] = [
            RuntimeFenceInitBoundaryV1::IntentWritingCreated,
            RuntimeFenceInitBoundaryV1::IntentWritingSynced,
            RuntimeFenceInitBoundaryV1::IntentNewPublished,
            RuntimeFenceInitBoundaryV1::AuthorizedIntentPublished,
            RuntimeFenceInitBoundaryV1::StagingCreated,
            RuntimeFenceInitBoundaryV1::BoundIntentWritingCreated,
            RuntimeFenceInitBoundaryV1::BoundIntentWritingSynced,
            RuntimeFenceInitBoundaryV1::BoundIntentNewPublished,
            RuntimeFenceInitBoundaryV1::BoundIntentExchanged,
            RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished,
            RuntimeFenceInitBoundaryV1::AdmissionLeafSynced,
            RuntimeFenceInitBoundaryV1::QuiescenceLeafSynced,
            RuntimeFenceInitBoundaryV1::StagingSealed,
            RuntimeFenceInitBoundaryV1::FinalPublished,
            RuntimeFenceInitBoundaryV1::IntentRemoved,
        ];

        if std::env::var_os(CHILD).is_some() {
            let opt_root = PathBuf::from(std::env::var_os(OPT_ROOT).context("missing opt root")?);
            let state_root =
                PathBuf::from(std::env::var_os(STATE_ROOT_ENV).context("missing state root")?);
            let source_commit = std::env::var(COMMIT)?;
            let boundary = BOUNDARIES[std::env::var(BOUNDARY)?.parse::<usize>()?];
            let activation_lock = acquire_vps_activation_lock_at(&opt_root)?;
            initialize_vps_runtime_fence_v1_at(
                &source_commit,
                &activation_lock,
                &state_root,
                |observed| {
                    if observed == boundary {
                        rustix::process::kill_process(
                            rustix::process::getpid(),
                            rustix::process::Signal::KILL,
                        )?;
                        anyhow::bail!("SIGKILL unexpectedly returned")
                    }
                    Ok(())
                },
            )?;
            anyhow::bail!("SIGKILL boundary was not reached")
        }

        for (index, boundary) in BOUNDARIES.into_iter().enumerate() {
            let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
            set_mode(sandbox.path(), 0o700)?;
            let opt_root = sandbox.path().join("opt");
            fs::create_dir(&opt_root)?;
            set_mode(&opt_root, 0o750)?;
            let state_root = sandbox.path().join("state");
            fs::create_dir(&state_root)?;
            set_mode(&state_root, 0o700)?;
            let source_commit = "a".repeat(40);
            let status = Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "vps_release_v2::tests::runtime_fence_initializer_converges_after_real_sigkill_at_every_mutation_boundary",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env(OPT_ROOT, &opt_root)
                .env(STATE_ROOT_ENV, &state_root)
                .env(COMMIT, &source_commit)
                .env(BOUNDARY, index.to_string())
                .status()?;
            ensure!(
                status.signal() == Some(9),
                "child was not SIGKILLed at {boundary:?}: {status}"
            );
            let staging_path = state_root.join(format!(".runtime-fence-{source_commit}.partial"));
            let retained_staging_identity =
                fs::symlink_metadata(&staging_path).ok().map(|metadata| {
                    use std::os::unix::fs::MetadataExt as _;
                    (metadata.dev(), metadata.ino())
                });
            let activation_lock = acquire_vps_activation_lock_at(&opt_root)?;
            let fixture = RuntimeFenceFixture {
                sandbox,
                opt_root,
                state_root,
                source_commit,
                activation_lock,
            };
            fixture.run()?;
            fixture.assert_sealed()?;
            if let Some(retained_staging_identity) = retained_staging_identity {
                use std::os::unix::fs::MetadataExt as _;

                let final_metadata = fs::symlink_metadata(fixture.final_root())?;
                ensure!(
                    (final_metadata.dev(), final_metadata.ino()) == retained_staging_identity,
                    "SIGKILL resume replaced the retained staging inode at {boundary:?}"
                );
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_final_plus_intent_adopts_only_the_bound_inode() -> Result<()> {
        use std::os::unix::fs::MetadataExt as _;

        let fixture = RuntimeFenceFixture::new()?;
        ensure!(
            fixture
                .fail_after(RuntimeFenceInitBoundaryV1::FinalPublished)
                .is_err(),
            "final publication crash injection unexpectedly completed"
        );
        let intent: RuntimeFenceInitIntentV1 =
            strict_json_from_slice(&fs::read(fixture.intent())?)?;
        let final_metadata = fs::symlink_metadata(fixture.final_root())?;
        ensure!(
            runtime_fence_bound_identity(&intent)? == (final_metadata.dev(), final_metadata.ino()),
            "final publication did not retain the journaled staging inode"
        );
        fixture.run()?;
        fixture.assert_sealed()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_rejects_unjournaled_and_foreign_staging() -> Result<()> {
        let fixture = RuntimeFenceFixture::new()?;
        fs::create_dir(fixture.staging())?;
        set_mode(&fixture.staging(), 0o700)?;
        ensure!(
            fixture.run().is_err(),
            "unjournaled runtime-fence staging was adopted"
        );

        let fixture = RuntimeFenceFixture::new()?;
        fs::create_dir(fixture.foreign_staging())?;
        set_mode(&fixture.foreign_staging(), 0o700)?;
        ensure!(
            fixture.run().is_err(),
            "foreign runtime-fence staging was ignored"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_rejects_symlink_hardlink_mode_device_and_extra_entries()
    -> Result<()> {
        use std::os::unix::fs::symlink;

        let fixture = RuntimeFenceFixture::new()?;
        ensure!(
            fixture
                .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
                .is_err(),
            "intent publication crash injection unexpectedly completed"
        );
        symlink("/dev/null", fixture.staging().join("db-admission.lock"))?;
        ensure!(fixture.run().is_err(), "runtime-fence symlink was accepted");

        let fixture = RuntimeFenceFixture::new()?;
        fixture
            .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
            .expect_err("intent publication crash injection unexpectedly completed");
        fs::write(fixture.staging().join("db-admission.lock"), b"")?;
        set_mode(&fixture.staging().join("db-admission.lock"), 0o400)?;
        fs::hard_link(
            fixture.staging().join("db-admission.lock"),
            fixture.state_root.join("second-link"),
        )?;
        ensure!(
            fixture.run().is_err(),
            "runtime-fence hard link was accepted"
        );

        let fixture = RuntimeFenceFixture::new()?;
        fixture
            .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
            .expect_err("intent publication crash injection unexpectedly completed");
        set_mode(&fixture.staging(), 0o755)?;
        ensure!(fixture.run().is_err(), "unsafe staging mode was repaired");

        let fixture = RuntimeFenceFixture::new()?;
        fixture
            .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
            .expect_err("intent publication crash injection unexpectedly completed");
        let mut intent: RuntimeFenceInitIntentV1 =
            strict_json_from_slice(&fs::read(fixture.intent())?)?;
        intent.staging_device = Some(
            intent
                .staging_device
                .context("fixture bound intent omitted device")?
                .checked_add(1)
                .context("fixture device overflow")?,
        );
        set_mode(&fixture.intent(), 0o600)?;
        fs::write(fixture.intent(), canonical_json_bytes(&intent)?)?;
        set_mode(&fixture.intent(), 0o400)?;
        ensure!(
            fixture.run().is_err(),
            "runtime-fence intent with a foreign device was accepted"
        );

        let fixture = RuntimeFenceFixture::new()?;
        fixture
            .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
            .expect_err("intent publication crash injection unexpectedly completed");
        fs::write(fixture.staging().join("unexpected"), b"")?;
        set_mode(&fixture.staging().join("unexpected"), 0o400)?;
        ensure!(fixture.run().is_err(), "extra staging entry was accepted");
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_never_repairs_a_missing_final_leaf() -> Result<()> {
        let fixture = RuntimeFenceFixture::new()?;
        fixture.run()?;
        set_mode(&fixture.final_root(), 0o700)?;
        fs::remove_file(fixture.final_root().join("db-quiescence.lock"))?;
        set_mode(&fixture.final_root(), 0o500)?;
        ensure!(
            fixture.run().is_err(),
            "initializer repaired an incomplete published runtime-fence"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_fence_initializer_rejects_activation_lock_substitution() -> Result<()> {
        use rustix::fs::{Mode, OFlags, open};
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = RuntimeFenceFixture::new()?;
        fs::rename(
            fixture.opt_root.join("activation.lock"),
            fixture.opt_root.join("activation.lock.displaced"),
        )?;
        let replacement = open(
            fixture.opt_root.join("activation.lock"),
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?;
        rustix::fs::fsync(&replacement)?;
        fs::set_permissions(
            fixture.opt_root.join("activation.lock"),
            fs::Permissions::from_mode(0o600),
        )?;
        ensure!(
            fixture.run().is_err(),
            "initializer accepted a substituted activation lock"
        );
        ensure!(
            fs::read_dir(&fixture.state_root)?.next().is_none(),
            "initializer mutated state after activation-lock substitution"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn write_source_file(path: &Path, bytes: &[u8], mode: u32) -> Result<File> {
        use std::io::Write as _;

        fs::create_dir_all(path.parent().context("source file has no parent")?)?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        set_mode(path, mode)?;
        Ok(file)
    }

    #[cfg(target_os = "linux")]
    fn source_consume_fixture() -> Result<SourceConsumeFixture> {
        let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
        set_mode(sandbox.path(), 0o750)?;
        let incoming = sandbox.path().join("incoming");
        fs::create_dir(&incoming)?;
        set_mode(&incoming, 0o750)?;
        let releases = sandbox.path().join("releases");
        fs::create_dir(&releases)?;
        set_mode(&releases, 0o750)?;
        let source_commit = "a".repeat(40);
        let logical_source = incoming.join(format!(".sources-{source_commit}"));
        fs::create_dir(&logical_source)?;
        set_mode(&logical_source, 0o700)?;

        let binary_roles = [
            VpsBinaryRoleV2::Admin,
            VpsBinaryRoleV2::ManifestTool,
            VpsBinaryRoleV2::Server,
            VpsBinaryRoleV2::Worker,
            VpsBinaryRoleV2::ReplayVerifier,
        ];
        let binaries = binary_roles
            .into_iter()
            .map(|role| {
                let bytes = role.output_name().as_bytes();
                VpsBinarySourceV2 {
                    role,
                    source: logical_source.join("bin").join(role.output_name()),
                    artifact: fact(bytes),
                }
            })
            .collect::<Vec<_>>();
        let config_roles = [
            VpsConfigRoleV2::Server,
            VpsConfigRoleV2::Worker,
            VpsConfigRoleV2::ApiEnvironment,
            VpsConfigRoleV2::WorkerEnvironment,
        ];
        let configs = config_roles
            .into_iter()
            .map(|role| {
                let bytes = role.output_name().as_bytes();
                VpsConfigSourceV2 {
                    role,
                    source: logical_source.join("config").join(role.output_name()),
                    artifact: fact(bytes),
                }
            })
            .collect::<Vec<_>>();
        let host_roles = [
            VpsHostFileRoleV2::UserTarget,
            VpsHostFileRoleV2::ApiService,
            VpsHostFileRoleV2::WorkerService,
            VpsHostFileRoleV2::BackupService,
            VpsHostFileRoleV2::BackupTimer,
            VpsHostFileRoleV2::DeployReleaseScript,
            VpsHostFileRoleV2::RollbackReleaseScript,
            VpsHostFileRoleV2::ValidateReleaseScript,
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            VpsHostFileRoleV2::RootOnceScript,
            VpsHostFileRoleV2::NginxChallenge,
            VpsHostFileRoleV2::NginxCloudflareOnly,
            VpsHostFileRoleV2::NginxApiLocations,
            VpsHostFileRoleV2::NginxVhost,
            VpsHostFileRoleV2::DeploymentReadme,
            VpsHostFileRoleV2::OperatorRunbook,
            VpsHostFileRoleV2::BackupRunbook,
        ];
        let host_files = host_roles
            .into_iter()
            .map(|role| {
                let bytes = role.output_path().as_bytes();
                VpsHostFileSourceV2 {
                    role,
                    source: logical_source.join("host").join(role.output_path()),
                    artifact: fact(bytes),
                }
            })
            .collect::<Vec<_>>();
        let plan = VpsReleasePlanV2 {
            schema_version: PLAN_SCHEMA_VERSION,
            source_commit: source_commit.clone(),
            publication_v3: logical_source.join("publication-v3"),
            binaries,
            configs,
            host_files,
            private_raw_roots: vec![
                PrivateRawRootV2 {
                    edition: OfficialContentEditionV1::Demo,
                    root: format!("{STATE_ROOT}/raw-content/demo").into(),
                },
                PrivateRawRootV2 {
                    edition: OfficialContentEditionV1::Full,
                    root: format!("{STATE_ROOT}/raw-content/full").into(),
                },
            ],
        };
        let plan_bytes = canonical_json_bytes(&plan)?;
        let plan_sha256 = Digest32::digest_bytes(&plan_bytes);

        for directory in [
            "bin",
            "config",
            "host",
            "host/systemd",
            "host/systemd/user",
            "host/deploy",
            "host/deploy/tests",
            "publication-v3",
        ] {
            fs::create_dir_all(logical_source.join(directory))?;
            set_mode(&logical_source.join(directory), 0o700)?;
        }
        let retained_admin_writer = plan
            .binaries
            .iter()
            .map(|binary| {
                write_source_file(&binary.source, binary.role.output_name().as_bytes(), 0o550)
                    .map(|file| (binary.role, file))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .find_map(|(role, file)| (role == VpsBinaryRoleV2::Admin).then_some(file))
            .context("fixture omitted retained admin writer")?;
        for config in &plan.configs {
            write_source_file(&config.source, config.role.output_name().as_bytes(), 0o440)?;
        }
        for host in &plan.host_files {
            write_source_file(
                &host.source,
                host.role.output_path().as_bytes(),
                canonical_file_mode(host.role.output_path()),
            )?;
        }
        write_source_file(
            &logical_source.join("vps-release-plan-v2.json"),
            &plan_bytes,
            0o400,
        )?;

        let candidate = releases.join(format!("{source_commit}.partial"));
        fs::create_dir(&candidate)?;
        fs::write(
            candidate.join(SOURCE_COMMIT_FILE),
            format!("{source_commit}\n"),
        )?;
        let publication_lock_sha256 = Digest32::digest_bytes(b"publication lock v3");
        let release_manifest = VpsReleaseManifestV2 {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source_commit: source_commit.clone(),
            database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
            deployment: canonical_user_deployment(),
            publication_lock_sha256,
            publication_manifest_sha256: Digest32::digest_bytes(b"publication manifest v3"),
            verifier_sha256: Digest32::digest_bytes(b"verifier"),
            files: vec![VpsReleaseFileV2 {
                path: "fixture-payload".into(),
                artifact: fact(b"fixture payload"),
                unix_mode: 0o440,
            }],
        };
        let release_manifest_bytes = canonical_json_bytes(&release_manifest)?;
        let release_manifest_sha256 = Digest32::digest_bytes(&release_manifest_bytes);
        fs::write(
            candidate.join(RELEASE_MANIFEST_FILE),
            release_manifest_bytes,
        )?;
        set_mode(&candidate.join(RELEASE_MANIFEST_FILE), 0o440)?;
        set_mode(&candidate, 0o550)?;

        Ok(SourceConsumeFixture {
            sandbox,
            incoming,
            releases,
            logical_source,
            candidate,
            plan,
            plan_bytes,
            plan_sha256,
            release_manifest_sha256,
            publication_lock_sha256,
            retained_admin_writer,
        })
    }

    #[cfg(target_os = "linux")]
    fn fixture_release_parent(fixture: &SourceConsumeFixture) -> Result<PathBuf> {
        ensure!(
            fixture.releases.is_dir(),
            "fixture releases parent disappeared"
        );
        Ok(fixture.releases.clone())
    }

    #[cfg(target_os = "linux")]
    fn consume_fixture_with<CV, PV, EL, AR, AU>(
        fixture: &SourceConsumeFixture,
        validate_candidate: CV,
        validate_publication: PV,
        ensure_lock: EL,
        after_rename: AR,
        after_root_unlink: AU,
    ) -> Result<()>
    where
        CV: Fn(&Path) -> Result<(Digest32, Digest32)>,
        PV: Fn(&PinnedVpsSourceRoot, &Path) -> Result<Digest32>,
        EL: Fn() -> Result<()>,
        AR: Fn(&Path) -> Result<()>,
        AU: Fn(&Path) -> Result<()>,
    {
        use std::os::fd::AsRawFd as _;

        let installed = fixture.releases.join(&fixture.plan.source_commit);
        let candidate_path = if fixture.candidate.is_dir() {
            &fixture.candidate
        } else {
            &installed
        };
        let candidate_fd = File::open(candidate_path)?;
        let candidate = pin_inherited_vps_candidate_root_at_with(
            &fixture.plan.source_commit,
            candidate_fd.as_raw_fd(),
            fixture.release_manifest_sha256,
            &fixture.incoming,
            &fixture.releases,
            synthetic_candidate_validator(
                fixture.release_manifest_sha256,
                fixture.plan.source_commit.clone(),
            ),
        )?;
        consume_vps_sources_in_with(
            &fixture.plan,
            &fixture.plan_bytes,
            fixture.plan_sha256,
            fixture.release_manifest_sha256,
            &fixture.incoming,
            &fixture.logical_source,
            &candidate,
            validate_candidate,
            validate_publication,
            ensure_lock,
            after_rename,
            after_root_unlink,
        )
    }

    #[cfg(target_os = "linux")]
    fn consume_fixture(fixture: &SourceConsumeFixture) -> Result<()> {
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        consume_fixture_with(
            fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        )
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_publication_is_opened_relative_to_retained_source_descriptor() -> Result<()> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::{AsRawFd as _, OwnedFd};
        use std::os::unix::fs::{MetadataExt as _, symlink};

        let sandbox = tempfile::tempdir()?;
        let logical_source = sandbox.path().join(".sources-descriptor-publication");
        let publication_path = logical_source.join("publication-v3");
        fs::create_dir(&logical_source)?;
        fs::create_dir(&publication_path)?;
        set_mode(&logical_source, 0o700)?;
        set_mode(&publication_path, 0o700)?;

        let parent_fd: OwnedFd = File::open(sandbox.path())?.into();
        let parent = rustix::fs::fstat(&parent_fd)?;
        let source = open_vps_source_root(
            &parent_fd,
            &parent,
            logical_source
                .file_name()
                .context("source fixture has no basename")?
                .to_owned(),
            0o700,
        )?;

        let old_procfd_path = PathBuf::from(format!(
            "/proc/self/fd/{}/./publication-v3",
            source.fd.as_raw_fd()
        ));
        let old_error = openat2(
            rustix::fs::CWD,
            &old_procfd_path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .expect_err("the obsolete procfd path unexpectedly crossed NO_MAGICLINKS");
        ensure!(
            old_error == rustix::io::Errno::LOOP,
            "obsolete procfd validation failed for an unexpected reason: {old_error}"
        );

        let observed_inode = with_pinned_vps_source_publication(
            &source,
            &logical_source,
            |logical_publication, publication| {
                ensure!(
                    logical_publication == publication_path,
                    "descriptor-relative validation changed the logical rebind path"
                );
                Ok(publication.metadata()?.ino())
            },
        )?;
        ensure!(
            observed_inode == fs::metadata(&publication_path)?.ino(),
            "descriptor-relative validation opened a different PublicationV3 inode"
        );

        let displaced = sandbox.path().join("displaced-source");
        let substituted = with_pinned_vps_source_publication(&source, &logical_source, |_, _| {
            fs::rename(&logical_source, &displaced)?;
            fs::create_dir(&logical_source)?;
            fs::create_dir(logical_source.join("publication-v3"))?;
            set_mode(&logical_source, 0o700)?;
            set_mode(&logical_source.join("publication-v3"), 0o700)?;
            Ok(())
        });
        assert!(
            substituted.is_err(),
            "descriptor-relative validation accepted coherent named-source substitution"
        );

        fs::remove_dir_all(&logical_source)?;
        fs::rename(&displaced, &logical_source)?;
        let authentic_publication = logical_source.join("authentic-publication");
        fs::rename(&publication_path, &authentic_publication)?;
        symlink("authentic-publication", &publication_path)?;
        assert!(
            with_pinned_vps_source_publication(&source, &logical_source, |_, _| Ok(())).is_err(),
            "descriptor-relative validation accepted a symlinked PublicationV3 child"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_activation_lock_requires_the_canonical_locked_open_file_description() -> Result<()>
    {
        use std::os::fd::AsRawFd as _;

        let unlocked_root = tempfile::tempdir()?;
        set_mode(unlocked_root.path(), 0o750)?;
        let unlocked_path = unlocked_root.path().join("activation.lock");
        let unlocked = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&unlocked_path)?;
        set_mode(&unlocked_path, 0o600)?;
        assert!(
            pin_inherited_vps_activation_lock_at(unlocked_root.path(), unlocked.as_raw_fd())
                .is_err(),
            "an unlocked canonical descriptor was accepted"
        );

        let locked_root = tempfile::tempdir()?;
        set_mode(locked_root.path(), 0o750)?;
        let held = acquire_vps_activation_lock_at(locked_root.path())?;
        let distinct_path = locked_root.path().join("distinct.lock");
        let distinct = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&distinct_path)?;
        set_mode(&distinct_path, 0o600)?;
        assert!(
            pin_inherited_vps_activation_lock_at(locked_root.path(), distinct.as_raw_fd()).is_err(),
            "a distinct locked-root file was accepted"
        );

        let reopened = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(locked_root.path().join("activation.lock"))?;
        assert!(
            pin_inherited_vps_activation_lock_at(locked_root.path(), reopened.as_raw_fd()).is_err(),
            "a reopened canonical inode with a distinct OFD inherited another OFD's lock"
        );
        let inherited = pin_inherited_vps_activation_lock_at(locked_root.path(), held.as_raw_fd())?;
        inherited.ensure_canonical()?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn synthetic_candidate_validator(
        expected_digest: Digest32,
        manifest_commit: String,
    ) -> impl Fn(&Path) -> Result<(Digest32, String)> {
        move |root| {
            ensure!(
                fs::read(root.join(SOURCE_COMMIT_FILE))?
                    == format!("{manifest_commit}\n").as_bytes(),
                "synthetic retained candidate source sentinel changed"
            );
            Ok((expected_digest, manifest_commit.clone()))
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_candidate_fd_accepts_partial_then_installed_repeated_resume() -> Result<()> {
        use std::os::fd::AsRawFd as _;

        let fixture = source_consume_fixture()?;
        let releases = fixture_release_parent(&fixture)?;
        let descriptor = File::open(&fixture.candidate)?;
        let digest = fixture.release_manifest_sha256;
        let commit = fixture.plan.source_commit.clone();

        let outer =
            pin_vps_activation_candidate_at(&fixture.candidate, &fixture.incoming, &releases)?;
        let outer_identity = rustix::fs::fstat(&outer)?;
        let descriptor_identity = rustix::fs::fstat(&descriptor)?;
        ensure!(
            outer_identity.st_dev == descriptor_identity.st_dev
                && outer_identity.st_ino == descriptor_identity.st_ino,
            "outer activation pin did not retain the candidate across a mounted install root"
        );

        let legacy_incoming_candidate = fixture.incoming.join(format!("{commit}.partial"));
        fs::create_dir(&legacy_incoming_candidate)?;
        fs::write(
            legacy_incoming_candidate.join(SOURCE_COMMIT_FILE),
            format!("{commit}\n"),
        )?;
        set_mode(&legacy_incoming_candidate, 0o550)?;
        assert!(
            pin_vps_activation_candidate_at(
                &legacy_incoming_candidate,
                &fixture.incoming,
                &releases,
            )
            .is_err(),
            "the legacy incoming candidate topology was accepted"
        );

        let partial = pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )
        .context("pin partial candidate through retained parent authorities")?;
        ensure!(
            partial.canonical_path()? == fixture.candidate,
            "partial candidate pin selected another pathname"
        );
        partial
            .ensure_canonical()
            .context("revalidate partial candidate")?;
        drop(partial);

        let installed = releases.join(&commit);
        fs::rename(&fixture.candidate, &installed)
            .context("rename partial candidate to installed test root")?;
        for _ in 0..2 {
            let resumed = pin_inherited_vps_candidate_root_at_with(
                &commit,
                descriptor.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )
            .context("resume candidate pin from installed test root")?;
            ensure!(
                resumed.canonical_path()? == installed,
                "installed resume selected another pathname"
            );
            resumed
                .ensure_canonical()
                .context("revalidate resumed installed candidate")?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_candidate_fd_rejects_both_neither_and_path_substitution() -> Result<()> {
        use std::os::fd::AsRawFd as _;

        let fixture = source_consume_fixture()?;
        let releases = fixture_release_parent(&fixture)?;
        let descriptor = File::open(&fixture.candidate)?;
        let digest = fixture.release_manifest_sha256;
        let commit = fixture.plan.source_commit.clone();
        let displaced = fixture.releases.join("displaced-candidate");
        fs::rename(&fixture.candidate, &displaced)?;
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                descriptor.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )
            .is_err(),
            "a retained candidate with neither canonical name was accepted"
        );

        let fixture = source_consume_fixture()?;
        let releases = fixture_release_parent(&fixture)?;
        let descriptor = File::open(&fixture.candidate)?;
        let digest = fixture.release_manifest_sha256;
        let commit = fixture.plan.source_commit.clone();
        let replacement_commit = commit.clone();
        let displaced = fixture.releases.join("displaced-candidate");
        let candidate = fixture.candidate.clone();
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                descriptor.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
                move |_| {
                    fs::rename(&candidate, &displaced)?;
                    fs::create_dir(&candidate)?;
                    set_mode(&candidate, 0o550)?;
                    Ok((digest, replacement_commit.clone()))
                },
            )
            .is_err(),
            "a retained candidate accepted a substituted canonical pathname"
        );

        let fixture = source_consume_fixture()?;
        let releases = fixture_release_parent(&fixture)?;
        let descriptor = File::open(&fixture.candidate)?;
        let digest = fixture.release_manifest_sha256;
        let commit = fixture.plan.source_commit.clone();
        let retained = pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )?;
        let displaced_parent = fixture.sandbox.path().join("displaced-incoming-parent");
        fs::rename(&fixture.incoming, &displaced_parent)?;
        fs::create_dir(&fixture.incoming)?;
        set_mode(&fixture.incoming, 0o750)?;
        assert!(
            retained.ensure_canonical().is_err(),
            "a retained candidate accepted replacement of its uploader-source parent"
        );

        let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
        set_mode(sandbox.path(), 0o750)?;
        let incoming = sandbox.path().join("incoming");
        fs::create_dir(&incoming)?;
        set_mode(&incoming, 0o750)?;
        let both_commit = ".";
        let candidate = incoming.join("..partial");
        fs::create_dir(&candidate)?;
        fs::write(candidate.join(SOURCE_COMMIT_FILE), b".\n")?;
        set_mode(&candidate, 0o550)?;
        let descriptor = File::open(&candidate)?;
        let digest = Digest32::digest_bytes(b"both names");
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                both_commit,
                descriptor.as_raw_fd(),
                digest,
                &incoming,
                &candidate,
                synthetic_candidate_validator(digest, both_commit.to_owned()),
            )
            .is_err(),
            "one retained inode reachable through both canonical slots was accepted"
        );
        set_mode(&candidate, 0o700)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_candidate_fd_rejects_closed_reused_wrong_and_identity_mismatch() -> Result<()> {
        use std::os::fd::AsRawFd as _;

        let fixture = source_consume_fixture()?;
        let releases = fixture_release_parent(&fixture)?;
        let digest = fixture.release_manifest_sha256;
        let commit = fixture.plan.source_commit.clone();

        let closed = File::open(&fixture.candidate)?;
        let closed_fd = closed.as_raw_fd();
        drop(closed);
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                closed_fd,
                digest,
                &fixture.incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )
            .is_err(),
            "a closed candidate descriptor was accepted"
        );

        let wrong_path = fixture.sandbox.path().join("wrong-regular-file");
        fs::write(&wrong_path, b"wrong")?;
        let wrong = File::open(&wrong_path)?;
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                wrong.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )
            .is_err(),
            "a regular file candidate descriptor was accepted"
        );
        let reusable_candidate = File::open(&fixture.candidate)?;
        let reused_fd = nix_legacy::fcntl::fcntl(
            reusable_candidate.as_raw_fd(),
            nix_legacy::fcntl::FcntlArg::F_DUPFD_CLOEXEC(512),
        )?;
        let reused = NixOwnedFdV2(reused_fd);
        nix_legacy::unistd::dup2(wrong.as_raw_fd(), reused.0)?;
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                reused.0,
                digest,
                &fixture.incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )
            .is_err(),
            "a candidate descriptor number reused for another inode was accepted"
        );

        let candidate = File::open(&fixture.candidate)?;
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                candidate.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
                synthetic_candidate_validator(
                    Digest32::digest_bytes(b"different manifest"),
                    commit.clone(),
                ),
            )
            .is_err(),
            "candidate manifest digest mismatch was accepted"
        );
        assert!(
            pin_inherited_vps_candidate_root_at_with(
                &commit,
                candidate.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
                |_| Ok((digest, "b".repeat(40))),
            )
            .is_err(),
            "candidate source commit mismatch was accepted"
        );
        assert!(
            pin_inherited_vps_candidate_root_at(
                &commit,
                candidate.as_raw_fd(),
                digest,
                &fixture.incoming,
                &releases,
            )
            .is_err(),
            "production candidate wrapper bypassed the full V2 release validator"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_candidate_descriptor_survives_actual_exec() -> Result<()> {
        use std::os::fd::AsRawFd as _;
        use std::process::Command;

        const CHILD: &str = "ROBIN_TEST_INHERITED_CANDIDATE_CHILD";
        const FD: &str = "ROBIN_TEST_INHERITED_CANDIDATE_FD";
        const INCOMING: &str = "ROBIN_TEST_INHERITED_CANDIDATE_INCOMING";
        const RELEASES: &str = "ROBIN_TEST_INHERITED_CANDIDATE_RELEASES";
        const COMMIT: &str = "ROBIN_TEST_INHERITED_CANDIDATE_COMMIT";
        const DIGEST: &str = "ROBIN_TEST_INHERITED_CANDIDATE_DIGEST";

        if std::env::var_os(CHILD).is_some() {
            use rustix::fs::{RenameFlags, renameat_with};
            use std::os::fd::AsFd as _;

            let fd = std::env::var(FD)?.parse::<std::os::fd::RawFd>()?;
            let incoming = PathBuf::from(std::env::var_os(INCOMING).context("missing incoming")?);
            let releases = PathBuf::from(std::env::var_os(RELEASES).context("missing releases")?);
            let commit = std::env::var(COMMIT)?;
            let digest = std::env::var(DIGEST)?.parse::<Digest32>()?;
            let candidate = pin_inherited_vps_candidate_root_at_with(
                &commit,
                fd,
                digest,
                &incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )?;
            candidate.ensure_canonical()?;
            let partial_name = candidate
                .partial_path
                .file_name()
                .context("child partial candidate has no basename")?;
            let installed_name = candidate
                .installed_path
                .file_name()
                .context("child installed candidate has no basename")?;
            renameat_with(
                candidate.parents.installed_parent_fd.as_fd(),
                partial_name,
                candidate.parents.installed_parent_fd.as_fd(),
                installed_name,
                RenameFlags::NOREPLACE,
            )?;
            rustix::fs::fsync(&candidate.parents.installed_parent_fd)?;
            candidate.ensure_canonical()?;
            ensure!(
                candidate.canonical_path()? == candidate.installed_path,
                "exec child did not retain the exact inode through promotion"
            );
            let resumed = pin_inherited_vps_candidate_root_at_with(
                &commit,
                fd,
                digest,
                &incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )?;
            resumed.ensure_canonical()?;
            return Ok(());
        }

        let fixture = source_consume_fixture()?;
        let releases = fixture_release_parent(&fixture)?;
        let candidate = File::open(&fixture.candidate)?;
        let fd = candidate.as_raw_fd();
        let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
        let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
        flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
        nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
        let status = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "vps_release_v2::tests::inherited_candidate_descriptor_survives_actual_exec",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(FD, fd.to_string())
            .env(INCOMING, &fixture.incoming)
            .env(RELEASES, &releases)
            .env(COMMIT, &fixture.plan.source_commit)
            .env(DIGEST, fixture.release_manifest_sha256.to_string())
            .status()?;
        ensure!(
            status.success(),
            "exec child rejected inherited candidate FD"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn candidate_parent_pinning_crosses_private_home_bind_mount() -> Result<()> {
        use std::os::fd::AsRawFd as _;
        use std::process::Command;

        const CHILD: &str = "ROBIN_TEST_MOUNTED_CANDIDATE_CHILD";
        const COMMIT: &str = "ROBIN_TEST_MOUNTED_CANDIDATE_COMMIT";
        const DIGEST: &str = "ROBIN_TEST_MOUNTED_CANDIDATE_DIGEST";

        if std::env::var_os(CHILD).is_some() {
            let commit = std::env::var(COMMIT)?;
            let digest = std::env::var(DIGEST)?.parse::<Digest32>()?;
            let install_root = Path::new(INSTALL_ROOT);
            let incoming = install_root.join("incoming");
            let releases = install_root.join("releases");
            let partial = releases.join(format!("{commit}.partial"));
            let installed = releases.join(&commit);
            let descriptor = File::open(&partial)?;

            let outer = pin_vps_activation_candidate_at(&partial, &incoming, &releases)?;
            let outer_identity = rustix::fs::fstat(&outer)?;
            let inherited_identity = rustix::fs::fstat(&descriptor)?;
            ensure!(
                outer_identity.st_dev == inherited_identity.st_dev
                    && outer_identity.st_ino == inherited_identity.st_ino,
                "mounted outer candidate pin retained another inode"
            );

            let candidate = pin_inherited_vps_candidate_root_at_with(
                &commit,
                descriptor.as_raw_fd(),
                digest,
                &incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )?;
            candidate.ensure_canonical()?;
            fs::rename(&partial, &installed)?;
            candidate.ensure_canonical()?;
            let resumed = pin_inherited_vps_candidate_root_at_with(
                &commit,
                descriptor.as_raw_fd(),
                digest,
                &incoming,
                &releases,
                synthetic_candidate_validator(digest, commit.clone()),
            )?;
            resumed.ensure_canonical()?;

            let displaced = install_root.join("releases-displaced");
            fs::rename(&releases, &displaced)?;
            fs::create_dir(&releases)?;
            set_mode(&releases, 0o750)?;
            ensure!(
                resumed.ensure_canonical().is_err(),
                "mounted candidate accepted replacement of its release parent"
            );
            return Ok(());
        }

        let fixture = source_consume_fixture()?;
        let test_executable = File::open(std::env::current_exe()?)?;
        let install_root = File::open(fixture.sandbox.path())?;
        for fd in [test_executable.as_raw_fd(), install_root.as_raw_fd()] {
            let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
            let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
            flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
            nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
        }
        let mut command = Command::new("/usr/bin/bwrap");
        command
            .args(["--ro-bind", "/", "/"])
            .args(["--proc", "/proc"])
            .args(["--dev", "/dev"])
            .args(["--unshare-all", "--share-net"])
            .args(["--tmpfs", "/tmp"])
            .args([
                "--ro-bind-fd",
                &test_executable.as_raw_fd().to_string(),
                "/tmp/robin-manifest-tool-test",
            ])
            .args(["--tmpfs", "/home"])
            .args(["--dir", "/home/robinhood"])
            .args(["--dir", "/home/robinhood/.local"])
            .args(["--dir", "/home/robinhood/.local/opt"])
            .arg("--bind-fd")
            .arg(install_root.as_raw_fd().to_string())
            .arg(INSTALL_ROOT)
            .arg("/tmp/robin-manifest-tool-test")
            .args([
                "--exact",
                "vps_release_v2::tests::candidate_parent_pinning_crosses_private_home_bind_mount",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(COMMIT, &fixture.plan.source_commit)
            .env(DIGEST, fixture.release_manifest_sha256.to_string());
        ensure!(
            command.status()?.success(),
            "candidate pin failed across a private /home install-root bind mount"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_consume_binds_candidate_v2_publication_v3_and_exact_plan() -> Result<()> {
        let fixture = source_consume_fixture()?;
        let wrong_release = Digest32::digest_bytes(b"wrong candidate");
        let publication = fixture.publication_lock_sha256;
        assert!(
            consume_fixture_with(
                &fixture,
                move |_| Ok((wrong_release, publication)),
                move |_, _| Ok(publication),
                || Ok(()),
                |_| Ok(()),
                |_| Ok(()),
            )
            .is_err(),
            "source consumption accepted a different V2 candidate"
        );

        let fixture = source_consume_fixture()?;
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        let substituted_publication = Digest32::digest_bytes(b"substituted publication v3");
        assert!(
            consume_fixture_with(
                &fixture,
                move |_| Ok((release, publication)),
                move |_, _| Ok(substituted_publication),
                || Ok(()),
                |_| Ok(()),
                |_| Ok(()),
            )
            .is_err(),
            "source consumption accepted a PublicationV3 lock different from the candidate"
        );

        let mut fixture = source_consume_fixture()?;
        fixture.plan.publication_v3 = fixture.logical_source.join("other-publication-v3");
        assert!(
            consume_fixture(&fixture).is_err(),
            "source consumption accepted a plan whose publication escaped the exact closure"
        );

        let fixture = source_consume_fixture()?;
        fs::write(
            fixture.candidate.join(SOURCE_COMMIT_FILE),
            format!("{}\n", "b".repeat(40)),
        )?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "source consumption accepted a candidate for another source commit"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_consume_recovers_prepared_rename_and_root_unlink_crashes() -> Result<()> {
        let fixture = source_consume_fixture()?;
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        assert!(
            consume_fixture_with(
                &fixture,
                move |_| Ok((release, publication)),
                move |_, _| Ok(publication),
                || Ok(()),
                |_| anyhow::bail!("injected crash after source rename"),
                |_| Ok(()),
            )
            .is_err()
        );
        ensure!(
            fixture.consuming().is_dir(),
            "rename crash lost consuming root"
        );
        ensure!(
            fixture.journal().is_file(),
            "rename crash lost prepared journal"
        );
        fs::rename(fixture.journal(), fixture.journal_temporary())?;
        consume_fixture(&fixture)?;
        ensure!(
            !fixture.logical_source.exists()
                && !fixture.consuming().exists()
                && !fixture.journal().exists()
                && !fixture.journal_temporary().exists(),
            "prepared-journal retry left source transaction evidence"
        );

        let fixture = source_consume_fixture()?;
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        assert!(
            consume_fixture_with(
                &fixture,
                move |_| Ok((release, publication)),
                move |_, _| Ok(publication),
                || Ok(()),
                |_| Ok(()),
                |_| anyhow::bail!("injected crash after source root unlink"),
            )
            .is_err()
        );
        ensure!(
            !fixture.logical_source.exists() && !fixture.consuming().exists(),
            "root-unlink crash restored a source root"
        );
        ensure!(
            fixture.journal().is_file(),
            "root-unlink crash lost prepared journal"
        );
        consume_fixture(&fixture)?;
        ensure!(
            !fixture.journal().exists() && !fixture.terminal_journal().exists(),
            "root-unlink recovery left source journals"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_consume_resume_uses_the_retained_installed_candidate() -> Result<()> {
        use std::os::fd::AsRawFd as _;

        let fixture = source_consume_fixture()?;
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        assert!(
            consume_fixture_with(
                &fixture,
                move |_| Ok((release, publication)),
                move |_, _| Ok(publication),
                || Ok(()),
                |_| anyhow::bail!("injected crash after source rename"),
                |_| Ok(()),
            )
            .is_err()
        );

        let retained_fd = File::open(&fixture.candidate)?;
        let releases = fixture_release_parent(&fixture)?;
        let installed = releases.join(&fixture.plan.source_commit);
        fs::rename(&fixture.candidate, &installed)
            .context("rename fixture candidate into installed release root")?;
        let retained_candidate = pin_inherited_vps_candidate_root_at_with(
            &fixture.plan.source_commit,
            retained_fd.as_raw_fd(),
            fixture.release_manifest_sha256,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(
                fixture.release_manifest_sha256,
                fixture.plan.source_commit.clone(),
            ),
        )?;
        retained_candidate
            .ensure_canonical()
            .context("revalidate retained installed fixture candidate")?;

        consume_vps_sources_in_with(
            &fixture.plan,
            &fixture.plan_bytes,
            fixture.plan_sha256,
            fixture.release_manifest_sha256,
            &fixture.incoming,
            &fixture.logical_source,
            &retained_candidate,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        )
        .context("resume source consumption through retained installed candidate")?;
        ensure!(
            !fixture.consuming().exists()
                && !fixture.journal().exists()
                && retained_candidate.canonical_path()?.is_dir(),
            "resume did not consume only the source while retaining the installed candidate"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_consume_rejects_root_substitution_and_unsafe_topologies() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let fixture = source_consume_fixture()?;
        let displaced = fixture.incoming.join("displaced-authentic-source");
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        assert!(
            consume_fixture_with(
                &fixture,
                move |_| Ok((release, publication)),
                move |_, _| Ok(publication),
                || Ok(()),
                |consuming| {
                    fs::rename(consuming, &displaced)?;
                    fs::create_dir(consuming)?;
                    fs::set_permissions(consuming, fs::Permissions::from_mode(0o700))?;
                    Ok(())
                },
                |_| Ok(()),
            )
            .is_err(),
            "a substituted consuming-root basename was accepted"
        );

        let fixture = source_consume_fixture()?;
        fs::write(fixture.logical_source.join("extra"), b"extra")?;
        set_mode(&fixture.logical_source.join("extra"), 0o440)?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "an extra source file was accepted"
        );

        let fixture = source_consume_fixture()?;
        fs::remove_file(&fixture.plan.configs[0].source)?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "a missing source file was accepted"
        );

        let fixture = source_consume_fixture()?;
        set_mode(&fixture.plan.configs[0].source, 0o600)?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "a wrong source mode was accepted"
        );

        let fixture = source_consume_fixture()?;
        fs::hard_link(
            &fixture.plan.configs[0].source,
            fixture.logical_source.join("hardlink"),
        )?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "a hard-linked source was accepted"
        );

        let fixture = source_consume_fixture()?;
        symlink(
            &fixture.plan.configs[0].source,
            fixture.logical_source.join("symlink"),
        )?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "a symlinked source was accepted"
        );

        let fixture = source_consume_fixture()?;
        nix_legacy::unistd::mkfifo(
            &fixture.logical_source.join("special.fifo"),
            nix_legacy::sys::stat::Mode::S_IRUSR,
        )?;
        assert!(
            consume_fixture(&fixture).is_err(),
            "a special source node was accepted"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_inventory_rejects_mount_owner_depth_and_count_boundaries() -> Result<()> {
        use std::os::fd::OwnedFd;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        assert!(
            reject_mounts_at_or_below(Path::new("/proc")).is_err(),
            "a source root that is itself a mount was accepted"
        );

        let root = tempfile::tempdir()?;
        fs::write(root.path().join("entry"), b"entry")?;
        let metadata = fs::metadata(root.path())?;
        let directory: OwnedFd = File::open(root.path())?.into();
        let mut entries = Vec::new();
        let mut seen = 1;
        assert!(
            inventory_pinned_vps_source_directory(
                &directory,
                Path::new(""),
                metadata.dev(),
                MAX_FAILED_VPS_STAGING_DEPTH + 1,
                &mut seen,
                &mut entries,
            )
            .is_err(),
            "source inventory exceeded its depth boundary"
        );
        let mut seen = MAX_FAILED_VPS_STAGING_ENTRIES;
        assert!(
            inventory_pinned_vps_source_directory(
                &directory,
                Path::new(""),
                metadata.dev(),
                0,
                &mut seen,
                &mut entries,
            )
            .is_err(),
            "source inventory exceeded its entry boundary"
        );

        let expected_entries = vec![VpsSourceConsumeEntryV1 {
            path: ".".to_owned(),
            kind: VpsSourceEntryKindV1::Directory,
            unix_mode: metadata.permissions().mode() & 0o777,
            sha256: None,
            byte_length: None,
        }];
        let mut seen = 1;
        assert!(
            clear_pinned_vps_directory(
                &directory,
                Path::new(""),
                rustix::process::geteuid().as_raw().wrapping_add(1),
                metadata.dev(),
                0,
                &mut seen,
                &expected_entries,
                &|| Ok(()),
            )
            .is_err(),
            "source cleanup accepted a root owned by a different authority"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn release_validation_allows_exact_root_bind_but_rejects_descendant_mount() -> Result<()> {
        use std::os::fd::AsRawFd as _;
        use std::process::Command;

        const CHILD: &str = "ROBIN_VPS_RELEASE_ROOT_MOUNT_CHILD";
        const NESTED: &str = "ROBIN_VPS_RELEASE_NESTED_MOUNT";
        const CANDIDATE: &str = "/tmp/robin-vps-release-candidate";

        if std::env::var_os(CHILD).is_some() {
            let result = reject_mounts_strictly_below(Path::new(CANDIDATE));
            if std::env::var_os(NESTED).is_some() {
                ensure!(
                    result.is_err(),
                    "release validation accepted a strict descendant mount"
                );
            } else {
                result.context("release validation rejected its exact authenticated root mount")?;
            }
            return Ok(());
        }

        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("nested"))?;
        let nested_source = tempfile::tempdir()?;
        let executable = File::open(std::env::current_exe()?)?;
        let candidate = File::open(root.path())?;
        let nested = File::open(nested_source.path())?;
        for fd in [
            executable.as_raw_fd(),
            candidate.as_raw_fd(),
            nested.as_raw_fd(),
        ] {
            let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
            let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
            flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
            nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
        }

        for nested_mount in [false, true] {
            let mut command = Command::new("/usr/bin/bwrap");
            command
                .args(["--ro-bind", "/", "/"])
                .args(["--proc", "/proc"])
                .args(["--dev", "/dev"])
                .args(["--unshare-all", "--share-net"])
                .args(["--tmpfs", "/tmp"])
                .args([
                    "--ro-bind-fd",
                    &executable.as_raw_fd().to_string(),
                    "/tmp/robin-manifest-tool-test",
                ])
                .args(["--dir", CANDIDATE])
                .args([
                    "--ro-bind-fd",
                    &candidate.as_raw_fd().to_string(),
                    CANDIDATE,
                ]);
            if nested_mount {
                command.args([
                    "--ro-bind-fd",
                    &nested.as_raw_fd().to_string(),
                    &format!("{CANDIDATE}/nested"),
                ]);
            }
            command
                .arg("/tmp/robin-manifest-tool-test")
                .args([
                    "--exact",
                    "vps_release_v2::tests::release_validation_allows_exact_root_bind_but_rejects_descendant_mount",
                    "--nocapture",
                ])
                .env(CHILD, "1");
            if nested_mount {
                command.env(NESTED, "1");
            }
            ensure!(
                command.status()?.success(),
                "real bwrap release-root mount policy regression failed"
            );
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_cleanup_rejects_delayed_equal_length_byte_rewrite() -> Result<()> {
        use std::cell::Cell;
        use std::os::unix::fs::FileExt as _;

        let fixture = source_consume_fixture()?;
        let release = fixture.release_manifest_sha256;
        let publication = fixture.publication_lock_sha256;
        let authority_checks = Cell::new(0_usize);
        let rewrote = Cell::new(false);
        let original_length = VpsBinaryRoleV2::Admin.output_name().len();
        let replacement = vec![b'x'; original_length];
        let result = consume_fixture_with(
            &fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || {
                let next = authority_checks.get() + 1;
                authority_checks.set(next);
                // With the canonical fixture, check 10 is the guarded boundary
                // between the first and second hashes of the first binary.
                if next == 10 {
                    fixture
                        .retained_admin_writer
                        .write_all_at(&replacement, 0)?;
                    fixture.retained_admin_writer.sync_data()?;
                    rewrote.set(true);
                }
                Ok(())
            },
            |_| Ok(()),
            |_| Ok(()),
        );
        ensure!(
            rewrote.get(),
            "test did not reach the delayed rewrite boundary"
        );
        assert!(
            result.is_err(),
            "source cleanup accepted an equal-length rewrite between its two hashes"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_subset_copy_rejects_coherent_root_substitution_after_validation() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let publication_path = sandbox.path().join("publication");
        fs::create_dir_all(publication_path.join("backend/manifests/builds"))?;
        fs::write(
            publication_path.join("backend/manifests/builds/reviewed.json"),
            b"reviewed",
        )?;
        let mut authority = ValidatedPublicationV3::synthetic_for_consumer_test(&publication_path)?;
        let authentic = sandbox.path().join("authentic-publication");
        fs::rename(&publication_path, &authentic)?;
        fs::create_dir_all(publication_path.join("backend/manifests/builds"))?;
        fs::write(
            publication_path.join("backend/manifests/builds/reviewed.json"),
            b"reviewed",
        )?;
        let destination = sandbox.path().join("bundle-manifests");

        ensure!(
            copy_publication_tree_exact(&mut authority, "backend/manifests", &destination,)
                .is_err(),
            "VPS subset materialization accepted a coherent post-validation root replacement"
        );
        ensure!(
            fs::read(destination.join("builds/reviewed.json"))? == b"reviewed",
            "test did not exercise copying from the retained reviewed file descriptor"
        );
        fs::remove_dir_all(authentic)?;
        fs::remove_dir_all(publication_path)?;
        Ok(())
    }

    #[test]
    fn source_commits_and_manifest_paths_are_strict() {
        assert!(valid_source_commit(&"a".repeat(40)));
        assert!(!valid_source_commit(&"A".repeat(40)));
        assert!(!valid_source_commit(&"a".repeat(39)));
        for bad in ["", "/etc/passwd", "a/../b", "a//b", "a\\b"] {
            assert!(!valid_relative_manifest_path(bad), "accepted {bad:?}");
        }
        assert!(valid_relative_manifest_path("private/verifier/catalog"));
        let commit = "a".repeat(40);
        assert!(valid_release_directory_name(&commit, &commit));
        assert!(valid_release_directory_name(
            &format!("{commit}.partial"),
            &commit
        ));
        assert!(!valid_release_directory_name(
            &format!("{commit}.new"),
            &commit
        ));
        assert!(candidate_release_directory_name(
            &format!("{commit}.partial"),
            &commit
        ));
        assert!(!candidate_release_directory_name(&commit, &commit));
        assert!(!candidate_release_directory_name(
            &format!("{commit}.partial.extra"),
            &commit
        ));
    }

    #[test]
    fn publication_subset_maps_only_nested_authority_operational_inputs() {
        assert_eq!(
            publication_file_to_vps_path(
                "private/official-content-authority/verifier-bundles/abc/catalog/profile.json"
            )
            .as_deref(),
            Some("private/verifier-bundles/abc/catalog/profile.json")
        );
        assert_eq!(
            publication_file_to_vps_path(
                "private/official-content-authority/private/source-tree-manifests-v2/abc.json"
            )
            .as_deref(),
            Some("private/source-tree-manifests-v2/abc.json")
        );
        assert_eq!(
            publication_file_to_vps_path("backend/manifests/builds/abc.json").as_deref(),
            Some("config/manifests/builds/abc.json")
        );
        assert!(
            publication_file_to_vps_path(
                "private/official-content-authority/private/projection-receipts-v2/abc.json"
            )
            .is_none()
        );
        assert!(publication_file_to_vps_path("private/verifier-bundles/legacy").is_none());
    }

    #[test]
    fn obsolete_typed_roles_and_privileged_paths_are_unrepresentable() {
        assert!(serde_json::from_str::<VpsBinaryRoleV2>(r#""sandbox_broker""#).is_err());
        assert!(serde_json::from_str::<VpsConfigRoleV2>(r#""sandbox_broker""#).is_err());
        assert!(serde_json::from_str::<VpsHostFileRoleV2>(r#""sandbox_broker_socket""#).is_err());
        for path in [
            "polkit/50-robin.rules",
            "systemd/system/robin.service",
            "systemd/robin.service",
            "systemd/user/robin-highscores-verifier-broker.service",
            "systemd/user/robin-highscores.socket",
        ] {
            assert!(forbidden_release_path(path), "accepted {path:?}");
        }
        assert!(!forbidden_release_path(
            "systemd/user/robin-highscores-api.service"
        ));
    }

    #[test]
    fn plan_requires_the_exact_single_user_role_closure() -> Result<()> {
        let binaries = [
            VpsBinaryRoleV2::Admin,
            VpsBinaryRoleV2::ManifestTool,
            VpsBinaryRoleV2::Server,
            VpsBinaryRoleV2::Worker,
            VpsBinaryRoleV2::ReplayVerifier,
        ]
        .into_iter()
        .map(|role| VpsBinarySourceV2 {
            role,
            source: role.output_name().into(),
            artifact: fact(role.output_name().as_bytes()),
        })
        .collect::<Vec<_>>();
        let configs = [
            VpsConfigRoleV2::Server,
            VpsConfigRoleV2::Worker,
            VpsConfigRoleV2::ApiEnvironment,
            VpsConfigRoleV2::WorkerEnvironment,
        ]
        .into_iter()
        .map(|role| VpsConfigSourceV2 {
            role,
            source: role.output_name().into(),
            artifact: fact(role.output_name().as_bytes()),
        })
        .collect::<Vec<_>>();
        let host_files = [
            VpsHostFileRoleV2::UserTarget,
            VpsHostFileRoleV2::ApiService,
            VpsHostFileRoleV2::WorkerService,
            VpsHostFileRoleV2::BackupService,
            VpsHostFileRoleV2::BackupTimer,
            VpsHostFileRoleV2::DeployReleaseScript,
            VpsHostFileRoleV2::RollbackReleaseScript,
            VpsHostFileRoleV2::ValidateReleaseScript,
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            VpsHostFileRoleV2::RootOnceScript,
            VpsHostFileRoleV2::NginxChallenge,
            VpsHostFileRoleV2::NginxCloudflareOnly,
            VpsHostFileRoleV2::NginxApiLocations,
            VpsHostFileRoleV2::NginxVhost,
            VpsHostFileRoleV2::DeploymentReadme,
            VpsHostFileRoleV2::OperatorRunbook,
            VpsHostFileRoleV2::BackupRunbook,
        ]
        .into_iter()
        .map(|role| VpsHostFileSourceV2 {
            role,
            source: role.output_path().into(),
            artifact: fact(role.output_path().as_bytes()),
        })
        .collect::<Vec<_>>();
        let mut plan = VpsReleasePlanV2 {
            schema_version: PLAN_SCHEMA_VERSION,
            source_commit: "a".repeat(40),
            publication_v3: "publication".into(),
            binaries,
            configs,
            host_files,
            private_raw_roots: vec![
                PrivateRawRootV2 {
                    edition: OfficialContentEditionV1::Demo,
                    root: format!("{STATE_ROOT}/raw-content/demo").into(),
                },
                PrivateRawRootV2 {
                    edition: OfficialContentEditionV1::Full,
                    root: format!("{STATE_ROOT}/raw-content/full").into(),
                },
            ],
        };
        validate_plan_shape(&plan)?;
        plan.binaries.remove(1);
        assert!(validate_plan_shape(&plan).is_err());
        Ok(())
    }

    #[test]
    fn user_deployment_identity_and_units_reject_root_or_current_drift() -> Result<()> {
        let mut deployment = canonical_user_deployment();
        deployment.user = "root".into();
        assert!(deployment.validate().is_err());

        let root = tempfile::tempdir()?;
        let unit = root.path().join("api.service");
        let commit = "a".repeat(40);
        let release = format!("{INSTALL_ROOT}/releases/{commit}");
        let valid = CANONICAL_API_SERVICE.replace("@SOURCE_COMMIT@", &commit);
        fs::write(&unit, &valid)?;
        validate_final_host_file(VpsHostFileRoleV2::ApiService, &unit, &commit)?;
        fs::write(&unit, format!("{valid}User=robinhood\n"))?;
        assert!(validate_final_host_file(VpsHostFileRoleV2::ApiService, &unit, &commit).is_err());
        fs::write(
            &unit,
            valid.replace(&release, &format!("{INSTALL_ROOT}/current")),
        )?;
        assert!(validate_final_host_file(VpsHostFileRoleV2::ApiService, &unit, &commit).is_err());
        Ok(())
    }

    #[test]
    fn backup_status_authority_matches_all_three_service_sandboxes() -> Result<()> {
        let root = tempfile::tempdir()?;
        let deploy_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy");
        let server = root.path().join("server.toml");
        let api = root.path().join("api.service");
        let worker = root.path().join("worker.service");
        let backup = root.path().join("backup.service");
        let timer = root.path().join("backup.timer");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let valid_server = format!(
            "backup_manifest_path = \"{BACKUP_STATUS_PATH}\"\nrelease_manifest_path = \"{INSTALL_ROOT}/releases/{commit}/{RELEASE_MANIFEST_FILE}\"\nmaximum_backup_age_hours = 32\n"
        );
        fs::write(&server, &valid_server)?;
        let source_unit = |name: &str| -> Result<String> {
            Ok(fs::read_to_string(deploy_root.join(name))?.replace("@SOURCE_COMMIT@", commit))
        };
        let api_valid = source_unit("robin-highscores-api.service")?;
        let worker_valid = source_unit("robin-highscores-worker.service")?;
        let backup_valid = source_unit("robin-highscores-backup.service")?;
        let timer_valid = source_unit("robin-highscores-backup.timer")?;
        fs::write(&api, &api_valid)?;
        fs::write(&worker, &worker_valid)?;
        fs::write(&backup, &backup_valid)?;
        fs::write(&timer, &timer_valid)?;
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)?;

        fs::write(
            &server,
            valid_server.replace(
                BACKUP_STATUS_PATH,
                &format!("{BACKUP_ROOT}/backup-status.json"),
            ),
        )?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        fs::write(&server, &valid_server)?;
        fs::write(&server, valid_server.replace(" = 32", " = 26"))?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        fs::write(&server, &valid_server)?;
        fs::write(
            &server,
            valid_server.replace(
                &format!("releases/{commit}/{RELEASE_MANIFEST_FILE}"),
                &format!("releases/{}/{RELEASE_MANIFEST_FILE}", "f".repeat(40)),
            ),
        )?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        fs::write(&server, &valid_server)?;

        for invalid_api in [
            api_valid.replace(&format!("ReadOnlyPaths={BACKUP_STATUS_ROOT}\n"), ""),
            api_valid.replace(
                &format!("ReadOnlyPaths={BACKUP_STATUS_ROOT}"),
                &format!("ReadWritePaths={BACKUP_STATUS_ROOT}"),
            ),
            format!("{api_valid}ReadOnlyPaths={BACKUP_ROOT}\n"),
        ] {
            fs::write(&api, invalid_api)?;
            assert!(
                validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                    .is_err()
            );
        }
        fs::write(&api, &api_valid)?;

        fs::write(
            &backup,
            backup_valid.replace(&format!("ReadWritePaths={BACKUP_STATUS_ROOT}\n"), ""),
        )?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        fs::write(
            &backup,
            backup_valid.replace(
                BACKUP_STATUS_PATH,
                &format!("{BACKUP_ROOT}/backup-status.json"),
            ),
        )?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        let unit_map = "/home/robinhood/.config/systemd/user/robin-highscores.target=/home/robinhood/.config/systemd/user/robin-highscores.target";
        for invalid_backup in [
            backup_valid.replace(
                unit_map,
                "/home/robinhood/.config/systemd/user=/home/robinhood/.config/systemd/user",
            ),
            backup_valid.replace(
                unit_map,
                &format!("{INSTALL_ROOT}/releases/{commit}={INSTALL_ROOT}/releases/{commit}"),
            ),
            backup_valid.replace(&format!(" --restore-source-map {unit_map}"), ""),
        ] {
            fs::write(&backup, invalid_backup)?;
            assert!(
                validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                    .is_err()
            );
        }
        fs::write(&backup, &backup_valid)?;
        fs::write(
            &timer,
            timer_valid.replace("RandomizedDelaySec=45m", "RandomizedDelaySec=46m"),
        )?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        fs::write(&timer, &timer_valid)?;

        fs::write(
            &worker,
            worker_valid.replace(
                &format!("InaccessiblePaths={BACKUP_STATUS_ROOT}"),
                &format!("ReadOnlyPaths={BACKUP_STATUS_ROOT}"),
            ),
        )?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn user_release_scripts_require_literal_install_root_assignment() -> Result<()> {
        let commit = "a".repeat(40);
        let deploy_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy");
        for (role, name) in [
            (VpsHostFileRoleV2::DeployReleaseScript, "deploy-release.sh"),
            (
                VpsHostFileRoleV2::RollbackReleaseScript,
                "rollback-release.sh",
            ),
        ] {
            validate_final_host_file(role, &deploy_root.join(name), &commit)?;
        }

        let valid = format!(
            "#!/bin/sh\nexpected_user=robinhood\nexpected_home=/home/robinhood\nopt_root={INSTALL_ROOT}\nstate_root={STATE_ROOT}\nsecret_root=$state_root/api-secrets\nfor managed_directory in \\\n    \"$state_root/database\" \\\n    \"$state_root/replays\" \\\n    \"$state_root/campaign-states\" \\\n    \"$state_root/backups\" \\\n    \"$state_root/status\" \\\n    \"$secret_root\"\ndo\n    install -d -m 0700 -- \"$managed_directory\"\ndone\nchmod 0700 -- \"$state_root\" \"$state_root/database\" \"$state_root/replays\" \"$state_root/campaign-states\" \"$state_root/backups\" \"$state_root/status\" \"$secret_root\"\nsystemctl --user restart robin-highscores.target\n"
        );
        let invalid = [
            valid.replace(
                &format!("opt_root={INSTALL_ROOT}\n"),
                "opt_root=$expected_home/.local/opt/robin-highscores\n",
            ),
            valid.replace(
                &format!("opt_root={INSTALL_ROOT}\n"),
                "opt_root=$HOME/.local/opt/robin-highscores\n",
            ),
            valid.replace(
                &format!("opt_root={INSTALL_ROOT}\n"),
                "    opt_root=$expected_home/.local/opt/robin-highscores\n",
            ),
            valid.replace(&format!("opt_root={INSTALL_ROOT}\n"), ""),
            valid.replace(
                &format!("opt_root={INSTALL_ROOT}\n"),
                "opt_root=/opt/robin-highscores\n",
            ),
        ];

        validate_literal_install_root_assignment(&valid)?;
        for adversarial in &invalid {
            assert!(
                validate_literal_install_root_assignment(adversarial).is_err(),
                "accepted a non-literal opt_root assignment: {adversarial:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn executable_and_nginx_host_inputs_are_byte_exact() -> Result<()> {
        let root = tempfile::tempdir()?;
        let candidate = root.path().join("host-input");
        let deploy_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy");
        let commit = "a".repeat(40);
        for (role, name) in [
            (VpsHostFileRoleV2::UserTarget, "robin-highscores.target"),
            (
                VpsHostFileRoleV2::ApiService,
                "robin-highscores-api.service",
            ),
            (
                VpsHostFileRoleV2::WorkerService,
                "robin-highscores-worker.service",
            ),
            (
                VpsHostFileRoleV2::BackupService,
                "robin-highscores-backup.service",
            ),
            (
                VpsHostFileRoleV2::BackupTimer,
                "robin-highscores-backup.timer",
            ),
            (VpsHostFileRoleV2::DeployReleaseScript, "deploy-release.sh"),
            (
                VpsHostFileRoleV2::RollbackReleaseScript,
                "rollback-release.sh",
            ),
            (
                VpsHostFileRoleV2::ValidateReleaseScript,
                "validate-release-bundle.sh",
            ),
            (
                VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
                "tests/real-runtime-fence-release-gate.sh",
            ),
            (
                VpsHostFileRoleV2::RealRuntimeFenceHarness,
                "tests/real-runtime-fence-e2e.py",
            ),
            (
                VpsHostFileRoleV2::RealRuntimeFenceSelftest,
                "tests/real-runtime-fence-e2e-selftest.py",
            ),
            (VpsHostFileRoleV2::RootOnceScript, "root-once.sh"),
            (
                VpsHostFileRoleV2::NginxChallenge,
                "nginx-robinhood-api.challenge.conf",
            ),
            (
                VpsHostFileRoleV2::NginxCloudflareOnly,
                "nginx-robinhood-cloudflare-only.conf",
            ),
            (
                VpsHostFileRoleV2::NginxApiLocations,
                "nginx-robinhood-api.locations.conf",
            ),
            (
                VpsHostFileRoleV2::NginxVhost,
                "nginx-robinhood-api.vhost.conf",
            ),
        ] {
            let canonical = fs::read_to_string(deploy_root.join(name))?
                .replace("@SOURCE_COMMIT@", &commit)
                .into_bytes();
            fs::write(&candidate, &canonical)?;
            validate_final_host_file(role, &candidate, &commit)?;
            let mut appended = canonical;
            appended.extend_from_slice(b"\nrm -rf -- /home/robinhood/.local/share\n");
            fs::write(&candidate, appended)?;
            assert!(
                validate_final_host_file(role, &candidate, &commit).is_err(),
                "{role:?} accepted appended executable authority"
            );
        }
        Ok(())
    }

    #[test]
    fn runtime_fence_selftest_template_accepts_deterministic_successor_only() -> Result<()> {
        let root = tempfile::tempdir()?;
        let candidate = root.path().join("real-runtime-fence-e2e-selftest.py");
        let canonical_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../robin_highscores/deploy/tests/real-runtime-fence-e2e-selftest.py");
        let canonical = fs::read_to_string(canonical_path)?;
        assert_eq!(
            Digest32::digest_bytes(canonical.as_bytes()).to_string(),
            "d1a1646e1d26d526a5040e433cc39bf1c24853cdd6cd1e3f9b39065f7b7fbbf7"
        );
        fs::write(&candidate, &canonical)?;
        validate_final_host_file(
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            &candidate,
            &"a".repeat(40),
        )?;

        let stale = canonical
            .replace("            (stable / \"child\").chmod(0o640)\n", "")
            .replace("            first_raw_digest = first_raw[-1][1]\n", "")
            .replace(
                concat!(
                    "            second_raw = MODULE.immutable_raw_metadata_tree(raw)\n",
                    "            self.assertNotEqual(second_raw, first_raw)\n",
                    "            self.assertNotEqual(second_raw[-1][1], first_raw_digest)\n",
                ),
                "            self.assertNotEqual(MODULE.immutable_raw_metadata_tree(raw), first_raw)\n",
            );
        assert_ne!(
            stale, canonical,
            "stale fixture must differ from its successor"
        );
        fs::write(&candidate, stale)?;
        assert!(
            validate_final_host_file(
                VpsHostFileRoleV2::RealRuntimeFenceSelftest,
                &candidate,
                &"a".repeat(40),
            )
            .is_err(),
            "accepted the retired timestamp-dependent selftest template"
        );

        fs::write(&candidate, format!("{canonical}# tampered\n"))?;
        assert!(
            validate_final_host_file(
                VpsHostFileRoleV2::RealRuntimeFenceSelftest,
                &candidate,
                &"a".repeat(40),
            )
            .is_err(),
            "accepted a tampered selftest template"
        );
        Ok(())
    }

    #[test]
    fn release_validator_system_unit_denylist_is_exact_and_role_bound() -> Result<()> {
        let root = tempfile::tempdir()?;
        let script = root.path().join("validate-release-bundle.sh");
        let commit = "a".repeat(40);
        let canonical_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../robin_highscores/deploy/validate-release-bundle.sh");
        let canonical = fs::read_to_string(canonical_path)?;
        fs::write(&script, &canonical)?;
        validate_final_host_file(VpsHostFileRoleV2::ValidateReleaseScript, &script, &commit)?;

        let adversarial = [
            format!("{canonical}\n# extra reference: {SYSTEM_UNIT_ROOT}\n"),
            canonical.replace(SYSTEM_UNIT_ROOT, r"/etc/systemd\/system"),
            canonical.replace(SYSTEM_UNIT_ROOT, "$system_unit_root"),
            canonical.replace(SYSTEM_UNIT_ROOT, "/usr/lib/systemd/system"),
            canonical.replace(
                VALIDATOR_SYSTEM_UNIT_DENYLIST_BLOCK,
                &format!("# deny root units at {SYSTEM_UNIT_ROOT}\n"),
            ),
            format!("{canonical}\nsystemctl enable robin-highscores-api.service\n"),
            format!(
                "{canonical}\nsystem_unit_parent=/etc/systemd\ninstall robin.service \"$system_unit_parent/system/robin.service\"\n"
            ),
        ];
        for candidate in adversarial {
            fs::write(&script, &candidate)?;
            assert!(
                validate_final_host_file(
                    VpsHostFileRoleV2::ValidateReleaseScript,
                    &script,
                    &commit,
                )
                .is_err(),
                "accepted noncanonical validator system-unit authority: {candidate:?}"
            );
        }

        for role in [
            VpsHostFileRoleV2::UserTarget,
            VpsHostFileRoleV2::ApiService,
            VpsHostFileRoleV2::WorkerService,
            VpsHostFileRoleV2::BackupService,
            VpsHostFileRoleV2::BackupTimer,
            VpsHostFileRoleV2::DeployReleaseScript,
            VpsHostFileRoleV2::RollbackReleaseScript,
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            VpsHostFileRoleV2::RootOnceScript,
            VpsHostFileRoleV2::NginxChallenge,
            VpsHostFileRoleV2::NginxCloudflareOnly,
            VpsHostFileRoleV2::NginxApiLocations,
            VpsHostFileRoleV2::NginxVhost,
            VpsHostFileRoleV2::DeploymentReadme,
            VpsHostFileRoleV2::OperatorRunbook,
            VpsHostFileRoleV2::BackupRunbook,
        ] {
            assert!(
                validate_system_unit_root_authority(role, SYSTEM_UNIT_ROOT).is_err(),
                "{role:?} accepted the root system-unit path"
            );
        }
        Ok(())
    }

    #[test]
    fn deploy_script_requires_the_complete_first_restore_layout() -> Result<()> {
        let root = tempfile::tempdir()?;
        let script = root.path().join("deploy-release.sh");
        let commit = "a".repeat(40);
        let canonical = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../robin_highscores/deploy/deploy-release.sh"),
        )?;
        for mutable_directory in [
            "database",
            "replays",
            "campaign-states",
            "backups",
            "status",
        ] {
            fs::write(
                &script,
                canonical.replace(
                    &format!("\"$state_root/{mutable_directory}\""),
                    &format!("\"$state_root/omitted-{mutable_directory}\""),
                ),
            )?;
            assert!(
                validate_final_host_file(VpsHostFileRoleV2::DeployReleaseScript, &script, &commit)
                    .is_err(),
                "deploy validation accepted a missing first-restore {mutable_directory} directory"
            );
        }
        fs::write(
            &script,
            canonical.replace(
                "secret_root=$state_root/api-secrets",
                "secret_root=$state_root/wrong",
            ),
        )?;
        assert!(
            validate_final_host_file(VpsHostFileRoleV2::DeployReleaseScript, &script, &commit)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn derived_root_once_checksum_manifest_is_exact() -> Result<()> {
        let root = tempfile::tempdir()?;
        let deploy = root.path().join("deploy");
        fs::create_dir(&deploy)?;
        for name in ROOT_ONCE_KIT_FILES {
            fs::write(deploy.join(name), format!("exact bytes for {name}\n"))?;
        }
        let canonical = expected_root_once_sha256sums(root.path())?;
        fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), &canonical)?;
        validate_root_once_sha256sums(root.path())?;

        let names = std::str::from_utf8(&canonical)?
            .lines()
            .map(|line| line.split_once("  ").map(|(_, name)| name))
            .collect::<Option<Vec<_>>>()
            .context("derived root-once checksum line is not canonical")?;
        assert_eq!(names, ROOT_ONCE_KIT_FILES);
        assert_eq!(canonical_file_mode(ROOT_ONCE_SHA256SUMS_FILE), 0o440);

        fs::remove_file(root.path().join(ROOT_ONCE_SHA256SUMS_FILE))?;
        assert!(validate_root_once_sha256sums(root.path()).is_err());

        let mut lines = canonical.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        lines.swap(0, 1);
        let reordered = lines.join(&b'\n');
        fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), reordered)?;
        assert!(validate_root_once_sha256sums(root.path()).is_err());

        let mut substituted = canonical.clone();
        substituted[0] = if substituted[0] == b'a' { b'b' } else { b'a' };
        fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), substituted)?;
        assert!(validate_root_once_sha256sums(root.path()).is_err());

        let mut extra = canonical.clone();
        extra.extend_from_slice(
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  extra.conf\n",
        );
        fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), extra)?;
        assert!(validate_root_once_sha256sums(root.path()).is_err());
        Ok(())
    }

    #[test]
    fn derived_deploy_bootstrap_checksum_manifest_is_exact() -> Result<()> {
        let root = tempfile::tempdir()?;
        let deploy = root.path().join("deploy");
        fs::create_dir(&deploy)?;
        for name in DEPLOY_BOOTSTRAP_FILES {
            fs::write(deploy.join(name), format!("exact bytes for {name}\n"))?;
        }
        let canonical = expected_deploy_bootstrap_sha256sums(root.path())?;
        fs::write(
            root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
            &canonical,
        )?;
        validate_deploy_bootstrap_sha256sums(root.path())?;

        let names = std::str::from_utf8(&canonical)?
            .lines()
            .map(|line| line.split_once("  ").map(|(_, name)| name))
            .collect::<Option<Vec<_>>>()
            .context("derived deploy bootstrap checksum line is not canonical")?;
        assert_eq!(names, DEPLOY_BOOTSTRAP_FILES);
        assert_eq!(canonical_file_mode(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE), 0o440);

        fs::remove_file(root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE))?;
        assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());

        let mut lines = canonical.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        lines.swap(0, 1);
        fs::write(
            root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
            lines.join(&b'\n'),
        )?;
        assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());

        let mut substituted = canonical.clone();
        substituted[0] = if substituted[0] == b'a' { b'b' } else { b'a' };
        fs::write(
            root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
            substituted,
        )?;
        assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());

        let mut extra = canonical;
        extra.extend_from_slice(
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  extra.sh\n",
        );
        fs::write(root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE), extra)?;
        assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());
        Ok(())
    }

    #[test]
    fn release_manifest_splits_historical_structure_from_current_candidate_admission() {
        let file = VpsReleaseFileV2 {
            path: "bin/tool".into(),
            artifact: fact(b"tool"),
            unix_mode: 0o440,
        };
        let mut manifest = VpsReleaseManifestV2 {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source_commit: "a".repeat(40),
            database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
            deployment: canonical_user_deployment(),
            publication_lock_sha256: Digest32::digest_bytes(b"lock"),
            publication_manifest_sha256: Digest32::digest_bytes(b"publication"),
            verifier_sha256: Digest32::digest_bytes(b"verifier"),
            files: vec![file.clone()],
        };
        manifest.validate().unwrap();
        manifest.files.push(file);
        assert!(manifest.validate().is_err());
        manifest.files.pop();
        manifest.files[0].unix_mode = 0o644;
        assert!(manifest.validate().is_err());
        manifest.files[0].unix_mode = 0o440;
        manifest.database_schema_version = MIN_SUPPORTED_DATABASE_SCHEMA_VERSION - 1;
        assert!(manifest.validate().is_err());
        manifest.database_schema_version = MIN_SUPPORTED_DATABASE_SCHEMA_VERSION;
        manifest.validate().unwrap();
        assert_eq!(
            manifest.validate_current_candidate().is_ok(),
            MIN_SUPPORTED_DATABASE_SCHEMA_VERSION == HIGHSCORES_DATABASE_SCHEMA_VERSION
        );
        manifest.database_schema_version += 1;
        assert_eq!(
            manifest.validate().is_ok(),
            manifest.database_schema_version <= HIGHSCORES_DATABASE_SCHEMA_VERSION
        );
        manifest.database_schema_version = HIGHSCORES_DATABASE_SCHEMA_VERSION;
        manifest.validate_current_candidate().unwrap();
        manifest.verifier_sha256 = Digest32::default();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn raw_root_declarations_require_exact_demo_full_roots() {
        let roots = [
            (
                OfficialContentEditionV1::Demo,
                "/home/robinhood/.local/share/robin-highscores/raw-content/demo",
            ),
            (
                OfficialContentEditionV1::Full,
                "/home/robinhood/.local/share/robin-highscores/raw-content/full",
            ),
        ]
        .into_iter()
        .map(|(edition, root)| PrivateRawRootV2 {
            edition,
            root: root.into(),
        })
        .collect::<Vec<_>>();
        let mut document = PrivateRawRootDeclarationsV2 {
            schema_version: RAW_ROOTS_SCHEMA_VERSION,
            roots,
        };
        document.validate().unwrap();
        document.schema_version = 1;
        assert!(document.validate().is_err());
        document.schema_version = RAW_ROOTS_SCHEMA_VERSION;
        document.roots.swap(0, 1);
        assert!(document.validate().is_err());
    }

    #[test]
    fn selected_raw_content_manifest_requires_exact_bytes() -> Result<()> {
        let root = tempfile::tempdir()?;
        let file = root.path().join("Data/mission.bin");
        fs::create_dir_all(file.parent().context("test path has no parent")?)?;
        fs::write(&file, b"exact raw input")?;
        let manifest = OfficialSourceTreeManifestV2 {
            schema_version: 2,
            edition: OfficialContentEditionV1::Demo,
            source_format: robin_run_protocol::OfficialProjectionSourceFormatV1::LooseNativeV1,
            closure_kind:
                robin_run_protocol::OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1,
            files: vec![robin_run_protocol::OfficialSourceFileV1 {
                path: "Data/mission.bin".into(),
                sha256: Digest32::digest_bytes(b"exact raw input"),
                byte_length: 15,
            }],
        };
        manifest.validate()?;
        validate_raw_content_against_manifest(root.path(), &manifest)?;
        fs::write(&file, b"substitution")?;
        assert!(validate_raw_content_against_manifest(root.path(), &manifest).is_err());
        Ok(())
    }

    #[test]
    fn sorted_sha256_and_mode_inventories_are_exact() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("z"), b"z")?;
        fs::write(root.path().join("a"), b"a")?;
        let sums = expected_sha256sums(root.path())?;
        let text = String::from_utf8(sums)?;
        let lines = text.lines().collect::<Vec<_>>();
        assert!(lines[0].ends_with("  a"));
        assert!(lines[1].ends_with("  z"));
        let modes = BTreeMap::from([
            ("a".to_owned(), ('f', 0o440)),
            ("z".to_owned(), ('f', 0o440)),
        ]);
        assert_eq!(
            String::from_utf8(mode_inventory_bytes(&modes)?)?,
            "f 0440  a\nf 0440  z\n"
        );
        assert_eq!(canonical_file_mode("bin/robin-highscores-server"), 0o550);
        assert_eq!(canonical_file_mode("deploy/deploy-release.sh"), 0o550);
        assert_eq!(canonical_file_mode("config/highscores-server.toml"), 0o440);
        Ok(())
    }

    #[test]
    fn payload_inventory_sorts_serialized_paths_after_path_conversion() -> Result<()> {
        let root = tempfile::tempdir()?;
        let verifier_config = root
            .path()
            .join("private/verifier/operator-config/catalog.json");
        let verifier_bundle = root
            .path()
            .join("private/verifier-bundles/content/catalog/component.json");
        fs::create_dir_all(
            verifier_config
                .parent()
                .context("verifier config path has no parent")?,
        )?;
        fs::create_dir_all(
            verifier_bundle
                .parent()
                .context("verifier bundle path has no parent")?,
        )?;
        fs::write(&verifier_config, b"config")?;
        fs::write(&verifier_bundle, b"bundle")?;

        let component_order = walk_regular_files(root.path())?
            .into_iter()
            .map(|(relative, _)| path_to_manifest(&relative))
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(
            component_order,
            [
                "private/verifier/operator-config/catalog.json",
                "private/verifier-bundles/content/catalog/component.json",
            ]
        );

        let files = payload_inventory(root.path())?;
        assert_eq!(
            files
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            [
                "private/verifier-bundles/content/catalog/component.json",
                "private/verifier/operator-config/catalog.json",
            ]
        );

        let sums = String::from_utf8(expected_sha256sums(root.path())?)?;
        let listed_paths = sums
            .lines()
            .map(|line| {
                line.split_once("  ")
                    .context("SHA256SUMS test line has no path")
                    .map(|(_, path)| path)
            })
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(
            listed_paths,
            [
                "private/verifier-bundles/content/catalog/component.json",
                "private/verifier/operator-config/catalog.json",
            ],
            "SHA256SUMS must use bytewise serialized-path order"
        );
        assert!(
            listed_paths.windows(2).all(|pair| pair[0] < pair[1]),
            "the authentic gate rejects a non-strictly-sorted SHA256SUMS"
        );
        VpsReleaseManifestV2 {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source_commit: "a".repeat(40),
            database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
            deployment: canonical_user_deployment(),
            publication_lock_sha256: Digest32::digest_bytes(b"lock"),
            publication_manifest_sha256: Digest32::digest_bytes(b"publication"),
            verifier_sha256: Digest32::digest_bytes(b"verifier"),
            files,
        }
        .validate_current_candidate()?;
        Ok(())
    }

    #[test]
    fn publication_subset_includes_exact_materialized_runtime_fence_closure() {
        let materialized = [
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
        ]
        .into_iter()
        .map(|role| role.output_path().to_owned())
        .collect::<BTreeSet<_>>();
        let mut admitted = BTreeSet::new();
        extend_real_runtime_fence_payload_paths(&mut admitted);

        assert_eq!(admitted, materialized);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn outer_activation_preflight_reads_inherited_procfds_then_executes_script() -> Result<()> {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::process::CommandExt as _;
        use std::process::Command;

        const CHILD: &str = "ROBIN_TEST_VPS_OUTER_PROC_FD_CHILD";
        const SCRIPT_FD: &str = "ROBIN_TEST_VPS_OUTER_SCRIPT_FD";
        const BOOTSTRAP_FD: &str = "ROBIN_TEST_VPS_OUTER_BOOTSTRAP_FD";
        const VALIDATOR_FD: &str = "ROBIN_TEST_VPS_OUTER_VALIDATOR_FD";
        const TOOL_FD: &str = "ROBIN_TEST_VPS_OUTER_TOOL_FD";
        const PLAN_FD: &str = "ROBIN_TEST_VPS_OUTER_PLAN_FD";
        const BOOTSTRAP_SHA: &str = "ROBIN_TEST_VPS_OUTER_BOOTSTRAP_SHA";
        const PLAN_SHA: &str = "ROBIN_TEST_VPS_OUTER_PLAN_SHA";
        const MARKER: &str = "ROBIN_TEST_VPS_OUTER_MARKER";

        if std::env::var_os(CHILD).is_some() {
            let descriptor_path = |name: &str| -> Result<PathBuf> {
                Ok(PathBuf::from(format!(
                    "/proc/self/fd/{}",
                    std::env::var(name)?.parse::<std::os::fd::RawFd>()?
                )))
            };
            let script = descriptor_path(SCRIPT_FD)?;
            let bootstrap = descriptor_path(BOOTSTRAP_FD)?;
            let validator = descriptor_path(VALIDATOR_FD)?;
            let tool = descriptor_path(TOOL_FD)?;
            let plan = descriptor_path(PLAN_FD)?;
            let raw = pin_vps_activation_exec_authorities(
                VpsActivationExecOperationV2::Deploy,
                &script,
                &bootstrap,
                &validator,
                &tool,
                &std::env::var(BOOTSTRAP_SHA)?,
                Some(&plan),
            )?;
            let (_, _, observed_plan_sha) = load_pinned_vps_plan(&plan, &std::env::var(PLAN_SHA)?)?;
            ensure!(
                observed_plan_sha.to_string() == std::env::var(PLAN_SHA)?,
                "inherited plan digest changed during outer preflight"
            );
            clear_vps_close_on_exec(raw[0])?;
            let error = Command::new(&script)
                .arg(std::env::var_os(MARKER).context("missing marker")?)
                .exec();
            return Err(error).context("exec safe inherited-FD activation script");
        }

        let fixture = source_consume_fixture()?;
        let root = fixture.sandbox.path();
        let script_path = root.join("safe-activation.sh");
        fs::write(
            &script_path,
            b"#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 91\nprintf reached > \"$1\"\n",
        )?;
        set_mode(&script_path, 0o500)?;
        let validator_path = root.join("validator.sh");
        fs::write(&validator_path, b"#!/bin/sh\nexit 0\n")?;
        set_mode(&validator_path, 0o500)?;
        let script_sha = Digest32::digest_bytes(&fs::read(&script_path)?);
        let validator_sha = Digest32::digest_bytes(&fs::read(&validator_path)?);
        let bootstrap_path = root.join("DEPLOY_BOOTSTRAP_SHA256SUMS");
        fs::write(
            &bootstrap_path,
            format!(
                "{script_sha}  deploy-release.sh\n{script_sha}  rollback-release.sh\n{validator_sha}  validate-release-bundle.sh\n"
            ),
        )?;
        set_mode(&bootstrap_path, 0o400)?;
        let bootstrap_sha = Digest32::digest_bytes(&fs::read(&bootstrap_path)?);
        let test_tool_path = root.join("robin-highscores-manifestctl");
        fs::copy(std::env::current_exe()?, &test_tool_path)?;
        set_mode(&test_tool_path, 0o550)?;
        let plan_path = fixture.logical_source.join("vps-release-plan-v2.json");
        let marker = root.join("reached-script");

        let script = File::open(script_path)?;
        let bootstrap = File::open(bootstrap_path)?;
        let validator = File::open(validator_path)?;
        let tool = File::open(&test_tool_path)?;
        let plan = File::open(plan_path)?;
        // Force a read-induced atime change on mounts that track access time.
        plan.set_times(std::fs::FileTimes::new().set_accessed(std::time::UNIX_EPOCH))?;
        for descriptor in [
            script.as_raw_fd(),
            bootstrap.as_raw_fd(),
            validator.as_raw_fd(),
            tool.as_raw_fd(),
            plan.as_raw_fd(),
        ] {
            clear_vps_close_on_exec(descriptor)?;
        }
        let status = Command::new(test_tool_path)
            .args([
                "--exact",
                "vps_release_v2::tests::outer_activation_preflight_reads_inherited_procfds_then_executes_script",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(SCRIPT_FD, script.as_raw_fd().to_string())
            .env(BOOTSTRAP_FD, bootstrap.as_raw_fd().to_string())
            .env(VALIDATOR_FD, validator.as_raw_fd().to_string())
            .env(TOOL_FD, tool.as_raw_fd().to_string())
            .env(PLAN_FD, plan.as_raw_fd().to_string())
            .env(BOOTSTRAP_SHA, bootstrap_sha.to_string())
            .env(PLAN_SHA, fixture.plan_sha256.to_string())
            .env(MARKER, &marker)
            .status()?;
        ensure!(status.success(), "inherited-procfd preflight child failed");
        ensure!(
            fs::read(&marker)? == b"reached",
            "outer preflight did not cross into the safe activation script"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_symlinks_and_mutable_raw_roots_fail_closed() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let root = tempfile::tempdir()?;
        let raw = root.path().join("raw");
        fs::create_dir(&raw)?;
        let file = raw.join("file");
        fs::write(&file, b"raw")?;
        fs::hard_link(&file, raw.join("alias"))?;
        assert!(reject_links_and_special_nodes(&raw, false).is_err());
        fs::remove_file(raw.join("alias"))?;
        symlink("file", raw.join("link"))?;
        assert!(reject_links_and_special_nodes(&raw, false).is_err());
        fs::remove_file(raw.join("link"))?;
        assert!(reject_links_and_special_nodes(&raw, true).is_err());
        fs::set_permissions(&file, fs::Permissions::from_mode(0o440))?;
        fs::set_permissions(&raw, fs::Permissions::from_mode(0o550))?;
        validate_immutable_raw_root(&raw)?;
        Ok(())
    }

    #[test]
    fn placeholders_and_private_static_paths_fail_closed() {
        assert!(reject_placeholders(&[b'0'; 64], "test").is_err());
        assert!(reject_placeholders(b"token=changeme", "test").is_err());
        assert!(reject_placeholders(b"digest=real", "test").is_ok());
        for path in [
            "cloudflare-public/a",
            "cloudflare-identity-signer/a",
            "raw/full/a",
        ] {
            assert!(
                path.starts_with("cloudflare-") || path.starts_with("raw/"),
                "test path classifier drifted"
            );
        }
    }

    #[test]
    fn worker_config_binds_exact_direct_sandbox_authority() -> Result<()> {
        let root = tempfile::tempdir()?;
        let verifier = Digest32::digest_bytes(b"verifier");
        let catalog = fact(b"catalog");
        let demo_source = Digest32::digest_bytes(b"demo-source");
        let full_source = Digest32::digest_bytes(b"full-source");
        let source_commit = "a".repeat(40);
        let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
        let catalog_path = format!(
            "{release_root}/private/verifier/operator-config/{}",
            catalog.sha256
        );
        let worker = root.path().join("worker.toml");
        let direct_config = format!(
            concat!(
                "server_config = \"{}/config/highscores-server.toml\"\n",
                "campaign_state_directory = \"{}/campaign-states\"\n",
                "verifier_job_config_catalog = \"{}\"\n",
                "verifier_job_config_catalog_sha256 = \"{}\"\n",
                "demo_raw_content_manifest = \"{}/private/source-tree-manifests-v2/{}.json\"\n",
                "full_raw_content_manifest = \"{}/private/source-tree-manifests-v2/{}.json\"\n",
                "[verifier_launcher]\n",
                "bwrap_program = \"/usr/bin/bwrap\"\n",
                "bwrap_sha256 = \"{}\"\n",
                "prlimit_program = \"/usr/bin/prlimit\"\n",
                "prlimit_sha256 = \"{}\"\n",
                "verifier_program = \"{}/bin/robin-replay-verifier\"\n",
                "verifier_sha256 = \"{}\"\n",
                "wall_timeout_seconds = 120\n",
                "cpu_limit_seconds = 120\n",
                "address_space_limit_bytes = 1073741824\n",
                "process_limit = 32\n",
                "open_files_limit = 128\n",
                "file_size_limit_bytes = 134217728\n",
                "max_request_bytes = 1048576\n",
                "[limits]\n",
                "max_campaign_bytes = 67108864\n"
            ),
            release_root,
            STATE_ROOT,
            catalog_path,
            catalog.sha256,
            release_root,
            demo_source,
            release_root,
            full_source,
            Digest32::digest_bytes(b"bwrap"),
            Digest32::digest_bytes(b"prlimit"),
            release_root,
            verifier,
        );
        fs::write(&worker, &direct_config)?;
        validate_final_config(
            VpsConfigRoleV2::Worker,
            &worker,
            verifier,
            &catalog,
            &source_commit,
            &BTreeSet::new(),
        )?;
        fs::write(
            &worker,
            direct_config.replace(
                "[verifier_launcher]",
                "broker_socket = \"/run/obsolete.sock\"\n[verifier_launcher]",
            ),
        )?;
        assert!(
            validate_final_config(
                VpsConfigRoleV2::Worker,
                &worker,
                verifier,
                &catalog,
                &source_commit,
                &BTreeSet::new(),
            )
            .is_err()
        );
        fs::write(
            &worker,
            direct_config.replace("wall_timeout_seconds = 120", "wall_timeout_seconds = 121"),
        )?;
        assert!(
            validate_final_config(
                VpsConfigRoleV2::Worker,
                &worker,
                verifier,
                &catalog,
                &source_commit,
                &BTreeSet::new(),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn server_config_binds_release_manifests_and_both_campaign_templates() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source_commit = "b".repeat(40);
        let demo = Digest32::digest_bytes(b"demo-state");
        let full = Digest32::digest_bytes(b"full-state");
        let campaigns = BTreeSet::from([demo, full]);
        let server = root.path().join("server.toml");
        fs::write(
            &server,
            format!(
                concat!(
                    "bind = \"127.0.0.1:8787\"\n",
                    "database_path = \"/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3\"\n",
                    "replay_directory = \"/home/robinhood/.local/share/robin-highscores/replays\"\n",
                    "campaign_state_directory = \"/home/robinhood/.local/share/robin-highscores/campaign-states\"\n",
                    "cursor_secret_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key\"\n",
                    "competition_run_grant_secret_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/competition-run-grant.key\"\n",
                    "run_preflight_grant_secret_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/run-preflight-grant.key\"\n",
                    "moderation_bearer_token_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token\"\n",
                    "backup_manifest_path = \"/home/robinhood/.local/share/robin-highscores/status/backup-status.json\"\n",
                    "release_manifest_path = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/vps-release-manifest-v2.json\"\n",
                    "maximum_backup_age_hours = 32\n",
                    "minimum_storage_free_bytes = 1073741824\n",
                    "max_replay_bytes = 16777216\n",
                    "max_campaign_bytes = 16777216\n",
                    "max_metadata_bytes = 65536\n",
                    "max_concurrent_uploads = 32\n",
                    "allowed_origins = []\n",
                    "manifest_directory = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/config/manifests\"\n",
                    "[[admission_profiles]]\n",
                    "canonical_campaign_state_path = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/private/campaign-states/{}\"\n",
                    "[[admission_profiles]]\n",
                    "canonical_campaign_state_path = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/private/campaign-states/{}\"\n"
                ),
                source_commit, source_commit, source_commit, demo, source_commit, full
            ),
        )?;
        validate_final_config(
            VpsConfigRoleV2::Server,
            &server,
            Digest32::digest_bytes(b"verifier"),
            &fact(b"catalog"),
            &source_commit,
            &campaigns,
        )?;
        Ok(())
    }

    #[test]
    fn assembly_failure_preserves_absent_output() {
        let root = tempfile::tempdir().unwrap();
        let plan = root.path().join("plan.json");
        fs::write(&plan, b"{}").unwrap();
        let output = root.path().join("release");
        assert!(assemble_vps_release_v2(&plan, &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn assembly_output_requires_release_sibling_partial_and_rejects_legacy_incoming() -> Result<()>
    {
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let partial = Path::new(INSTALL_ROOT)
            .join("releases")
            .join(format!("{commit}.partial"));
        validate_vps_release_assembly_output(&partial, commit)?;

        let legacy = Path::new(INSTALL_ROOT)
            .join("incoming")
            .join(format!("{commit}.partial"));
        assert!(
            validate_vps_release_assembly_output(&legacy, commit).is_err(),
            "the legacy incoming assembly output was accepted"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_lock_projection_requires_exact_pinned_canonical_v2() -> Result<()> {
        use std::os::fd::AsRawFd as _;

        let fixture = source_consume_fixture()?;
        let manifest_path = fixture.candidate.join(RELEASE_MANIFEST_FILE);
        let manifest = File::open(&manifest_path)?;
        assert_eq!(
            project_vps_publication_lock_v2(
                manifest.as_raw_fd(),
                &fixture.release_manifest_sha256.to_string(),
            )?,
            fixture.publication_lock_sha256
        );
        assert!(project_vps_publication_lock_v2(manifest.as_raw_fd(), &"0".repeat(64)).is_err());

        let wrong_type = File::open(&fixture.candidate)?;
        assert!(
            project_vps_publication_lock_v2(
                wrong_type.as_raw_fd(),
                &fixture.release_manifest_sha256.to_string(),
            )
            .is_err()
        );

        let malformed = fixture.sandbox.path().join("noncanonical-vps-v2.json");
        let mut malformed_bytes = fs::read(&manifest_path)?;
        malformed_bytes.push(b'\n');
        fs::write(&malformed, &malformed_bytes)?;
        set_mode(&malformed, 0o440)?;
        let malformed_file = File::open(&malformed)?;
        assert!(
            project_vps_publication_lock_v2(
                malformed_file.as_raw_fd(),
                &Digest32::digest_bytes(&malformed_bytes).to_string(),
            )
            .is_err()
        );

        let wrong_schema = fixture.sandbox.path().join("wrong-schema-vps-v2.json");
        let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        document["schema_version"] = serde_json::Value::from(1);
        let wrong_schema_bytes = serde_json::to_vec(&document)?;
        fs::write(&wrong_schema, &wrong_schema_bytes)?;
        set_mode(&wrong_schema, 0o440)?;
        let wrong_schema_file = File::open(&wrong_schema)?;
        assert!(
            project_vps_publication_lock_v2(
                wrong_schema_file.as_raw_fd(),
                &Digest32::digest_bytes(&wrong_schema_bytes).to_string(),
            )
            .is_err()
        );

        for (name, mode) in [
            ("group-writable-vps-v2.json", 0o640),
            ("world-readable-vps-v2.json", 0o444),
        ] {
            let unsafe_mode = fixture.sandbox.path().join(name);
            fs::write(&unsafe_mode, fs::read(&manifest_path)?)?;
            set_mode(&unsafe_mode, mode)?;
            let unsafe_mode_file = File::open(&unsafe_mode)?;
            assert!(
                project_vps_publication_lock_v2(
                    unsafe_mode_file.as_raw_fd(),
                    &fixture.release_manifest_sha256.to_string(),
                )
                .is_err(),
                "publication projection accepted manifest mode {mode:o}"
            );
        }

        let hardlinked = fixture.sandbox.path().join("hardlinked-vps-v2.json");
        let hardlink_alias = fixture.sandbox.path().join("hardlinked-vps-v2.alias");
        fs::write(&hardlinked, fs::read(&manifest_path)?)?;
        set_mode(&hardlinked, 0o440)?;
        fs::hard_link(&hardlinked, &hardlink_alias)?;
        let hardlinked_file = File::open(&hardlinked)?;
        assert!(
            project_vps_publication_lock_v2(
                hardlinked_file.as_raw_fd(),
                &fixture.release_manifest_sha256.to_string(),
            )
            .is_err(),
            "publication projection accepted a hard-linked manifest"
        );

        let displaced = fixture.candidate.join("vps-release-manifest-v2.displaced");
        set_mode(&fixture.candidate, 0o750)?;
        fs::rename(&manifest_path, &displaced)?;
        fs::write(&manifest_path, b"{}")?;
        set_mode(&manifest_path, 0o440)?;
        set_mode(&fixture.candidate, 0o550)?;
        assert_eq!(
            project_vps_publication_lock_v2(
                manifest.as_raw_fd(),
                &fixture.release_manifest_sha256.to_string(),
            )?,
            fixture.publication_lock_sha256,
            "path replacement changed the retained descriptor authority"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failed_sealed_vps_staging_cleanup_is_bounded_and_nonfollowing() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("release");
        let staging = staging_directory(&output)?;
        let staging_path = staging.path().to_path_buf();
        let sealed = staging.path().join("nested/sealed");
        fs::create_dir_all(&sealed)?;
        fs::write(sealed.join("payload"), b"sealed payload")?;
        fs::set_permissions(sealed.join("payload"), fs::Permissions::from_mode(0o440))?;
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o550))?;
        fs::set_permissions(
            sealed.parent().context("sealed path has no parent")?,
            fs::Permissions::from_mode(0o550),
        )?;

        let outside = sandbox.path().join("outside");
        fs::create_dir(&outside)?;
        fs::write(outside.join("sentinel"), b"outside remains unchanged")?;
        symlink(&outside, staging.path().join("outside-link"))?;

        discard_failed_vps_staging(staging)?;
        ensure!(!staging_path.exists(), "failed VPS staging path remains");
        ensure!(
            fs::read(outside.join("sentinel"))? == b"outside remains unchanged",
            "VPS cleanup followed an external symlink"
        );
        ensure!(
            fs::metadata(&outside)?.permissions().mode() & 0o777 != 0o700,
            "VPS cleanup changed external directory permissions"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pinned_vps_promotion_validates_and_renames_the_same_inode() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let sandbox = tempfile::tempdir()?;
        let releases = sandbox.path().join("releases");
        fs::create_dir(&releases)?;
        fs::set_permissions(&releases, fs::Permissions::from_mode(0o750))?;
        let commit = "a".repeat(40);
        let partial = releases.join(format!("{commit}.partial"));
        let output = releases.join(&commit);
        fs::create_dir(&partial)?;
        fs::write(partial.join(SOURCE_COMMIT_FILE), format!("{commit}\n"))?;
        fs::write(partial.join(SHA256SUMS_FILE), b"authenticated sums\n")?;
        fs::write(partial.join("sentinel"), b"same pinned release inode")?;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o550))?;
        let expected_sums = Digest32::digest_bytes(b"authenticated sums\n");
        let expected_manifest = Digest32::digest_bytes(b"validated pinned manifest");

        let observed_manifest = promote_pinned_vps_release_with(
            &partial,
            &output,
            &releases,
            &commit,
            expected_sums,
            |pinned| {
                ensure!(
                    fs::symlink_metadata(pinned)?.is_dir(),
                    "pinned root alias did not resolve to a directory"
                );
                ensure!(
                    fs::read(pinned.join("sentinel"))? == b"same pinned release inode",
                    "validator observed a substituted release"
                );
                Ok(expected_manifest)
            },
        )?;
        assert_eq!(observed_manifest, expected_manifest);
        assert!(!partial.exists());
        assert_eq!(
            fs::read(output.join("sentinel"))?,
            b"same pinned release inode"
        );
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn vps_persistence_noreplace_race_cleans_staging_without_overwrite() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("release");
        let staging = staging_directory(&output)?;
        let staging_path = staging.path().to_path_buf();
        fs::create_dir(staging.path().join("sealed"))?;
        fs::write(staging.path().join("sealed/data"), b"candidate")?;
        fs::set_permissions(
            staging.path().join("sealed"),
            fs::Permissions::from_mode(0o550),
        )?;
        fs::create_dir(&output)?;
        fs::write(output.join("winner"), b"racing installer")?;

        let persist_error = persist_vps_staging(&staging, &output)
            .expect_err("NOREPLACE VPS persistence overwrote a racing output");
        ensure!(
            persist_error
                .downcast_ref::<VpsReleaseInstalledButParentSyncFailed>()
                .is_none(),
            "pre-rename VPS failure was misclassified as installed"
        );
        discard_failed_vps_staging(staging)?;
        ensure!(!staging_path.exists(), "raced VPS staging path remains");
        ensure!(
            fs::read(output.join("winner"))? == b"racing installer",
            "NOREPLACE VPS persistence modified the racing output"
        );
        ensure!(
            !output.join("sealed/data").exists(),
            "NOREPLACE VPS persistence partially merged the candidate"
        );
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn vps_post_rename_sync_failure_reports_exact_installed_identity() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("release");
        let staging = staging_directory(&output)?;
        let staging_path = staging.path().to_path_buf();
        fs::write(staging.path().join("complete"), b"complete")?;
        let outcome = persist_vps_staging_with(&staging, &output, |_| {
            anyhow::bail!("injected VPS parent sync failure")
        })?;
        let VpsPersistenceOutcome::InstalledButParentSyncFailed(sync_error) = outcome else {
            anyhow::bail!("post-rename VPS failure was not reported as installed")
        };
        ensure!(!staging_path.exists(), "renamed VPS staging path remains");
        ensure!(
            fs::read(output.join("complete"))? == b"complete",
            "installed VPS output is incomplete after parent sync failure"
        );
        let _installed_path = staging.keep();
        let manifest_sha256 = Digest32::digest_bytes(b"release manifest");
        let source_commit = "a".repeat(40);
        let classified =
            vps_installed_durability_error(&output, manifest_sha256, &source_commit, sync_error);
        let installed = classified
            .downcast_ref::<VpsReleaseInstalledButParentSyncFailed>()
            .context("installed VPS durability error is not downcastable")?;
        ensure!(
            installed.output == output
                && installed.release_manifest_sha256 == manifest_sha256
                && installed.source_commit == source_commit,
            "installed VPS durability error lost exact release identity"
        );
        ensure!(
            installed.source.to_string() == "injected VPS parent sync failure",
            "installed VPS durability error lost its source"
        );
        Ok(())
    }
}
