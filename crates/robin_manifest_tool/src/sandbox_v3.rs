//! Fixed Linux sandbox for BuildManifestV2-bound projection export.
//!
//! No API in this module accepts an arbitrary command, environment variable,
//! sandbox destination, or exporter argument. Operator-controlled host paths
//! are mounted at one fixed virtual topology and the exporter receives the one
//! frozen V2 argv contract.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use robin_run_protocol::{
    ArtifactRefV1, BuildManifestV2, ContentManifestV1, OfficialBuiltInOverlaySourceManifestV2,
    OfficialContentEditionV1, OfficialProjectionAuthorityManifestV2,
    OfficialProjectionExportReportV2, OfficialProjectionSourceFormatV1,
    OfficialSimulationProjectionReceiptV2, OfficialSourceTreeManifestV2, RulesConfigIdentityV1,
    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, SimulationContentComponentDocumentV1,
    SimulationContentComponentKindV1, Validate as _, canonical_json_bytes,
    official_content_subjects_v1, simulation_content_component_relative_path_v1,
};
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt as _;

use crate::{
    MAX_DOCUMENT_BYTES, artifact_from_bytes, artifact_from_file, read_regular_file_bounded,
    strict_json_from_slice, validate_mount_root, validate_regular_file, walk_regular_files,
};

const BUBBLEWRAP: &str = "/usr/bin/bwrap";
const PRLIMIT: &str = "/usr/bin/prlimit";
const EXPECTED_BUBBLEWRAP_VERSION: &str = "bubblewrap 0.12.0";
const EXPECTED_PRLIMIT_VERSION: &str = "prlimit from util-linux 2.41.5";
const TOOL_OUTPUT_LIMIT: usize = 4096;

const EXPORTER_SANDBOX_PATH: &str = "/authority/exporter";
const SOURCE_SANDBOX_PATH: &str = "/input";
const CORE_SANDBOX_PATH: &str = "/core";
const OUTPUT_SANDBOX_PATH: &str = "/output";

