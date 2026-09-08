//! Private verifier worker-process boundary.
//!
//! Production execution is deliberately Linux-only. Each replay is handled by
//! a fresh, unprivileged bubblewrap sandbox launched through `prlimit`, with
//! sealed executable/input artifacts, private namespaces, an empty root, and
//! hard process resource and wall-time limits.

use crate::model::WorkerJob;
use robin_run_protocol::{
    CanonicalDocument as _, Digest32, MAX_VERIFIER_JOB_CONFIG_BYTES_V1, OpaqueId,
    SCHEMA_VERSION_V1, SignedSubmissionV1, Validate as _, VerificationLimitsV1,
    VerificationRequestV1, VerificationStatusV1, VerifierJobConfigV1, VerifierJobRouteV1,
    VerifierWorkerOutputV1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

const MAX_RESULT_BYTES: u64 = 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_REQUEST_BYTES_HARD: u64 = 16 * 1024 * 1024;
const MAX_ADDRESS_SPACE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_FILE_SIZE_BYTES: u64 = 1024 * 1024 * 1024;
const PRIVATE_TMPFS_BYTES: u64 = 16 * 1024 * 1024;
const RESULT_READ_TIMEOUT: Duration = Duration::from_secs(5);
const BWRAP_PROGRAM: &str = "/usr/bin/bwrap";
const PRLIMIT_PROGRAM: &str = "/usr/bin/prlimit";
// Keep every descriptor below the minimum configured RLIMIT_NOFILE (16).
// bubblewrap consumes the descriptors while constructing the mount namespace,
// leaving the remaining descriptor budget available to the verifier.
const FIRST_SANDBOX_FD: i32 = 3;

/// Serialized direct-launcher policy embedded in `highscores-worker.toml`.
/// Every field is operator authority and unknown fields are rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectVerifierLauncherConfig {
    pub bwrap_program: PathBuf,
    pub bwrap_sha256: String,
    pub prlimit_program: PathBuf,
    pub prlimit_sha256: String,
    pub verifier_program: PathBuf,
    pub verifier_sha256: String,
    pub wall_timeout_seconds: u64,
    pub cpu_limit_seconds: u64,
    pub address_space_limit_bytes: u64,
    pub process_limit: u32,
    pub open_files_limit: u32,
    pub file_size_limit_bytes: u64,
    pub max_request_bytes: u64,
}

impl DirectVerifierLauncherConfig {
    pub fn verifier_digest(&self) -> Result<[u8; 32], ProcessError> {
        parse_digest(&self.verifier_sha256, "verifier_sha256")
    }

