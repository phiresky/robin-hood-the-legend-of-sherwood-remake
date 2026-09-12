//! staging responsibilities of the admitted release pipeline.
use super::*;

pub(super) fn materialize_publication(
    root: &Path,
    loaded: &LoadedPublication,
) -> Result<PublicationTreeAuthorityV3> {
    let authority_root = &loaded.plan.official_content_authority;
    // Backend immutable authorities.
    copy_directory_exact(
        &authority_root.join("manifests/content-manifests"),
        &root.join("backend/manifests/content-manifests"),
    )?;
    copy_directory_exact(
        &authority_root.join("manifests/campaign-content-manifests"),
        &root.join("backend/manifests/campaign-content-manifests"),
    )?;
    copy_directory_exact(
        &authority_root.join("manifests/rules-configs"),
        &root.join("backend/manifests/rules-configs"),
    )?;
    write_digest_document(
        &root.join("backend"),
        "manifests/builds",
        loaded.build_sha256,
        &loaded.authority.build,
    )?;
    for (digest, config) in &loaded.rules_configs {
        let path = root
            .join("backend/manifests/rules-configs")
            .join(format!("{digest}.json"));
        if !path.exists() {
            write_canonical(&path, config)?;
        }
    }
    for (digest, policy) in &loaded.policies {
        write_digest_document(&root.join("backend"), "manifests/policies", *digest, policy)?;
    }
    for (digest, published) in &loaded.published {
        write_digest_document(
            &root.join("backend"),
            "manifests/ruleset-manifests",
            *digest,
            &published.manifest,
        )?;
        write_digest_document(
            &root.join("backend"),
            "manifests/published-rulesets",
            *digest,
            published,
        )?;
    }
    fs::create_dir_all(root.join("backend/manifests/competitions"))?;
    for (digest, competition) in &loaded.competitions {
        write_digest_document(
            &root.join("backend"),
            "manifests/competitions",
            *digest,
            competition,
        )?;
    }

    // Preserve exactly one complete, independently validatable Plan-V3
    // authority below the publication. Downstream assemblers select their
    // operational inputs from this tree; publication files stay regular,
    // singleton files so link topology cannot bypass release checks.
    let private_root = create_private_publication_root(root)?;
    let authority_output = private_root.join("official-content-authority");
    let copied_official_authority =
        copy_directory_exact_preserving_modes(authority_root, &authority_output)?;
    copy_artifact_exact(
        &loaded.plan.viewer_build_report,
        &root.join("private/viewer-build-reports-v2").join(format!(
            "{}.json",
            loaded.viewer_build_report_artifact.sha256
        )),
        &loaded.viewer_build_report_artifact,
    )?;
    // Verifier program and operator config are never public.
    copy_artifact_exact(
        &loaded.build_draft.verifier,
        &root
            .join("private/verifier/bin")
            .join(loaded.authority.build.verifier.artifact.sha256.to_string()),
        &loaded.authority.build.verifier.artifact,
    )?;
    copy_artifact_exact(
        &loaded.plan.verifier_operator_config.source,
        &root.join("private/verifier/operator-config").join(
            loaded
                .plan
                .verifier_operator_config
                .artifact
                .sha256
                .to_string(),
        ),
        &loaded.plan.verifier_operator_config.artifact,
    )?;
    let mut copied_campaigns = BTreeMap::<Digest32, ArtifactRefV1>::new();
    for state in &loaded.plan.campaign_states {
        if let Some(previous) = copied_campaigns.get(&state.artifact.sha256) {
            ensure!(
                previous == &state.artifact,
                "logical campaign pins disagree about one physical artifact"
            );
            continue;
        }
        copy_artifact_exact(
            &state.source,
            &root
                .join("private/campaign-states")
                .join(state.artifact.sha256.to_string()),
            &state.artifact,
        )?;
        copied_campaigns.insert(state.artifact.sha256, state.artifact.clone());
    }

    // The normal public-static origin contains the application closure only.
    // Demo datadir payload bytes are assembled and deployed independently by
    // robinhood-datadir-assets; this publication binds only its canonical
    // authority and deployment receipt below.
    fs::create_dir_all(root.join("cloudflare-public"))?;
    write_digest_document(
        &root.join("cloudflare-public"),
        "manifests/builds",
        loaded.build_sha256,
        &loaded.authority.build,
    )?;
    let viewer_sources = loaded
        .build_draft
        .viewer_engine_artifacts
        .iter()
        .map(|source| (source.published_path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        viewer_sources.len() == loaded.build_draft.viewer_engine_artifacts.len(),
        "viewer source paths repeat"
    );
    for named in &loaded.authority.build.viewer.engine.artifacts {
        let source = viewer_sources
            .get(named.path.as_str())
            .context("BuildManifestV2 viewer source is absent")?;
        let relative = build_artifact_object_path_v1(loaded.build_sha256, named)?;
        copy_artifact_exact(
            &source.source,
            &root.join("cloudflare-public").join(relative),
            &named.artifact,
        )?;
    }
    let pages_sources = loaded
        .build_draft
        .pages_shell_artifacts
        .iter()
        .map(|source| (source.published_path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        pages_sources.len() == loaded.build_draft.pages_shell_artifacts.len(),
        "public-static shell source paths repeat"
    );
    for file in &loaded
        .authority
        .build
        .viewer
        .pages_shell
        .public_origin_artifacts
    {
        let source = pages_sources
            .get(file.path.as_str())
            .context("BuildManifestV2 public-static source is absent")?;
        copy_artifact_exact(
            &source.source,
            &root.join("cloudflare-public").join(&file.path),
            &file.artifact,
        )?;
    }
    let signer_sources = loaded
        .build_draft
        .identity_signer_artifacts
        .iter()
        .map(|source| (source.published_path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        signer_sources.len() == loaded.build_draft.identity_signer_artifacts.len(),
        "identity signer source paths repeat"
    );
    for file in &loaded
        .authority
        .build
        .viewer
        .identity_signer
        .identity_signer_origin_artifacts
    {
        let source = signer_sources
            .get(file.path.as_str())
            .context("BuildManifestV2 identity signer source is absent")?;
        copy_artifact_exact(
            &source.source,
            &root.join("cloudflare-identity-signer").join(&file.path),
            &file.artifact,
        )?;
    }

    copy_artifact_exact(
        &loaded.plan.datadir_release_authority.source,
        &root.join(DATADIR_AUTHORITY_PATH),
        &loaded.plan.datadir_release_authority.artifact,
    )?;
    copy_artifact_exact(
        &loaded.plan.datadir_deployment_receipt.source,
        &root.join(DATADIR_DEPLOYMENT_RECEIPT_PATH),
        &loaded.plan.datadir_deployment_receipt.artifact,
    )?;

    let backend = backend_publication(loaded)?;
    write_canonical(&root.join("backend/publication-v3.json"), &backend)?;
    write_canonical(
        &root.join("deployment/exposure-v3.json"),
        &DeploymentExposureV3::official(),
    )?;
    Ok(copied_official_authority)
}

pub(super) fn backend_publication(loaded: &LoadedPublication) -> Result<BackendPublicationV3> {
    let mut content = loaded.authority.content.keys().copied().collect::<Vec<_>>();
    content.sort();
    let mut campaigns = loaded
        .authority
        .campaigns
        .keys()
        .copied()
        .collect::<Vec<_>>();
    campaigns.sort();
    let document = BackendPublicationV3 {
        schema_version: BACKEND_PUBLICATION_SCHEMA_VERSION,
        build_manifest_sha256: loaded.build_sha256,
        content_manifest_sha256: content,
        campaign_content_manifest_sha256: campaigns,
        rules_config_sha256: loaded.rules_configs.keys().copied().collect(),
        ruleset_manifest_sha256: loaded.published.keys().copied().collect(),
        competition_manifest_sha256: loaded.competitions.keys().copied().collect(),
        policy_manifest_sha256: loaded.policies.keys().copied().collect(),
        verifier_program: loaded.authority.build.verifier.artifact.clone(),
        verifier_operator_config: loaded.plan.verifier_operator_config.artifact.clone(),
        campaign_states: loaded.campaign_states.clone(),
    };
    document.validate()?;
    Ok(document)
}

pub(super) fn publication_manifest(loaded: &LoadedPublication) -> Result<PublicationManifestV3> {
    let public_static_files = loaded
        .authority
        .build
        .viewer
        .pages_shell
        .public_origin_artifacts
        .iter()
        .map(|file| PublicStaticFileArtifactV3 {
            published_path: file.path.clone(),
            artifact: file.artifact.clone(),
        })
        .collect::<Vec<_>>();
    let identity_signer_files = loaded
        .authority
        .build
        .viewer
        .identity_signer
        .identity_signer_origin_artifacts
        .iter()
        .map(|file| PublicStaticFileArtifactV3 {
            published_path: file.path.clone(),
            artifact: file.artifact.clone(),
        })
        .collect::<Vec<_>>();
    let document = PublicationManifestV3 {
        schema_version: PUBLICATION_MANIFEST_SCHEMA_VERSION,
        projection_authority_matrix_sha256: loaded.authority.matrix.canonical_digest()?,
        official_content_digests_sha256: loaded.authority.digests.canonical_digest()?,
        build_manifest_sha256: loaded.build_sha256,
        viewer_build_report: loaded.viewer_build_report_artifact.clone(),
        datadir_release_authority: loaded.plan.datadir_release_authority.artifact.clone(),
        datadir_deployment_receipt: loaded.plan.datadir_deployment_receipt.artifact.clone(),
        verifier_operator_config: loaded.plan.verifier_operator_config.artifact.clone(),
        campaign_states: loaded.campaign_states.clone(),
        rules_config_sha256: loaded.rules_configs.keys().copied().collect(),
        policy_manifest_sha256: loaded.policies.keys().copied().collect(),
        ruleset_manifest_sha256: loaded.published.keys().copied().collect(),
        published_rulesets: loaded
            .published
            .iter()
            .map(|(digest, published)| {
                Ok(PublishedRulesetArtifactV3 {
                    ruleset_manifest_sha256: *digest,
                    artifact: ArtifactRefV1 {
                        sha256: Digest32::digest_bytes(&canonical_json_bytes(published)?),
                        byte_length: u64::try_from(canonical_json_bytes(published)?.len())?,
                        media_type: "application/json".into(),
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?,
        competition_manifest_sha256: loaded.competitions.keys().copied().collect(),
        public_static_files,
        identity_signer_files,
    };
    document.validate()?;
    Ok(document)
}

pub(super) fn copy_directory_exact(source: &Path, destination: &Path) -> Result<()> {
    validate_mount_root(&fs::canonicalize(source)?)?;
    ensure!(!destination.exists(), "copy destination already exists");
    fs::create_dir_all(destination)?;
    for (relative, absolute) in walk_regular_files(source)? {
        copy_file_exact(&absolute, &destination.join(relative))?;
    }
    let source_files = file_artifacts(source)?;
    let destination_files = file_artifacts(destination)?;
    ensure!(
        source_files == destination_files,
        "copied directory changed"
    );
    Ok(())
}

pub(super) fn copy_directory_exact_preserving_modes(
    source: &Path,
    destination: &Path,
) -> Result<PublicationTreeAuthorityV3> {
    copy_directory_exact_preserving_modes_with(source, destination, || {})
}

pub(super) fn copy_directory_exact_preserving_modes_with<F>(
    source: &Path,
    destination: &Path,
    before_acceptance: F,
) -> Result<PublicationTreeAuthorityV3>
where
    F: FnOnce(),
{
    validate_mount_root(&fs::canonicalize(source)?)?;
    ensure!(!destination.exists(), "copy destination already exists");
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("mode-preserving authority copy requires Linux openat2");
    #[cfg(target_os = "linux")]
    let source_root = open_publication_root_v3(source)?;
    #[cfg(target_os = "linux")]
    let mut inventory = publication_tree_inventory_v3_from_fd(source, &source_root)?;
    let source_snapshot = inventory.snapshot();
    let expected_authority = inventory.authority();
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{Mode, OFlags, ResolveFlags, fchmod, mkdirat, openat2};
        use std::os::fd::AsFd as _;
        use std::os::unix::fs::PermissionsExt as _;

        let destination_parent_path = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let destination_name = destination
            .file_name()
            .context("PublicationV3 copy destination has no basename")?;
        let destination_parent = open_publication_root_v3(destination_parent_path)?;
        mkdirat(
            destination_parent.as_fd(),
            destination_name,
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )?;
        let destination_root =
            open_publication_child_v3(&destination_parent, Path::new(destination_name))?;
        let destination_root_identity = publication_node_identity_v3(&destination_root.metadata()?);
        ensure!(
            destination_root.metadata()?.is_dir()
                && destination_root_identity.owner == rustix::process::geteuid().as_raw()
                && destination_root_identity.device
                    == publication_node_identity_v3(&destination_parent.metadata()?).device,
            "PublicationV3 copy destination root is not an owned same-device directory"
        );
        let rebound_destination =
            open_publication_child_v3(&destination_parent, Path::new(destination_name))?;
        ensure!(
            publication_node_identity_v3(&rebound_destination.metadata()?)
                == destination_root_identity,
            "PublicationV3 copy destination root was substituted after mkdirat"
        );
        let mut destination_directories = BTreeMap::<String, fs::File>::new();
        destination_directories.insert(".".into(), destination_root.try_clone()?);
        let mut destination_files = BTreeMap::<String, fs::File>::new();
        for directory in inventory
            .directories
            .iter()
            .filter(|directory| directory.path != ".")
        {
            let relative = Path::new(&directory.path);
            let parent = relative
                .parent()
                .filter(|path| !path.as_os_str().is_empty());
            let parent_key = parent
                .map(path_to_manifest)
                .transpose()?
                .unwrap_or_else(|| ".".into());
            let name = relative
                .file_name()
                .context("PublicationV3 directory has no basename")?;
            let parent_descriptor = destination_directories
                .get(&parent_key)
                .with_context(|| format!("PublicationV3 copy omits parent {parent_key}"))?;
            mkdirat(
                parent_descriptor.as_fd(),
                name,
                Mode::RUSR | Mode::WUSR | Mode::XUSR,
            )?;
            let child = open_publication_child_v3(parent_descriptor, Path::new(name))?;
            destination_directories.insert(directory.path.clone(), child);
        }
        for source_file in &mut inventory.files {
            let output_descriptor = openat2(
                destination_root.as_fd(),
                Path::new(&source_file.path),
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::CREATE | OFlags::EXCL,
                Mode::RUSR | Mode::WUSR,
                ResolveFlags::BENEATH
                    | ResolveFlags::NO_SYMLINKS
                    | ResolveFlags::NO_MAGICLINKS
                    | ResolveFlags::NO_XDEV,
            )?;
            let mut output = fs::File::from(output_descriptor);
            source_file.file.seek(std::io::SeekFrom::Start(0))?;
            let copied = std::io::copy(&mut source_file.file, &mut output)?;
            ensure!(
                copied == source_file.artifact.byte_length,
                "PublicationV3 pinned copy length changed at {}",
                source_file.path
            );
            output.set_permissions(fs::Permissions::from_mode(source_file.unix_mode))?;
            output.sync_all()?;
            ensure!(
                publication_node_identity_v3(&source_file.file.metadata()?) == source_file.identity,
                "PublicationV3 source changed while copied at {}",
                source_file.path
            );
            let output_identity = publication_node_identity_v3(&output.metadata()?);
            ensure!(
                output_identity.links == 1,
                "PublicationV3 destination was hard-linked at {}",
                source_file.path
            );
            ensure!(
                stable_publication_file_artifact_v3(
                    &mut output,
                    &output_identity,
                    &source_file.path,
                )? == source_file.artifact,
                "PublicationV3 pinned copy bytes changed at {}",
                source_file.path
            );
            let rebound =
                open_publication_child_v3(&destination_root, Path::new(&source_file.path))?;
            ensure!(
                publication_node_identity_v3(&rebound.metadata()?) == output_identity,
                "PublicationV3 destination path was substituted after copy at {}",
                source_file.path
            );
            ensure!(
                destination_files
                    .insert(source_file.path.clone(), output)
                    .is_none(),
                "PublicationV3 destination file path repeats"
            );
        }
        let mut directories_by_depth = inventory.directories.clone();
        directories_by_depth.sort_by_key(|directory| {
            std::cmp::Reverse(Path::new(&directory.path).components().count())
        });
        for directory in directories_by_depth {
            let descriptor = destination_directories
                .get(&directory.path)
                .with_context(|| format!("PublicationV3 copy omits {}", directory.path))?;
            fchmod(descriptor.as_fd(), Mode::from_raw_mode(directory.unix_mode))?;
            descriptor.sync_all()?;
        }
        before_acceptance();
        ensure!(
            publication_tree_inventory_v3_from_fd(source, &source_root)?.snapshot()
                == source_snapshot,
            "PublicationV3 source changed before copy acceptance"
        );
        let destination_inventory =
            publication_tree_inventory_v3_from_fd(destination, &destination_root)?;
        ensure!(
            destination_inventory.authority() == expected_authority,
            "PublicationV3 mode-preserving copy changed its authority"
        );
        for file in &destination_inventory.files {
            let retained = destination_files
                .get(&file.path)
                .with_context(|| format!("PublicationV3 copy dropped output FD {}", file.path))?;
            ensure!(
                publication_node_identity_v3(&retained.metadata()?) == file.identity,
                "PublicationV3 destination file identity changed before acceptance at {}",
                file.path
            );
        }
        for (directory, (_, identity)) in destination_inventory
            .directories
            .iter()
            .zip(&destination_inventory.directory_identities)
        {
            let retained = destination_directories
                .get(&directory.path)
                .with_context(|| {
                    format!("PublicationV3 copy dropped directory FD {}", directory.path)
                })?;
            ensure!(
                publication_node_identity_v3(&retained.metadata()?) == *identity,
                "PublicationV3 destination directory identity changed before acceptance at {}",
                directory.path
            );
        }
        destination_parent.sync_all()?;
        let accepted = publication_tree_inventory_v3_from_fd(destination, &destination_root)?;
        ensure!(
            accepted.snapshot() == destination_inventory.snapshot(),
            "PublicationV3 destination changed after its final identity rebind"
        );
    }
    Ok(expected_authority)
}

pub(super) fn create_private_publication_root(root: &Path) -> Result<PathBuf> {
    let root_metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect publication staging root {}", root.display()))?;
    ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "publication staging root is not a real directory"
    );

    let private_root = root.join("private");
    match fs::symlink_metadata(&private_root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => anyhow::bail!("private publication root already exists"),
    }
    // Create exactly the missing leaf below the already validated staging
    // directory. Do not use create_dir_all here: the private authority must
    // never follow or manufacture an unchecked ancestry.
    fs::create_dir(&private_root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&private_root, fs::Permissions::from_mode(0o755))?;
    }
    let private_metadata = fs::symlink_metadata(&private_root)?;
    ensure!(
        private_metadata.is_dir() && !private_metadata.file_type().is_symlink(),
        "private publication root is not a real directory"
    );
    Ok(private_root)
}

pub(super) fn copy_file_exact(source: &Path, destination: &Path) -> Result<()> {
    let artifact = artifact_from_file(source, "application/octet-stream")?;
    copy_artifact_exact(source, destination, &artifact)
}

pub(super) fn make_private_executables_and_states_read_only(
    root: &Path,
    loaded: &LoadedPublication,
) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let verifier = root
            .join("private/verifier/bin")
            .join(loaded.authority.build.verifier.artifact.sha256.to_string());
        fs::set_permissions(&verifier, fs::Permissions::from_mode(0o555))?;
        ensure!(
            fs::metadata(&verifier)?.permissions().mode() & 0o111 != 0,
            "verifier program is not executable"
        );
        for directory in [
            root.join("private/campaign-states"),
            root.join("private/verifier/operator-config"),
        ] {
            for (_, file) in walk_regular_files(&directory)? {
                fs::set_permissions(file, fs::Permissions::from_mode(0o444))?;
            }
            fs::set_permissions(directory, fs::Permissions::from_mode(0o555))?;
        }
    }
    Ok(())
}

pub(super) fn make_lock_files_read_only(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for file in [
            root.join("publication-lock-v3.json"),
            root.join("publication-lock-v3.sha256"),
        ] {
            fs::set_permissions(file, fs::Permissions::from_mode(0o444))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("operator publications require Unix permission semantics")
}
