//! Private verifier worker-process boundary.
//!
//! Production execution is deliberately Linux-only. Each replay is handled by
//! a fresh, unprivileged bubblewrap sandbox launched through `prlimit`, with
//! read-only inputs, the edition's raw content bound read-only, private
//! namespaces, an empty root, and hard process resource and wall-time limits.
//!
//! The verifier CLI contract inside the sandbox is fixed:
//! `/run/robin-verifier --job /run/robin-input/job.json
//! --replay /run/robin-input/replay.rhrec --content-root /run/robin-content
//! --result /run/robin-result.json`.

use robin_run_protocol::{Digest32, Validate as _, VerifierOutputV2};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::File;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

/// Upper bound of the verifier's typed result document.
pub const MAX_RESULT_BYTES: u64 = 1024 * 1024;
const MAX_ADDRESS_SPACE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_FILE_SIZE_BYTES: u64 = 1024 * 1024 * 1024;
const PRIVATE_TMPFS_BYTES: u64 = 16 * 1024 * 1024;

pub const SANDBOX_VERIFIER: &str = "/run/robin-verifier";
pub const SANDBOX_JOB: &str = "/run/robin-input/job.json";
pub const SANDBOX_REPLAY: &str = "/run/robin-input/replay.rhrec";
pub const SANDBOX_CONTENT_ROOT: &str = "/run/robin-content";
pub const SANDBOX_RESULT: &str = "/run/robin-result.json";

/// Direct-launcher policy embedded in `worker.toml`. Every field is operator
/// authority and unknown fields are rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierLauncherConfig {
    pub bwrap_program: PathBuf,
    pub prlimit_program: PathBuf,
    pub verifier_program: PathBuf,
    pub wall_timeout_seconds: u64,
    pub cpu_limit_seconds: u64,
    pub address_space_limit_bytes: u64,
    pub process_limit: u32,
    pub open_files_limit: u32,
    pub file_size_limit_bytes: u64,
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
    /// Stable classification alongside the bounded operator diagnostics.
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

/// Host paths of one sandbox launch.
#[derive(Debug, Clone)]
pub struct SandboxPaths {
    pub verifier_program: PathBuf,
    pub job: PathBuf,
    pub replay: PathBuf,
    pub content_root: PathBuf,
    pub result: PathBuf,
}