    pub fn process_config(
        &self,
        max_campaign_bytes: u64,
    ) -> Result<VerifierProcessConfig, ProcessError> {
        Ok(VerifierProcessConfig {
            bwrap_program: self.bwrap_program.clone(),
            bwrap_sha256: parse_digest(&self.bwrap_sha256, "bwrap_sha256")?,
            prlimit_program: self.prlimit_program.clone(),
            prlimit_sha256: parse_digest(&self.prlimit_sha256, "prlimit_sha256")?,
            verifier_program: self.verifier_program.clone(),
            verifier_sha256: self.verifier_digest()?,
            wall_timeout: Duration::from_secs(self.wall_timeout_seconds),
            cpu_limit_seconds: self.cpu_limit_seconds,
            address_space_limit_bytes: self.address_space_limit_bytes,
            process_limit: self.process_limit,
            open_files_limit: self.open_files_limit,
            file_size_limit_bytes: self.file_size_limit_bytes,
            max_request_bytes: self.max_request_bytes,
            max_campaign_bytes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VerifierProcessConfig {
    pub bwrap_program: PathBuf,
    pub bwrap_sha256: [u8; 32],
    pub prlimit_program: PathBuf,
    pub prlimit_sha256: [u8; 32],
    pub verifier_program: PathBuf,
    pub verifier_sha256: [u8; 32],
    pub wall_timeout: Duration,
    pub cpu_limit_seconds: u64,
    pub address_space_limit_bytes: u64,
    pub process_limit: u32,
    pub open_files_limit: u32,
    pub file_size_limit_bytes: u64,
    pub max_request_bytes: u64,
    pub max_campaign_bytes: u64,
}

#[derive(Debug)]
pub struct VerifierOutput {
    pub outcome: VerifierWorkerOutputV1,
    /// SHA-256 of the exact sealed raw request bytes passed to the sandbox.
    pub request_artifact_sha256: Digest32,
    /// SHA-256 of the canonical per-job authority sealed into this sandbox.
    pub job_config_artifact_sha256: Digest32,
    /// Exact canonical final campaign, present only for a typed Verified result.
    pub final_campaign: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("invalid verifier configuration: {0}")]
    Configuration(String),
    #[error("verifier I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("verifier timed out")]
    Timeout,
    #[error("sandboxed verifier exited unsuccessfully: {0}")]
    Exit(String),
    #[error("verifier result is invalid: {0}")]
    InvalidResult(String),
}

impl ProcessError {
    /// Stable classification for logs. In particular, verifier-authored
    /// result text and host filesystem details remain in private failure
    /// storage rather than the service journal.
    pub const fn safe_log_code(&self) -> &'static str {
        match self {
            Self::Configuration(_) => "verifier_configuration",
            Self::Io(_) => "verifier_io",
            Self::Timeout => "verifier_timeout",
            Self::Exit(_) => "verifier_exit",
            Self::InvalidResult(_) => "verifier_invalid_result",
        }
    }
}

pub fn build_verification_request(
    job: &WorkerJob,
    limits: VerificationLimitsV1,
) -> Result<VerificationRequestV1, ProcessError> {
    let submission: SignedSubmissionV1 = serde_json::from_str(&job.envelope_json)
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    submission
        .validate()
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    let artifacts = &submission.submission.artifacts;
    if artifacts.replay.artifact.sha256.as_bytes() != &job.replay_sha256
        || artifacts.replay.artifact.byte_length != job.replay_bytes
        || artifacts.starting_campaign.sha256.as_bytes() != &job.starting_campaign_sha256
        || artifacts.starting_campaign.byte_length != job.starting_campaign_bytes
    {
        return Err(ProcessError::InvalidResult(
            "stored replay reference differs from signed submission".to_owned(),
        ));
    }
    let request = VerificationRequestV1 {
        schema_version: SCHEMA_VERSION_V1,
        request_id: OpaqueId::new(job.submission_id.clone())
            .map_err(|error| ProcessError::InvalidResult(error.to_string()))?,
        submission,
        limits,
    };
    request
        .validate()
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    Ok(request)
}

impl VerifierProcessConfig {
    pub async fn validate(&self) -> Result<(), ProcessError> {
        #[cfg(not(target_os = "linux"))]
        return Err(ProcessError::Configuration(
            "production verifier containment requires Linux".to_owned(),
        ));

        #[cfg(target_os = "linux")]
        {
            if self.bwrap_program != Path::new(BWRAP_PROGRAM)
                || self.prlimit_program != Path::new(PRLIMIT_PROGRAM)
            {
                return Err(ProcessError::Configuration(
                    "direct verifier launch requires exact /usr/bin/bwrap and /usr/bin/prlimit paths"
                        .to_owned(),
                ));
            }
            validate_absolute_normalized_path(&self.verifier_program, "verifier_program")?;
            drop(sealed_executable(
                &self.bwrap_program,
                self.bwrap_sha256,
                "bwrap_program",
            )?);
            drop(sealed_executable(
                &self.prlimit_program,
                self.prlimit_sha256,
                "prlimit_program",
            )?);
            drop(sealed_executable(
                &self.verifier_program,
                self.verifier_sha256,
                "verifier_program",
            )?);
            if self.wall_timeout < Duration::from_secs(1)
                || self.wall_timeout > Duration::from_secs(60 * 60)
                || self.cpu_limit_seconds == 0
                || self.cpu_limit_seconds > self.wall_timeout.as_secs()
                || !(64 * 1024 * 1024..=MAX_ADDRESS_SPACE_BYTES)
                    .contains(&self.address_space_limit_bytes)
                || !(1..=128).contains(&self.process_limit)
                || !(16..=4096).contains(&self.open_files_limit)
                || self.file_size_limit_bytes < MAX_RESULT_BYTES.max(self.max_campaign_bytes)
                || self.file_size_limit_bytes > MAX_FILE_SIZE_BYTES
                || !(1..=MAX_REQUEST_BYTES_HARD).contains(&self.max_request_bytes)
                || self.max_campaign_bytes == 0
            {
                return Err(ProcessError::Configuration(
                    "invalid direct verifier timeout, artifact, or rlimit policy".to_owned(),
                ));
            }
            Ok(())
        }
    }

    pub async fn run(
        &self,
        request: &VerificationRequestV1,
        job_config: &VerifierJobConfigV1,
        replay: &mut tokio::fs::File,
        starting_campaign: &mut tokio::fs::File,
    ) -> Result<VerifierOutput, ProcessError> {
        request
            .validate()
            .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
        job_config
            .validate()
            .map_err(|error| ProcessError::Configuration(error.to_string()))?;
        if job_config.template.route != VerifierJobRouteV1::from_request(request) {
            return Err(ProcessError::Configuration(
                "per-job verifier route differs from the authenticated request".to_owned(),
            ));
        }
        let job_config_bytes = job_config
            .canonical_bytes()
            .map_err(|error| ProcessError::Configuration(error.to_string()))?;
        self.run_isolated(
            request,
            job_config,
            &job_config_bytes,
            replay,
            starting_campaign,
        )
        .await
    }

    async fn run_isolated(
        &self,
        request: &VerificationRequestV1,
        job_config: &VerifierJobConfigV1,
        job_config_bytes: &[u8],
        replay: &mut tokio::fs::File,
        starting_campaign: &mut tokio::fs::File,
    ) -> Result<VerifierOutput, ProcessError> {
        let request_bytes = serde_json::to_vec(request)
            .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
        if request_bytes.is_empty() || request_bytes.len() as u64 > self.max_request_bytes {
            return Err(ProcessError::InvalidResult(
                "verification request exceeds its sealed-input limit".to_owned(),
            ));
        }
        let request_artifact_sha256 = Digest32::digest_bytes(&request_bytes);
        let request_file = sealed_data_file(&request_bytes, None, "verification_request")?;
        let replay_ref = &request.submission.submission.artifacts.replay.artifact;
        let starting_ref = &request.submission.submission.artifacts.starting_campaign;
        let replay_file = stage_sealed_file(
            replay,
            request.limits.max_input_bytes,
            Some(*replay_ref.sha256.as_bytes()),
            Some(replay_ref.byte_length),
            "replay",
        )
        .await?;
        let starting_file = stage_sealed_file(
            starting_campaign,
            request.limits.max_campaign_bytes,
            Some(*starting_ref.sha256.as_bytes()),
            Some(starting_ref.byte_length),
            "starting_campaign",
        )
        .await?;
        if job_config_bytes.len() > MAX_VERIFIER_JOB_CONFIG_BYTES_V1 {
            return Err(ProcessError::Configuration(
                "per-job verifier config exceeds its sealed-input limit".to_owned(),
            ));
        }
        let job_config_sha256 = Digest32::digest_bytes(job_config_bytes);
        let job_config_file = sealed_data_file(
            job_config_bytes,
            Some(job_config_sha256.into_bytes()),
            "job_config",
        )?;
        let outputs = create_shared_outputs()?;
        let content_catalog_root = open_pinned_directory(
            &job_config.template.content_catalog_root,
            "content_catalog_root",
        )?;
        let raw_content_root =
            open_pinned_directory(&job_config.template.raw_content_root, "raw_content_root")?;
        let launcher_config = self.clone();
        let content_catalog_destination = job_config.template.content_catalog_root.clone();
        let raw_content_destination = job_config.template.raw_content_root.clone();
        let (output_directory, result_file, final_campaign_file) =
            tokio::task::spawn_blocking(move || {
                launch_direct_verifier(
                    &launcher_config,
                    SandboxArtifacts {
                        request: &request_file,
                        replay: &replay_file,
                        job_config: &job_config_file,
                        starting_campaign: &starting_file,
                        result: &outputs.result,
                        final_campaign: &outputs.final_campaign,
                        content_catalog: &content_catalog_root,
                        raw_content: &raw_content_root,
                        content_catalog_destination: &content_catalog_destination,
                        raw_content_destination: &raw_content_destination,
                    },
                )?;
                Ok::<_, ProcessError>((outputs.directory, outputs.result, outputs.final_campaign))
            })
            .await
            .map_err(|_| ProcessError::Exit("direct sandbox launcher task failed".to_owned()))??;
        // The private output names remain linked until bubblewrap has exited;
        // all reads below use the exact descriptors opened before launch.
        let _output_directory = output_directory;
        let mut result_file = tokio::fs::File::from_std(result_file);
        let mut final_campaign_file = tokio::fs::File::from_std(final_campaign_file);

        let bytes = read_bounded_open_file(&mut result_file, MAX_RESULT_BYTES).await?;
        let outcome = decode_worker_output(&bytes)?;
        validate_admission_request_binding(&outcome, request_artifact_sha256)?;
        let request_digest = request
            .canonical_digest()
            .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
        let campaigns = match &outcome {
            VerifierWorkerOutputV1::VerificationResult { result, .. } => {
                if result.request_id != request.request_id
                    || result.verification_request_sha256 != request_digest
                    || result.artifacts != request.submission.submission.artifacts
                {
                    return Err(ProcessError::InvalidResult(
                        "result does not bind the exact request artifact tuple".to_owned(),
                    ));
                }
                if !matches!(result.status, VerificationStatusV1::Verified(_)) {
                    if final_campaign_file.metadata().await?.len() != 0 {
                        return Err(ProcessError::InvalidResult(
                            "non-verified result created campaign state".to_owned(),
                        ));
                    }
                    None
                } else {
                    let VerificationStatusV1::Verified(ref verified) = result.status else {
                        unreachable!()
                    };
                    let output =
                        read_bounded_open_file(&mut final_campaign_file, self.max_campaign_bytes)
                            .await?;
                    if Digest32::digest_bytes(&output) != verified.final_campaign.sha256
                        || u64::try_from(output.len()).ok()
                            != Some(verified.final_campaign.byte_length)
                    {
                        return Err(ProcessError::InvalidResult(
                            "final campaign output does not match the typed artifact reference"
                                .to_owned(),
                        ));
                    }
                    Some(output)
                }
            }
            VerifierWorkerOutputV1::AdmissionFailure { .. } => {
                if final_campaign_file.metadata().await?.len() != 0 {
                    return Err(ProcessError::InvalidResult(
                        "admission failure created campaign state".to_owned(),
                    ));
                }
                None
            }
        };
        Ok(VerifierOutput {
            outcome,
            request_artifact_sha256,
            job_config_artifact_sha256: job_config_sha256,
            final_campaign: campaigns,
        })
    }
}

struct SandboxArtifacts<'a> {
    request: &'a File,
    replay: &'a File,
    job_config: &'a File,
    starting_campaign: &'a File,
    result: &'a File,
    final_campaign: &'a File,
    content_catalog: &'a File,
    raw_content: &'a File,
    content_catalog_destination: &'a Path,
    raw_content_destination: &'a Path,
}

struct SandboxFdNumbers {
    verifier: i32,
    request: i32,
    replay: i32,
    job_config: i32,
    starting_campaign: i32,
    result: i32,
    final_campaign: i32,
    content_catalog: i32,
    raw_content: i32,
}

struct SharedOutputs {
    directory: tempfile::TempDir,
    result: File,
    final_campaign: File,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct OutputIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_os = "linux")]