const AUTHORITY_FILES: [&str; 7] = [
    "build-manifest.json",
    "core-overlay-manifest.json",
    "execution-policy.json",
    "exporter",
    "projection-authority-manifest.json",
    "rules-config.json",
    "source-manifest.json",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxToolIdentityV1 {
    pub path: String,
    pub version: String,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxResourceLimitsV1 {
    pub core_bytes: u64,
    pub open_files: u64,
    pub address_space_bytes: u64,
    pub file_size_bytes: u64,
    pub cpu_seconds: u64,
    pub processes: u64,
    pub stack_bytes: u64,
    pub wall_timeout_seconds: u64,
    pub diagnostic_bytes_per_stream: u64,
}

impl SandboxResourceLimitsV1 {
    pub const OFFICIAL_V1: Self = Self {
        core_bytes: 0,
        open_files: 256,
        address_space_bytes: 16 * 1024 * 1024 * 1024,
        file_size_bytes: 512 * 1024 * 1024,
        cpu_seconds: 12 * 60 * 60,
        // Export decoding is fixed to four Rayon workers. Leave bounded room
        // for the exporter main/helper threads and sandbox launcher without
        // granting a fork-bomb-sized process namespace.
        processes: 32,
        stack_bytes: 64 * 1024 * 1024,
        wall_timeout_seconds: 12 * 60 * 60,
        diagnostic_bytes_per_stream: 8 * 1024 * 1024,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxRuntimeIdentityV1 {
    pub schema_version: u32,
    pub bubblewrap: SandboxToolIdentityV1,
    pub prlimit: SandboxToolIdentityV1,
    pub limits: SandboxResourceLimitsV1,
}

#[derive(Debug, Clone)]
pub struct SandboxedProjectionRequest {
    pub authority_root: PathBuf,
    pub source_root: PathBuf,
    pub core_overlay_root: PathBuf,
    pub output_root: PathBuf,
    pub edition: OfficialContentEditionV1,
    pub source_format: OfficialProjectionSourceFormatV1,
    pub expected_exporter: ArtifactRefV1,
}

#[derive(Debug)]
pub struct SandboxedProjectionOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub runtime: SandboxRuntimeIdentityV1,
}

#[derive(Debug)]
pub struct ValidatedSandboxProjection {
    pub report: OfficialProjectionExportReportV2,
    pub receipt: OfficialSimulationProjectionReceiptV2,
    pub content: Vec<ContentManifestV1>,
    pub runtime: SandboxRuntimeIdentityV1,
    /// Exact canonical single-object stdout retained for the private execution
    /// record. It has already been cross-validated against every authority.
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
struct BoundedOutput {
    bytes: Vec<u8>,
    total_bytes: u64,
}

/// Prove the exact host safety prerequisites before authoring starts. These
/// identities are recorded in the private execution record but are not part
/// of the content receipt authority.
pub fn probe_sandbox_runtime_v1() -> Result<SandboxRuntimeIdentityV1> {
    Ok(SandboxRuntimeIdentityV1 {
        schema_version: 1,
        bubblewrap: probe_tool(BUBBLEWRAP, EXPECTED_BUBBLEWRAP_VERSION)?,
        prlimit: probe_tool(PRLIMIT, EXPECTED_PRLIMIT_VERSION)?,
        limits: SandboxResourceLimitsV1::OFFICIAL_V1,
    })
}

/// Execute exactly one Demo/Full × loose/RHDDNA10 lane.
pub fn run_projection_exporter_v2(
    request: &SandboxedProjectionRequest,
) -> Result<SandboxedProjectionOutput> {
    validate_request(request)?;
    let runtime = probe_sandbox_runtime_v1()?;
    crate::validate_static_projection_exporter(
        &request.authority_root.join("exporter"),
        &request.expected_exporter,
    )?;

    let bwrap_args = bubblewrap_arguments(request)?;
    let limits = &runtime.limits;
    let mut command = Command::new(PRLIMIT);
    command
        .arg(format!("--core={0}:{0}", limits.core_bytes))
        .arg(format!("--nofile={0}:{0}", limits.open_files))
        .arg(format!("--as={0}:{0}", limits.address_space_bytes))
        .arg(format!("--fsize={0}:{0}", limits.file_size_bytes))
        .arg(format!("--cpu={0}:{0}", limits.cpu_seconds))
        .arg(format!("--nproc={0}:{0}", limits.processes))
        .arg(format!("--stack={0}:{0}", limits.stack_bytes))
        .arg("--")
        .arg(BUBBLEWRAP)
        .args(&bwrap_args)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let (status, stdout, stderr) = run_bounded_command(
        &mut command,
        Duration::from_secs(limits.wall_timeout_seconds),
        usize::try_from(limits.diagnostic_bytes_per_stream)
            .context("diagnostic limit does not fit usize")?,
    )?;
    if !status.success() {
        bail!(projection_failure_message(status, &stderr));
    }
    // Re-probe absolute tools after execution and reject an in-place host-tool
    // replacement during the run.
    ensure!(
        probe_sandbox_runtime_v1()? == runtime,
        "sandbox prerequisites changed during projection export"
    );
    crate::validate_static_projection_exporter(
        &request.authority_root.join("exporter"),
        &request.expected_exporter,
    )?;
    Ok(SandboxedProjectionOutput {
        status,
        stdout,
        stderr,
        runtime,
    })
}

/// Admit a successful child only after strict stdout, receipt and catalog
/// validation. This function does not publish or rename anything; the caller
/// must validate all four lanes and their matrix before atomically committing
/// a release tree.
pub fn validate_projection_output_v2(
    request: &SandboxedProjectionRequest,
    output: SandboxedProjectionOutput,
    build: &BuildManifestV2,
    projection_authority: &OfficialProjectionAuthorityManifestV2,
    rules_config: &RulesConfigIdentityV1,
    source_tree: &OfficialSourceTreeManifestV2,
    built_in_overlay: &OfficialBuiltInOverlaySourceManifestV2,
) -> Result<ValidatedSandboxProjection> {
    ensure!(
        output.status.success(),
        "projection child was not successful"
    );
    ensure!(
        !output.stdout.is_empty()
            && !output.stdout.contains(&b'\n')
            && !output.stdout.contains(&b'\r'),
        "projection stdout must be one canonical JSON object without a newline"
    );
    let report: OfficialProjectionExportReportV2 =
        strict_json_from_slice(&output.stdout).context("parse typed projection stdout report")?;
    report.validate()?;
    ensure!(
        canonical_json_bytes(&report)? == output.stdout,
        "projection stdout report is not byte-for-byte canonical JSON"
    );
    ensure!(
        report.catalog_root == "/output/catalog" && report.receipt_root == "/output/receipt",
        "projection report names unexpected sandbox output roots"
    );
    ensure!(
        report.edition == request.edition && report.source_format == request.source_format,
        "projection report lane differs from the requested lane"
    );
    ensure_exact_output_topology(&request.output_root)?;

    let receipt_root = request.output_root.join("receipt");
    ensure!(
        exact_directory_files(&receipt_root)?
            == ["projection-receipt.json", "source-tree-manifest.json"].map(str::to_owned),
        "projection receipt directory has an unexpected inventory"
    );
    let emitted_source_bytes = read_regular_file_bounded(
        &receipt_root.join("source-tree-manifest.json"),
        MAX_DOCUMENT_BYTES,
    )?;
    let emitted_source: OfficialSourceTreeManifestV2 =
        strict_json_from_slice(&emitted_source_bytes)
            .context("parse emitted V2 source manifest")?;
    emitted_source.validate()?;
    ensure!(
        canonical_json_bytes(&emitted_source)? == emitted_source_bytes
            && emitted_source == *source_tree,
        "exporter emitted a substituted or noncanonical source manifest"
    );
    let receipt_bytes = read_regular_file_bounded(
        &receipt_root.join("projection-receipt.json"),
        MAX_DOCUMENT_BYTES,
    )?;
    let receipt: OfficialSimulationProjectionReceiptV2 =
        strict_json_from_slice(&receipt_bytes).context("parse emitted V2 projection receipt")?;
    receipt.validate_against(
        build,
        projection_authority,
        rules_config,
        source_tree,
        built_in_overlay,
    )?;
    ensure!(
        canonical_json_bytes(&receipt)? == receipt_bytes,
        "projection receipt is not byte-for-byte canonical JSON"
    );
    report.validate_against(
        &receipt,
        build,
        projection_authority,
        rules_config,
        source_tree,
        built_in_overlay,
    )?;

    let content = validate_catalog(&request.output_root.join("catalog"), &receipt)?;
    ensure!(
        u32::try_from(
            content
                .len()
                .checked_mul(8)
                .context("component count overflow")?
        )
        .ok()
            == Some(report.component_file_count),
        "catalog component count differs from the stdout report"
    );
    Ok(ValidatedSandboxProjection {
        report,
        receipt,
        content,
        runtime: output.runtime,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn ensure_exact_output_topology(root: &Path) -> Result<()> {
    let mut entries = fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut names = Vec::new();
    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "projection output root contains a non-directory entry"
        );
        names.push(
            entry
                .file_name()
                .to_str()
                .context("projection output path is not UTF-8")?
                .to_owned(),
        );
    }
    ensure!(
        names == ["catalog", "receipt"],
        "projection output root must contain exactly catalog and receipt"
    );
    Ok(())
}

fn validate_catalog(
    root: &Path,
    receipt: &OfficialSimulationProjectionReceiptV2,
) -> Result<Vec<ContentManifestV1>> {
    validate_mount_root(root)?;
    let kinds = [
        SimulationContentComponentKindV1::Profiles,
        SimulationContentComponentKindV1::LoadedLevel,
        SimulationContentComponentKindV1::MissionScripts,
        SimulationContentComponentKindV1::SpriteSimulationMetadata,
        SimulationContentComponentKindV1::MapGeometryMetadata,
        SimulationContentComponentKindV1::LocalizedDeterministicText,
        SimulationContentComponentKindV1::SoundDurationTables,
        SimulationContentComponentKindV1::InterfaceSimulationMetadata,
    ];
    let expected_subjects = official_content_subjects_v1(receipt.edition);
    let receipt_subjects = receipt
        .subjects
        .iter()
        .map(|subject| subject.content_manifest.subject.clone())
        .collect::<Vec<_>>();
    ensure!(
        receipt_subjects == expected_subjects,
        "receipt subject order differs from the authentic edition matrix"
    );
    let mut expected_paths = Vec::new();
    let mut content = Vec::with_capacity(receipt.subjects.len());
    for subject_receipt in &receipt.subjects {
        let manifest = &subject_receipt.content_manifest;
        ensure!(
            manifest.components.len() == kinds.len(),
            "content manifest has an incomplete component set"
        );
        for (kind, component) in kinds.iter().zip(&manifest.components) {
            ensure!(
                component.kind == *kind,
                "component manifest order is not canonical"
            );
            let relative = simulation_content_component_relative_path_v1(&manifest.subject, *kind)?;
            let path = root.join(&relative);
            let bytes = read_regular_file_bounded(&path, MAX_DOCUMENT_BYTES)?;
            let document: SimulationContentComponentDocumentV1 =
                SimulationContentComponentDocumentV1::from_bitcode(&bytes)
                    .with_context(|| format!("parse projection component {relative}"))?;
            document.validate()?;
            ensure!(
                document.kind == *kind
                    && document.component_schema_version == component.component_schema_version
                    && document.bitcode_bytes()? == bytes,
                "projection component document does not match its typed manifest"
            );
            ensure!(
                artifact_from_bytes(&bytes, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1)
                    == component.artifact,
                "projection component bytes differ from their content manifest artifact"
            );
            expected_paths.push(PathBuf::from(relative));
        }
        content.push(manifest.clone());
    }
    expected_paths.sort();
    let actual_paths = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, _)| relative)
        .collect::<Vec<_>>();
    ensure!(
        actual_paths == expected_paths,
        "projection catalog has missing or extra files"
    );
    Ok(content)
}

fn validate_request(request: &SandboxedProjectionRequest) -> Result<()> {
    for root in [
        &request.authority_root,
        &request.source_root,
        &request.core_overlay_root,
        &request.output_root,
    ] {
        validate_mount_root(root)?;
    }
    let roots = [
        fs::canonicalize(&request.authority_root)?,
        fs::canonicalize(&request.source_root)?,
        fs::canonicalize(&request.core_overlay_root)?,
        fs::canonicalize(&request.output_root)?,
    ];
    ensure!(
        roots.iter().enumerate().all(|(left_index, left)| {
            roots.iter().enumerate().all(|(right_index, right)| {
                left_index == right_index || (!left.starts_with(right) && !right.starts_with(left))
            })
        }),
        "sandbox host roots must be distinct and non-overlapping"
    );
    let authority = exact_directory_files(&request.authority_root)?;
    ensure!(
        authority == AUTHORITY_FILES.map(str::to_owned),
        "authority root must contain exactly the seven fixed V2 inputs"
    );
    ensure!(
        exact_directory_files(&request.output_root)?.is_empty(),
        "sandbox output root must be empty"
    );
    ensure!(
        !request.output_root.join("catalog").exists()
            && !request.output_root.join("receipt").exists(),
        "sandbox output directories must be absent"
    );
    Ok(())
}

fn exact_directory_files(root: &Path) -> Result<Vec<String>> {
    let mut entries = fs::read_dir(root)
        .with_context(|| format!("enumerate exact sandbox directory {}", root.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut names = Vec::with_capacity(entries.len());
    for entry in &entries {
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "sandbox directory contains non-regular entry {}",
            entry.path().display()
        );
        names.push(
            entry
                .file_name()
                .to_str()
                .context("sandbox input filename is not UTF-8")?
                .to_owned(),
        );
    }
    Ok(names)
}

fn bubblewrap_arguments(request: &SandboxedProjectionRequest) -> Result<Vec<OsString>> {
    let edition = match request.edition {
        OfficialContentEditionV1::Demo => "demo",
        OfficialContentEditionV1::Full => "full",
    };
    let source = match request.source_format {
        OfficialProjectionSourceFormatV1::LooseNativeV1 => "loose-native",
        OfficialProjectionSourceFormatV1::ShippingDatadirV10 => "shipping-datadir-v10",
    };
    let mut args = Vec::<OsString>::new();
    let mut push = |value: &dyn AsRef<OsStr>| args.push(value.as_ref().to_os_string());
    for value in [
        "--unshare-all",
        "--clearenv",
        "--die-with-parent",
        "--new-session",
        // bubblewrap 0.12 expands --unshare-all to --unshare-user-try. Its
        // --disable-userns safety control intentionally requires a mandatory
        // user namespace, so spell that contract out explicitly.
        "--unshare-user",
        "--disable-userns",
        "--hostname",
        "robin-projection-v2",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--size",
        "4294967296",
        "--tmpfs",
        "/tmp",
        "--ro-bind",
    ] {
        push(&value);
    }
    push(&request.authority_root);
    push(&"/authority");
    push(&"--ro-bind");
    push(&request.source_root);
    push(&SOURCE_SANDBOX_PATH);
    push(&"--ro-bind");
    push(&request.core_overlay_root);
    push(&CORE_SANDBOX_PATH);
    push(&"--bind");
    push(&request.output_root);
    push(&OUTPUT_SANDBOX_PATH);
    for (name, value) in [
        ("LC_ALL", "C.UTF-8"),
        ("LANG", "C.UTF-8"),
        ("TZ", "UTC"),
        ("TMPDIR", "/tmp"),
        ("SOURCE_DATE_EPOCH", "0"),
        ("RUST_BACKTRACE", "0"),
    ] {
        push(&"--setenv");
        push(&name);
        push(&value);
    }
    for value in [
        "--chdir",
        "/",
        "--",
        EXPORTER_SANDBOX_PATH,
        "--source-root",
        SOURCE_SANDBOX_PATH,
        "--source-manifest",
        "/authority/source-manifest.json",
        "--core-overlay-root",
        CORE_SANDBOX_PATH,
        "--core-overlay-manifest",
        "/authority/core-overlay-manifest.json",
        "--public-build-manifest",
        "/authority/build-manifest.json",
        "--projection-authority-manifest",
        "/authority/projection-authority-manifest.json",
        "--rules-config",
        "/authority/rules-config.json",
        "--execution-policy",
        "/authority/execution-policy.json",
        "--catalog-output",
        "/output/catalog",
        "--receipt-output",
        "/output/receipt",
        "--edition",
        edition,
        "--source",
        source,
    ] {
        push(&value);
    }
    Ok(args)
}

fn projection_failure_message(status: ExitStatus, stderr: &[u8]) -> String {
    let retained = &stderr[..stderr.len().min(TOOL_OUTPUT_LIMIT)];
    let truncation = if retained.len() == stderr.len() {
        ""
    } else {
        " (truncated)"
    };
    format!(
        "projection exporter exited with {status}; captured stderr{truncation}: {:?}",
        String::from_utf8_lossy(retained)
    )
}

fn probe_tool(path: &str, expected_version: &str) -> Result<SandboxToolIdentityV1> {
    let path = Path::new(path);
    let metadata = validate_regular_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "{} is not executable",
            path.display()
        );
    }
    ensure!(
        fs::canonicalize(path)? == path,
        "sandbox prerequisite path is not normalized: {}",
        path.display()
    );
    let output = Command::new(path)
        .arg("--version")
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("execute sandbox prerequisite {}", path.display()))?;
    ensure!(
        output.status.success(),
        "{} --version failed",
        path.display()
    );
    ensure!(
        output.stdout.len() <= TOOL_OUTPUT_LIMIT && output.stderr.len() <= TOOL_OUTPUT_LIMIT,
        "{} --version produced excessive diagnostics",
        path.display()
    );
    let version = std::str::from_utf8(&output.stdout)
        .context("sandbox prerequisite version is not UTF-8")?
        .trim_end_matches(['\r', '\n']);
    ensure!(
        output.stderr.is_empty() && version == expected_version,
        "unexpected sandbox prerequisite version for {}: {:?}",
        path.display(),
        version
    );
    Ok(SandboxToolIdentityV1 {
        path: path
            .to_str()
            .context("sandbox tool path is not UTF-8")?
            .into(),
        version: version.into(),
        artifact: artifact_from_file(path, "application/octet-stream")?,
    })
}

fn run_bounded_command(
    command: &mut Command,
    timeout: Duration,
    maximum_stream_bytes: usize,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let mut child = command.spawn().context("spawn projection sandbox")?;
    let stdout = child
        .stdout
        .take()
        .context("sandbox stdout was not piped")?;
    let stderr = child
        .stderr
        .take()
        .context("sandbox stderr was not piped")?;
    let stdout_thread = thread::spawn(move || drain_bounded(stdout, maximum_stream_bytes));
    let stderr_thread = thread::spawn(move || drain_bounded(stderr, maximum_stream_bytes));
    let status = match child.wait_timeout(timeout)? {
        Some(status) => status,
        None => {
            child.kill().context("kill timed-out projection sandbox")?;
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            bail!("projection sandbox exceeded {} seconds", timeout.as_secs());
        }
    };
    let stdout = stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("sandbox stdout drain panicked"))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| anyhow::anyhow!("sandbox stderr drain panicked"))??;
    ensure!(
        stdout.total_bytes <= maximum_stream_bytes as u64,
        "projection stdout exceeded the {} byte diagnostic limit",
        maximum_stream_bytes
    );
    ensure!(
        stderr.total_bytes <= maximum_stream_bytes as u64,
        "projection stderr exceeded the {} byte diagnostic limit",
        maximum_stream_bytes
    );
    Ok((status, stdout.bytes, stderr.bytes))
}

