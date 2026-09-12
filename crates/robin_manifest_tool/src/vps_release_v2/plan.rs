//! plan responsibilities of the admitted release pipeline.
use super::*;

pub(super) fn validate_plan_shape(plan: &VpsReleasePlanV2) -> Result<()> {
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

pub(super) fn ensure_strict_roles<T: Ord + Copy>(
    roles: impl Iterator<Item = T>,
    label: &str,
) -> Result<()> {
    let roles = roles.collect::<Vec<_>>();
    ensure!(
        roles.windows(2).all(|pair| pair[0] < pair[1]),
        "{label} repeat or are not in canonical order"
    );
    Ok(())
}

pub(super) fn validate_assembly_inputs(
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

pub(super) fn validate_pinned_file(path: &Path, expected: &ArtifactRefV1) -> Result<()> {
    reject_hardlink(path)?;
    ensure!(
        artifact_from_file(path, &expected.media_type)? == *expected,
        "pinned release source {} differs from its artifact identity",
        path.display()
    );
    Ok(())
}

pub(super) fn validate_linux_executable_source(path: &Path) -> Result<()> {
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

pub(super) fn validate_linux_elf(path: &Path) -> Result<()> {
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