fn launch_direct_verifier(
    config: &VerifierProcessConfig,
    artifacts: SandboxArtifacts<'_>,
) -> Result<(), ProcessError> {
    use command_fds::{CommandFdExt as _, FdMapping};
    use std::os::unix::process::CommandExt as _;

    let result_identity = validate_output_file(artifacts.result, "result output", 0)?;
    let final_campaign_identity =
        validate_output_file(artifacts.final_campaign, "final campaign output", 0)?;
    let prlimit = sealed_executable(
        &config.prlimit_program,
        config.prlimit_sha256,
        "prlimit_program",
    )?;
    let bwrap = sealed_executable(&config.bwrap_program, config.bwrap_sha256, "bwrap_program")?;
    let verifier = sealed_executable(
        &config.verifier_program,
        config.verifier_sha256,
        "verifier_program",
    )?;

    // bubblewrap 0.8 cannot mount an unlinked memfd with `--ro-bind-fd`.
    // `--ro-bind-data` copies the exact sealed regular-file descriptors into
    // private files, while `--ro-bind-fd` retains only pinned directory roots.
    // Writable output files have private, persistent names for `--bind-fd` and
    // are consumed and checked exclusively through their original descriptors.
    let inherited = [
        &bwrap,
        &verifier,
        artifacts.request,
        artifacts.replay,
        artifacts.job_config,
        artifacts.starting_campaign,
        artifacts.result,
        artifacts.final_campaign,
        artifacts.content_catalog,
        artifacts.raw_content,
    ]
    .into_iter()
    .map(inheritable_copy)
    .collect::<Result<Vec<_>, _>>()?;
    let fds = SandboxFdNumbers {
        verifier: FIRST_SANDBOX_FD + 1,
        request: FIRST_SANDBOX_FD + 2,
        replay: FIRST_SANDBOX_FD + 3,
        job_config: FIRST_SANDBOX_FD + 4,
        starting_campaign: FIRST_SANDBOX_FD + 5,
        result: FIRST_SANDBOX_FD + 6,
        final_campaign: FIRST_SANDBOX_FD + 7,
        content_catalog: FIRST_SANDBOX_FD + 8,
        raw_content: FIRST_SANDBOX_FD + 9,
    };
    let mut command = std::process::Command::new(proc_parent_fd_path(&prlimit));
    command
        .env_clear()
        .args(fixed_prlimit_arguments(
            config,
            &PathBuf::from(format!("/proc/self/fd/{FIRST_SANDBOX_FD}")),
        ))
        .args(fixed_bwrap_arguments(
            &fds,
            artifacts.content_catalog_destination,
            artifacts.raw_content_destination,
        )?)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .fd_mappings(
            inherited
                .into_iter()
                .enumerate()
                .map(|(index, parent_fd)| FdMapping {
                    parent_fd,
                    child_fd: FIRST_SANDBOX_FD + index as i32,
                })
                .collect(),
        )
        .map_err(|_| ProcessError::Configuration("sandbox FD mapping collision".to_owned()))?;
    let mut child = command.spawn()?;
    let status = wait_for_process_group(&mut child, config.wall_timeout)?;
    status_result(status)?;
    validate_output_identity(
        artifacts.result,
        result_identity,
        "result output",
        MAX_RESULT_BYTES,
    )?;
    validate_output_identity(
        artifacts.final_campaign,
        final_campaign_identity,
        "final campaign output",
        config.max_campaign_bytes,
    )
}