fn drain_bounded(mut input: impl std::io::Read, maximum: usize) -> Result<BoundedOutput> {
    let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
    let mut total_bytes = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes
            .checked_add(read as u64)
            .context("diagnostic byte count overflow")?;
        if bytes.len() < maximum {
            let retained = read.min(maximum - bytes.len());
            bytes.write_all(&buffer[..retained])?;
        }
    }
    Ok(BoundedOutput { bytes, total_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_runner_rejects_timeout_and_oversize_output() {
        let mut timeout = Command::new("/bin/sh");
        timeout
            .args(["-c", "exec sleep 10"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        assert!(run_bounded_command(&mut timeout, Duration::from_millis(50), 1024).is_err());

        let mut oversized = Command::new("/usr/bin/printf");
        oversized
            .arg("%01024d")
            .arg("0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        assert!(run_bounded_command(&mut oversized, Duration::from_secs(2), 32).is_err());
    }

    #[test]
    fn bwrap_argv_has_closed_topology_and_no_network_or_home() -> Result<()> {
        let request = SandboxedProjectionRequest {
            authority_root: PathBuf::from("/host/authority"),
            source_root: PathBuf::from("/host/input"),
            core_overlay_root: PathBuf::from("/host/core"),
            output_root: PathBuf::from("/host/output"),
            edition: OfficialContentEditionV1::Full,
            source_format: OfficialProjectionSourceFormatV1::ShippingDatadirV10,
            expected_exporter: ArtifactRefV1 {
                sha256: robin_run_protocol::Digest32::from_bytes([1; 32]),
                byte_length: 1,
                media_type: robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2.into(),
            },
        };
        let arguments = bubblewrap_arguments(&request)?
            .into_iter()
            .map(|argument| argument.into_string().unwrap())
            .collect::<Vec<_>>();
        assert!(arguments.iter().any(|argument| argument == "--unshare-all"));
        let unshare_user = arguments
            .iter()
            .position(|argument| argument == "--unshare-user")
            .expect("mandatory user namespace argument");
        let disable_userns = arguments
            .iter()
            .position(|argument| argument == "--disable-userns")
            .expect("nested user namespace disable argument");
        assert_eq!(unshare_user + 1, disable_userns);
        assert!(arguments.iter().any(|argument| argument == "--clearenv"));
        assert!(!arguments.iter().any(|argument| argument == "--share-net"));
        assert!(!arguments.iter().any(|argument| argument.contains("HOME")));
        assert!(!arguments.iter().any(|argument| argument == "/usr"));
        assert_eq!(arguments.last().unwrap(), "shipping-datadir-v10");
        Ok(())
    }

    #[test]
    fn projection_failure_surfaces_bounded_stderr() {
        let status = Command::new("/bin/sh")
            .args(["-c", "exit 19"])
            .status()
            .unwrap();
        let message = projection_failure_message(status, b"bwrap: rejected sandbox argv\n");
        assert!(message.contains("exit status: 19"));
        assert!(message.contains("bwrap: rejected sandbox argv\\n"));

        let oversized = vec![b'x'; TOOL_OUTPUT_LIMIT + 1];
        let message = projection_failure_message(status, &oversized);
        assert!(message.contains("captured stderr (truncated)"));
        assert!(message.ends_with(&format!("{:?}", "x".repeat(TOOL_OUTPUT_LIMIT))));
    }

    #[test]
    fn output_topology_rejects_extra_partial_and_symlink_entries() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("catalog"))?;
        assert!(ensure_exact_output_topology(root.path()).is_err());
        fs::create_dir(root.path().join("receipt"))?;
        ensure_exact_output_topology(root.path())?;

        fs::write(root.path().join("unexpected"), b"partial run marker")?;
        assert!(ensure_exact_output_topology(root.path()).is_err());
        fs::remove_file(root.path().join("unexpected"))?;

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("catalog", root.path().join("redirect"))?;
            assert!(ensure_exact_output_topology(root.path()).is_err());
        }
        Ok(())
    }
}