impl VerifierLauncherConfig {
    /// Check paths and limits. Programs are resolved by their configured
    /// paths; there is deliberately no executable hash pin.
    pub fn validate(&self) -> Result<(), ProcessError> {
        for (name, path) in [
            ("bwrap_program", &self.bwrap_program),
            ("prlimit_program", &self.prlimit_program),
            ("verifier_program", &self.verifier_program),
        ] {
            validate_absolute_normalized_path(path, name)?;
            let metadata = std::fs::metadata(path).map_err(|error| {
                ProcessError::Configuration(format!("{name} is not accessible: {error}"))
            })?;
            let mode = metadata.permissions().mode();
            if !metadata.is_file() || mode & 0o111 == 0 || mode & 0o6000 != 0 {
                return Err(ProcessError::Configuration(format!(
                    "{name} must be an executable regular file without set-id bits"
                )));
            }
        }
        if self.wall_timeout_seconds == 0
            || self.wall_timeout_seconds > 60 * 60
            || self.cpu_limit_seconds == 0
            || self.cpu_limit_seconds > self.wall_timeout_seconds
            || !(64 * 1024 * 1024..=MAX_ADDRESS_SPACE_BYTES)
                .contains(&self.address_space_limit_bytes)
            || !(1..=128).contains(&self.process_limit)
            || !(16..=4096).contains(&self.open_files_limit)
            || self.file_size_limit_bytes < MAX_RESULT_BYTES
            || self.file_size_limit_bytes > MAX_FILE_SIZE_BYTES
        {
            return Err(ProcessError::Configuration(
                "invalid direct verifier timeout or rlimit policy".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn wall_timeout(&self) -> Duration {
        Duration::from_secs(self.wall_timeout_seconds)
    }

    /// Run one verifier job. `job_bytes` is the exact job document; `replay`
    /// is the verified content-addressed replay. Returns the bytes of the
    /// result file after a zero exit. A non-zero exit, timeout, missing or
    /// oversized result, or any sandbox fault is an error.
    pub async fn run(
        &self,
        job_bytes: &[u8],
        replay: tokio::fs::File,
        replay_bytes: u64,
        content_root: &Path,
    ) -> Result<Vec<u8>, ProcessError> {
        validate_content_root(content_root)?;
        let staging = stage_inputs(job_bytes, replay, replay_bytes).await?;
        let paths = SandboxPaths {
            verifier_program: self.verifier_program.clone(),
            job: staging.directory.path().join("job.json"),
            replay: staging.directory.path().join("replay.rhrec"),
            content_root: content_root.to_owned(),
            result: staging.directory.path().join("result.json"),
        };
        let launcher = self.clone();
        let result = staging.result;
        let result_identity = validate_output_file(&result, 0)?;
        let result = crate::physical_work::spawn_blocking(move || {
            launch(&launcher, &paths)?;
            validate_output_identity(&result, result_identity, MAX_RESULT_BYTES)?;
            read_result(result)
        })
        .await
        .map_err(|_| ProcessError::Exit("direct sandbox launcher task failed".to_owned()))??;
        drop(staging.directory);
        Ok(result)
    }
}

/// Decode and bind a verifier result to the exact job and replay handed to it.
pub fn decode_output(
    bytes: &[u8],
    job_bytes: &[u8],
    replay_sha256: Digest32,
) -> Result<VerifierOutputV2, ProcessError> {
    let output: VerifierOutputV2 = robin_run_protocol::strict_json::from_slice(bytes)
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    output
        .validate()
        .map_err(|error| ProcessError::InvalidResult(error.to_string()))?;
    if output.job_sha256 != Digest32::digest_bytes(job_bytes) {
        return Err(ProcessError::InvalidResult(
            "result does not bind the exact job document".to_owned(),
        ));
    }
    if output.replay_sha256 != replay_sha256 {
        return Err(ProcessError::InvalidResult(
            "result does not bind the stored replay".to_owned(),
        ));
    }
    Ok(output)
}

struct StagedInputs {
    directory: tempfile::TempDir,
    result: File,
}

async fn stage_inputs(
    job_bytes: &[u8],
    mut replay: tokio::fs::File,
    replay_bytes: u64,
) -> Result<StagedInputs, ProcessError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt as _;
    use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

    if job_bytes.is_empty() || job_bytes.len() > robin_run_protocol::MAX_VERIFIER_JOB_BYTES_V2 {
        return Err(ProcessError::Configuration(
            "verifier job document is empty or exceeds its byte limit".to_owned(),
        ));
    }
    let directory = tempfile::Builder::new()
        .prefix("robin-verifier-job-")
        .tempdir()?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    let create = |name: &str, mode: u32| -> Result<File, ProcessError> {
        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(directory.path().join(name))?)
    };
    {
        use std::io::Write as _;
        let mut job = create("job.json", 0o600)?;
        job.write_all(job_bytes)?;
        job.sync_all()?;
        job.set_permissions(std::fs::Permissions::from_mode(0o400))?;
    }
    {
        let mut staged = tokio::fs::File::from_std(create("replay.rhrec", 0o600)?);
        replay.seek(std::io::SeekFrom::Start(0)).await?;
        let copied =
            tokio::io::copy(&mut (&mut replay).take(replay_bytes + 1), &mut staged).await?;
        if copied != replay_bytes {
            return Err(ProcessError::InvalidResult(
                "staged replay length differs from the stored artifact".to_owned(),
            ));
        }
        staged.sync_all().await?;
        staged
            .set_permissions(std::fs::Permissions::from_mode(0o400))
            .await?;
    }
    let result = create("result.json", 0o600)?;
    Ok(StagedInputs { directory, result })
}

fn validate_content_root(path: &Path) -> Result<(), ProcessError> {
    validate_absolute_normalized_path(path, "content root")?;
    let descriptor = crate::secure_fs::open_dir_no_symlinks(path).map_err(|error| {
        ProcessError::Configuration(format!(
            "content root must be an existing directory without symlinks: {error}"
        ))
    })?;
    if !File::from(descriptor).metadata()?.is_dir() {
        return Err(ProcessError::Configuration(
            "content root must be a directory".to_owned(),
        ));
    }
    Ok(())
}

fn launch(config: &VerifierLauncherConfig, paths: &SandboxPaths) -> Result<(), ProcessError> {
    use std::os::unix::process::CommandExt as _;
    // An anonymous file cannot block the child on a full pipe. The child
    // file-size limit bounds disk usage; only a small prefix is read into logs.
    let mut stderr = tempfile::tempfile()?;
    let mut command = std::process::Command::new(&config.prlimit_program);
    command
        .env_clear()
        .args(prlimit_arguments(config))
        .arg(&config.bwrap_program)
        .args(bwrap_arguments(paths))
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr.try_clone()?);
    let mut child = command.spawn()?;
    let status = wait_for_process_group(&mut child, config.wall_timeout())?;
    status_result(status).map_err(|error| match read_stderr_diagnostic(&mut stderr) {
        Ok(detail) if !detail.is_empty() => {
            ProcessError::Exit(format!("{status}; stderr: {detail}"))
        }
        Ok(_) => error,
        Err(read_error) => {
            ProcessError::Exit(format!("{status}; stderr capture failed: {read_error}"))
        }
    })
}

/// `prlimit` options, followed by `--`; the caller appends the bwrap program.
pub fn prlimit_arguments(config: &VerifierLauncherConfig) -> Vec<OsString> {
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
    .collect()
}

/// The complete bubblewrap argument vector, ending with the verifier CLI.
pub fn bwrap_arguments(paths: &SandboxPaths) -> Vec<OsString> {
    let size = PRIVATE_TMPFS_BYTES.to_string();
    let mut arguments: Vec<OsString> = Vec::new();
    let mut push = |values: &[&str]| arguments.extend(values.iter().map(OsString::from));
    push(&[
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
        &size,
        "--tmpfs",
        "/",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--size",
        &size,
        "--tmpfs",
        "/run",
        "--size",
        &size,
        "--tmpfs",
        "/tmp",
        "--dir",
        "/var",
        "--size",
        &size,
        "--tmpfs",
        "/var/tmp",
        "--size",
        &size,
        "--tmpfs",
        "/home",
        "--dir",
        "/home/verifier",
        "--chmod",
        "0700",
        "/home/verifier",
        "--dir",
        "/run/robin-input",
    ]);
    for (flag, source, destination) in [
        ("--ro-bind", &paths.verifier_program, SANDBOX_VERIFIER),
        ("--ro-bind", &paths.job, SANDBOX_JOB),
        ("--ro-bind", &paths.replay, SANDBOX_REPLAY),
        ("--ro-bind", &paths.content_root, SANDBOX_CONTENT_ROOT),
        ("--bind", &paths.result, SANDBOX_RESULT),
    ] {
        arguments.push(flag.into());
        arguments.push(source.as_os_str().to_owned());
        arguments.push(destination.into());
    }
    arguments.extend(
        [
            "--remount-ro",
            "/",
            "--chdir",
            "/run",
            "--",
            SANDBOX_VERIFIER,
            "--job",
            SANDBOX_JOB,
            "--replay",
            SANDBOX_REPLAY,
            "--content-root",
            SANDBOX_CONTENT_ROOT,
            "--result",
            SANDBOX_RESULT,
        ]
        .into_iter()
        .map(OsString::from),
    );
    arguments
}

fn validate_absolute_normalized_path(path: &Path, name: &str) -> Result<(), ProcessError> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir))
        || components.any(|component| !matches!(component, Component::Normal(_)))
        || path.file_name().is_none()
    {
        return Err(ProcessError::Configuration(format!(
            "{name} must be an absolute normalized path"
        )));
    }
    Ok(())
}

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