#[cfg(not(target_os = "linux"))]
fn launch_direct_verifier(
    _config: &VerifierProcessConfig,
    _artifacts: SandboxArtifacts<'_>,
) -> Result<(), ProcessError> {
    Err(ProcessError::Configuration(
        "direct verifier launch requires Linux".to_owned(),
    ))
}

fn fixed_prlimit_arguments(config: &VerifierProcessConfig, bwrap: &Path) -> Vec<OsString> {
    [
        "--core=0".to_owned(),
        format!("--fsize={}", config.file_size_limit_bytes),
        format!("--as={}", config.address_space_limit_bytes),
        format!("--cpu={}", config.cpu_limit_seconds),
        format!("--nofile={}", config.open_files_limit),
        format!("--nproc={}", config.process_limit),
        "--".to_owned(),
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(bwrap.as_os_str().to_owned()))
    .collect()
}

fn fixed_bwrap_arguments(
    fds: &SandboxFdNumbers,
    content_catalog_destination: &Path,
    raw_content_destination: &Path,
) -> Result<Vec<OsString>, ProcessError> {
    validate_absolute_normalized_path(content_catalog_destination, "content_catalog_root")?;
    validate_absolute_normalized_path(raw_content_destination, "raw_content_root")?;
    if content_catalog_destination == raw_content_destination
        || content_catalog_destination.starts_with(raw_content_destination)
        || raw_content_destination.starts_with(content_catalog_destination)
    {
        return Err(ProcessError::Configuration(
            "operator content roots overlap".to_owned(),
        ));
    }

    let mut arguments = Vec::new();
    push_arguments(
        &mut arguments,
        &[
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--assert-userns-disabled",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
            "--setenv",
            "PATH",
            "/run",
            "--setenv",
            "HOME",
            "/home/verifier",
            "--setenv",
            "TMPDIR",
            "/tmp",
            "--setenv",
            "LANG",
            "C",
            "--setenv",
            "LC_ALL",
            "C",
            "--hostname",
            "robin-verifier",
            "--size",
        ],
    );
    arguments.push(PRIVATE_TMPFS_BYTES.to_string().into());
    push_arguments(
        &mut arguments,
        &["--tmpfs", "/", "--proc", "/proc", "--dev", "/dev"],
    );
    for destination in ["/run", "/tmp"] {
        arguments.push("--size".into());
        arguments.push(PRIVATE_TMPFS_BYTES.to_string().into());
        arguments.push("--tmpfs".into());
        arguments.push(destination.into());
    }
    push_arguments(&mut arguments, &["--dir", "/var"]);
    arguments.push("--size".into());
    arguments.push(PRIVATE_TMPFS_BYTES.to_string().into());
    push_arguments(&mut arguments, &["--tmpfs", "/var/tmp"]);
    arguments.push("--size".into());
    arguments.push(PRIVATE_TMPFS_BYTES.to_string().into());
    push_arguments(
        &mut arguments,
        &[
            "--tmpfs",
            "/home",
            "--dir",
            "/home/verifier",
            "--chmod",
            "0700",
            "/home/verifier",
            "--dir",
            "/run/robin-input",
        ],
    );

    let mut parents = BTreeSet::new();
    for destination in [content_catalog_destination, raw_content_destination] {
        parents.extend(mount_parent_directories(destination)?);
    }
    let predefined = [
        Path::new("/proc"),
        Path::new("/dev"),
        Path::new("/run"),
        Path::new("/tmp"),
        Path::new("/var"),
        Path::new("/var/tmp"),
        Path::new("/home"),
        Path::new("/home/verifier"),
    ];
    let mut parents = parents
        .into_iter()
        .filter(|path| !predefined.contains(&path.as_path()))
        .collect::<Vec<_>>();
    parents.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    for parent in parents {
        arguments.push("--dir".into());
        arguments.push(parent.into_os_string());
    }

    for (source, destination, permissions) in [
        (fds.verifier, Path::new("/run/robin-verifier")),
        (fds.request, Path::new("/run/robin-input/request.json")),
        (fds.replay, Path::new("/run/robin-input/replay.rhrec")),
        (
            fds.job_config,
            Path::new("/run/robin-input/verifier-config.json"),
        ),
        (
            fds.starting_campaign,
            Path::new("/run/robin-input/starting.campaign"),
        ),
    ]
    .into_iter()
    .map(|(source, destination)| {
        let permissions = if source == fds.verifier {
            "0500"
        } else {
            "0400"
        };
        (source, destination, permissions)
    }) {
        arguments.push("--perms".into());
        arguments.push(permissions.into());
        arguments.push("--ro-bind-data".into());
        arguments.push(source.to_string().into());
        arguments.push(destination.as_os_str().to_owned());
    }
    for (source, destination) in [
        (fds.content_catalog, content_catalog_destination),
        (fds.raw_content, raw_content_destination),
    ] {
        arguments.push("--ro-bind-fd".into());
        arguments.push(source.to_string().into());
        arguments.push(destination.as_os_str().to_owned());
    }
    for (source, destination) in [
        (fds.result, Path::new("/run/robin-result.json")),
        (fds.final_campaign, Path::new("/run/robin-final.campaign")),
    ] {
        arguments.push("--bind-fd".into());
        arguments.push(source.to_string().into());
        arguments.push(destination.as_os_str().to_owned());
    }
    push_arguments(
        &mut arguments,
        &[
            "--remount-ro",
            "/",
            "--chdir",
            "/run",
            "--",
            "/run/robin-verifier",
            "--request",
            "/run/robin-input/request.json",
            "--replay",
            "/run/robin-input/replay.rhrec",
            "--config",
            "/run/robin-input/verifier-config.json",
            "--starting-campaign",
            "/run/robin-input/starting.campaign",
            "--final-campaign",
            "/run/robin-final.campaign",
            "--result",
            "/run/robin-result.json",
        ],
    );
    Ok(arguments)
}

