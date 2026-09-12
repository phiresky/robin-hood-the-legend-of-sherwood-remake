//! Descriptor-pinned, read-only pre-activation authority probing.
//!
//! This module deliberately does not accept a server config pathname or a
//! mutable-state root from the command line. The candidate release is entered
//! through one inherited directory descriptor; production state and raw-data
//! locations are fixed deployment identities.

use crate::config::{ManifestRegistry, ServerConfig, ViewerContentRequirementConfig};
use crate::verifier::DirectVerifierLauncherConfig;
use anyhow::Context as _;
use cap_std::fs::Dir;
use robin_run_protocol::{
    ArtifactRefV1, CanonicalCampaignStatePinV1, CanonicalDocument as _, Digest32,
    HIGHSCORES_DATABASE_SCHEMA_VERSION, MAX_VERIFIER_JOB_CONFIG_BYTES_V1, OfficialContentEditionV1,
    OfficialSourceTreeManifestV2, RunScopeKindV1, Validate as _, VerificationLimitsV1,
    VerifierJobConfigCatalogV1, VerifierJobRouteV1, canonical_json_bytes,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

const RELEASE_MANIFEST: &str = "vps-release-manifest-v2.json";
const MAX_RELEASE_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_AUTHORITY_ENTRIES: usize = 100_000;
const MAX_AUTHORITY_TREE_DEPTH: usize = 32;
const MAX_AUTHORITY_COMPONENT_BYTES: usize = 255;
const MAX_AUTHORITY_RELATIVE_PATH_BYTES: usize = 4_096;
const MAX_AUTHORITY_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_AUTHORITY_AGGREGATE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_AUTHENTICATED_SEMANTIC_BYTES: u64 = 512 * 1024 * 1024;
const STATE_ROOT: &str = "/home/robinhood/.local/share/robin-highscores";
const RELEASES_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores/releases";
const DEMO_RAW_ROOT: &str = "/home/robinhood/.local/share/robin-highscores/raw-content/demo";
const FULL_RAW_ROOT: &str = "/home/robinhood/.local/share/robin-highscores/raw-content/full";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum BackupAuthorityStateV2 {
    Absent,
    Present,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSelfRoleV2 {
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateReleaseAttestationV2 {
    pub source_commit: String,
    pub database_schema_version: i64,
    pub vps_release_manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAuthorityProbeV2 {
    pub backup_authority_state: BackupAuthorityStateV2,
    pub schema_version: u32,
    pub source_commit: String,
    pub vps_release_manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VpsDeploymentV2 {
    user: String,
    home: PathBuf,
    install_root: PathBuf,
    persistent_state_root: PathBuf,
    current_link: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VpsReleaseFileV2 {
    path: String,
    artifact: ArtifactRefV1,
    unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VpsReleaseManifestV2 {
    schema_version: u32,
    source_commit: String,
    database_schema_version: i64,
    deployment: VpsDeploymentV2,
    publication_lock_sha256: Digest32,
    publication_manifest_sha256: Digest32,
    verifier_sha256: Digest32,
    files: Vec<VpsReleaseFileV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRootV2 {
    edition: OfficialContentEditionV1,
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRootDeclarationsV2 {
    schema_version: u32,
    roots: Vec<RawRootV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeWorkerConfigV2 {
    server_config: PathBuf,
    worker_id: String,
    campaign_state_directory: PathBuf,
    verifier_launcher: DirectVerifierLauncherConfig,
    verifier_job_config_catalog: PathBuf,
    verifier_job_config_catalog_sha256: String,
    demo_raw_content_manifest: PathBuf,
    full_raw_content_manifest: PathBuf,
    poll_interval_ms: u64,
    lease_seconds: u64,
    retry_seconds: u64,
    max_verifier_attempts: u32,
    limits: VerificationLimitsV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScannedFileV2 {
    sha256: Digest32,
    byte_length: u64,
    unix_mode: u32,
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScannedTreeV2 {
    directories: BTreeMap<String, u32>,
    files: BTreeMap<String, ScannedFileV2>,
    authenticated_semantic_files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
struct AuthenticatedCandidateV2 {
    tree: ScannedTreeV2,
    verifier_sha256: Digest32,
}

#[derive(Debug, Clone, Copy)]
struct ExpectedFileV2 {
    maximum_byte_length: u64,
    exact_byte_length: Option<u64>,
}

#[derive(Debug)]
struct ExpectedTreeV2 {
    directories: BTreeSet<String>,
    files: BTreeMap<String, ExpectedFileV2>,
}

/// Authenticate a candidate root and the exact running executable role.
///
/// The returned `File` is a retained duplicate of the authenticated root and
/// must stay alive while a caller consumes any candidate-relative authority.
pub fn attest_candidate_release_root_v2(
    root_guard: File,
    expected_vps_release_manifest_sha256: &str,
    expected_self_role: CandidateSelfRoleV2,
) -> anyhow::Result<(CandidateReleaseAttestationV2, File)> {
    #[cfg(target_os = "linux")]
    let self_executable = File::open("/proc/self/exe")?;
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("runtime authority attestation requires Linux procfs");
    attest_candidate_release_root_v2_with_self(
        root_guard,
        expected_vps_release_manifest_sha256,
        expected_self_role,
        self_executable,
    )
}

fn attest_candidate_release_root_v2_with_self(
    root_guard: File,
    expected_vps_release_manifest_sha256: &str,
    expected_self_role: CandidateSelfRoleV2,
    self_executable: File,
) -> anyhow::Result<(CandidateReleaseAttestationV2, File)> {
    let (attestation, root_guard, _candidate) = authenticate_candidate_release_root_v2_with_self(
        root_guard,
        expected_vps_release_manifest_sha256,
        expected_self_role,
        self_executable,
    )?;
    Ok((attestation, root_guard))
}

fn authenticate_candidate_release_root_v2_with_self(
    root_guard: File,
    expected_vps_release_manifest_sha256: &str,
    expected_self_role: CandidateSelfRoleV2,
    self_executable: File,
) -> anyhow::Result<(
    CandidateReleaseAttestationV2,
    File,
    AuthenticatedCandidateV2,
)> {
    validate_nonzero_digest(
        expected_vps_release_manifest_sha256,
        "expected VPS manifest",
    )?;
    validate_candidate_root_metadata(&root_guard)?;
    let root = Dir::from_std_file(root_guard.try_clone()?);
    let manifest_bytes = read_candidate_file(
        &root,
        Path::new(RELEASE_MANIFEST),
        MAX_RELEASE_MANIFEST_BYTES,
    )?;
    let actual_manifest_sha256 = hex::encode(Sha256::digest(&manifest_bytes));
    anyhow::ensure!(
        actual_manifest_sha256 == expected_vps_release_manifest_sha256,
        "candidate VPS release manifest differs from the out-of-band digest"
    );
    let value: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&value)? == manifest_bytes,
        "candidate VPS release manifest is not canonical JSON"
    );
    let manifest: VpsReleaseManifestV2 = serde_json::from_value(value)?;
    validate_vps_manifest(&manifest)?;
    let expected_tree = candidate_expected_tree(&manifest, manifest_bytes.len())?;
    let tree = scan_candidate_tree(&root, &expected_tree)?;
    validate_authenticated_semantic_closure(&tree)?;
    anyhow::ensure!(
        authenticated_candidate_file(&tree, RELEASE_MANIFEST, MAX_RELEASE_MANIFEST_BYTES)?
            == manifest_bytes,
        "candidate VPS release manifest changed between authentication and structural scan"
    );
    validate_candidate_inventory(&tree, &manifest)?;
    validate_required_candidate_shape(&manifest)?;
    validate_self_role(&tree, &manifest, expected_self_role, self_executable)?;
    let verifier_sha256 = manifest.verifier_sha256;
    Ok((
        CandidateReleaseAttestationV2 {
            source_commit: manifest.source_commit,
            database_schema_version: manifest.database_schema_version,
            vps_release_manifest_sha256: actual_manifest_sha256,
        },
        root_guard,
        AuthenticatedCandidateV2 {
            tree,
            verifier_sha256,
        },
    ))
}

/// Perform the complete config-free authority probe. No database or mutable
/// state is created, repaired, chmodded, renamed, or otherwise changed.
pub fn probe_runtime_authority_v2(
    root_guard: File,
    expected_vps_release_manifest_sha256: &str,
    backup_authority_state: BackupAuthorityStateV2,
) -> anyhow::Result<RuntimeAuthorityProbeV2> {
    #[cfg(target_os = "linux")]
    let self_executable = File::open("/proc/self/exe")?;
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("runtime authority attestation requires Linux procfs");
    let (attestation, retained_root, candidate) = authenticate_candidate_release_root_v2_with_self(
        root_guard,
        expected_vps_release_manifest_sha256,
        CandidateSelfRoleV2::Admin,
        self_executable,
    )?;
    probe_runtime_authority_v2_inner(
        &candidate,
        &attestation,
        backup_authority_state,
        Path::new(STATE_ROOT),
        Path::new(DEMO_RAW_ROOT),
        Path::new(FULL_RAW_ROOT),
    )?;
    let (final_attestation, final_root_guard) = attest_candidate_release_root_v2(
        retained_root.try_clone()?,
        expected_vps_release_manifest_sha256,
        CandidateSelfRoleV2::Admin,
    )?;
    anyhow::ensure!(
        final_attestation == attestation,
        "candidate release authority changed during runtime probing"
    );
    drop(final_root_guard);
    Ok(RuntimeAuthorityProbeV2 {
        backup_authority_state,
        schema_version: 2,
        source_commit: attestation.source_commit,
        vps_release_manifest_sha256: attestation.vps_release_manifest_sha256,
    })
}

fn probe_runtime_authority_v2_inner(
    candidate: &AuthenticatedCandidateV2,
    attestation: &CandidateReleaseAttestationV2,
    backup_authority_state: BackupAuthorityStateV2,
    state_root: &Path,
    demo_raw_root: &Path,
    full_raw_root: &Path,
) -> anyhow::Result<()> {
    validate_exact_runtime_documents(&candidate.tree, &attestation.source_commit)?;
    let server = load_candidate_server_config(&candidate.tree, &attestation.source_commit)?;
    let worker =
        load_candidate_worker_config(&candidate.tree, &attestation.source_commit, &server)?;
    validate_secret_authority(&server, state_root, backup_authority_state)?;
    validate_grant_authority(&server, state_root)?;
    let catalog = load_candidate_catalog(&candidate.tree, &worker, &attestation.source_commit)?;
    validate_worker_authority(
        &candidate.tree,
        &server,
        &worker,
        &catalog,
        candidate.verifier_sha256,
        demo_raw_root,
        full_raw_root,
    )?;
    Ok(())
}

fn validate_vps_manifest(manifest: &VpsReleaseManifestV2) -> anyhow::Result<()> {
    anyhow::ensure!(
        manifest.schema_version == 2
            && valid_source_commit(&manifest.source_commit)
            && manifest.database_schema_version == HIGHSCORES_DATABASE_SCHEMA_VERSION,
        "candidate VPS manifest schema, source commit, or database schema is not current"
    );
    anyhow::ensure!(
        manifest.deployment
            == VpsDeploymentV2 {
                user: "robinhood".to_owned(),
                home: PathBuf::from("/home/robinhood"),
                install_root: PathBuf::from("/home/robinhood/.local/opt/robin-highscores"),
                persistent_state_root: PathBuf::from(STATE_ROOT),
                current_link: PathBuf::from("/home/robinhood/.local/opt/robin-highscores/current",),
            },
        "candidate VPS manifest has the wrong deployment identity"
    );
    anyhow::ensure!(
        !manifest.publication_lock_sha256.is_zero()
            && !manifest.publication_manifest_sha256.is_zero()
            && !manifest.verifier_sha256.is_zero()
            && !manifest.files.is_empty()
            && manifest
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path),
        "candidate VPS manifest contains a zero identity or unordered inventory"
    );
    for file in &manifest.files {
        anyhow::ensure!(
            valid_relative_path(&file.path)
                && file.artifact.validate().is_ok()
                && file.unix_mode == canonical_file_mode(&file.path),
            "candidate VPS manifest contains an unsafe artifact or mode"
        );
    }
    let bundled_verifier = manifest
        .files
        .iter()
        .find(|file| file.path == "bin/robin-replay-verifier")
        .ok_or_else(|| anyhow::anyhow!("candidate VPS manifest omits bundled verifier"))?;
    anyhow::ensure!(
        manifest.verifier_sha256 == bundled_verifier.artifact.sha256,
        "candidate VPS manifest verifier identity differs from its bundled verifier artifact"
    );
    Ok(())
}

fn validate_required_candidate_shape(manifest: &VpsReleaseManifestV2) -> anyhow::Result<()> {
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
        "private/raw-root-declarations-v2.json",
        "systemd/user/robin-highscores.target",
        "systemd/user/robin-highscores-api.service",
        "systemd/user/robin-highscores-worker.service",
        "systemd/user/robin-highscores-backup.service",
        "systemd/user/robin-highscores-backup.timer",
    ] {
        anyhow::ensure!(
            paths.contains(required),
            "candidate release omits {required}"
        );
    }
    Ok(())
}

fn validate_candidate_inventory(
    tree: &ScannedTreeV2,
    manifest: &VpsReleaseManifestV2,
) -> anyhow::Result<()> {
    let mut expected_files = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    expected_files.extend([
        "MODE_INVENTORY".to_owned(),
        "SHA256SUMS".to_owned(),
        "SOURCE_COMMIT".to_owned(),
        RELEASE_MANIFEST.to_owned(),
    ]);
    anyhow::ensure!(
        tree.files.keys().cloned().collect::<BTreeSet<_>>() == expected_files,
        "candidate release regular-file inventory differs from its manifest"
    );
    for expected in &manifest.files {
        let actual = tree
            .files
            .get(&expected.path)
            .ok_or_else(|| anyhow::anyhow!("candidate payload is missing"))?;
        anyhow::ensure!(
            actual.sha256 == expected.artifact.sha256
                && actual.byte_length == expected.artifact.byte_length
                && actual.unix_mode == expected.unix_mode,
            "candidate payload differs from its manifest at {}",
            expected.path
        );
    }
    anyhow::ensure!(
        authenticated_candidate_file(tree, "SOURCE_COMMIT", 128)?
            == format!("{}\n", manifest.source_commit).as_bytes(),
        "candidate SOURCE_COMMIT differs from its manifest"
    );
    let expected_modes = canonical_mode_inventory_bytes(
        tree.directories
            .iter()
            .map(|(path, mode)| (path.clone(), ('d', *mode))),
        tree.files
            .iter()
            .map(|(path, file)| (path.clone(), ('f', file.unix_mode))),
    )?;
    anyhow::ensure!(
        authenticated_candidate_file(tree, "MODE_INVENTORY", 64 * 1024 * 1024)? == expected_modes,
        "candidate MODE_INVENTORY is missing, reordered, or substituted"
    );
    let mut expected_sums = Vec::new();
    for (path, file) in &tree.files {
        if path != "SHA256SUMS" {
            writeln!(&mut expected_sums, "{}  {path}", file.sha256)?;
        }
    }
    anyhow::ensure!(
        authenticated_candidate_file(tree, "SHA256SUMS", 64 * 1024 * 1024)? == expected_sums,
        "candidate SHA256SUMS is missing, reordered, or substituted"
    );
    Ok(())
}

fn canonical_mode_inventory_bytes(
    entries: impl IntoIterator<Item = (String, (char, u32))>,
    additional_entries: impl IntoIterator<Item = (String, (char, u32))>,
) -> anyhow::Result<Vec<u8>> {
    let mut sorted = BTreeMap::new();
    for (path, entry) in entries.into_iter().chain(additional_entries) {
        anyhow::ensure!(
            sorted.insert(path, entry).is_none(),
            "candidate mode inventory contains a file/directory collision"
        );
    }
    let mut bytes = Vec::new();
    for (path, (kind, mode)) in sorted {
        anyhow::ensure!(
            matches!((kind, mode), ('d', 0o550) | ('f', 0o440) | ('f', 0o550)),
            "candidate mode inventory contains a mutable or invalid mode"
        );
        writeln!(&mut bytes, "{kind} {mode:04o}  {path}")?;
    }
    Ok(bytes)
}

fn validate_self_role(
    tree: &ScannedTreeV2,
    manifest: &VpsReleaseManifestV2,
    role: CandidateSelfRoleV2,
    mut self_executable: File,
) -> anyhow::Result<()> {
    let relative = match role {
        CandidateSelfRoleV2::Admin => "bin/robin-highscores-admin",
    };
    let expected = manifest
        .files
        .iter()
        .find(|file| file.path == relative)
        .ok_or_else(|| anyhow::anyhow!("candidate manifest omits the executing role"))?;
    let candidate = tree
        .files
        .get(relative)
        .ok_or_else(|| anyhow::anyhow!("authenticated candidate omits executing role"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let right = self_executable.metadata()?;
        anyhow::ensure!(
            candidate.device == right.dev() && candidate.inode == right.ino(),
            "running executable is not the exact candidate role inode"
        );
    }
    let (artifact, _) = hash_file_bounded(
        &mut self_executable,
        expected.artifact.byte_length,
        Some(expected.artifact.byte_length),
        false,
    )?;
    anyhow::ensure!(
        artifact.sha256 == expected.artifact.sha256
            && artifact.byte_length == expected.artifact.byte_length,
        "running executable differs from the candidate role authority"
    );
    Ok(())
}

fn validate_exact_runtime_documents(
    tree: &ScannedTreeV2,
    source_commit: &str,
) -> anyhow::Result<()> {
    for relative in ["config/api.env", "config/worker.env"] {
        anyhow::ensure!(
            authenticated_candidate_file(tree, relative, 1024)? == b"RUST_LOG=info\n",
            "candidate {relative} is not the exact production environment"
        );
    }
    for (relative, template) in [
        (
            "systemd/user/robin-highscores.target",
            include_str!("../deploy/robin-highscores.target"),
        ),
        (
            "systemd/user/robin-highscores-api.service",
            include_str!("../deploy/robin-highscores-api.service"),
        ),
        (
            "systemd/user/robin-highscores-worker.service",
            include_str!("../deploy/robin-highscores-worker.service"),
        ),
        (
            "systemd/user/robin-highscores-backup.service",
            include_str!("../deploy/robin-highscores-backup.service"),
        ),
        (
            "systemd/user/robin-highscores-backup.timer",
            include_str!("../deploy/robin-highscores-backup.timer"),
        ),
    ] {
        let expected = template.replace("@SOURCE_COMMIT@", source_commit);
        anyhow::ensure!(
            authenticated_candidate_file(tree, relative, MAX_CONFIG_BYTES)? == expected.as_bytes(),
            "candidate unit {relative} differs from the compiled runtime contract"
        );
    }
    Ok(())
}

fn load_candidate_server_config(
    tree: &ScannedTreeV2,
    source_commit: &str,
) -> anyhow::Result<ServerConfig> {
    let bytes =
        authenticated_candidate_file(tree, "config/highscores-server.toml", MAX_CONFIG_BYTES)?;
    let mut config: ServerConfig = toml::from_str(std::str::from_utf8(bytes)?)?;
    config.manifests = std::sync::Arc::new(ManifestRegistry::load_from_candidate(
        &tree.authenticated_semantic_files,
    )?);
    config.moderation_bearer_token = None;
    validate_server_fixed_paths(&config, source_commit)?;
    config.validate_for_runtime_probe(&tree.authenticated_semantic_files, source_commit)?;
    Ok(config)
}

fn validate_server_fixed_paths(config: &ServerConfig, source_commit: &str) -> anyhow::Result<()> {
    let state = Path::new(STATE_ROOT);
    anyhow::ensure!(
        config.bind.to_string() == "127.0.0.1:8787"
            && config.database_path == state.join("database/highscores.sqlite3")
            && config.replay_directory == state.join("replays")
            && config.campaign_state_directory == state.join("campaign-states")
            && config.cursor_secret_path == state.join("api-secrets/cursor-hmac.key")
            && config.competition_run_grant_secret_path
                == state.join("api-secrets/competition-run-grant.key")
            && config.run_preflight_grant_secret_path
                == state.join("api-secrets/run-preflight-grant.key")
            && config.backup_authority_hmac_secret_path
                == state.join("api-secrets/backup-authority-hmac.key")
            && config.moderation_bearer_token_path.as_deref()
                == Some(state.join("api-secrets/moderation-bearer.token").as_path())
            && config.backup_manifest_path.as_deref()
                == Some(state.join("status/backup-status.json").as_path())
            && config.allowed_origins.is_empty()
            && config.trusted_proxy_cidrs == ["127.0.0.1/32", "::1/128"],
        "candidate server config differs from fixed production runtime paths"
    );
    let release = Path::new(RELEASES_ROOT).join(source_commit);
    anyhow::ensure!(
        config.manifest_directory.as_deref() == Some(release.join("config/manifests").as_path())
            && config.release_manifest_path.as_deref()
                == Some(release.join(RELEASE_MANIFEST).as_path()),
        "candidate server config does not bind its exact release identity"
    );
    Ok(())
}

fn load_candidate_worker_config(
    tree: &ScannedTreeV2,
    source_commit: &str,
    server: &ServerConfig,
) -> anyhow::Result<ProbeWorkerConfigV2> {
    let bytes =
        authenticated_candidate_file(tree, "config/highscores-worker.toml", MAX_CONFIG_BYTES)?;
    let worker: ProbeWorkerConfigV2 = toml::from_str(std::str::from_utf8(bytes)?)?;
    let release = Path::new(RELEASES_ROOT).join(source_commit);
    let minimum_lease = worker
        .verifier_launcher
        .wall_timeout_seconds
        .checked_add(30)
        .ok_or_else(|| anyhow::anyhow!("worker timeout overflows"))?;
    anyhow::ensure!(
        !worker.worker_id.is_empty()
            && worker.worker_id.len() <= 128
            && worker.server_config == release.join("config/highscores-server.toml")
            && worker.campaign_state_directory == server.campaign_state_directory
            && worker.verifier_launcher.verifier_program
                == release.join("bin/robin-replay-verifier")
            && worker.poll_interval_ms > 0
            && worker.lease_seconds > minimum_lease
            && worker.retry_seconds > 0
            && (1..=100).contains(&worker.max_verifier_attempts),
        "candidate worker config differs from its exact release or runtime limits"
    );
    worker.limits.validate()?;
    worker
        .verifier_launcher
        .process_config(worker.limits.max_campaign_bytes)?;
    anyhow::ensure!(
        server.max_replay_bytes <= worker.limits.max_input_bytes
            && server.max_campaign_bytes <= worker.limits.max_campaign_bytes,
        "server admission limits exceed the candidate worker envelope"
    );
    Ok(worker)
}

fn validate_secret_authority(
    server: &ServerConfig,
    state_root: &Path,
    backup_authority_state: BackupAuthorityStateV2,
) -> anyhow::Result<()> {
    let configured_state = Path::new(STATE_ROOT);
    let secrets = [
        (&server.cursor_secret_path, "cursor-hmac.key", 32_u64, false),
        (
            &server.competition_run_grant_secret_path,
            "competition-run-grant.key",
            32,
            false,
        ),
        (
            &server.run_preflight_grant_secret_path,
            "run-preflight-grant.key",
            32,
            false,
        ),
        (
            server
                .moderation_bearer_token_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("moderation secret path is absent"))?,
            "moderation-bearer.token",
            64,
            true,
        ),
    ];
    for (configured, name, length, printable) in secrets {
        anyhow::ensure!(
            configured == &configured_state.join("api-secrets").join(name),
            "candidate config substitutes secret path {name}"
        );
        let bytes = read_private_secret(&state_root.join("api-secrets").join(name), length)?;
        if printable {
            anyhow::ensure!(
                bytes.iter().all(u8::is_ascii_graphic),
                "moderation bearer token must be exactly 64 printable ASCII bytes"
            );
        }
    }
    let backup = state_root.join("api-secrets/backup-authority-hmac.key");
    match backup_authority_state {
        BackupAuthorityStateV2::Absent => validate_secret_absent(&backup)?,
        BackupAuthorityStateV2::Present => {
            let bytes = read_private_secret(&backup, 32)?;
            anyhow::ensure!(
                bytes.iter().any(|byte| *byte != 0),
                "backup authority is all zero"
            );
        }
    }
    Ok(())
}

fn validate_grant_authority(server: &ServerConfig, state_root: &Path) -> anyhow::Result<()> {
    let competition = read_private_secret(
        &state_root.join("api-secrets/competition-run-grant.key"),
        32,
    )?;
    let preflight =
        read_private_secret(&state_root.join("api-secrets/run-preflight-grant.key"), 32)?;
    let competition: [u8; 32] = competition.try_into().expect("exact secret length");
    let preflight: [u8; 32] = preflight.try_into().expect("exact secret length");
    let competition_public = ed25519_dalek::SigningKey::from_bytes(&competition)
        .verifying_key()
        .to_bytes();
    let preflight_public = ed25519_dalek::SigningKey::from_bytes(&preflight)
        .verifying_key()
        .to_bytes();
    anyhow::ensure!(
        competition_grant_authority_matches(
            !server.competitions.is_empty(),
            server
                .manifests
                .competitions
                .values()
                .map(|manifest| manifest.competition_run_grant_public_key.as_bytes()),
            &competition_public,
        ),
        "candidate competition manifests do not bind the installed grant authority"
    );
    anyhow::ensure!(
        !server.manifests.rulesets.is_empty()
            && server.manifests.rulesets.values().all(|published| published
                .manifest
                .run_preflight_grant_public_key
                .as_bytes()
                == &preflight_public),
        "candidate rulesets do not bind the installed preflight authority"
    );
    Ok(())
}

fn competition_grant_authority_matches<'a>(
    has_configured_competitions: bool,
    manifest_public_keys: impl IntoIterator<Item = &'a [u8; 32]>,
    expected_public_key: &[u8; 32],
) -> bool {
    let mut manifest_public_keys = manifest_public_keys.into_iter().peekable();
    if !has_configured_competitions {
        return manifest_public_keys.peek().is_none();
    }
    manifest_public_keys.peek().is_some()
        && manifest_public_keys.all(|public_key| public_key == expected_public_key)
}

fn load_candidate_catalog(
    tree: &ScannedTreeV2,
    worker: &ProbeWorkerConfigV2,
    source_commit: &str,
) -> anyhow::Result<VerifierJobConfigCatalogV1> {
    validate_nonzero_digest(
        &worker.verifier_job_config_catalog_sha256,
        "worker catalog digest",
    )?;
    let expected_path = Path::new(RELEASES_ROOT)
        .join(source_commit)
        .join("private/verifier/operator-config")
        .join(&worker.verifier_job_config_catalog_sha256);
    anyhow::ensure!(
        worker.verifier_job_config_catalog == expected_path,
        "candidate worker catalog path is not its exact release identity"
    );
    let relative = format!(
        "private/verifier/operator-config/{}",
        worker.verifier_job_config_catalog_sha256
    );
    let bytes =
        authenticated_candidate_file(tree, &relative, MAX_VERIFIER_JOB_CONFIG_BYTES_V1 as u64)?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(bytes)) == worker.verifier_job_config_catalog_sha256,
        "candidate worker catalog differs from its configured digest"
    );
    let catalog: VerifierJobConfigCatalogV1 = serde_json::from_slice(bytes)?;
    catalog.validate()?;
    anyhow::ensure!(
        catalog.canonical_bytes()? == bytes,
        "worker catalog is not canonical"
    );
    Ok(catalog)
}

fn validate_worker_authority(
    tree: &ScannedTreeV2,
    server: &ServerConfig,
    worker: &ProbeWorkerConfigV2,
    catalog: &VerifierJobConfigCatalogV1,
    release_verifier_sha256: Digest32,
    demo_raw_root: &Path,
    full_raw_root: &Path,
) -> anyhow::Result<()> {
    let verifier_digest = parse_digest(&worker.verifier_launcher.verifier_sha256, "verifier")?;
    anyhow::ensure!(
        verifier_digest == release_verifier_sha256,
        "worker verifier identity differs from the authenticated release verifier"
    );
    for profile in &server.admission_profiles {
        let build_digest = parse_digest(&profile.build_manifest_id, "profile build")?;
        let ruleset_digest = parse_digest(&profile.ruleset_id, "profile ruleset")?;
        let build = server
            .manifests
            .builds
            .get(&build_digest)
            .ok_or_else(|| anyhow::anyhow!("profile build is unavailable"))?;
        let ruleset = server
            .manifests
            .rulesets
            .get(&ruleset_digest)
            .ok_or_else(|| anyhow::anyhow!("profile ruleset is unavailable"))?;
        let policy = server
            .manifests
            .policies
            .get(&ruleset.manifest.verifier_policy.manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("profile verifier policy is unavailable"))?;
        validate_runtime_verifier_binding(
            release_verifier_sha256,
            verifier_digest,
            build.semantics().verifier.sha256,
        )?;
        anyhow::ensure!(
            policy.kind == ruleset.manifest.verifier_policy.kind
                && policy.version == ruleset.manifest.verifier_policy.version,
            "candidate verifier executable or policy differs from profile {}",
            profile.id
        );
    }
    validate_catalog_covers_server(catalog, server)?;
    let declarations_bytes = authenticated_candidate_file(
        tree,
        "private/raw-root-declarations-v2.json",
        MAX_CONFIG_BYTES,
    )?;
    let declarations: RawRootDeclarationsV2 = serde_json::from_slice(declarations_bytes)?;
    anyhow::ensure!(
        canonical_json_bytes(&declarations)? == declarations_bytes
            && declarations
                == RawRootDeclarationsV2 {
                    schema_version: 2,
                    roots: vec![
                        RawRootV2 {
                            edition: OfficialContentEditionV1::Demo,
                            root: PathBuf::from(DEMO_RAW_ROOT),
                        },
                        RawRootV2 {
                            edition: OfficialContentEditionV1::Full,
                            root: PathBuf::from(FULL_RAW_ROOT),
                        },
                    ],
                },
        "candidate raw-root declarations are not exact Demo/Full production authority"
    );
    for (edition, configured, actual_root) in [
        (
            OfficialContentEditionV1::Demo,
            &worker.demo_raw_content_manifest,
            demo_raw_root,
        ),
        (
            OfficialContentEditionV1::Full,
            &worker.full_raw_content_manifest,
            full_raw_root,
        ),
    ] {
        let expected_parent = Path::new(RELEASES_ROOT)
            .join(
                server
                    .release_manifest_path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .and_then(Path::file_name)
                    .ok_or_else(|| anyhow::anyhow!("server release path has no commit"))?,
            )
            .join("private/source-tree-manifests-v2");
        anyhow::ensure!(
            configured.parent() == Some(expected_parent.as_path()),
            "worker raw manifest escapes its candidate release"
        );
        let name = configured
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("worker raw manifest has no UTF-8 filename"))?;
        let digest = name
            .strip_suffix(".json")
            .ok_or_else(|| anyhow::anyhow!("worker raw manifest lacks .json"))?;
        validate_nonzero_digest(digest, "raw source manifest digest")?;
        let relative = format!("private/source-tree-manifests-v2/{name}");
        let bytes = authenticated_candidate_file(tree, &relative, 64 * 1024 * 1024)?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(bytes)) == digest,
            "raw source manifest differs from its digest filename"
        );
        let manifest: OfficialSourceTreeManifestV2 = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        anyhow::ensure!(
            manifest.canonical_bytes()? == bytes && manifest.edition == edition,
            "raw source manifest is noncanonical or selects the wrong edition"
        );
        validate_raw_tree(actual_root, &manifest)?;
    }
    Ok(())
}

fn validate_runtime_verifier_binding(
    release_verifier_sha256: Digest32,
    worker_verifier_sha256: Digest32,
    build_verifier_sha256: Digest32,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        release_verifier_sha256 == worker_verifier_sha256
            && worker_verifier_sha256 == build_verifier_sha256,
        "release, worker, and admitted build verifier identities differ"
    );
    Ok(())
}

pub fn validate_catalog_covers_server(
    catalog: &VerifierJobConfigCatalogV1,
    server: &ServerConfig,
) -> anyhow::Result<()> {
    for entry in &catalog.entries {
        let route = &entry.route;
        let build = server
            .manifests
            .builds
            .get(&route.build_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references an unavailable build"))?;
        let content = server
            .manifests
            .content_manifests
            .get(&route.content_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references unavailable content"))?;
        let rules = server
            .manifests
            .rules_configs
            .get(&route.rules_config_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references unavailable rules"))?;
        let ruleset = server
            .manifests
            .rulesets
            .get(&route.ruleset_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("job catalog references unavailable ruleset"))?;
        anyhow::ensure!(
            build.public_document() == &entry.build_manifest
                && content == &entry.content_manifest
                && rules == &entry.rules_config
                && ruleset.manifest == entry.ruleset_manifest,
            "job catalog embeds a substituted manifest"
        );
        match (
            route.campaign_content_manifest_sha256,
            &entry.campaign_content_manifest,
        ) {
            (None, None) => {}
            (Some(digest), Some(document))
                if server.manifests.campaign_content_manifests.get(&digest) == Some(document) => {}
            _ => anyhow::bail!("job catalog campaign authority is unavailable or substituted"),
        }
        match (
            route.competition_manifest_sha256,
            &entry.competition_manifest,
        ) {
            (None, None) => {}
            (Some(digest), Some(document))
                if server.manifests.competitions.get(&digest) == Some(document) => {}
            _ => anyhow::bail!("job catalog competition authority is unavailable or substituted"),
        }
    }

    let mut expected = BTreeMap::<Vec<u8>, CanonicalCampaignStatePinV1>::new();
    for profile in &server.admission_profiles {
        let content_digest = parse_digest(&profile.content_manifest_id, "profile content")?;
        let content = server
            .manifests
            .content_manifests
            .get(&content_digest)
            .ok_or_else(|| anyhow::anyhow!("profile content is unavailable"))?;
        anyhow::ensure!(
            (!profile.viewer_available && profile.viewer_content_requirement.is_none())
                || matches!(
                    (content.edition, profile.viewer_content_requirement),
                    (
                        OfficialContentEditionV1::Demo,
                        Some(ViewerContentRequirementConfig::BundledDemo),
                    ) | (
                        OfficialContentEditionV1::Full,
                        Some(ViewerContentRequirementConfig::UserLocalRetail),
                    )
                ),
            "profile has a substituted viewer entitlement"
        );
        let build = parse_digest(&profile.build_manifest_id, "profile build")?;
        let rules = parse_digest(&profile.config_id, "profile rules")?;
        let ruleset = parse_digest(&profile.ruleset_id, "profile ruleset")?;
        let competitions = std::iter::once(None)
            .chain(
                server
                    .competitions
                    .iter()
                    .filter(|competition| competition.admission_profile_id == profile.id)
                    .map(|competition| {
                        parse_digest(&competition.manifest_sha256, "competition").map(Some)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
            )
            .collect::<Vec<_>>();
        let mut scopes = profile
            .allowed_scopes
            .iter()
            .map(|scope| match scope.as_str() {
                "individual_level" => Ok(RunScopeKindV1::IndividualLevel),
                "campaign_genesis" | "campaign_continuation" => Ok(RunScopeKindV1::Campaign),
                _ => anyhow::bail!("invalid profile scope"),
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        scopes.sort_by_key(|scope| match scope {
            RunScopeKindV1::IndividualLevel => 0,
            RunScopeKindV1::Campaign => 1,
        });
        scopes.dedup();
        for scope_kind in scopes {
            let campaign = match scope_kind {
                RunScopeKindV1::IndividualLevel => None,
                RunScopeKindV1::Campaign => Some(parse_digest(
                    profile
                        .campaign_content_manifest_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("campaign profile omits catalog"))?,
                    "campaign catalog",
                )?),
            };
            for competition in &competitions {
                let route = VerifierJobRouteV1 {
                    schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
                    scope_kind,
                    content_edition: content.edition,
                    content_subject: content.subject.clone(),
                    build_manifest_sha256: build,
                    content_manifest_sha256: content_digest,
                    campaign_content_manifest_sha256: campaign,
                    rules_config_sha256: rules,
                    ruleset_manifest_sha256: ruleset,
                    competition_manifest_sha256: *competition,
                };
                route.validate()?;
                let bytes = canonical_json_bytes(&route)?;
                if let Some(previous) =
                    expected.insert(bytes, profile.canonical_campaign_state.clone())
                {
                    anyhow::ensure!(
                        previous == profile.canonical_campaign_state,
                        "profiles substitute campaign state for one worker route"
                    );
                }
            }
        }
    }
    let actual = catalog
        .entries
        .iter()
        .map(|entry| {
            Ok((
                canonical_json_bytes(&entry.route)?,
                entry.canonical_campaign_state.clone(),
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    anyhow::ensure!(
        actual == expected,
        "worker catalog is not the exact admitted route matrix"
    );
    Ok(())
}

fn validate_raw_tree(root: &Path, manifest: &OfficialSourceTreeManifestV2) -> anyhow::Result<()> {
    let root_file = open_ambient_directory(root)?;
    let initial_root_metadata = root_file.metadata()?;
    let root_dir = Dir::from_std_file(root_file);
    let expected_tree = expected_tree_from_files(manifest.files.iter().map(|file| {
        (
            file.path.clone(),
            ExpectedFileV2 {
                maximum_byte_length: file.byte_length,
                exact_byte_length: Some(file.byte_length),
            },
        )
    }))?;
    let tree = scan_read_only_raw_tree(&root_dir, &expected_tree)?;
    let rebound_file = open_ambient_directory(root)?;
    let rebound_metadata = rebound_file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        anyhow::ensure!(
            initial_root_metadata.dev() == rebound_metadata.dev()
                && initial_root_metadata.ino() == rebound_metadata.ino(),
            "raw content root pathname was rebound during authority probing"
        );
    }
    let rebound_dir = Dir::from_std_file(rebound_file);
    anyhow::ensure!(
        authority_mount_id_with_policy(&root_dir, true)?
            == authority_mount_id_with_policy(&rebound_dir, true)?,
        "raw content root mount identity changed during authority probing"
    );
    let rebound_tree = scan_read_only_raw_tree(&rebound_dir, &expected_tree)?;
    anyhow::ensure!(
        tree == rebound_tree,
        "raw content authority changed during its double scan"
    );
    let expected_files = manifest
        .files
        .iter()
        .map(|file| (file.path.clone(), (file.sha256, file.byte_length)))
        .collect::<BTreeMap<_, _>>();
    let actual_files = tree
        .files
        .iter()
        .map(|(path, file)| (path.clone(), (file.sha256, file.byte_length)))
        .collect::<BTreeMap<_, _>>();
    anyhow::ensure!(
        actual_files == expected_files,
        "raw content regular-file inventory differs from its selected manifest"
    );
    let mut expected_directories = BTreeSet::from([".".to_owned()]);
    for path in expected_files.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(path_to_manifest(directory)? + "/");
            parent = directory.parent();
        }
    }
    anyhow::ensure!(
        tree.directories.keys().cloned().collect::<BTreeSet<_>>() == expected_directories,
        "raw content directory inventory differs from its selected manifest"
    );
    Ok(())
}

fn validate_candidate_root_metadata(root: &File) -> anyhow::Result<()> {
    let metadata = root.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        anyhow::ensure!(
            metadata.is_dir()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.permissions().mode() & 0o7777 == 0o550,
            "candidate root must be an effective-user-owned directory mode 0550"
        );
    }
    Ok(())
}

fn candidate_expected_tree(
    manifest: &VpsReleaseManifestV2,
    manifest_byte_length: usize,
) -> anyhow::Result<ExpectedTreeV2> {
    let manifest_files = manifest.files.iter().map(|file| {
        (
            file.path.clone(),
            ExpectedFileV2 {
                maximum_byte_length: file.artifact.byte_length,
                exact_byte_length: Some(file.artifact.byte_length),
            },
        )
    });
    let authority_files = [
        ("MODE_INVENTORY", MAX_RELEASE_MANIFEST_BYTES, None),
        ("SHA256SUMS", MAX_RELEASE_MANIFEST_BYTES, None),
        ("SOURCE_COMMIT", 41, Some(41)),
        (
            RELEASE_MANIFEST,
            MAX_RELEASE_MANIFEST_BYTES,
            Some(u64::try_from(manifest_byte_length)?),
        ),
    ]
    .into_iter()
    .map(|(path, maximum_byte_length, exact_byte_length)| {
        (
            path.to_owned(),
            ExpectedFileV2 {
                maximum_byte_length,
                exact_byte_length,
            },
        )
    });
    let mut expected = expected_tree_from_files(manifest_files.chain(authority_files))?;
    // Competition admission is optional, but the authored manifest registry
    // has a fixed directory for that document class even when it is empty.
    // VpsReleaseManifestV2 inventories files, so an empty directory cannot be
    // recovered from file parents. Admit only this exact supported empty class
    // and only when the candidate actually carries a manifest registry.
    if expected.directories.contains("config/manifests") {
        expected
            .directories
            .insert("config/manifests/competitions".to_owned());
        anyhow::ensure!(
            expected
                .files
                .len()
                .saturating_add(expected.directories.len())
                <= MAX_AUTHORITY_ENTRIES,
            "candidate typed closure exceeds its topology-entry limit"
        );
    }
    Ok(expected)
}

fn expected_tree_from_files(
    files: impl IntoIterator<Item = (String, ExpectedFileV2)>,
) -> anyhow::Result<ExpectedTreeV2> {
    let mut bounded_files = BTreeMap::new();
    for (path, expected) in files {
        anyhow::ensure!(
            bounded_files.len() < MAX_AUTHORITY_ENTRIES,
            "authority typed closure exceeds its file-count limit"
        );
        anyhow::ensure!(
            bounded_files.insert(path, expected).is_none(),
            "authority typed closure repeats a file path"
        );
    }
    let files = bounded_files;
    anyhow::ensure!(
        !files.is_empty() && files.len() <= MAX_AUTHORITY_ENTRIES,
        "authority typed closure has an invalid file count"
    );
    let mut directories = BTreeSet::from([String::new()]);
    let mut aggregate = 0_u64;
    for (path, expected) in &files {
        validate_authority_relative_path(path)?;
        anyhow::ensure!(
            expected.maximum_byte_length <= MAX_AUTHORITY_FILE_BYTES
                && expected
                    .exact_byte_length
                    .is_none_or(|exact| exact <= expected.maximum_byte_length),
            "authority typed closure file exceeds its byte limit at {path}"
        );
        aggregate = aggregate
            .checked_add(expected.maximum_byte_length)
            .ok_or_else(|| anyhow::anyhow!("authority typed closure byte size overflow"))?;
        anyhow::ensure!(
            aggregate <= MAX_AUTHORITY_AGGREGATE_BYTES,
            "authority typed closure exceeds its aggregate byte limit"
        );
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
    }
    anyhow::ensure!(
        files.len().saturating_add(directories.len()) <= MAX_AUTHORITY_ENTRIES,
        "authority typed closure exceeds its topology-entry limit"
    );
    anyhow::ensure!(
        files.keys().all(|path| !directories.contains(path)),
        "authority typed closure contains a file/directory collision"
    );
    Ok(ExpectedTreeV2 { directories, files })
}

fn scan_candidate_tree(root: &Dir, expected: &ExpectedTreeV2) -> anyhow::Result<ScannedTreeV2> {
    scan_tree(root, true, true, expected)
}

fn scan_read_only_raw_tree(root: &Dir, expected: &ExpectedTreeV2) -> anyhow::Result<ScannedTreeV2> {
    // `validate_raw_tree` reaches this scanner only for the fixed Demo/Full
    // roots after the candidate's declarations have selected those exact
    // paths. The deployment gate pins each root as one read-only bind mount.
    // The scanner may therefore accept that root mount while retaining the
    // existing strict mount-id rejection for every descendant and file.
    scan_tree(root, false, true, expected)
}

fn scan_tree(
    root: &Dir,
    executable_modes: bool,
    allow_root_mount: bool,
    expected: &ExpectedTreeV2,
) -> anyhow::Result<ScannedTreeV2> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as _;
        let root_metadata = root.dir_metadata()?;
        let root_mount_id = authority_mount_id_with_policy(root, allow_root_mount)?;
        anyhow::ensure!(
            root_metadata.uid() == rustix::process::geteuid().as_raw()
                && root_metadata.mode() & 0o7777 == 0o550,
            "authority root has the wrong owner or mode"
        );
        let root_device = root_metadata.dev();
        let mut scanned = ScannedTreeV2 {
            directories: BTreeMap::from([(".".to_owned(), 0o550)]),
            files: BTreeMap::new(),
            authenticated_semantic_files: BTreeMap::new(),
        };
        scan_directories_iterative(
            root.try_clone()?,
            root_device,
            root_mount_id,
            executable_modes,
            expected,
            &mut scanned,
        )?;
        let actual_directories = scanned
            .directories
            .keys()
            .map(|path| path.trim_end_matches('/'))
            .map(|path| if path == "." { "" } else { path })
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            actual_directories == expected.directories.iter().map(String::as_str).collect()
                && scanned.files.keys().eq(expected.files.keys()),
            "authority tree differs from its typed closure"
        );
        Ok(scanned)
    }
    #[cfg(not(unix))]
    anyhow::bail!("authority tree scanning requires Unix metadata")
}

#[cfg(unix)]
fn scan_directories_iterative(
    root: Dir,
    root_device: u64,
    root_mount_id: u64,
    executable_modes: bool,
    expected: &ExpectedTreeV2,
    scanned: &mut ScannedTreeV2,
) -> anyhow::Result<()> {
    use cap_std::fs::MetadataExt as _;
    let mut pending = vec![(root, PathBuf::new(), 0_usize)];
    let mut visited = 1_usize;
    let mut aggregate = 0_u64;
    let mut semantic_aggregate = 0_u64;
    while let Some((directory, relative_root, depth)) = pending.pop() {
        anyhow::ensure!(
            depth <= MAX_AUTHORITY_TREE_DEPTH,
            "authority tree exceeds its depth limit"
        );
        for entry in directory.entries()? {
            let entry = entry?;
            visited = visited
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("authority topology count overflow"))?;
            anyhow::ensure!(
                visited <= MAX_AUTHORITY_ENTRIES
                    && visited <= expected.files.len() + expected.directories.len(),
                "authority tree exceeds its typed topology-entry limit"
            );
            let name = entry.file_name();
            anyhow::ensure!(
                !name.as_encoded_bytes().is_empty()
                    && name.as_encoded_bytes().len() <= MAX_AUTHORITY_COMPONENT_BYTES,
                "authority tree contains an oversized name"
            );
            let relative = relative_root.join(&name);
            let path = path_to_manifest(&relative)?;
            validate_authority_relative_path(&path)?;
            let file_type = entry.file_type()?;
            anyhow::ensure!(
                file_type.is_dir() || file_type.is_file(),
                "authority tree contains a symlink or special node"
            );
            let listed = entry.metadata()?;
            anyhow::ensure!(
                listed.dev() == root_device && listed.uid() == rustix::process::geteuid().as_raw(),
                "authority tree crosses a device or owner boundary"
            );
            if file_type.is_dir() {
                anyhow::ensure!(
                    expected.directories.contains(&path),
                    "authority tree contains a directory outside its typed closure"
                );
                anyhow::ensure!(
                    listed.mode() & 0o7777 == 0o550,
                    "authority directory mode is not 0550"
                );
                let child = open_candidate_directory(&directory, Path::new(&name))?;
                let opened = child.dir_metadata()?;
                anyhow::ensure!(
                    listed.dev() == opened.dev() && listed.ino() == opened.ino(),
                    "authority directory was substituted while opening"
                );
                anyhow::ensure!(
                    authority_mount_id(&child)? == root_mount_id,
                    "authority tree contains a nested mount"
                );
                anyhow::ensure!(
                    scanned
                        .directories
                        .insert(format!("{path}/"), 0o550)
                        .is_none(),
                    "authority tree repeats a directory"
                );
                pending.push((child, relative, depth + 1));
            } else {
                anyhow::ensure!(
                    file_type.is_file() && listed.is_file(),
                    "authority tree contains a symlink or special node"
                );
                let expected_file = expected.files.get(&path).ok_or_else(|| {
                    anyhow::anyhow!("authority tree contains a file outside its typed closure")
                })?;
                let mut file = open_candidate_file(&directory, Path::new(&name))?;
                let opened = file.metadata()?;
                use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
                anyhow::ensure!(
                    listed.dev() == opened.dev()
                        && listed.ino() == opened.ino()
                        && opened.nlink() == 1,
                    "authority file was substituted or hard-linked"
                );
                anyhow::ensure!(
                    authority_file_mount_id(&file)? == root_mount_id,
                    "authority tree contains a mounted file"
                );
                let expected_mode = if executable_modes {
                    canonical_file_mode(&path)
                } else {
                    0o440
                };
                anyhow::ensure!(
                    opened.permissions().mode() & 0o7777 == expected_mode,
                    "authority file has a noncanonical mode at {path}"
                );
                let capture_semantic = executable_modes && is_candidate_semantic_path(&path);
                if capture_semantic {
                    semantic_aggregate =
                        checked_authenticated_semantic_bytes(semantic_aggregate, opened.len())?;
                }
                let (scanned_file, authenticated_bytes) = hash_file_bounded(
                    &mut file,
                    expected_file.maximum_byte_length,
                    expected_file.exact_byte_length,
                    capture_semantic,
                )?;
                let scanned_file = scanned_file.with_mode(expected_mode);
                aggregate = aggregate
                    .checked_add(scanned_file.byte_length)
                    .ok_or_else(|| anyhow::anyhow!("authority scanned byte size overflow"))?;
                anyhow::ensure!(
                    aggregate <= MAX_AUTHORITY_AGGREGATE_BYTES,
                    "authority tree exceeds its aggregate byte limit"
                );
                let authenticated_path = authenticated_bytes.as_ref().map(|_| path.clone());
                anyhow::ensure!(
                    scanned.files.insert(path, scanned_file).is_none(),
                    "authority tree repeats a file"
                );
                if let Some(bytes) = authenticated_bytes {
                    let path = authenticated_path
                        .ok_or_else(|| anyhow::anyhow!("authenticated file path vanished"))?;
                    anyhow::ensure!(
                        scanned
                            .authenticated_semantic_files
                            .insert(path, bytes)
                            .is_none(),
                        "authority tree repeats authenticated semantic bytes"
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn authority_mount_id(directory: &Dir) -> anyhow::Result<u64> {
    authority_mount_id_with_policy(directory, false)
}

#[cfg(target_os = "linux")]
fn authority_mount_id_with_policy(directory: &Dir, allow_mount_root: bool) -> anyhow::Result<u64> {
    use rustix::fs::{AtFlags, StatxAttributes, StatxFlags, statx};
    use std::os::fd::AsFd as _;
    let stat = statx(
        directory.as_fd(),
        Path::new(""),
        AtFlags::EMPTY_PATH | AtFlags::NO_AUTOMOUNT,
        StatxFlags::MNT_ID,
    )?;
    anyhow::ensure!(
        StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID),
        "kernel did not report candidate mount identity"
    );
    if !allow_mount_root {
        anyhow::ensure!(
            !stat
                .stx_attributes_mask
                .contains(StatxAttributes::MOUNT_ROOT)
                || !stat.stx_attributes.contains(StatxAttributes::MOUNT_ROOT),
            "authority directory is itself a mount root"
        );
    }
    Ok(stat.stx_mnt_id)
}

#[cfg(target_os = "linux")]
fn authority_file_mount_id(file: &File) -> anyhow::Result<u64> {
    use rustix::fs::{AtFlags, StatxFlags, statx};
    let stat = statx(
        file,
        Path::new(""),
        AtFlags::EMPTY_PATH | AtFlags::NO_AUTOMOUNT,
        StatxFlags::MNT_ID,
    )?;
    anyhow::ensure!(
        StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID),
        "kernel did not report authority file mount identity"
    );
    Ok(stat.stx_mnt_id)
}

impl ScannedFileV2 {
    fn with_mode(mut self, unix_mode: u32) -> Self {
        self.unix_mode = unix_mode;
        self
    }
}

fn hash_file_bounded(
    file: &mut File,
    maximum: u64,
    exact: Option<u64>,
    capture_bytes: bool,
) -> anyhow::Result<(ScannedFileV2, Option<Vec<u8>>)> {
    anyhow::ensure!(
        maximum <= MAX_AUTHORITY_FILE_BYTES && exact.is_none_or(|value| value <= maximum),
        "authority file declares an invalid byte bound"
    );
    let initial_length = file.metadata()?.len();
    anyhow::ensure!(
        initial_length <= maximum && exact.is_none_or(|value| initial_length == value),
        "authority file metadata exceeds or differs from its byte bound"
    );
    let mut hasher = Sha256::new();
    let mut byte_length = 0_u64;
    let mut captured =
        capture_bytes.then(|| Vec::with_capacity(usize::try_from(initial_length).unwrap_or(0)));
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(u64::try_from(count)?)
            .ok_or_else(|| anyhow::anyhow!("authority file size overflow"))?;
        anyhow::ensure!(
            byte_length <= maximum,
            "authority file exceeds its byte limit"
        );
        hasher.update(&buffer[..count]);
        if let Some(captured) = &mut captured {
            captured.extend_from_slice(&buffer[..count]);
        }
    }
    anyhow::ensure!(
        byte_length == initial_length && exact.is_none_or(|expected| byte_length == expected),
        "authority file changed length or differs from its exact byte length"
    );
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = file.metadata()?;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    Ok((
        ScannedFileV2 {
            sha256: Digest32::from_bytes(hasher.finalize().into()),
            byte_length,
            unix_mode: 0,
            device,
            inode,
        },
        captured,
    ))
}

fn open_candidate_file(root: &Dir, relative: &Path) -> anyhow::Result<File> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        let descriptor = openat2(
            root.as_fd(),
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let file = File::from(descriptor);
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "candidate path is not a regular file"
        );
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("candidate authority reads require Linux openat2")
}

fn open_candidate_directory(root: &Dir, relative: &Path) -> anyhow::Result<Dir> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;
        let descriptor = openat2(
            root.as_fd(),
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let file = File::from(descriptor);
        anyhow::ensure!(
            file.metadata()?.is_dir(),
            "candidate path is not a directory"
        );
        Ok(Dir::from_std_file(file))
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("candidate authority reads require Linux openat2")
}

fn read_candidate_file(root: &Dir, relative: &Path, maximum: u64) -> anyhow::Result<Vec<u8>> {
    let file = open_candidate_file(root, relative)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.len() <= maximum,
        "candidate file exceeds its byte limit"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= maximum,
        "candidate file grew while reading"
    );
    Ok(bytes)
}

fn authenticated_candidate_file<'a>(
    tree: &'a ScannedTreeV2,
    relative: &str,
    maximum: u64,
) -> anyhow::Result<&'a [u8]> {
    let scanned = tree
        .files
        .get(relative)
        .ok_or_else(|| anyhow::anyhow!("authenticated candidate omits {relative}"))?;
    let bytes = tree
        .authenticated_semantic_files
        .get(relative)
        .ok_or_else(|| {
            anyhow::anyhow!("candidate semantic bytes were not retained for {relative}")
        })?;
    anyhow::ensure!(
        bytes.len() <= usize::try_from(maximum)?
            && u64::try_from(bytes.len())? == scanned.byte_length
            && Digest32::digest_bytes(bytes) == scanned.sha256,
        "retained candidate semantic bytes differ from authenticated inventory at {relative}"
    );
    Ok(bytes)
}

fn validate_authenticated_semantic_closure(tree: &ScannedTreeV2) -> anyhow::Result<()> {
    let expected_paths = tree
        .files
        .keys()
        .filter(|path| is_candidate_semantic_path(path))
        .cloned()
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        tree.authenticated_semantic_files
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            == expected_paths,
        "retained candidate semantic paths differ from authenticated inventory"
    );
    let mut aggregate = 0_u64;
    for path in expected_paths {
        let bytes = authenticated_candidate_file(tree, &path, MAX_AUTHORITY_FILE_BYTES)?;
        aggregate = checked_authenticated_semantic_bytes(aggregate, u64::try_from(bytes.len())?)?;
    }
    Ok(())
}

fn checked_authenticated_semantic_bytes(current: u64, next: u64) -> anyhow::Result<u64> {
    let total = current
        .checked_add(next)
        .ok_or_else(|| anyhow::anyhow!("candidate semantic byte size overflow"))?;
    anyhow::ensure!(
        total <= MAX_AUTHENTICATED_SEMANTIC_BYTES,
        "candidate semantic authority exceeds its retained-byte limit"
    );
    Ok(total)
}

fn is_candidate_semantic_path(path: &str) -> bool {
    matches!(
        path,
        RELEASE_MANIFEST
            | "MODE_INVENTORY"
            | "SHA256SUMS"
            | "SOURCE_COMMIT"
            | "private/raw-root-declarations-v2.json"
    ) || path.starts_with("config/")
        || path.starts_with("systemd/user/")
        || path.starts_with("private/campaign-states/")
        || path.starts_with("private/source-tree-manifests-v2/")
        || path.starts_with("private/verifier/operator-config/")
}

fn read_private_secret(path: &Path, expected_length: u64) -> anyhow::Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let parent = path.parent().context("secret has no parent")?;
        let name = path.file_name().context("secret has no filename")?;
        let parent_fd = openat2(
            rustix::fs::CWD,
            parent,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let parent_file = File::from(parent_fd);
        let parent_metadata = parent_file.metadata()?;
        anyhow::ensure!(
            parent_metadata.is_dir()
                && parent_metadata.uid() == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o7777 == 0o700,
            "secret parent must be effective-user-owned mode 0700"
        );
        let fd = openat2(
            &parent_file,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let mut file = File::from(fd);
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.nlink() == 1
                && metadata.permissions().mode() & 0o7777 == 0o400
                && metadata.len() == expected_length,
            "secret must be an exact owner-only, single-link regular file"
        );
        let mut bytes = Vec::with_capacity(usize::try_from(expected_length)?);
        file.read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 == expected_length,
            "secret changed length while reading"
        );
        Ok(bytes)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("runtime secret validation requires Linux openat2")
}

fn validate_secret_absent(path: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{AtFlags, Mode, OFlags, ResolveFlags, openat2, statat};
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let parent = path.parent().context("secret has no parent")?;
        let name = path.file_name().context("secret has no filename")?;
        let parent_fd = openat2(
            rustix::fs::CWD,
            parent,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        let parent_metadata = File::from(parent_fd.try_clone()?).metadata()?;
        anyhow::ensure!(
            parent_metadata.is_dir()
                && parent_metadata.uid() == rustix::process::geteuid().as_raw()
                && parent_metadata.permissions().mode() & 0o7777 == 0o700,
            "secret parent must be effective-user-owned mode 0700"
        );
        match statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Ok(_) => anyhow::bail!("backup authority must be absent for clean-host probing"),
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("runtime secret validation requires Linux openat2")
}

fn open_ambient_directory(path: &Path) -> anyhow::Result<File> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        let fd = openat2(
            rustix::fs::CWD,
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        Ok(File::from(fd))
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("raw authority validation requires Linux openat2")
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

fn valid_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && Path::new(value).to_str() == Some(value)
}

fn validate_nonzero_digest(value: &str, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.bytes().any(|byte| byte != b'0'),
        "{label} is not canonical nonzero lowercase SHA-256"
    );
    Ok(())
}

fn parse_digest(value: &str, label: &str) -> anyhow::Result<Digest32> {
    validate_nonzero_digest(value, label)?;
    Ok(value.parse()?)
}

fn path_to_manifest(path: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(
        !path.as_os_str().is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "authority tree contains an unsafe path"
    );
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("authority tree path is not UTF-8"))
}