fn read_stderr_diagnostic(stderr: &mut File) -> std::io::Result<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    const MAX_STDERR_BYTES: u64 = 8192;
    stderr.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    stderr.take(MAX_STDERR_BYTES + 1).read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MAX_STDERR_BYTES as usize;
    bytes.truncate(MAX_STDERR_BYTES as usize);
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str(" [stderr truncated]");
    }
    Ok(text)
}

fn status_result(status: ExitStatus) -> Result<(), ProcessError> {
    if status.success() {
        Ok(())
    } else {
        Err(ProcessError::Exit(status.to_string()))
    }
}

#[derive(Clone, Copy)]
struct OutputIdentity {
    device: u64,
    inode: u64,
}

fn validate_output_file(file: &File, expected_size: u64) -> Result<OutputIdentity, ProcessError> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() != expected_size
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(ProcessError::Configuration(
            "result output must be an exact private read-write inode".to_owned(),
        ));
    }
    Ok(OutputIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn validate_output_identity(
    file: &File,
    expected: OutputIdentity,
    maximum_size: u64,
) -> Result<(), ProcessError> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = file.metadata()?;
    if metadata.dev() != expected.device
        || metadata.ino() != expected.inode
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > maximum_size
    {
        return Err(ProcessError::InvalidResult(
            "result output is missing, replaced, empty, or oversized".to_owned(),
        ));
    }
    Ok(())
}