fn push_arguments(arguments: &mut Vec<OsString>, values: &[&str]) {
    arguments.extend(values.iter().map(OsString::from));
}

fn mount_parent_directories(path: &Path) -> Result<Vec<PathBuf>, ProcessError> {
    validate_absolute_normalized_path(path, "operator content root")?;
    let normal_components = path
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    if normal_components < 2 {
        return Err(ProcessError::Configuration(
            "operator content root is too broad".to_owned(),
        ));
    }
    let mut parents = Vec::new();
    let mut current = PathBuf::from("/");
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    for component in &components[..components.len() - 1] {
        current.push(component);
        parents.push(current.clone());
    }
    Ok(parents)
}

fn validate_absolute_normalized_path(path: &Path, name: &str) -> Result<(), ProcessError> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir))
        || components.any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProcessError::Configuration(format!(
            "{name} must be an absolute normalized path"
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn inheritable_copy(file: &File) -> Result<std::os::fd::OwnedFd, ProcessError> {
    rustix::io::dup(file)
        .map_err(std::io::Error::from)
        .map_err(ProcessError::Io)
}

#[cfg(target_os = "linux")]
fn proc_parent_fd_path(file: &File) -> PathBuf {
    use std::os::fd::AsRawFd as _;
    PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        file.as_raw_fd()
    ))
}

#[cfg(target_os = "linux")]
fn wait_for_process_group(
    child: &mut Child,
    timeout: Duration,
) -> Result<ExitStatus, ProcessError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let process_group = rustix::process::Pid::from_child(child);
            let _ =
                rustix::process::kill_process_group(process_group, rustix::process::Signal::KILL);
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn status_result(status: ExitStatus) -> Result<(), ProcessError> {
    if status.success() {
        Ok(())
    } else {
        Err(ProcessError::Exit(status.to_string()))
    }
}

fn decode_worker_output(bytes: &[u8]) -> Result<VerifierWorkerOutputV1, ProcessError> {
    let output: VerifierWorkerOutputV1 = serde_json::from_slice(bytes)
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    output
        .validate()
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    Ok(output)
}