fn validate_authority_relative_path(path: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        path.len() <= MAX_AUTHORITY_RELATIVE_PATH_BYTES,
        "authority tree path exceeds its byte limit"
    );
    let components = Path::new(path).components().collect::<Vec<_>>();
    anyhow::ensure!(
        !components.is_empty()
            && components.len() <= MAX_AUTHORITY_TREE_DEPTH
            && components.iter().all(|component| {
                matches!(component, Component::Normal(value) if !value.as_encoded_bytes().is_empty() && value.as_encoded_bytes().len() <= MAX_AUTHORITY_COMPONENT_BYTES)
            }),
        "authority tree path is invalid or exceeds its component limits"
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    struct CandidateFixture {
        temporary: tempfile::TempDir,
        manifest_digest: String,
    }

    impl CandidateFixture {
        fn root(&self) -> &Path {
            self.temporary.path()
        }

        fn root_file(&self) -> File {
            File::open(self.root()).unwrap()
        }

        fn self_file(&self) -> File {
            File::open(self.root().join("bin/robin-highscores-admin")).unwrap()
        }
    }

    fn artifact(bytes: &[u8]) -> ArtifactRefV1 {
        ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: bytes.len() as u64,
            media_type: "application/octet-stream".to_owned(),
        }
    }

    #[test]
    fn competition_grant_authority_distinguishes_dormant_and_active_registries() {
        let installed = [0x31; 32];
        let substituted = [0x42; 32];

        assert!(competition_grant_authority_matches(
            false,
            std::iter::empty(),
            &installed,
        ));
        assert!(!competition_grant_authority_matches(
            false,
            [&installed].into_iter(),
            &installed,
        ));
        assert!(!competition_grant_authority_matches(
            true,
            std::iter::empty(),
            &installed,
        ));
        assert!(competition_grant_authority_matches(
            true,
            [&installed, &installed].into_iter(),
            &installed,
        ));
        assert!(!competition_grant_authority_matches(
            true,
            [&installed, &substituted].into_iter(),
            &installed,
        ));
    }

    fn fixture(database_schema_version: i64) -> CandidateFixture {
        let temporary = tempfile::tempdir().unwrap();
        let payloads = [
            "bin/robin-highscores-admin",
            "bin/robin-highscores-manifestctl",
            "bin/robin-highscores-server",
            "bin/robin-highscores-worker",
            "bin/robin-replay-verifier",
            "config/highscores-server.toml",
            "config/highscores-worker.toml",
            "config/api.env",
            "config/worker.env",
            "private/raw-root-declarations-v2.json",
            "systemd/user/robin-highscores.target",
            "systemd/user/robin-highscores-api.service",
            "systemd/user/robin-highscores-worker.service",
            "systemd/user/robin-highscores-backup.service",
            "systemd/user/robin-highscores-backup.timer",
        ];
        let mut files = Vec::new();
        for path in payloads {
            let bytes = format!("fixture:{path}").into_bytes();
            let absolute = temporary.path().join(path);
            std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
            std::fs::write(&absolute, &bytes).unwrap();
            files.push(VpsReleaseFileV2 {
                path: path.to_owned(),
                artifact: artifact(&bytes),
                unix_mode: canonical_file_mode(path),
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let verifier_sha256 = files
            .iter()
            .find(|file| file.path == "bin/robin-replay-verifier")
            .unwrap()
            .artifact
            .sha256;
        let manifest = VpsReleaseManifestV2 {
            schema_version: 2,
            source_commit: "a".repeat(40),
            database_schema_version,
            deployment: VpsDeploymentV2 {
                user: "robinhood".to_owned(),
                home: PathBuf::from("/home/robinhood"),
                install_root: PathBuf::from("/home/robinhood/.local/opt/robin-highscores"),
                persistent_state_root: PathBuf::from(STATE_ROOT),
                current_link: PathBuf::from("/home/robinhood/.local/opt/robin-highscores/current"),
            },
            publication_lock_sha256: Digest32::from_bytes([1; 32]),
            publication_manifest_sha256: Digest32::from_bytes([2; 32]),
            verifier_sha256,
            files,
        };
        let manifest_bytes = canonical_json_bytes(&manifest).unwrap();
        std::fs::write(temporary.path().join(RELEASE_MANIFEST), &manifest_bytes).unwrap();
        std::fs::write(
            temporary.path().join("SOURCE_COMMIT"),
            format!("{}\n", manifest.source_commit),
        )
        .unwrap();

        let mut directories = BTreeSet::from([".".to_owned()]);
        let mut all_files = manifest
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        all_files.extend([
            "MODE_INVENTORY".to_owned(),
            "SHA256SUMS".to_owned(),
            "SOURCE_COMMIT".to_owned(),
            RELEASE_MANIFEST.to_owned(),
        ]);
        for file in &all_files {
            let mut parent = Path::new(file).parent();
            while let Some(path) = parent {
                if path.as_os_str().is_empty() {
                    break;
                }
                directories.insert(format!("{}/", path_to_manifest(path).unwrap()));
                parent = path.parent();
            }
        }
        let mode_bytes = canonical_mode_inventory_bytes(
            directories
                .iter()
                .map(|directory| (directory.clone(), ('d', 0o550))),
            all_files
                .iter()
                .map(|file| (file.clone(), ('f', canonical_file_mode(file)))),
        )
        .unwrap();
        std::fs::write(temporary.path().join("MODE_INVENTORY"), mode_bytes).unwrap();

        let mut sums = Vec::new();
        for file in &all_files {
            if file == "SHA256SUMS" {
                continue;
            }
            let bytes = std::fs::read(temporary.path().join(file)).unwrap();
            writeln!(&mut sums, "{}  {file}", Digest32::digest_bytes(&bytes)).unwrap();
        }
        std::fs::write(temporary.path().join("SHA256SUMS"), sums).unwrap();
        for file in &all_files {
            std::fs::set_permissions(
                temporary.path().join(file),
                std::fs::Permissions::from_mode(canonical_file_mode(file)),
            )
            .unwrap();
        }
        let mut actual_directories = directories
            .iter()
            .filter(|path| path.as_str() != ".")
            .map(|path| temporary.path().join(path.trim_end_matches('/')))
            .collect::<Vec<_>>();
        actual_directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in actual_directories {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o550)).unwrap();
        }
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o550)).unwrap();
        CandidateFixture {
            temporary,
            manifest_digest: hex::encode(Sha256::digest(manifest_bytes)),
        }
    }

    #[test]
    fn candidate_attestation_is_descriptor_pinned_and_exact() {
        let fixture = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION);
        let (attestation, guard) = attest_candidate_release_root_v2_with_self(
            fixture.root_file(),
            &fixture.manifest_digest,
            CandidateSelfRoleV2::Admin,
            fixture.self_file(),
        )
        .unwrap();
        assert_eq!(attestation.source_commit, "a".repeat(40));
        assert_eq!(
            attestation.vps_release_manifest_sha256,
            fixture.manifest_digest
        );
        assert!(guard.metadata().unwrap().is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn authority_scans_allow_exact_root_binds_but_reject_descendant_mounts() {
        use std::os::fd::AsRawFd as _;
        use std::process::Command;

        const CHILD: &str = "ROBIN_RUNTIME_AUTHORITY_ROOT_MOUNT_CHILD";
        const NESTED: &str = "ROBIN_RUNTIME_AUTHORITY_NESTED_MOUNT";
        const CANDIDATE: &str = "/tmp/robin-runtime-authority-candidate";
        const TEST_NAME: &str = "runtime_authority::tests::authority_scans_allow_exact_root_binds_but_reject_descendant_mounts";

        let expected =
            expected_tree_from_files([("nested/payload".to_owned(), expected_file(3))]).unwrap();
        if std::env::var_os(CHILD).is_some() {
            let root = Dir::from_std_file(File::open(CANDIDATE).unwrap());
            let candidate_result = scan_candidate_tree(&root, &expected);
            let root = Dir::from_std_file(File::open(CANDIDATE).unwrap());
            let raw_result = scan_read_only_raw_tree(&root, &expected);
            if std::env::var_os(NESTED).is_some() {
                assert!(
                    candidate_result.is_err(),
                    "candidate scan accepted a descendant mount"
                );
                assert!(raw_result.is_err(), "raw scan accepted a descendant mount");
            } else {
                candidate_result
                    .expect("candidate scan rejected its exact descriptor-bound root mount");
                raw_result.expect("raw scan rejected its exact descriptor-bound root mount");
            }
            return;
        }

        let candidate_root = tempfile::tempdir().unwrap();
        let nested = candidate_root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("payload"), b"one").unwrap();
        let nested_source = tempfile::tempdir().unwrap();
        std::fs::write(nested_source.path().join("payload"), b"one").unwrap();
        for path in [nested.join("payload"), nested_source.path().join("payload")] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o440)).unwrap();
        }
        for path in [candidate_root.path(), &nested, nested_source.path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o550)).unwrap();
        }

        let executable = File::open(std::env::current_exe().unwrap()).unwrap();
        let candidate = File::open(candidate_root.path()).unwrap();
        let nested_mount = File::open(nested_source.path()).unwrap();
        for fd in [
            executable.as_raw_fd(),
            candidate.as_raw_fd(),
            nested_mount.as_raw_fd(),
        ] {
            let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD).unwrap();
            let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
            flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
            nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags)).unwrap();
        }

        for with_nested_mount in [false, true] {
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
                    "/tmp/robin-runtime-authority-test",
                ])
                .args(["--dir", CANDIDATE])
                .args([
                    "--ro-bind-fd",
                    &candidate.as_raw_fd().to_string(),
                    CANDIDATE,
                ]);
            if with_nested_mount {
                command.args([
                    "--ro-bind-fd",
                    &nested_mount.as_raw_fd().to_string(),
                    &format!("{CANDIDATE}/nested"),
                ]);
            }
            command
                .arg("/tmp/robin-runtime-authority-test")
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD, "1");
            if with_nested_mount {
                command.env(NESTED, "1");
            }
            assert!(
                command.status().unwrap().success(),
                "real bwrap authority-root mount policy regression failed"
            );
        }

        for path in [&nested, candidate_root.path(), nested_source.path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750)).unwrap();
        }
    }

    #[test]
    fn release_verifier_identity_rejects_top_level_and_bundled_substitutions() {
        let fixture = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION);
        let bytes = std::fs::read(fixture.root().join(RELEASE_MANIFEST)).unwrap();
        let manifest: VpsReleaseManifestV2 = serde_json::from_slice(&bytes).unwrap();
        validate_vps_manifest(&manifest).unwrap();

        let mut substituted_top = manifest.clone();
        substituted_top.verifier_sha256 = Digest32::from_bytes([0x91; 32]);
        assert!(validate_vps_manifest(&substituted_top).is_err());

        let mut substituted_bundle = manifest;
        substituted_bundle
            .files
            .iter_mut()
            .find(|file| file.path == "bin/robin-replay-verifier")
            .unwrap()
            .artifact
            .sha256 = Digest32::from_bytes([0x92; 32]);
        assert!(validate_vps_manifest(&substituted_bundle).is_err());
    }

    #[test]
    fn runtime_verifier_identity_rejects_worker_and_build_substitutions() {
        let release = Digest32::from_bytes([0x71; 32]);
        validate_runtime_verifier_binding(release, release, release).unwrap();
        assert!(
            validate_runtime_verifier_binding(release, Digest32::from_bytes([0x72; 32]), release,)
                .is_err()
        );
        assert!(
            validate_runtime_verifier_binding(release, release, Digest32::from_bytes([0x73; 32]),)
                .is_err()
        );
    }

    #[test]
    fn authenticated_semantic_bytes_defeat_transient_same_uid_aba() {
        let temporary = tempfile::tempdir().unwrap();
        let config = temporary.path().join("config");
        std::fs::create_dir(&config).unwrap();
        let authority = config.join("authority");
        let parked = config.join("authority.parked");
        let original = b"authority-a";
        let transient = b"transient-b";
        assert_eq!(original.len(), transient.len());
        std::fs::write(&authority, original).unwrap();
        std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o440)).unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o550)).unwrap();
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o550)).unwrap();
        let expected = expected_tree_from_files([(
            "config/authority".to_owned(),
            expected_file(original.len() as u64),
        )])
        .unwrap();
        let root = Dir::from_std_file(File::open(temporary.path()).unwrap());
        let before = scan_candidate_tree(&root, &expected).unwrap();
        validate_authenticated_semantic_closure(&before).unwrap();

        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::rename(&authority, &parked).unwrap();
        std::fs::write(&authority, transient).unwrap();
        std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o440)).unwrap();
        assert_eq!(
            authenticated_candidate_file(&before, "config/authority", 1024).unwrap(),
            original
        );
        std::fs::remove_file(&authority).unwrap();
        std::fs::rename(&parked, &authority).unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o550)).unwrap();

        let root = Dir::from_std_file(File::open(temporary.path()).unwrap());
        let after = scan_candidate_tree(&root, &expected).unwrap();
        assert_eq!(before, after);
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    }

    #[test]
    fn candidate_attestation_rejects_future_schema_extra_files_and_hardlinks() {
        let future = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION + 1);
        assert!(
            attest_candidate_release_root_v2_with_self(
                future.root_file(),
                &future.manifest_digest,
                CandidateSelfRoleV2::Admin,
                future.self_file(),
            )
            .is_err()
        );

        let extra = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION);
        std::fs::set_permissions(extra.root(), std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::write(extra.root().join("extra"), b"extra").unwrap();
        std::fs::set_permissions(
            extra.root().join("extra"),
            std::fs::Permissions::from_mode(0o440),
        )
        .unwrap();
        std::fs::set_permissions(extra.root(), std::fs::Permissions::from_mode(0o550)).unwrap();
        assert!(
            attest_candidate_release_root_v2_with_self(
                extra.root_file(),
                &extra.manifest_digest,
                CandidateSelfRoleV2::Admin,
                extra.self_file(),
            )
            .is_err()
        );

        let linked = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION);
        std::fs::set_permissions(
            linked.root().join("bin"),
            std::fs::Permissions::from_mode(0o750),
        )
        .unwrap();
        std::fs::hard_link(
            linked.root().join("bin/robin-highscores-admin"),
            linked.root().join("bin/alias"),
        )
        .unwrap();
        std::fs::set_permissions(
            linked.root().join("bin"),
            std::fs::Permissions::from_mode(0o550),
        )
        .unwrap();
        assert!(
            attest_candidate_release_root_v2_with_self(
                linked.root_file(),
                &linked.manifest_digest,
                CandidateSelfRoleV2::Admin,
                linked.self_file(),
            )
            .is_err()
        );
    }

    fn expected_file(maximum_byte_length: u64) -> ExpectedFileV2 {
        ExpectedFileV2 {
            maximum_byte_length,
            exact_byte_length: Some(maximum_byte_length),
        }
    }

    #[test]
    fn typed_tree_closure_enforces_count_depth_name_path_and_byte_bounds() {
        let excessive_count = (0..=MAX_AUTHORITY_ENTRIES)
            .map(|index| (format!("file-{index:06}"), expected_file(0)))
            .collect::<Vec<_>>();
        assert!(expected_tree_from_files(excessive_count).is_err());

        let excessive_depth = format!("{}leaf", "directory/".repeat(MAX_AUTHORITY_TREE_DEPTH));
        assert!(expected_tree_from_files([(excessive_depth, expected_file(0))]).is_err());

        let excessive_name = format!("{}x", "n".repeat(MAX_AUTHORITY_COMPONENT_BYTES));
        assert!(expected_tree_from_files([(excessive_name, expected_file(0))]).is_err());

        let excessive_path = format!(
            "{}/leaf",
            (0..17)
                .map(|_| "p".repeat(250))
                .collect::<Vec<_>>()
                .join("/")
        );
        assert!(excessive_path.len() > MAX_AUTHORITY_RELATIVE_PATH_BYTES);
        assert!(expected_tree_from_files([(excessive_path, expected_file(0))]).is_err());

        assert!(
            expected_tree_from_files([(
                "huge".to_owned(),
                expected_file(MAX_AUTHORITY_FILE_BYTES + 1),
            )])
            .is_err()
        );
        let excessive_aggregate = (0..=MAX_AUTHORITY_AGGREGATE_BYTES / MAX_AUTHORITY_FILE_BYTES)
            .map(|index| {
                (
                    format!("aggregate-{index}"),
                    expected_file(MAX_AUTHORITY_FILE_BYTES),
                )
            })
            .collect::<Vec<_>>();
        assert!(expected_tree_from_files(excessive_aggregate).is_err());
    }

    #[test]
    fn candidate_tree_types_the_optional_empty_competition_registry_only() {
        let fixture = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION);
        let manifest_bytes = std::fs::read(fixture.root().join(RELEASE_MANIFEST)).unwrap();
        let mut manifest: VpsReleaseManifestV2 = serde_json::from_slice(&manifest_bytes).unwrap();
        manifest.files.push(VpsReleaseFileV2 {
            path: "config/manifests/builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json".to_owned(),
            artifact: artifact(b"build"),
            unix_mode: 0o440,
        });
        manifest
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));

        let expected = candidate_expected_tree(&manifest, manifest_bytes.len()).unwrap();
        assert!(
            expected
                .directories
                .contains("config/manifests/competitions")
        );
        assert!(!expected.directories.contains("config/manifests/optional"));

        manifest
            .files
            .retain(|file| !file.path.starts_with("config/manifests/"));
        let without_registry = candidate_expected_tree(&manifest, manifest_bytes.len()).unwrap();
        assert!(
            !without_registry
                .directories
                .contains("config/manifests/competitions")
        );
    }

    #[test]
    fn candidate_mode_inventory_globally_sorts_serialized_paths() {
        let bytes = canonical_mode_inventory_bytes(
            [
                (".".to_owned(), ('d', 0o550)),
                ("bin/".to_owned(), ('d', 0o550)),
                ("private/verifier/".to_owned(), ('d', 0o550)),
            ],
            [
                ("MODE_INVENTORY".to_owned(), ('f', 0o440)),
                ("SHA256SUMS".to_owned(), ('f', 0o440)),
                ("bin/robin-highscores-admin".to_owned(), ('f', 0o550)),
                ("private/verifier-bundles/payload".to_owned(), ('f', 0o440)),
            ],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            concat!(
                "d 0550  .\n",
                "f 0440  MODE_INVENTORY\n",
                "f 0440  SHA256SUMS\n",
                "d 0550  bin/\n",
                "f 0550  bin/robin-highscores-admin\n",
                "f 0440  private/verifier-bundles/payload\n",
                "d 0550  private/verifier/\n",
            )
        );
        assert!(
            canonical_mode_inventory_bytes(
                [("collision".to_owned(), ('d', 0o550))],
                [("collision".to_owned(), ('f', 0o440))],
            )
            .is_err()
        );
    }

    #[test]
    fn semantic_retention_matches_runtime_consumers_and_keeps_its_fixed_cap() {
        for retained in [
            RELEASE_MANIFEST,
            "MODE_INVENTORY",
            "SHA256SUMS",
            "SOURCE_COMMIT",
            "config/api.env",
            "config/worker.env",
            "config/highscores-server.toml",
            "config/highscores-worker.toml",
            "config/manifests/builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/content-manifests/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/campaign-content-manifests/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/rules-configs/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/ruleset-manifests/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/published-rulesets/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/competitions/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "config/manifests/policies/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "systemd/user/robin-highscores.target",
            "systemd/user/robin-highscores-api.service",
            "systemd/user/robin-highscores-worker.service",
            "systemd/user/robin-highscores-backup.service",
            "systemd/user/robin-highscores-backup.timer",
            "private/raw-root-declarations-v2.json",
            "private/campaign-states/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bin",
            "private/source-tree-manifests-v2/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
            "private/verifier/operator-config/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(
                is_candidate_semantic_path(retained),
                "runtime consumer path is not retained: {retained}"
            );
        }
        for streamed_only in [
            "private/verifier-bundles/aaaaaaaa/catalog/field-missions/map_geometry_metadata.json",
            "private/official-content-authority/public/content/object",
            "private/verifier/bin/verifier",
        ] {
            assert!(
                !is_candidate_semantic_path(streamed_only),
                "unused large private payload is retained: {streamed_only}"
            );
        }

        assert_eq!(
            checked_authenticated_semantic_bytes(MAX_AUTHENTICATED_SEMANTIC_BYTES - 1, 1).unwrap(),
            MAX_AUTHENTICATED_SEMANTIC_BYTES
        );
        assert!(checked_authenticated_semantic_bytes(MAX_AUTHENTICATED_SEMANTIC_BYTES, 1).is_err());
        assert!(checked_authenticated_semantic_bytes(u64::MAX, 1).is_err());
        assert_eq!(MAX_AUTHENTICATED_SEMANTIC_BYTES, 512 * 1024 * 1024);
    }

    #[test]
    fn semantic_retention_captures_campaign_state_but_streams_large_private_payloads() {
        let temporary = tempfile::tempdir().unwrap();
        let campaign_relative = "private/campaign-states/campaign.bin";
        let streamed_relative = "private/verifier-bundles/content/payload.bin";
        for (relative, bytes) in [
            (campaign_relative, b"campaign"),
            (streamed_relative, b"bundle!!"),
        ] {
            let path = temporary.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
        }
        let mut directories = [
            "private",
            "private/campaign-states",
            "private/verifier-bundles",
            "private/verifier-bundles/content",
        ]
        .map(|relative| temporary.path().join(relative));
        for directory in directories.iter().rev() {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o550)).unwrap();
        }
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o550)).unwrap();

        let expected = expected_tree_from_files([
            (campaign_relative.to_owned(), expected_file(8)),
            (streamed_relative.to_owned(), expected_file(8)),
        ])
        .unwrap();
        let root = Dir::from_std_file(File::open(temporary.path()).unwrap());
        let tree = scan_candidate_tree(&root, &expected).unwrap();
        validate_authenticated_semantic_closure(&tree).unwrap();
        assert_eq!(
            authenticated_candidate_file(&tree, campaign_relative, 8).unwrap(),
            b"campaign"
        );
        assert!(
            !tree
                .authenticated_semantic_files
                .contains_key(streamed_relative)
        );

        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        for directory in &mut directories {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o750)).unwrap();
        }
    }

    #[test]
    fn raw_tree_rejects_unexpected_entries_and_oversized_files_before_streaming() {
        use std::os::unix::fs::symlink;

        let unexpected = tempfile::tempdir().unwrap();
        std::fs::write(unexpected.path().join("expected"), b"x").unwrap();
        std::fs::write(unexpected.path().join("unexpected"), b"x").unwrap();
        for path in [
            unexpected.path().join("expected"),
            unexpected.path().join("unexpected"),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o440)).unwrap();
        }
        std::fs::set_permissions(unexpected.path(), std::fs::Permissions::from_mode(0o550))
            .unwrap();
        let expected =
            expected_tree_from_files([("expected".to_owned(), expected_file(1))]).unwrap();
        let directory = Dir::from_std_file(File::open(unexpected.path()).unwrap());
        assert!(scan_read_only_raw_tree(&directory, &expected).is_err());
        std::fs::set_permissions(unexpected.path(), std::fs::Permissions::from_mode(0o750))
            .unwrap();

        let oversized = tempfile::tempdir().unwrap();
        let oversized_path = oversized.path().join("payload");
        let file = File::create(&oversized_path).unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();
        drop(file);
        std::fs::set_permissions(&oversized_path, std::fs::Permissions::from_mode(0o440)).unwrap();
        std::fs::set_permissions(oversized.path(), std::fs::Permissions::from_mode(0o550)).unwrap();
        let expected =
            expected_tree_from_files([("payload".to_owned(), expected_file(1))]).unwrap();
        let directory = Dir::from_std_file(File::open(oversized.path()).unwrap());
        assert!(scan_read_only_raw_tree(&directory, &expected).is_err());
        std::fs::set_permissions(oversized.path(), std::fs::Permissions::from_mode(0o750)).unwrap();

        let linked = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"x").unwrap();
        symlink(outside.path(), linked.path().join("payload")).unwrap();
        std::fs::set_permissions(linked.path(), std::fs::Permissions::from_mode(0o550)).unwrap();
        let expected =
            expected_tree_from_files([("payload".to_owned(), expected_file(1))]).unwrap();
        let directory = Dir::from_std_file(File::open(linked.path()).unwrap());
        assert!(scan_read_only_raw_tree(&directory, &expected).is_err());
        std::fs::set_permissions(linked.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
    }

    #[test]
    fn candidate_attestation_rejects_a_different_executable_inode() {
        let fixture = fixture(HIGHSCORES_DATABASE_SCHEMA_VERSION);
        let other = tempfile::tempfile().unwrap();
        assert!(
            attest_candidate_release_root_v2_with_self(
                fixture.root_file(),
                &fixture.manifest_digest,
                CandidateSelfRoleV2::Admin,
                other,
            )
            .is_err()
        );
    }

    #[test]
    fn probe_receipt_is_canonical_and_has_no_trailing_linefeed() {
        let receipt = RuntimeAuthorityProbeV2 {
            backup_authority_state: BackupAuthorityStateV2::Present,
            schema_version: 2,
            source_commit: "a".repeat(40),
            vps_release_manifest_sha256: "b".repeat(64),
        };
        let bytes = canonical_json_bytes(&receipt).unwrap();
        assert!(!bytes.ends_with(b"\n"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({
                "backup_authority_state": "present",
                "schema_version": 2,
                "source_commit": "a".repeat(40),
                "vps_release_manifest_sha256": "b".repeat(64),
            })
        );
    }

    #[test]
    fn secret_probe_enforces_exact_metadata_length_and_absence() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let secret = temporary.path().join("secret");
        std::fs::write(&secret, [7; 32]).unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(read_private_secret(&secret, 32).unwrap(), [7; 32]);
        assert!(read_private_secret(&secret, 31).is_err());
        assert!(validate_secret_absent(&secret).is_err());
        std::fs::remove_file(&secret).unwrap();
        validate_secret_absent(&secret).unwrap();
    }
}