fn read_result(mut file: File) -> Result<Vec<u8>, ProcessError> {
    use std::io::{Read as _, Seek as _};
    file.seek(std::io::SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_RESULT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_RESULT_BYTES {
        return Err(ProcessError::InvalidResult(
            "result output grew beyond its byte limit".to_owned(),
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        CanonicalValue, InputProvenanceStatusV1, SCHEMA_VERSION_V2, VerificationRejectionCodeV1,
        VerificationRejectionV1, VerificationStatusV2,
    };

    fn config() -> VerifierLauncherConfig {
        VerifierLauncherConfig {
            bwrap_program: "/usr/bin/bwrap".into(),
            prlimit_program: "/usr/bin/prlimit".into(),
            verifier_program: "/usr/bin/true".into(),
            wall_timeout_seconds: 5,
            cpu_limit_seconds: 5,
            address_space_limit_bytes: 256 * 1024 * 1024,
            process_limit: 32,
            open_files_limit: 64,
            file_size_limit_bytes: 1024 * 1024,
        }
    }

    fn paths(root: &Path) -> SandboxPaths {
        SandboxPaths {
            verifier_program: root.join("bin/robin-replay-verifier"),
            job: root.join("staging/job.json"),
            replay: root.join("staging/replay.rhrec"),
            content_root: root.join("raw-content/demo"),
            result: root.join("staging/result.json"),
        }
    }

    #[test]
    fn launcher_validation_checks_paths_executables_and_limits() {
        config().validate().unwrap();
        let mut relative = config();
        relative.verifier_program = "bin/robin-replay-verifier".into();
        assert!(relative.validate().is_err());
        let mut missing = config();
        missing.verifier_program = "/nonexistent/robin-replay-verifier".into();
        assert!(missing.validate().is_err());
        let directory = tempfile::tempdir().unwrap();
        let not_executable = directory.path().join("verifier");
        std::fs::write(&not_executable, b"x").unwrap();
        std::fs::set_permissions(&not_executable, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut plain_file = config();
        plain_file.verifier_program = not_executable;
        assert!(plain_file.validate().is_err());
        let mut cpu = config();
        cpu.cpu_limit_seconds = 6;
        assert!(cpu.validate().is_err());
        let mut tiny_files = config();
        tiny_files.file_size_limit_bytes = MAX_RESULT_BYTES - 1;
        assert!(tiny_files.validate().is_err());
    }

    #[test]
    fn launcher_argv_is_the_exact_fixed_contract() {
        let root = Path::new("/srv/$(touch pwned)");
        let prlimit = prlimit_arguments(&config());
        assert_eq!(
            prlimit,
            [
                "--core=0",
                "--fsize=1048576",
                "--as=268435456",
                "--cpu=5",
                "--nofile=64",
                "--nproc=32",
                "--"
            ]
            .map(OsString::from)
        );
        let arguments = bwrap_arguments(&paths(root));
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
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| argument.as_os_str() == "--bind")
                .count(),
            1,
            "only the result file may be a writable host-backed mount"
        );
        assert!(arguments.windows(3).any(|window| {
            window[0] == "--ro-bind"
                && window[1] == root.join("raw-content/demo")
                && window[2] == SANDBOX_CONTENT_ROOT
        }));
        let separator = arguments
            .iter()
            .position(|argument| argument == "--")
            .unwrap();
        assert_eq!(
            arguments[separator + 1..],
            [
                SANDBOX_VERIFIER,
                "--job",
                SANDBOX_JOB,
                "--replay",
                SANDBOX_REPLAY,
                "--content-root",
                SANDBOX_CONTENT_ROOT,
                "--result",
                SANDBOX_RESULT,
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn actual_bwrap_mounts_inputs_read_only_result_writable_and_clears_environment() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let paths = paths(root);
        std::fs::create_dir_all(&paths.content_root).unwrap();
        std::fs::create_dir_all(paths.job.parent().unwrap()).unwrap();
        std::fs::create_dir_all(paths.verifier_program.parent().unwrap()).unwrap();
        std::fs::write(paths.content_root.join("raw.marker"), b"raw").unwrap();
        std::fs::write(&paths.job, b"job").unwrap();
        std::fs::write(&paths.replay, b"replay").unwrap();
        std::fs::write(&paths.result, b"").unwrap();
        std::fs::copy("/bin/sh", &paths.verifier_program).unwrap();
        let mut arguments = bwrap_arguments(&paths);
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
                SANDBOX_VERIFIER,
                "-c",
                r#"{ IFS= read -r job || :; } < /run/robin-input/job.json && test "$job" = job && { IFS= read -r replay || :; } < /run/robin-input/replay.rhrec && test "$replay" = replay &&test -r /run/robin-content/raw.marker && test "$HOME" = /home/verifier && test -z "${ROBIN_SENTINEL+x}" && ! (printf x > /run/robin-content/write-probe) 2>/dev/null && ! (printf x > /run/robin-input/job.json) 2>/dev/null && printf ok > /run/robin-result.json"#,
            ]
            .into_iter()
            .map(OsString::from),
        );
        let status = std::process::Command::new("/usr/bin/bwrap")
            .args(arguments)
            .env_clear()
            .env("ROBIN_SENTINEL", "must-not-cross")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .status()
            .unwrap();
        assert!(status.success(), "bubblewrap probe failed: {status}");
        assert_eq!(std::fs::read(&paths.result).unwrap(), b"ok");
        assert_eq!(std::fs::read(&paths.job).unwrap(), b"job");
        assert!(!paths.content_root.join("write-probe").exists());
    }

    #[test]
    fn stderr_diagnostic_is_bounded_and_tolerates_non_utf8() {
        use std::io::Write as _;
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&[0xff; 16_384]).unwrap();
        let detail = read_stderr_diagnostic(&mut file).unwrap();
        assert_eq!(detail.chars().count(), 8192 + " [stderr truncated]".len());
        assert!(detail.ends_with(" [stderr truncated]"));
    }

    #[test]
    fn wall_timeout_kills_and_reaps_the_fresh_process_group() {
        use std::os::unix::process::CommandExt as _;
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

    #[tokio::test]
    async fn failing_verifier_is_a_process_error_and_inputs_are_staged_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let content = directory.path().join("content");
        std::fs::create_dir(&content).unwrap();
        let replay_path = directory.path().join("replay");
        std::fs::write(&replay_path, b"replay").unwrap();
        // `/usr/bin/false` cannot run inside the empty sandbox root, so bwrap
        // exits non-zero: exactly the infrastructure-failure path.
        let mut launcher = config();
        launcher.verifier_program = "/usr/bin/false".into();
        let replay = tokio::fs::File::open(&replay_path).await.unwrap();
        let error = launcher.run(b"{}", replay, 6, &content).await.unwrap_err();
        assert!(matches!(error, ProcessError::Exit(_)), "{error}");
        assert!(error.to_string().contains("stderr:"), "{error}");

        let replay = tokio::fs::File::open(&replay_path).await.unwrap();
        assert!(matches!(
            launcher.run(b"{}", replay, 5, &content).await,
            Err(ProcessError::InvalidResult(_))
        ));
        let replay = tokio::fs::File::open(&replay_path).await.unwrap();
        assert!(
            launcher
                .run(b"{}", replay, 6, Path::new("relative"))
                .await
                .is_err()
        );
    }

    #[test]
    fn output_must_decode_validate_and_bind_job_and_replay() {
        let job = b"{\"job\":1}";
        let replay = Digest32::digest_bytes(b"replay");
        let rejected = VerifierOutputV2 {
            schema_version: SCHEMA_VERSION_V2,
            job_sha256: Digest32::digest_bytes(job),
            replay_sha256: replay,
            input_provenance: Some(InputProvenanceStatusV1::Rankable),
            status: VerificationStatusV2::Rejected(VerificationRejectionV1 {
                code: VerificationRejectionCodeV1::StateHashMismatch,
                detail_code: Some("frame_12".into()),
            }),
        };
        let bytes = serde_json::to_vec(&rejected).unwrap();
        assert_eq!(decode_output(&bytes, job, replay).unwrap(), rejected);
        assert!(decode_output(&bytes, b"other job", replay).is_err());
        assert!(decode_output(&bytes, job, Digest32::digest_bytes(b"other")).is_err());

        let mut unknown = serde_json::to_value(&rejected).unwrap();
        unknown["extra"] = serde_json::json!(1);
        assert!(decode_output(&serde_json::to_vec(&unknown).unwrap(), job, replay).is_err());
        let duplicate = br#"{"schema_version":2,"schema_version":2}"#;
        assert!(decode_output(duplicate, job, replay).is_err());

        let mut invalid = rejected;
        invalid.schema_version = 1;
        assert!(decode_output(&serde_json::to_vec(&invalid).unwrap(), job, replay).is_err());
        let _ = CanonicalValue::Null;
    }
}