fn validate_admission_request_binding(
    output: &VerifierWorkerOutputV1,
    request_artifact_sha256: Digest32,
) -> Result<(), ProcessError> {
    if let VerifierWorkerOutputV1::AdmissionFailure {
        request_artifact_sha256: bound_request,
        ..
    } = output
        && *bound_request != request_artifact_sha256
    {
        return Err(ProcessError::InvalidResult(
            "admission failure does not bind the exact sealed request artifact".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn sealed_data_file(
    bytes: &[u8],
    expected_sha256: Option<[u8; 32]>,
    name: &str,
) -> Result<std::fs::File, ProcessError> {
    use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
    use std::io::{Seek as _, Write as _};
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if bytes.is_empty() || expected_sha256.is_some_and(|expected| expected != actual) {
        return Err(ProcessError::InvalidResult(format!(
            "staged {name} bytes are empty or fail their exact digest"
        )));
    }
    let fd = memfd_create(name, MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)
        .map_err(std::io::Error::from)?;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)?;
    file.flush()?;
    file.seek(std::io::SeekFrom::Start(0))?;
    fcntl_add_seals(
        &file,
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
    )
    .map_err(std::io::Error::from)?;
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
fn sealed_data_file(
    _bytes: &[u8],
    _expected_sha256: Option<[u8; 32]>,
    _name: &str,
) -> Result<std::fs::File, ProcessError> {
    Err(ProcessError::Configuration(
        "sealed verifier inputs require Linux memfd sealing".to_owned(),
    ))
}

async fn stage_sealed_file(
    source: &mut tokio::fs::File,
    limit: u64,
    expected_sha256: Option<[u8; 32]>,
    expected_byte_length: Option<u64>,
    name: &str,
) -> Result<std::fs::File, ProcessError> {
    source.seek(std::io::SeekFrom::Start(0)).await?;
    let capacity = usize::try_from(limit.min(64 * 1024))
        .map_err(|_| ProcessError::Configuration("staging capacity overflows".to_owned()))?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let next = bytes.len().checked_add(read).ok_or_else(|| {
            ProcessError::InvalidResult(format!("staged {name} length overflows"))
        })?;
        if u64::try_from(next).map_or(true, |length| length > limit) {
            return Err(ProcessError::InvalidResult(format!(
                "staged {name} exceeds its byte limit"
            )));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    if expected_byte_length.is_some_and(|expected| expected != bytes.len() as u64) {
        return Err(ProcessError::InvalidResult(format!(
            "staged {name} length differs from its signed artifact reference"
        )));
    }
    sealed_data_file(&bytes, expected_sha256, name)
}

fn parse_digest(value: &str, name: &str) -> Result<[u8; 32], ProcessError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ProcessError::Configuration(format!(
            "{name} must be 64 lowercase hexadecimal digits"
        )));
    }
    let bytes = hex::decode(value)
        .map_err(|_| ProcessError::Configuration(format!("{name} is not hexadecimal")))?;
    bytes
        .try_into()
        .map_err(|_| ProcessError::Configuration(format!("{name} has the wrong length")))
}

fn read_std_bounded(source: &mut File, limit: u64, name: &str) -> Result<Vec<u8>, ProcessError> {
    use std::io::{Read as _, Seek as _};
    source.seek(std::io::SeekFrom::Start(0))?;
    let read_limit = limit
        .checked_add(1)
        .ok_or_else(|| ProcessError::Configuration(format!("{name} byte limit overflows")))?;
    let capacity = usize::try_from(source.metadata()?.len().min(limit))
        .map_err(|_| ProcessError::Configuration(format!("{name} length overflows")))?;
    let mut bytes = Vec::with_capacity(capacity);
    source.take(read_limit).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(ProcessError::Configuration(format!(
            "{name} is empty or oversized"
        )));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn open_pinned_file(path: &Path, name: &str) -> Result<File, ProcessError> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    validate_absolute_normalized_path(path, name)?;
    let descriptor = openat2(
        rustix::fs::CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(std::io::Error::from)?;
    let file = File::from(descriptor);
    if !file.metadata()?.is_file() {
        return Err(ProcessError::Configuration(format!(
            "{name} must resolve to a regular file"
        )));
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn open_pinned_directory(path: &Path, name: &str) -> Result<File, ProcessError> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    validate_absolute_normalized_path(path, name)?;
    mount_parent_directories(path)?;
    let descriptor = openat2(
        rustix::fs::CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(std::io::Error::from)?;
    let file = File::from(descriptor);
    if !file.metadata()?.is_dir() {
        return Err(ProcessError::Configuration(format!(
            "{name} must resolve to a directory"
        )));
    }
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
fn open_pinned_directory(_path: &Path, _name: &str) -> Result<File, ProcessError> {
    Err(ProcessError::Configuration(
        "pinned verifier directories require Linux openat2".to_owned(),
    ))
}

fn sealed_executable(
    path: &Path,
    expected_sha256: [u8; 32],
    name: &str,
) -> Result<File, ProcessError> {
    let mut source = open_pinned_file(path, name)?;
    let mode = source.metadata()?.permissions().mode();
    if mode & 0o111 == 0 || mode & 0o6000 != 0 {
        return Err(ProcessError::Configuration(format!(
            "{name} must be executable without set-id bits"
        )));
    }
    let bytes = read_std_bounded(&mut source, MAX_EXECUTABLE_BYTES, name)?;
    let file =
        sealed_data_file(&bytes, Some(expected_sha256), name).map_err(|error| match error {
            ProcessError::InvalidResult(detail) => ProcessError::Configuration(detail),
            other => other,
        })?;
    file.set_permissions(std::fs::Permissions::from_mode(0o500))?;
    Ok(file)
}

#[cfg(target_os = "linux")]
fn validate_output_file(
    file: &File,
    name: &str,
    expected_size: u64,
) -> Result<OutputIdentity, ProcessError> {
    use std::os::unix::fs::MetadataExt as _;

    let flags = rustix::fs::fcntl_getfl(file).map_err(std::io::Error::from)?;
    let metadata = file.metadata()?;
    if flags & rustix::fs::OFlags::ACCMODE != rustix::fs::OFlags::RDWR
        || !file.metadata()?.is_file()
        || metadata.len() != expected_size
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(ProcessError::Configuration(format!(
            "{name} must be an exact private read-write output inode"
        )));
    }
    Ok(OutputIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(target_os = "linux")]
fn validate_output_identity(
    file: &File,
    expected: OutputIdentity,
    name: &str,
    maximum_size: u64,
) -> Result<(), ProcessError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file.metadata()?;
    if metadata.dev() != expected.device
        || metadata.ino() != expected.inode
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
        || !metadata.is_file()
        || metadata.len() > maximum_size
    {
        return Err(ProcessError::InvalidResult(format!(
            "{name} identity, mode, link count, or bounded size changed"
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn create_shared_outputs() -> Result<SharedOutputs, ProcessError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

    let directory = tempfile::Builder::new()
        .prefix("robin-verifier-output-")
        .tempdir()?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    let metadata = directory.path().metadata()?;
    if !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 {
        return Err(ProcessError::Configuration(
            "verifier output directory is not private".to_owned(),
        ));
    }
    let create = |name: &str| -> Result<File, ProcessError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.path().join(name))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        validate_output_file(&file, name, 0)?;
        Ok(file)
    };
    let result = create("result.json")?;
    let final_campaign = create("final.campaign")?;
    Ok(SharedOutputs {
        directory,
        result,
        final_campaign,
    })
}

#[cfg(not(target_os = "linux"))]
fn create_shared_outputs() -> Result<SharedOutputs, ProcessError> {
    Err(ProcessError::Configuration(
        "private verifier outputs require Linux".to_owned(),
    ))
}

async fn read_bounded_open_file(
    file: &mut tokio::fs::File,
    limit: u64,
) -> Result<Vec<u8>, ProcessError> {
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(ProcessError::InvalidResult(
            "verifier output is missing, non-regular, empty, or oversized".to_owned(),
        ));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .map_err(|_| ProcessError::InvalidResult("output length overflows".to_owned()))?,
    );
    match tokio::time::timeout(
        RESULT_READ_TIMEOUT,
        file.take(limit + 1).read_to_end(&mut bytes),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            return Err(ProcessError::InvalidResult(
                "timed out reading verifier output".to_owned(),
            ));
        }
    };
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(ProcessError::InvalidResult(
            "verifier output grew beyond its byte limit".to_owned(),
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use command_fds::{CommandFdExt as _, FdMapping};
    use std::os::unix::process::CommandExt as _;

    fn executable(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn sha256_file(path: &Path) -> [u8; 32] {
        Sha256::digest(std::fs::read(path).unwrap()).into()
    }

    fn config(verifier_program: PathBuf, verifier_sha256: [u8; 32]) -> VerifierProcessConfig {
        VerifierProcessConfig {
            bwrap_program: BWRAP_PROGRAM.into(),
            bwrap_sha256: sha256_file(Path::new(BWRAP_PROGRAM)),
            prlimit_program: PRLIMIT_PROGRAM.into(),
            prlimit_sha256: sha256_file(Path::new(PRLIMIT_PROGRAM)),
            verifier_program,
            verifier_sha256,
            wall_timeout: Duration::from_secs(5),
            cpu_limit_seconds: 5,
            address_space_limit_bytes: 256 * 1024 * 1024,
            process_limit: 32,
            open_files_limit: 64,
            file_size_limit_bytes: 1024 * 1024,
            max_request_bytes: 1024 * 1024,
            max_campaign_bytes: 1024 * 1024,
        }
    }

    #[tokio::test]
    async fn validation_pins_all_three_executables_and_exact_system_launcher_paths() {
        let directory = tempfile::tempdir().unwrap();
        let verifier = directory.path().join("robin-replay-verifier");
        executable(&verifier, b"verifier");
        let config = config(verifier.clone(), Sha256::digest(b"verifier").into());
        config.validate().await.unwrap();

        let mut wrong_bwrap = config.clone();
        wrong_bwrap.bwrap_sha256[0] ^= 1;
        assert!(matches!(
            wrong_bwrap.validate().await,
            Err(ProcessError::Configuration(_))
        ));
        let mut wrong_prlimit = config.clone();
        wrong_prlimit.prlimit_sha256[0] ^= 1;
        assert!(matches!(
            wrong_prlimit.validate().await,
            Err(ProcessError::Configuration(_))
        ));
        let mut wrong_verifier = config.clone();
        wrong_verifier.verifier_sha256[0] ^= 1;
        assert!(wrong_verifier.validate().await.is_err());

        let mut substituted_launcher = config.clone();
        substituted_launcher.bwrap_program = verifier.clone();
        assert!(substituted_launcher.validate().await.is_err());

        let target = directory.path().join("other-verifier");
        executable(&target, b"verifier");
        std::fs::remove_file(&verifier).unwrap();
        std::os::unix::fs::symlink(target, verifier).unwrap();
        assert!(config.validate().await.is_err());
    }

    #[tokio::test]
    async fn staged_input_is_hard_limited_and_sealed() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source");
        tokio::fs::write(&source_path, b"12345").await.unwrap();
        let mut source = tokio::fs::File::open(source_path).await.unwrap();
        let result = stage_sealed_file(&mut source, 4, None, None, "test").await;
        assert!(matches!(result, Err(ProcessError::InvalidResult(_))));

        let mut source = tokio::fs::File::open(directory.path().join("source"))
            .await
            .unwrap();
        let mut staged = stage_sealed_file(
            &mut source,
            5,
            Some(Sha256::digest(b"12345").into()),
            Some(5),
            "test",
        )
        .await
        .unwrap();
        use std::io::Write as _;
        assert!(staged.write_all(b"x").is_err());
    }

    #[tokio::test]
    async fn verifier_outputs_are_private_exact_open_inodes() {
        let outputs = create_shared_outputs().unwrap();
        assert_eq!(
            outputs.result.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        let identity = validate_output_file(&outputs.result, "test output", 0).unwrap();
        validate_output_identity(&outputs.result, identity, "test output", 1).unwrap();
    }

    #[test]
    fn launcher_arguments_are_fixed_literal_argv_with_private_namespaces_and_rlimits() {
        let directory = tempfile::tempdir().unwrap();
        let verifier = directory.path().join("robin-replay-verifier");
        executable(&verifier, b"verifier");
        let config = config(verifier, Sha256::digest(b"verifier").into());
        let prlimit = fixed_prlimit_arguments(&config, Path::new("/proc/self/fd/4"));
        for required in [
            "--core=0",
            "--fsize=1048576",
            "--as=268435456",
            "--cpu=5",
            "--nofile=64",
            "--nproc=32",
        ] {
            assert!(prlimit.iter().any(|argument| argument == required));
        }

        let fds = SandboxFdNumbers {
            verifier: 5,
            request: 6,
            replay: 7,
            job_config: 8,
            starting_campaign: 9,
            result: 10,
            final_campaign: 11,
            content_catalog: 12,
            raw_content: 13,
        };
        let catalog = Path::new("/home/robinhood/releases/abc/$(touch pwned)/catalog");
        let raw = Path::new("/srv/robin-raw/full;echo-pwned");
        let arguments = fixed_bwrap_arguments(&fds, catalog, raw).unwrap();
        for required in [
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--assert-userns-disabled",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
            "--remount-ro",
        ] {
            assert!(arguments.iter().any(|argument| argument == required));
        }
        assert!(!arguments.iter().any(|argument| argument == "/bin/sh"));
        assert!(arguments.iter().any(|argument| argument == catalog));
        assert!(arguments.iter().any(|argument| argument == raw));
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| argument.as_os_str() == "--bind-fd")
                .count(),
            2,
            "only the two private output inodes may be writable host-backed mounts"
        );
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| argument.as_os_str() == "--ro-bind-data")
                .count(),
            5,
            "the verifier and four regular inputs are copied from sealed descriptors"
        );
        assert!(arguments.windows(4).any(|window| {
            window[0] == "--perms"
                && window[1] == "0500"
                && window[2] == "--ro-bind-data"
                && window[3] == OsString::from(fds.verifier.to_string())
        }));
        assert!(arguments.windows(3).any(|window| {
            window[0] == "--ro-bind-fd"
                && window[1] == OsString::from(fds.content_catalog.to_string())
                && window[2] == catalog
        }));
        assert!(arguments.windows(3).any(|window| {
            window[0] == "--ro-bind-fd"
                && window[1] == OsString::from(fds.raw_content.to_string())
                && window[2] == raw
        }));
    }

    #[test]
    fn actual_bwrap_mounts_exact_operator_roots_read_only_and_clears_environment() {
        let directory = tempfile::tempdir().unwrap();
        let catalog = directory.path().join("operator/catalog");
        let raw = directory.path().join("operator/raw");
        std::fs::create_dir_all(&catalog).unwrap();
        std::fs::create_dir_all(&raw).unwrap();
        std::fs::write(catalog.join("catalog.marker"), b"catalog").unwrap();
        std::fs::write(raw.join("raw.marker"), b"raw").unwrap();
        let catalog_fd = open_pinned_directory(&catalog, "catalog").unwrap();
        let raw_directory_fd = open_pinned_directory(&raw, "raw").unwrap();
        let shell_path = Path::new("/bin/sh")
            .canonicalize()
            .expect("sandbox probe requires a system POSIX shell");
        let shell = sealed_executable(&shell_path, sha256_file(&shell_path), "test shell").unwrap();
        let inputs = (0..4)
            .map(|index| sealed_data_file(b"x", None, &format!("test-input-{index}")).unwrap())
            .collect::<Vec<_>>();
        let outputs = create_shared_outputs().unwrap();
        let files = [
            &shell,
            &inputs[0],
            &inputs[1],
            &inputs[2],
            &inputs[3],
            &outputs.result,
            &outputs.final_campaign,
            &catalog_fd,
            &raw_directory_fd,
        ];
        let inherited = files
            .into_iter()
            .map(inheritable_copy)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let fds = SandboxFdNumbers {
            verifier: FIRST_SANDBOX_FD,
            request: FIRST_SANDBOX_FD + 1,
            replay: FIRST_SANDBOX_FD + 2,
            job_config: FIRST_SANDBOX_FD + 3,
            starting_campaign: FIRST_SANDBOX_FD + 4,
            result: FIRST_SANDBOX_FD + 5,
            final_campaign: FIRST_SANDBOX_FD + 6,
            content_catalog: FIRST_SANDBOX_FD + 7,
            raw_content: FIRST_SANDBOX_FD + 8,
        };
        let mut arguments = fixed_bwrap_arguments(&fds, &catalog, &raw).unwrap();
        let remount = arguments
            .iter()
            .position(|argument| argument == "--remount-ro")
            .unwrap();
        arguments.splice(
            remount..remount,
            [
                "--ro-bind",
                "/usr",
                "/usr",
                "--symlink",
                "usr/bin",
                "/bin",
                "--symlink",
                "usr/lib",
                "/lib",
                "--symlink",
                "usr/lib64",
                "/lib64",
            ]
            .into_iter()
            .map(OsString::from),
        );
        let separator = arguments
            .iter()
            .position(|argument| argument == "--")
            .unwrap();
        arguments.truncate(separator + 1);
        arguments.extend(
            [
                OsString::from("/run/robin-verifier"),
                OsString::from("-c"),
                OsString::from(
                    r#"test -r "$1/catalog.marker" && test -r "$2/raw.marker" && test "$HOME" = /home/verifier && test "$TMPDIR" = /tmp && test -z "${ROBIN_SENTINEL+x}" && ! (printf x > "$1/write-probe") 2>/dev/null && ! (printf x > "$2/write-probe") 2>/dev/null"#,
                ),
                OsString::from("sandbox-probe"),
                catalog.as_os_str().to_owned(),
                raw.as_os_str().to_owned(),
            ],
        );
        let mut command = std::process::Command::new(BWRAP_PROGRAM);
        command
            .args(arguments)
            .env_clear()
            .env("ROBIN_SENTINEL", "must-not-cross")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        command
            .fd_mappings(
                inherited
                    .into_iter()
                    .enumerate()
                    .map(|(index, parent_fd)| FdMapping {
                        parent_fd,
                        child_fd: FIRST_SANDBOX_FD + index as i32,
                    })
                    .collect(),
            )
            .unwrap();
        let status = command.status().unwrap();
        assert!(status.success(), "bubblewrap probe failed: {status}");
        assert!(!catalog.join("write-probe").exists());
        assert!(!raw.join("write-probe").exists());
    }

    #[test]
    fn wall_timeout_kills_and_reaps_the_fresh_process_group() {
        let mut command = std::process::Command::new("/usr/bin/sleep");
        command
            .arg("60")
            .env_clear()
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        assert!(matches!(
            wait_for_process_group(&mut child, Duration::from_millis(10)),
            Err(ProcessError::Timeout)
        ));
        assert!(!Path::new("/proc").join(pid.to_string()).exists());
    }

    #[test]
    fn verifier_output_requires_the_new_typed_envelope_and_exact_request_digest() {
        use robin_run_protocol::VerifierAdmissionFailureCodeV1;

        let request_digest = Digest32::from_bytes([7; 32]);
        let output = VerifierWorkerOutputV1::AdmissionFailure {
            schema_version: SCHEMA_VERSION_V1,
            request_artifact_sha256: request_digest,
            code: VerifierAdmissionFailureCodeV1::MalformedRequest,
            bounded_detail: Some("invalid_json".to_owned()),
        };
        let bytes = serde_json::to_vec(&output).unwrap();
        let decoded = decode_worker_output(&bytes).unwrap();
        validate_admission_request_binding(&decoded, request_digest).unwrap();
        assert!(
            validate_admission_request_binding(&decoded, Digest32::from_bytes([8; 32])).is_err()
        );

        let mut legacy = serde_json::to_value(output).unwrap();
        legacy.as_object_mut().unwrap().remove("outcome");
        assert!(decode_worker_output(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }
}
