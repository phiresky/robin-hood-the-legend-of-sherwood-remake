//! closure responsibilities of the admitted release pipeline.
use super::*;

pub(super) fn validate_publication_closure(loaded: &LoadedPublication) -> Result<()> {
    validate_document_closure(
        loaded.build_sha256,
        &loaded.authority.content,
        &loaded.authority.campaigns,
        &loaded.rules_configs,
        &loaded.policies,
        &loaded.published,
        &loaded.competitions,
    )
}

pub(super) fn validate_document_closure(
    build_sha256: Digest32,
    content: &BTreeMap<Digest32, robin_run_protocol::ContentManifestV1>,
    campaigns: &BTreeMap<Digest32, robin_run_protocol::CampaignContentManifestV1>,
    rules_configs: &BTreeMap<Digest32, RulesConfigIdentityV1>,
    policies: &BTreeMap<Digest32, ImmutablePolicyManifestV1>,
    published_rulesets: &BTreeMap<Digest32, PublishedRulesetV1>,
    competitions: &BTreeMap<Digest32, CompetitionManifestV1>,
) -> Result<()> {
    ensure!(!policies.is_empty(), "publication has no policies");
    ensure!(
        !published_rulesets.is_empty(),
        "publication has no rulesets"
    );
    validate_authentic_content_catalogs(content, campaigns)?;
    for config in rules_configs.values() {
        validate_complete_ranked_rules_config(config)?;
    }
    let full_campaign_digest = campaigns
        .iter()
        .find_map(|(digest, catalog)| {
            (catalog.edition == OfficialContentEditionV1::Full).then_some(*digest)
        })
        .context("publication has no Full campaign catalog")?;
    for (digest, published) in published_rulesets {
        published.validate()?;
        ensure!(
            published.manifest.canonical_digest()? == *digest,
            "published ruleset immutable digest mismatch"
        );
        let ruleset = &published.manifest;
        ensure!(
            ruleset.input_provenance_eligibility
                == InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly
                && ruleset.replay_schema_versions == [CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1]
                && ruleset.network_protocol_versions
                    == [robin_engine::multiplayer::NET_PROTOCOL_VERSION]
                && ruleset.achievement_policies == official_achievement_policies_v1(),
            "official ruleset does not use the current canonical replay/network schema and exact Required achievements"
        );
        ensure!(
            ruleset.allowed_build_manifest_sha256 == [build_sha256],
            "ruleset does not bind the one exact active BuildManifestV2"
        );
        ensure!(
            rules_configs.contains_key(&ruleset.rules_config_sha256),
            "ruleset references an absent rules config"
        );
        ensure!(
            ruleset
                .allowed_content_manifest_sha256
                .iter()
                .all(|candidate| content.contains_key(candidate)),
            "ruleset references absent official content"
        );
        let editions = ruleset
            .allowed_content_manifest_sha256
            .iter()
            .map(|digest| content[digest].edition)
            .collect::<BTreeSet<_>>();
        ensure!(editions.len() == 1, "ruleset mixes official editions");
        let edition = *editions.iter().next().context("ruleset content is empty")?;
        ensure!(
            ruleset.canonical_campaign_state.edition == edition,
            "ruleset canonical campaign state edition differs from its content edition"
        );
        let complete = content
            .iter()
            .filter(|(_, manifest)| manifest.edition == edition)
            .map(|(digest, _)| *digest)
            .collect::<Vec<_>>();
        ensure!(
            ruleset.allowed_content_manifest_sha256 == complete,
            "ruleset does not bind the complete authentic edition matrix"
        );
        validate_official_campaign_board_policy(edition, ruleset, full_campaign_digest)?;
        for identity in [
            &ruleset.input_provenance_policy,
            &ruleset.command_admission_policy,
            &ruleset.submission_admission_policy,
            &ruleset.verifier_policy,
        ] {
            let policy = policies
                .get(&identity.manifest_sha256)
                .context("ruleset policy document is absent")?;
            ensure!(
                policy.kind == identity.kind && policy.version == identity.version,
                "ruleset policy identity differs from its document"
            );
        }
    }
    for competition in competitions.values() {
        competition.validate()?;
        let ruleset = published_rulesets
            .get(&competition.ruleset_manifest_sha256)
            .context("competition references an absent ruleset")?;
        ensure!(
            competition.rules_config_sha256 == ruleset.manifest.rules_config_sha256,
            "competition rules config differs from ruleset"
        );
        let allowed = match competition.content {
            RunContentIdentityV1::Mission {
                content_manifest_sha256,
            } => {
                content.contains_key(&content_manifest_sha256)
                    && ruleset
                        .manifest
                        .allowed_content_manifest_sha256
                        .binary_search(&content_manifest_sha256)
                        .is_ok()
            }
            RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256,
            } => ruleset
                .manifest
                .allowed_campaign_content_manifest_sha256
                .binary_search(&campaign_content_manifest_sha256)
                .is_ok(),
        };
        ensure!(allowed, "competition content is outside its ruleset");
    }
    Ok(())
}

pub(super) fn validate_official_campaign_board_policy(
    edition: OfficialContentEditionV1,
    ruleset: &RulesetManifestV1,
    full_campaign_digest: Digest32,
) -> Result<()> {
    validate_official_campaign_offer_fields(
        edition,
        &ruleset.board_scopes,
        &ruleset.allowed_campaign_content_manifest_sha256,
        &ruleset.campaign_completion_policy,
        full_campaign_digest,
    )
}

pub(super) fn validate_official_campaign_offer_fields(
    edition: OfficialContentEditionV1,
    board_scopes: &[RulesetBoardScopeV1],
    allowed_campaign_content_manifest_sha256: &[Digest32],
    campaign_completion_policy: &CampaignCompletionPolicyRequirementV1,
    full_campaign_digest: Digest32,
) -> Result<()> {
    let full_campaign = board_scopes
        .binary_search(&RulesetBoardScopeV1::FullCampaign)
        .is_ok();
    match (edition, full_campaign) {
        (OfficialContentEditionV1::Demo, false) => ensure!(
            allowed_campaign_content_manifest_sha256.is_empty()
                && campaign_completion_policy == &CampaignCompletionPolicyRequirementV1::NotOffered,
            "Demo ruleset must not bind or offer a campaign completion policy"
        ),
        (OfficialContentEditionV1::Demo, true) => {
            anyhow::bail!("Demo ruleset advertises FullCampaign")
        }
        (OfficialContentEditionV1::Full, true) => ensure!(
            allowed_campaign_content_manifest_sha256 == [full_campaign_digest]
                && campaign_completion_policy
                    == &CampaignCompletionPolicyRequirementV1::Required(
                        official_full_campaign_completion_policy_v1(),
                    ),
            "FullCampaign ruleset must bind the exact Full catalog and 100% H12_Not_MP completion"
        ),
        (OfficialContentEditionV1::Full, false) => ensure!(
            allowed_campaign_content_manifest_sha256.is_empty()
                && campaign_completion_policy == &CampaignCompletionPolicyRequirementV1::NotOffered,
            "non-campaign Full ruleset must not bind or offer a campaign completion policy"
        ),
    }
    Ok(())
}

pub(super) fn validate_authentic_content_catalogs(
    content: &BTreeMap<Digest32, robin_run_protocol::ContentManifestV1>,
    campaigns: &BTreeMap<Digest32, robin_run_protocol::CampaignContentManifestV1>,
) -> Result<()> {
    ensure!(
        campaigns.len() == 2,
        "publication requires Demo and Full catalogs"
    );
    for edition in [
        OfficialContentEditionV1::Demo,
        OfficialContentEditionV1::Full,
    ] {
        let mut edition_manifests = content
            .iter()
            .filter(|(_, manifest)| manifest.edition == edition)
            .map(|(digest, manifest)| (manifest.subject.clone(), *digest))
            .collect::<Vec<_>>();
        edition_manifests.sort_by(|left, right| left.0.cmp(&right.0));
        let expected_subjects = official_content_subjects_v1(edition);
        ensure!(
            edition_manifests
                .iter()
                .map(|(subject, _)| subject)
                .eq(expected_subjects.iter()),
            "publication content is not the authentic {edition:?} subject matrix"
        );
        let edition_catalogs = campaigns
            .values()
            .filter(|catalog| catalog.edition == edition)
            .collect::<Vec<_>>();
        ensure!(
            edition_catalogs.len() == 1,
            "publication does not contain one exact {edition:?} catalog"
        );
        let expected_entries = edition_manifests
            .into_iter()
            .map(
                |(subject, content_manifest_sha256)| robin_run_protocol::CampaignContentEntryV1 {
                    subject,
                    content_manifest_sha256,
                },
            )
            .collect::<Vec<_>>();
        ensure!(
            edition_catalogs[0].entries == expected_entries,
            "publication {edition:?} catalog is substituted"
        );
    }
    Ok(())
}

pub(super) fn expected_publication_topology_from_loaded_v3(
    loaded: &LoadedPublication,
    manifest: &PublicationManifestV3,
    official_authority: &PublicationTreeAuthorityV3,
) -> Result<ExpectedPublicationTopologyV3> {
    let mut expected = ExpectedPublicationTopologyV3::new();
    for directory in [
        "backend/manifests/builds",
        "backend/manifests/content-manifests",
        "backend/manifests/campaign-content-manifests",
        "backend/manifests/rules-configs",
        "backend/manifests/ruleset-manifests",
        "backend/manifests/published-rulesets",
        "backend/manifests/competitions",
        "backend/manifests/policies",
        "cloudflare-public",
        "cloudflare-identity-signer",
        "deployment",
    ] {
        expected.register_directory(directory)?;
    }

    for (digest, document) in &loaded.authority.content {
        expected.register_canonical(
            format!("backend/manifests/content-manifests/{digest}.json"),
            document,
        )?;
    }
    for (digest, document) in &loaded.authority.campaigns {
        expected.register_canonical(
            format!("backend/manifests/campaign-content-manifests/{digest}.json"),
            document,
        )?;
    }
    expected.register_canonical(
        format!("backend/manifests/builds/{}.json", loaded.build_sha256),
        &loaded.authority.build,
    )?;
    for (digest, document) in &loaded.rules_configs {
        expected.register_canonical(
            format!("backend/manifests/rules-configs/{digest}.json"),
            document,
        )?;
    }
    for (digest, document) in &loaded.policies {
        expected.register_canonical(
            format!("backend/manifests/policies/{digest}.json"),
            document,
        )?;
    }
    for (digest, published) in &loaded.published {
        expected.register_canonical(
            format!("backend/manifests/ruleset-manifests/{digest}.json"),
            &published.manifest,
        )?;
        expected.register_canonical(
            format!("backend/manifests/published-rulesets/{digest}.json"),
            published,
        )?;
    }
    for (digest, document) in &loaded.competitions {
        expected.register_canonical(
            format!("backend/manifests/competitions/{digest}.json"),
            document,
        )?;
    }
    let backend = backend_publication(loaded)?;
    expected.register_canonical("backend/publication-v3.json".into(), &backend)?;

    let typed_official = expected_official_authority_topology_v3(&loaded.authority)?;
    let copied_official_files = official_authority
        .files
        .iter()
        .map(|(path, artifact, _)| (path.clone(), artifact))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        copied_official_files
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            == typed_official.files,
        "copied Plan-V3 authority differs from its independently derived typed file closure"
    );
    let copied_official_directories = official_authority
        .directories
        .iter()
        .map(|directory| {
            if directory.path == "." {
                String::new()
            } else {
                directory.path.clone()
            }
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        copied_official_directories == typed_official.directories,
        "copied Plan-V3 authority differs from its independently derived typed directory closure"
    );
    for path in &typed_official.files {
        let artifact = copied_official_files
            .get(path)
            .with_context(|| format!("copied Plan-V3 authority omits typed file {path}"))?;
        let executable = path.starts_with("private/build-artifacts/projection-exporters/");
        expected.register_file(
            format!("private/official-content-authority/{path}"),
            artifact,
            executable,
        )?;
    }
    for directory in &typed_official.directories {
        let path = if directory.is_empty() {
            "private/official-content-authority".to_owned()
        } else {
            format!("private/official-content-authority/{directory}")
        };
        expected.register_directory(&path)?;
    }
    expected.register_file(
        format!(
            "private/viewer-build-reports-v2/{}.json",
            loaded.viewer_build_report_artifact.sha256
        ),
        &loaded.viewer_build_report_artifact,
        false,
    )?;
    expected.register_file(
        format!(
            "private/verifier/bin/{}",
            loaded.authority.build.verifier.artifact.sha256
        ),
        &loaded.authority.build.verifier.artifact,
        true,
    )?;
    expected.register_file(
        format!(
            "private/verifier/operator-config/{}",
            loaded.plan.verifier_operator_config.artifact.sha256
        ),
        &loaded.plan.verifier_operator_config.artifact,
        false,
    )?;
    let mut campaigns = BTreeMap::new();
    for state in &loaded.plan.campaign_states {
        match campaigns.insert(state.artifact.sha256, state.artifact.clone()) {
            Some(previous) => ensure!(
                previous == state.artifact,
                "conflicting expected PublicationV3 campaign artifact"
            ),
            None => expected.register_file(
                format!("private/campaign-states/{}", state.artifact.sha256),
                &state.artifact,
                false,
            )?,
        }
    }

    expected.register_canonical(
        format!(
            "cloudflare-public/manifests/builds/{}.json",
            loaded.build_sha256
        ),
        &loaded.authority.build,
    )?;
    for named in &loaded.authority.build.viewer.engine.artifacts {
        expected.register_file(
            format!(
                "cloudflare-public/{}",
                build_artifact_object_path_v1(loaded.build_sha256, named)?
            ),
            &named.artifact,
            false,
        )?;
    }
    for file in &manifest.public_static_files {
        expected.register_file(
            format!("cloudflare-public/{}", file.published_path),
            &file.artifact,
            false,
        )?;
    }
    for file in &manifest.identity_signer_files {
        expected.register_file(
            format!("cloudflare-identity-signer/{}", file.published_path),
            &file.artifact,
            false,
        )?;
    }
    expected.register_file(
        DATADIR_AUTHORITY_PATH.into(),
        &loaded.plan.datadir_release_authority.artifact,
        false,
    )?;
    expected.register_file(
        DATADIR_DEPLOYMENT_RECEIPT_PATH.into(),
        &loaded.plan.datadir_deployment_receipt.artifact,
        false,
    )?;
    expected.register_canonical(
        "deployment/exposure-v3.json".into(),
        &DeploymentExposureV3::official(),
    )?;
    expected.register_canonical("publication-manifest-v3.json".into(), manifest)?;
    expected.register_bytes(
        "publication-manifest-v3.sha256".into(),
        manifest.canonical_digest()?.to_string().as_bytes(),
    )?;
    Ok(expected)
}

pub(super) fn publication_lock(
    topology: &ExpectedPublicationTopologyV3,
    manifest_sha256: Digest32,
) -> Result<PublicationLockV3> {
    topology.lock(manifest_sha256)
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn publication_lock_from_actual_for_test(
    root: &Path,
    manifest_sha256: Digest32,
) -> Result<PublicationLockV3> {
    let inventory = publication_tree_inventory_v3(root)?;
    let files = inventory
        .files
        .iter()
        .map(|file| ReleaseFileV1 {
            exposure: release_file_exposure(&file.path),
            path: file.path.clone(),
            artifact: file.artifact.clone(),
        })
        .collect();
    let file_modes = inventory
        .files
        .iter()
        .map(|file| PublicationFileModeV3 {
            path: file.path.clone(),
            unix_mode: file.unix_mode,
        })
        .collect();
    let lock = PublicationLockV3 {
        schema_version: PUBLICATION_LOCK_SCHEMA_VERSION,
        publication_manifest_sha256: manifest_sha256,
        files,
        directories: inventory.directories,
        file_modes,
    };
    lock.validate()?;
    Ok(lock)
}

pub(super) fn validate_publication_inventory_against_lock_v3(
    inventory: &PublicationTreeInventoryV3,
    lock: &PublicationLockV3,
) -> Result<()> {
    let mut actual = Vec::new();
    let mut actual_modes = Vec::new();
    for file in &inventory.files {
        let path = file.path.clone();
        if matches!(
            path.as_str(),
            "publication-lock-v3.json" | "publication-lock-v3.sha256"
        ) {
            continue;
        }
        actual.push(ReleaseFileV1 {
            exposure: release_file_exposure(&path),
            path: path.clone(),
            artifact: file.artifact.clone(),
        });
        actual_modes.push(PublicationFileModeV3 {
            path,
            unix_mode: file.unix_mode,
        });
    }
    actual.sort_by(|left, right| left.path.cmp(&right.path));
    actual_modes.sort_by(|left, right| left.path.cmp(&right.path));
    ensure!(
        actual == lock.files,
        "publication file inventory differs from lock"
    );
    ensure!(
        actual_modes == lock.file_modes,
        "publication file mode inventory differs from lock"
    );
    ensure!(
        inventory.directories == lock.directories,
        "publication directory inventory differs from lock"
    );
    Ok(())
}

/// Validate an already-authored publication strictly from its canonical lock.
pub fn validate_publication_v3(root: &Path) -> Result<Digest32> {
    validate_mount_root(root)?;
    #[cfg(target_os = "linux")]
    {
        Ok(validate_publication_v3_authority(root)?.lock_sha256)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("PublicationV3 validation requires Linux openat2 filesystem authority")
}

/// Validate a PublicationV3 tree through an already-pinned directory handle.
///
/// The caller owns and pins `descriptor`; `diagnostic_root` is used only for
/// mount/root-rebind diagnostics. The complete descendant closure remains
/// rooted in that descriptor.
#[cfg(target_os = "linux")]
pub(crate) fn validate_pinned_publication_v3(
    root_rebind_path: &Path,
    descriptor: &fs::File,
) -> Result<ValidatedPublicationV3> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = descriptor.metadata()?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.dev() != 0,
        "pinned PublicationV3 root is not an EUID-owned real directory"
    );
    let root = descriptor.try_clone()?;
    let root_identity = publication_node_identity_v3(&metadata);
    let (root_parent_path, root_parent, root_name) =
        pin_publication_root_parent_v3(root_rebind_path)?;
    let root_parent_identity = publication_node_identity_v3(&root_parent.metadata()?);
    let rebound = open_publication_child_v3(&root_parent, Path::new(&root_name))?;
    ensure!(
        publication_node_identity_v3(&rebound.metadata()?) == root_identity,
        "pinned PublicationV3 root differs from its parent-relative path"
    );
    let (lock_sha256, inventory) = validate_publication_v3_contents(root_rebind_path, &root)?;
    Ok(ValidatedPublicationV3 {
        root_path: root_rebind_path.to_path_buf(),
        root,
        root_parent_path,
        root_parent,
        root_parent_identity,
        root_name,
        inventory,
        lock_sha256,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn validate_publication_v3_authority(
    root_path: &Path,
) -> Result<ValidatedPublicationV3> {
    let root = open_publication_root_v3(root_path)?;
    validate_pinned_publication_v3(root_path, &root)
}

#[cfg(target_os = "linux")]
pub(super) fn validate_publication_v3_contents(
    root: &Path,
    root_descriptor: &fs::File,
) -> Result<(Digest32, PublicationTreeInventoryV3)> {
    let mut inventory = publication_tree_inventory_v3_from_fd(root, root_descriptor)?;
    let initial_snapshot = inventory.snapshot();
    let manifest: PublicationManifestV3 =
        load_inventory_document_v3(&mut inventory, "publication-manifest-v3.json")?;
    let manifest_sha256 = manifest.canonical_digest()?;
    ensure!(
        read_inventory_file_v3(
            &mut inventory,
            "publication-manifest-v3.sha256",
            MAX_DOCUMENT_BYTES,
        )? == manifest_sha256.to_string().as_bytes(),
        "publication manifest sidecar mismatch"
    );
    let lock: PublicationLockV3 =
        load_inventory_document_v3(&mut inventory, "publication-lock-v3.json")?;
    ensure!(
        lock.publication_manifest_sha256 == manifest_sha256,
        "publication lock does not bind its manifest"
    );
    let lock_sha256 = lock.canonical_digest()?;
    ensure!(
        read_inventory_file_v3(
            &mut inventory,
            "publication-lock-v3.sha256",
            MAX_DOCUMENT_BYTES,
        )? == lock_sha256.to_string().as_bytes(),
        "publication lock sidecar mismatch"
    );
    ensure!(
        inventory
            .files
            .iter()
            .find(|file| file.path == "publication-lock-v3.json")
            .is_some_and(|file| file.unix_mode == 0o444)
            && inventory
                .files
                .iter()
                .find(|file| file.path == "publication-lock-v3.sha256")
                .is_some_and(|file| file.unix_mode == 0o444),
        "publication lock files must be read-only"
    );
    validate_publication_inventory_against_lock_v3(&inventory, &lock)?;
    ensure_required_backend_layout(&inventory)?;
    validate_deployment_exposure(&mut inventory)?;
    ensure_no_full_public_leak(&mut inventory)?;
    validate_materialized_document_closure(&mut inventory, &manifest)?;
    ensure!(
        publication_tree_inventory_v3_from_fd(root, root_descriptor)?.snapshot()
            == initial_snapshot,
        "PublicationV3 tree changed while its document closure was validated"
    );
    Ok((lock_sha256, inventory))
}

pub(super) fn validate_deployment_exposure(
    inventory: &mut PublicationTreeInventoryV3,
) -> Result<()> {
    let exposure: DeploymentExposureV3 =
        load_inventory_document_v3(inventory, "deployment/exposure-v3.json")?;
    exposure.validate_exact()?;
    for relative in [
        &exposure.public_static_root,
        &exposure.identity_signer_static_root,
        &exposure.backend_api_manifest_root,
    ] {
        ensure!(
            inventory_has_directory_v3(inventory, relative),
            "deployment root is not an exact non-symlink directory: {relative}"
        );
    }
    ensure!(
        exposure.cloudflare_routes.first().is_some_and(|route| {
            route.pattern == "robinhood.phiresky.xyz/api*" && route.script.is_none()
        }),
        "Cloudflare topology does not keep the complete /api prefix on the VPS origin"
    );
    ensure!(
        exposure.cloudflare_routes.get(1).is_some_and(|route| {
            route.pattern == "robinhood.phiresky.xyz/.well-known/acme-challenge/*"
                && route.script.is_none()
        }),
        "Cloudflare topology does not keep the narrow HTTP-01 challenge path on nginx"
    );
    Ok(())
}

pub(super) fn release_file_exposure(path: &str) -> ReleaseFileExposureV1 {
    if path.starts_with("cloudflare-public/") || path.starts_with("cloudflare-identity-signer/") {
        ReleaseFileExposureV1::PublicStatic
    } else if path.starts_with("backend/manifests/") {
        ReleaseFileExposureV1::BackendManifest
    } else {
        ReleaseFileExposureV1::OperatorPrivate
    }
}

pub(super) fn validate_pinned_projection_exporter_v3(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
    expected: &ArtifactRefV1,
) -> Result<()> {
    ensure!(
        expected.media_type == robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        "projection exporter has the wrong media type"
    );
    let file = inventory
        .files
        .iter()
        .find(|file| file.path == path)
        .context("PublicationV3 omits its projection exporter")?;
    ensure!(
        file.unix_mode & 0o111 != 0
            && inventory_artifact_v3(inventory, path, &expected.media_type)? == *expected,
        "projection exporter mode or artifact identity is substituted"
    );
    let bytes = read_inventory_file_v3(inventory, path, expected.byte_length)?;
    let elf = Elf::parse(&bytes).context("projection exporter is not a valid ELF executable")?;
    ensure!(
        elf.is_64
            && elf.little_endian
            && elf.header.e_machine == header::EM_X86_64
            && matches!(elf.header.e_type, header::ET_EXEC | header::ET_DYN)
            && elf.entry != 0
            && elf.interpreter.is_none()
            && elf
                .program_headers
                .iter()
                .all(|header| header.p_type != program_header::PT_INTERP)
            && elf.libraries.is_empty()
            && elf.program_headers.iter().any(|program| {
                program.p_type == program_header::PT_LOAD
                    && program.p_flags & program_header::PF_X != 0
                    && program.p_filesz != 0
            }),
        "projection exporter is not an exact static x86-64 ELF"
    );
    Ok(())
}

pub(super) fn authority_relative_v3(relative: &str) -> String {
    format!("private/official-content-authority/{relative}")
}

pub(super) fn load_authority_document_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    relative: &str,
    expected_files: &mut BTreeSet<String>,
) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    ensure!(
        expected_files.insert(relative.to_owned()),
        "embedded Plan-V3 authority repeats expected path {relative}"
    );
    load_inventory_document_v3(inventory, &authority_relative_v3(relative))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpectedOfficialAuthorityTopologyV3 {
    pub(super) files: BTreeSet<String>,
    pub(super) directories: BTreeSet<String>,
}

pub(super) fn expected_official_authority_topology_v3(
    authority: &ValidatedOfficialContentV3,
) -> Result<ExpectedOfficialAuthorityTopologyV3> {
    fn register(files: &mut BTreeSet<String>, path: String) -> Result<()> {
        ensure!(
            valid_publication_relative_path_v3(&path),
            "invalid typed Plan-V3 authority path {path}"
        );
        ensure!(
            files.insert(path.clone()),
            "typed Plan-V3 authority repeats expected path {path}"
        );
        Ok(())
    }

    let mut files = BTreeSet::new();
    for path in [
        "official-content-digests.json",
        "official-content-digests.sha256",
        "projection-authority-matrix-v3.json",
        "projection-authority-matrix-v3.sha256",
    ] {
        register(&mut files, path.to_owned())?;
    }
    register(
        &mut files,
        format!(
            "manifests/builds-v2/{}.json",
            authority.matrix.build_manifest_sha256
        ),
    )?;
    for digest in [
        authority
            .build
            .viewer
            .engine
            .wasm_bindgen_cli
            .authority_sha256,
        authority
            .build
            .viewer
            .engine
            .binaryen_wasm_opt
            .authority_sha256,
        authority
            .build
            .viewer
            .engine
            .wabt_wasm_strip
            .authority_sha256,
    ] {
        register(
            &mut files,
            format!("manifests/build-tool-authorities/{digest}.json"),
        )?;
    }
    register(
        &mut files,
        format!(
            "private/projection-authority-manifests-v2/{}.json",
            authority.matrix.projection_authority_manifest_sha256
        ),
    )?;
    register(
        &mut files,
        format!(
            "manifests/rules-configs/{}.json",
            authority.matrix.rules_config_sha256
        ),
    )?;
    register(
        &mut files,
        format!(
            "private/projection-execution-policies/{}.json",
            authority.matrix.execution_policy_sha256
        ),
    )?;
    register(
        &mut files,
        format!(
            "private/core-overlay-source-manifests-v2/{}.json",
            authority.matrix.core_overlay_manifest_sha256
        ),
    )?;
    register(
        &mut files,
        format!(
            "private/build-artifacts/projection-exporters/{}",
            authority
                .projection_authority
                .projection_exporter
                .artifact
                .sha256
        ),
    )?;

    for lane in &authority.matrix.lanes {
        register(
            &mut files,
            format!(
                "private/source-tree-manifests-v2/{}.json",
                lane.source_tree_manifest_sha256
            ),
        )?;
        register(
            &mut files,
            format!(
                "private/projection-receipts-v2/{}.json",
                lane.projection_receipt_sha256
            ),
        )?;
        let execution_root = format!(
            "private/projection-executions/{}",
            lane.projection_receipt_sha256
        );
        for name in ["record.json", "stdout.json", "stderr.log"] {
            register(&mut files, format!("{execution_root}/{name}"))?;
        }
    }

    for (digest, manifest) in &authority.content {
        register(
            &mut files,
            format!("manifests/content-manifests/{digest}.json"),
        )?;
        register(
            &mut files,
            format!("private/verifier-source-bindings-v2/{digest}.json"),
        )?;
        register(
            &mut files,
            format!("verifier-bundles/{digest}/manifest.json"),
        )?;
        for component in &manifest.components {
            let component_relative =
                simulation_content_component_relative_path_v1(&manifest.subject, component.kind)?;
            register(
                &mut files,
                format!("verifier-bundles/{digest}/catalog/{component_relative}"),
            )?;
        }
    }
    for digest in authority.campaigns.keys() {
        register(
            &mut files,
            format!("manifests/campaign-content-manifests/{digest}.json"),
        )?;
    }

    register(
        &mut files,
        format!(
            "public/manifests/campaign-content-manifests/{}.json",
            authority.digests.demo_campaign_content_manifest_sha256
        ),
    )?;
    for digest in &authority.digests.demo_content_manifest_sha256 {
        let manifest = authority
            .content
            .get(digest)
            .context("typed Demo content digest has no admitted manifest")?;
        register(
            &mut files,
            format!("public/manifests/content-manifests/{digest}.json"),
        )?;
        for component in &manifest.components {
            register(
                &mut files,
                format!(
                    "public/{}",
                    demo_content_object_path_v1(*digest, component)?
                ),
            )?;
        }
    }

    let mut directories = BTreeSet::from([String::new()]);
    for file in &files {
        let mut parent = Path::new(file).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
    }
    Ok(ExpectedOfficialAuthorityTopologyV3 { files, directories })
}

pub(super) fn validate_embedded_official_content_v3(
    inventory: &mut PublicationTreeInventoryV3,
) -> Result<ValidatedOfficialContentV3> {
    let mut expected_files = BTreeSet::new();
    let digests: crate::OfficialContentDigestsV1 = load_authority_document_v3(
        inventory,
        "official-content-digests.json",
        &mut expected_files,
    )?;
    expected_files.insert("official-content-digests.sha256".into());
    ensure!(
        read_inventory_file_v3(
            inventory,
            &authority_relative_v3("official-content-digests.sha256"),
            64,
        )? == digests.canonical_digest()?.to_string().as_bytes(),
        "embedded official-content digest sidecar differs"
    );
    let matrix: OfficialProjectionAuthorityMatrixV3 = load_authority_document_v3(
        inventory,
        "projection-authority-matrix-v3.json",
        &mut expected_files,
    )?;
    expected_files.insert("projection-authority-matrix-v3.sha256".into());
    ensure!(
        read_inventory_file_v3(
            inventory,
            &authority_relative_v3("projection-authority-matrix-v3.sha256"),
            64,
        )? == matrix.canonical_digest()?.to_string().as_bytes(),
        "embedded projection matrix sidecar differs"
    );

    let build_relative = format!("manifests/builds-v2/{}.json", matrix.build_manifest_sha256);
    let build: BuildManifestV2 =
        load_authority_document_v3(inventory, &build_relative, &mut expected_files)?;
    validate_current_official_ranked_build_v2(&build)?;
    ensure!(
        build.canonical_digest()? == matrix.build_manifest_sha256,
        "embedded BuildManifestV2 path digest mismatch"
    );
    let tool_digests = [
        build.viewer.engine.wasm_bindgen_cli.authority_sha256,
        build.viewer.engine.binaryen_wasm_opt.authority_sha256,
        build.viewer.engine.wabt_wasm_strip.authority_sha256,
    ];
    let mut tools = Vec::new();
    for digest in tool_digests {
        let relative = format!("manifests/build-tool-authorities/{digest}.json");
        let tool: BuildToolAuthorityDocumentV1 =
            load_authority_document_v3(inventory, &relative, &mut expected_files)?;
        ensure!(
            tool.canonical_digest()? == digest,
            "embedded build-tool authority path digest mismatch"
        );
        tools.push(tool);
    }
    build.validate_wasm_tool_authorities(&tools[0], &tools[1], &tools[2])?;

    let projection_relative = format!(
        "private/projection-authority-manifests-v2/{}.json",
        matrix.projection_authority_manifest_sha256
    );
    let projection_authority: OfficialProjectionAuthorityManifestV2 =
        load_authority_document_v3(inventory, &projection_relative, &mut expected_files)?;
    ensure!(
        projection_authority.canonical_digest()? == matrix.projection_authority_manifest_sha256,
        "embedded projection authority path digest mismatch"
    );
    projection_authority.validate_against(&build)?;
    let rules_relative = format!(
        "manifests/rules-configs/{}.json",
        matrix.rules_config_sha256
    );
    let rules: RulesConfigIdentityV1 =
        load_authority_document_v3(inventory, &rules_relative, &mut expected_files)?;
    ensure!(
        rules.canonical_digest()? == matrix.rules_config_sha256,
        "embedded projection rules path digest mismatch"
    );
    validate_official_projection_rules_config_v1(&rules)?;
    let execution_relative = format!(
        "private/projection-execution-policies/{}.json",
        matrix.execution_policy_sha256
    );
    let execution_policy: OfficialProjectionExecutionPolicyV1 =
        load_authority_document_v3(inventory, &execution_relative, &mut expected_files)?;
    ensure!(
        execution_policy.canonical_digest()? == matrix.execution_policy_sha256
            && execution_policy.rules_config == rules,
        "embedded projection execution policy is substituted"
    );
    let core_relative = format!(
        "private/core-overlay-source-manifests-v2/{}.json",
        matrix.core_overlay_manifest_sha256
    );
    let core_manifest: OfficialBuiltInOverlaySourceManifestV2 =
        load_authority_document_v3(inventory, &core_relative, &mut expected_files)?;
    ensure!(
        core_manifest.canonical_digest()? == matrix.core_overlay_manifest_sha256,
        "embedded core overlay manifest is substituted"
    );
    let exporter_relative = format!(
        "private/build-artifacts/projection-exporters/{}",
        projection_authority.projection_exporter.artifact.sha256
    );
    expected_files.insert(exporter_relative.clone());
    validate_pinned_projection_exporter_v3(
        inventory,
        &authority_relative_v3(&exporter_relative),
        &projection_authority.projection_exporter.artifact,
    )?;

    let mut receipts = Vec::with_capacity(4);
    for lane in &matrix.lanes {
        let source_relative = format!(
            "private/source-tree-manifests-v2/{}.json",
            lane.source_tree_manifest_sha256
        );
        let source: OfficialSourceTreeManifestV2 =
            load_authority_document_v3(inventory, &source_relative, &mut expected_files)?;
        ensure!(
            source.canonical_digest()? == lane.source_tree_manifest_sha256
                && source.edition == lane.edition
                && source.source_format == lane.source_format,
            "embedded matrix source authority is substituted"
        );
        let receipt_relative = format!(
            "private/projection-receipts-v2/{}.json",
            lane.projection_receipt_sha256
        );
        let receipt: OfficialSimulationProjectionReceiptV2 =
            load_authority_document_v3(inventory, &receipt_relative, &mut expected_files)?;
        ensure!(
            receipt.canonical_digest()? == lane.projection_receipt_sha256
                && receipt.edition == lane.edition
                && receipt.exporter.source_format == lane.source_format,
            "embedded projection receipt is substituted"
        );
        receipt.validate_against(
            &build,
            &projection_authority,
            &rules,
            &source,
            &core_manifest,
        )?;
        let execution_root = format!(
            "private/projection-executions/{}",
            lane.projection_receipt_sha256
        );
        let record_relative = format!("{execution_root}/record.json");
        let record: OfficialProjectionExecutionRecordV3 =
            load_authority_document_v3(inventory, &record_relative, &mut expected_files)?;
        ensure!(
            record.canonical_digest()? == lane.execution_record_sha256,
            "embedded projection execution record is substituted"
        );
        record.report.validate_against(
            &receipt,
            &build,
            &projection_authority,
            &rules,
            &source,
            &core_manifest,
        )?;
        let stdout_relative = format!("{execution_root}/stdout.json");
        expected_files.insert(stdout_relative.clone());
        ensure!(
            inventory_artifact_v3(
                inventory,
                &authority_relative_v3(&stdout_relative),
                "application/json",
            )? == record.stdout,
            "embedded projection stdout differs from its record"
        );
        let stderr_relative = format!("{execution_root}/stderr.log");
        expected_files.insert(stderr_relative.clone());
        let stderr = inventory_artifact_v3(
            inventory,
            &authority_relative_v3(&stderr_relative),
            "text/plain",
        )?;
        ensure!(
            stderr.sha256 == record.stderr.sha256
                && stderr.byte_length == record.stderr.byte_length,
            "embedded projection stderr differs from its record"
        );
        receipts.push(receipt);
    }
    validate_official_projection_receipt_matrix_v2(&receipts)?;

    let receipt_content = receipts
        .iter()
        .filter(|receipt| {
            receipt.exporter.source_format == OfficialProjectionSourceFormatV1::LooseNativeV1
        })
        .flat_map(|receipt| {
            receipt
                .subjects
                .iter()
                .map(|subject| subject.content_manifest.clone())
        })
        .map(|document| Ok((document.canonical_digest()?, document)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let expected_content_digests = digests
        .demo_content_manifest_sha256
        .iter()
        .chain(&digests.full_content_manifest_sha256)
        .copied()
        .collect::<BTreeSet<_>>();
    ensure!(
        receipt_content.keys().copied().collect::<BTreeSet<_>>() == expected_content_digests,
        "embedded official content index differs from its receipt matrix"
    );
    let mut content = BTreeMap::new();
    for digest in expected_content_digests {
        let relative = format!("manifests/content-manifests/{digest}.json");
        let document: ContentManifestV1 =
            load_authority_document_v3(inventory, &relative, &mut expected_files)?;
        ensure!(
            document.canonical_digest()? == digest
                && receipt_content.get(&digest) == Some(&document),
            "embedded content manifest differs from its receipt"
        );
        content.insert(digest, document);
    }

    let mut campaigns = BTreeMap::new();
    for (edition, digest) in [
        (
            OfficialContentEditionV1::Demo,
            digests.demo_campaign_content_manifest_sha256,
        ),
        (
            OfficialContentEditionV1::Full,
            digests.full_campaign_content_manifest_sha256,
        ),
    ] {
        let relative = format!("manifests/campaign-content-manifests/{digest}.json");
        let campaign: CampaignContentManifestV1 =
            load_authority_document_v3(inventory, &relative, &mut expected_files)?;
        let expected_entries = official_content_subjects_v1(edition)
            .into_iter()
            .map(|subject| {
                let (content_digest, _) = content
                    .iter()
                    .find(|(_, manifest)| {
                        manifest.edition == edition && manifest.subject == subject
                    })
                    .context("embedded campaign subject has no content manifest")?;
                Ok(robin_run_protocol::CampaignContentEntryV1 {
                    subject,
                    content_manifest_sha256: *content_digest,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            campaign.canonical_digest()? == digest
                && campaign.edition == edition
                && campaign.entries == expected_entries,
            "embedded campaign authority is substituted"
        );
        campaigns.insert(digest, campaign);
    }

    for (digest, manifest) in &content {
        let edition_lanes = matrix
            .lanes
            .iter()
            .filter(|lane| lane.edition == manifest.edition)
            .collect::<Vec<_>>();
        ensure!(
            edition_lanes.len() == 2,
            "embedded edition matrix is incomplete"
        );
        let (loose, shipping) =
            if edition_lanes[0].source_format == OfficialProjectionSourceFormatV1::LooseNativeV1 {
                (edition_lanes[0], edition_lanes[1])
            } else {
                (edition_lanes[1], edition_lanes[0])
            };
        let binding_relative = format!("private/verifier-source-bindings-v2/{digest}.json");
        let binding: VerifierSourceBindingV2 =
            load_authority_document_v3(inventory, &binding_relative, &mut expected_files)?;
        ensure!(
            binding.content_manifest_sha256 == *digest
                && binding.edition == manifest.edition
                && binding.subject == manifest.subject
                && binding.build_manifest_sha256 == matrix.build_manifest_sha256
                && binding.projection_authority_manifest_sha256
                    == matrix.projection_authority_manifest_sha256
                && binding.rules_config_sha256 == matrix.rules_config_sha256
                && binding.execution_policy_sha256 == matrix.execution_policy_sha256
                && binding.core_overlay_manifest_sha256 == matrix.core_overlay_manifest_sha256
                && binding.loose_projection_receipt_sha256 == loose.projection_receipt_sha256
                && binding.loose_source_tree_manifest_sha256 == loose.source_tree_manifest_sha256
                && binding.shipping_projection_receipt_sha256 == shipping.projection_receipt_sha256
                && binding.shipping_source_tree_manifest_sha256
                    == shipping.source_tree_manifest_sha256,
            "embedded verifier source binding is substituted"
        );
        let bundle_manifest = format!("verifier-bundles/{digest}/manifest.json");
        let bundled: ContentManifestV1 =
            load_authority_document_v3(inventory, &bundle_manifest, &mut expected_files)?;
        ensure!(
            &bundled == manifest,
            "embedded verifier bundle manifest is substituted"
        );
        for component in &manifest.components {
            let component_relative =
                simulation_content_component_relative_path_v1(&manifest.subject, component.kind)?;
            let bundle_relative = format!("verifier-bundles/{digest}/catalog/{component_relative}");
            expected_files.insert(bundle_relative.clone());
            ensure!(
                inventory_artifact_v3(
                    inventory,
                    &authority_relative_v3(&bundle_relative),
                    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
                )? == component.artifact,
                "embedded verifier component differs from its manifest"
            );
        }
    }

    let demo_campaign = &campaigns[&digests.demo_campaign_content_manifest_sha256];
    let public_campaign = format!(
        "public/manifests/campaign-content-manifests/{}.json",
        digests.demo_campaign_content_manifest_sha256
    );
    let published_demo: CampaignContentManifestV1 =
        load_authority_document_v3(inventory, &public_campaign, &mut expected_files)?;
    ensure!(
        &published_demo == demo_campaign,
        "embedded public Demo campaign is substituted"
    );
    for digest in &digests.demo_content_manifest_sha256 {
        let manifest = &content[digest];
        let public_manifest = format!("public/manifests/content-manifests/{digest}.json");
        let published: ContentManifestV1 =
            load_authority_document_v3(inventory, &public_manifest, &mut expected_files)?;
        ensure!(
            &published == manifest,
            "embedded public Demo manifest is substituted"
        );
        for component in &manifest.components {
            let relative = format!(
                "public/{}",
                demo_content_object_path_v1(*digest, component)?
            );
            expected_files.insert(relative.clone());
            ensure!(
                inventory_artifact_v3(
                    inventory,
                    &authority_relative_v3(&relative),
                    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
                )? == component.artifact,
                "embedded public Demo component differs from its manifest"
            );
        }
    }

    let validated = ValidatedOfficialContentV3 {
        digests,
        matrix,
        build,
        projection_authority,
        rules,
        execution_policy,
        core_manifest,
        content,
        campaigns,
    };
    let typed_topology = expected_official_authority_topology_v3(&validated)?;
    ensure!(
        expected_files == typed_topology.files,
        "embedded Plan-V3 validation did not address its complete typed file closure"
    );
    let actual_files = inventory_relative_files_v3(inventory, "private/official-content-authority");
    ensure!(
        actual_files == typed_topology.files,
        "embedded Plan-V3 authority contains a missing or extra file"
    );
    let authority_prefix = "private/official-content-authority";
    let actual_directories = inventory
        .directories
        .iter()
        .filter_map(|directory| {
            if directory.path == authority_prefix {
                Some(String::new())
            } else {
                directory
                    .path
                    .strip_prefix(&format!("{authority_prefix}/"))
                    .map(str::to_owned)
            }
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        actual_directories == typed_topology.directories,
        "embedded Plan-V3 authority contains a missing or extra directory"
    );
    for file in inventory.files.iter().filter(|file| {
        file.path
            .starts_with("private/official-content-authority/verifier-bundles/")
    }) {
        ensure!(
            file.unix_mode & 0o222 == 0,
            "embedded verifier bundle file is writable"
        );
    }
    for directory in inventory.directories.iter().filter(|directory| {
        directory
            .path
            .starts_with("private/official-content-authority/verifier-bundles")
    }) {
        ensure!(
            directory.unix_mode & 0o222 == 0,
            "embedded verifier bundle directory is writable"
        );
    }
    Ok(validated)
}

pub(super) fn load_admitted_profile_managers_from_inventory_v3(
    inventory: &mut PublicationTreeInventoryV3,
    authority: &ValidatedOfficialContentV3,
) -> Result<AdmittedProfileManagersV1> {
    fn load_edition(
        inventory: &mut PublicationTreeInventoryV3,
        authority: &ValidatedOfficialContentV3,
        edition: OfficialContentEditionV1,
        digests: &[Digest32],
    ) -> Result<robin_engine::profiles::ProfileManager> {
        ensure!(
            !digests.is_empty(),
            "official edition has no content subjects"
        );
        let mut admitted_artifact = None;
        let mut admitted_document: Option<SimulationContentComponentDocumentV1> = None;
        for digest in digests {
            let manifest = authority
                .content
                .get(digest)
                .context("official content digest has no validated manifest")?;
            ensure!(
                manifest.edition == edition,
                "official profile selector crossed edition boundaries"
            );
            let components = manifest
                .components
                .iter()
                .filter(|component| component.kind == SimulationContentComponentKindV1::Profiles)
                .collect::<Vec<_>>();
            ensure!(
                components.len() == 1,
                "official content subject must contain exactly one Profiles component"
            );
            let component = components[0];
            if let Some(expected) = &admitted_artifact {
                ensure!(
                    expected == &component.artifact,
                    "official edition subjects disagree about their Profiles artifact"
                );
            } else {
                admitted_artifact = Some(component.artifact.clone());
            }
            let relative = simulation_content_component_relative_path_v1(
                &manifest.subject,
                SimulationContentComponentKindV1::Profiles,
            )?;
            let path = format!(
                "private/official-content-authority/verifier-bundles/{digest}/catalog/{relative}"
            );
            let bytes = read_inventory_file_v3(inventory, &path, 128 * 1024 * 1024)?;
            ensure!(
                inventory_artifact_v3(
                    inventory,
                    &path,
                    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
                )? == component.artifact,
                "authenticated Profiles component differs from its content manifest"
            );
            let document: SimulationContentComponentDocumentV1 =
                SimulationContentComponentDocumentV1::from_bitcode(&bytes)
                    .context("decode authenticated Profiles component")?;
            document.validate()?;
            ensure!(
                document.kind == SimulationContentComponentKindV1::Profiles
                    && document.bitcode_bytes()? == bytes,
                "authenticated Profiles component is not canonical typed Profiles"
            );
            if let Some(expected) = &admitted_document {
                ensure!(
                    expected == &document,
                    "official edition Profiles component bytes are not identical"
                );
            } else {
                admitted_document = Some(document);
            }
        }
        let document = admitted_document.context("official edition has no Profiles component")?;
        robin_engine::simulation_inputs::profile_manager_from_component_document_v1(&document)
            .map_err(anyhow::Error::msg)
            .context("strictly decode admitted ProfileManager")
    }

    Ok(AdmittedProfileManagersV1 {
        demo: load_edition(
            inventory,
            authority,
            OfficialContentEditionV1::Demo,
            &authority.digests.demo_content_manifest_sha256,
        )?,
        full: load_edition(
            inventory,
            authority,
            OfficialContentEditionV1::Full,
            &authority.digests.full_content_manifest_sha256,
        )?,
    })
}

pub(super) fn validate_materialized_document_closure(
    inventory: &mut PublicationTreeInventoryV3,
    manifest: &PublicationManifestV3,
) -> Result<()> {
    let backend: BackendPublicationV3 =
        load_inventory_document_v3(inventory, "backend/publication-v3.json")?;
    ensure!(
        backend.build_manifest_sha256 == manifest.build_manifest_sha256
            && backend.rules_config_sha256 == manifest.rules_config_sha256
            && backend.policy_manifest_sha256 == manifest.policy_manifest_sha256
            && backend.ruleset_manifest_sha256 == manifest.ruleset_manifest_sha256
            && backend.competition_manifest_sha256 == manifest.competition_manifest_sha256
            && backend.verifier_operator_config == manifest.verifier_operator_config
            && backend.campaign_states == manifest.campaign_states,
        "backend publication summary differs from the publication manifest"
    );

    // This is the offline publication trust boundary. Revalidate the complete
    // copied Plan-V3 authority once, including its exact inventory, all four
    // sources/receipts/executions/bindings, core and tool authority, and every
    // verifier bundle before consuming any nested artifact.
    let authority = validate_embedded_official_content_v3(inventory)?;
    let digests = &authority.digests;
    ensure!(
        digests.canonical_digest()? == manifest.official_content_digests_sha256
            && read_inventory_file_v3(
                inventory,
                "private/official-content-authority/official-content-digests.sha256",
                64,
            )? == manifest
                .official_content_digests_sha256
                .to_string()
                .as_bytes(),
        "publication content digest authority is substituted"
    );
    let matrix = &authority.matrix;
    ensure!(
        matrix.canonical_digest()? == manifest.projection_authority_matrix_sha256
            && read_inventory_file_v3(
                inventory,
                "private/official-content-authority/projection-authority-matrix-v3.sha256",
                64,
            )? == manifest
                .projection_authority_matrix_sha256
                .to_string()
                .as_bytes(),
        "publication projection matrix authority is substituted"
    );
    let projection_authority = &authority.projection_authority;

    let build: BuildManifestV2 = load_one_inventory_addressed_document_v3(
        inventory,
        "backend/manifests/builds",
        manifest.build_manifest_sha256,
    )?;
    ensure!(
        build == authority.build,
        "backend BuildManifestV2 differs from the fully validated official authority"
    );
    validate_materialized_datadir_binding(inventory, manifest)?;
    projection_authority.validate_against(&build)?;
    let mut tool_authority_digests = vec![
        build.viewer.engine.wasm_bindgen_cli.authority_sha256,
        build.viewer.engine.binaryen_wasm_opt.authority_sha256,
        build.viewer.engine.wabt_wasm_strip.authority_sha256,
    ];
    tool_authority_digests.sort();
    let tool_authorities: BTreeMap<Digest32, robin_run_protocol::BuildToolAuthorityDocumentV1> =
        load_inventory_addressed_documents_v3(
            inventory,
            "private/official-content-authority/manifests/build-tool-authorities",
            &tool_authority_digests,
        )?;
    build.validate_wasm_tool_authorities(
        tool_authorities
            .get(&build.viewer.engine.wasm_bindgen_cli.authority_sha256)
            .context("wasm-bindgen authority document is absent")?,
        tool_authorities
            .get(&build.viewer.engine.binaryen_wasm_opt.authority_sha256)
            .context("Binaryen authority document is absent")?,
        tool_authorities
            .get(&build.viewer.engine.wabt_wasm_strip.authority_sha256)
            .context("WABT authority document is absent")?,
    )?;
    let viewer_report_path = format!(
        "private/viewer-build-reports-v2/{}.json",
        manifest.viewer_build_report.sha256
    );
    validate_inventory_artifact_v3(
        inventory,
        &viewer_report_path,
        &manifest.viewer_build_report,
    )?;
    let viewer_build_report: OfficialViewerBuildReportV2 =
        load_inventory_document_v3(inventory, &viewer_report_path)?;
    viewer_build_report.validate_against(&build)?;
    let mut expected_content = digests
        .demo_content_manifest_sha256
        .iter()
        .chain(&digests.full_content_manifest_sha256)
        .copied()
        .collect::<Vec<_>>();
    expected_content.sort();
    ensure!(
        backend.content_manifest_sha256 == expected_content,
        "backend content manifest index differs from official authority"
    );
    let content: BTreeMap<Digest32, ContentManifestV1> = load_inventory_addressed_documents_v3(
        inventory,
        "backend/manifests/content-manifests",
        &backend.content_manifest_sha256,
    )?;
    ensure!(
        content == authority.content,
        "backend content catalog differs from the fully validated official authority"
    );
    let mut expected_campaigns = vec![
        digests.demo_campaign_content_manifest_sha256,
        digests.full_campaign_content_manifest_sha256,
    ];
    expected_campaigns.sort();
    ensure!(
        backend.campaign_content_manifest_sha256 == expected_campaigns,
        "backend campaign catalog index differs from official authority"
    );
    let campaigns: BTreeMap<Digest32, CampaignContentManifestV1> =
        load_inventory_addressed_documents_v3(
            inventory,
            "backend/manifests/campaign-content-manifests",
            &backend.campaign_content_manifest_sha256,
        )?;
    ensure!(
        campaigns == authority.campaigns,
        "backend campaign catalog differs from the fully validated official authority"
    );
    let rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1> =
        load_inventory_addressed_documents_v3(
            inventory,
            "backend/manifests/rules-configs",
            &manifest.rules_config_sha256,
        )?;
    ensure!(
        rules_configs.get(&authority.rules.canonical_digest()?) == Some(&authority.rules),
        "publication omits or substitutes the Plan-V3 authority rules config"
    );
    let policies: BTreeMap<Digest32, ImmutablePolicyManifestV1> =
        load_inventory_addressed_documents_v3(
            inventory,
            "backend/manifests/policies",
            &manifest.policy_manifest_sha256,
        )?;
    let rulesets: BTreeMap<Digest32, RulesetManifestV1> = load_inventory_addressed_documents_v3(
        inventory,
        "backend/manifests/ruleset-manifests",
        &manifest.ruleset_manifest_sha256,
    )?;
    let published = load_inventory_published_rulesets_v3(
        inventory,
        "backend/manifests/published-rulesets",
        &manifest.published_rulesets,
    )?;
    ensure!(
        published
            .iter()
            .all(|(digest, status)| rulesets.get(digest) == Some(&status.manifest)),
        "mutable publication status embeds a substituted immutable ruleset"
    );
    let competitions: BTreeMap<Digest32, CompetitionManifestV1> =
        load_inventory_addressed_documents_v3(
            inventory,
            "backend/manifests/competitions",
            &manifest.competition_manifest_sha256,
        )?;
    validate_document_closure(
        manifest.build_manifest_sha256,
        &content,
        &campaigns,
        &rules_configs,
        &policies,
        &published,
        &competitions,
    )?;

    // Recover each edition's exact typed ProfileManager from the completely
    // revalidated authority, then independently re-derive every template.
    let admitted_profiles =
        load_admitted_profile_managers_from_inventory_v3(inventory, &authority)?;

    ensure!(
        backend.verifier_program == build.verifier.artifact,
        "backend verifier artifact differs from BuildManifestV2"
    );
    validate_inventory_artifact_v3(
        inventory,
        &format!("private/verifier/bin/{}", backend.verifier_program.sha256),
        &backend.verifier_program,
    )?;
    validate_inventory_artifact_v3(
        inventory,
        &format!(
            "private/verifier/operator-config/{}",
            backend.verifier_operator_config.sha256
        ),
        &backend.verifier_operator_config,
    )?;
    for state in &backend.campaign_states {
        let rules = rules_configs
            .get(&state.rules_config_sha256)
            .context("materialized campaign state references absent rules")?;
        let bytes = read_inventory_file_v3(
            inventory,
            &format!("private/campaign-states/{}", state.artifact.sha256),
            crate::campaign_template_v1::MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1 as u64,
        )?;
        let requirement = state.requirement();
        ensure!(
            validate_canonical_campaign_template_v1(
                &bytes,
                requirement,
                rules,
                admitted_profiles.get(state.edition),
            )? == state.artifact,
            "materialized campaign state differs from its exact pin"
        );
    }
    let expected_campaign_files = backend
        .campaign_states
        .iter()
        .map(|state| state.artifact.sha256.to_string())
        .collect::<BTreeSet<_>>();
    let actual_campaign_files = inventory_relative_files_v3(inventory, "private/campaign-states");
    ensure!(
        actual_campaign_files == expected_campaign_files,
        "physical campaign template inventory differs from logical campaign pins"
    );
    for file in &manifest.public_static_files {
        validate_inventory_artifact_v3(
            inventory,
            &format!("cloudflare-public/{}", file.published_path),
            &file.artifact,
        )?;
    }
    for file in &manifest.identity_signer_files {
        validate_inventory_artifact_v3(
            inventory,
            &format!("cloudflare-identity-signer/{}", file.published_path),
            &file.artifact,
        )?;
    }
    let public_build: BuildManifestV2 = load_inventory_document_v3(
        inventory,
        &format!(
            "cloudflare-public/manifests/builds/{}.json",
            manifest.build_manifest_sha256
        ),
    )?;
    ensure!(
        public_build == build,
        "Cloudflare public build manifest is substituted"
    );
    for named in &build.viewer.engine.artifacts {
        validate_inventory_artifact_v3(
            inventory,
            &format!(
                "cloudflare-public/{}",
                build_artifact_object_path_v1(manifest.build_manifest_sha256, named)?
            ),
            &named.artifact,
        )?;
    }
    validate_public_tree_inventory(inventory, manifest, &build)?;
    validate_public_privacy(inventory, manifest, matrix, projection_authority, digests)?;
    expected_publication_topology_from_validated_v3(
        inventory, manifest, &backend, &build, &authority,
    )?
    .validate_inventory(inventory)?;
    Ok(())
}

pub(super) fn expected_publication_topology_from_validated_v3(
    inventory: &PublicationTreeInventoryV3,
    manifest: &PublicationManifestV3,
    backend: &BackendPublicationV3,
    build: &BuildManifestV2,
    authority: &ValidatedOfficialContentV3,
) -> Result<ExpectedPublicationTopologyV3> {
    let mut expected = ExpectedPublicationTopologyV3::new();
    for directory in [
        "backend/manifests/builds",
        "backend/manifests/content-manifests",
        "backend/manifests/campaign-content-manifests",
        "backend/manifests/rules-configs",
        "backend/manifests/ruleset-manifests",
        "backend/manifests/published-rulesets",
        "backend/manifests/competitions",
        "backend/manifests/policies",
        "cloudflare-public",
        "cloudflare-identity-signer",
        "deployment",
    ] {
        expected.register_directory(directory)?;
    }
    for path in [
        "publication-manifest-v3.json".to_owned(),
        "publication-manifest-v3.sha256".to_owned(),
        "publication-lock-v3.json".to_owned(),
        "publication-lock-v3.sha256".to_owned(),
        "backend/publication-v3.json".to_owned(),
        DATADIR_AUTHORITY_PATH.to_owned(),
        DATADIR_DEPLOYMENT_RECEIPT_PATH.to_owned(),
        "deployment/exposure-v3.json".to_owned(),
        format!(
            "private/viewer-build-reports-v2/{}.json",
            manifest.viewer_build_report.sha256
        ),
        format!("private/verifier/bin/{}", backend.verifier_program.sha256),
        format!(
            "private/verifier/operator-config/{}",
            backend.verifier_operator_config.sha256
        ),
        format!(
            "cloudflare-public/manifests/builds/{}.json",
            manifest.build_manifest_sha256
        ),
    ] {
        let executable = path.starts_with("private/verifier/bin/");
        expected.register_inventory_file(inventory, path, executable)?;
    }
    for (directory, digests) in [
        (
            "backend/manifests/builds",
            vec![manifest.build_manifest_sha256],
        ),
        (
            "backend/manifests/content-manifests",
            backend.content_manifest_sha256.clone(),
        ),
        (
            "backend/manifests/campaign-content-manifests",
            backend.campaign_content_manifest_sha256.clone(),
        ),
        (
            "backend/manifests/rules-configs",
            manifest.rules_config_sha256.clone(),
        ),
        (
            "backend/manifests/policies",
            manifest.policy_manifest_sha256.clone(),
        ),
        (
            "backend/manifests/ruleset-manifests",
            manifest.ruleset_manifest_sha256.clone(),
        ),
        (
            "backend/manifests/published-rulesets",
            manifest
                .published_rulesets
                .iter()
                .map(|entry| entry.ruleset_manifest_sha256)
                .collect(),
        ),
        (
            "backend/manifests/competitions",
            manifest.competition_manifest_sha256.clone(),
        ),
    ] {
        for digest in digests {
            expected.register_inventory_file(
                inventory,
                format!("{directory}/{digest}.json"),
                false,
            )?;
        }
    }
    let mut campaigns = BTreeSet::new();
    for state in &backend.campaign_states {
        if campaigns.insert(state.artifact.sha256) {
            expected.register_inventory_file(
                inventory,
                format!("private/campaign-states/{}", state.artifact.sha256),
                false,
            )?;
        }
    }
    for named in &build.viewer.engine.artifacts {
        expected.register_inventory_file(
            inventory,
            format!(
                "cloudflare-public/{}",
                build_artifact_object_path_v1(manifest.build_manifest_sha256, named)?
            ),
            false,
        )?;
    }
    for file in &manifest.public_static_files {
        expected.register_inventory_file(
            inventory,
            format!("cloudflare-public/{}", file.published_path),
            false,
        )?;
    }
    for file in &manifest.identity_signer_files {
        expected.register_inventory_file(
            inventory,
            format!("cloudflare-identity-signer/{}", file.published_path),
            false,
        )?;
    }
    let typed_official = expected_official_authority_topology_v3(authority)?;
    for file in &typed_official.files {
        let path = format!("private/official-content-authority/{file}");
        expected.register_inventory_file(
            inventory,
            path.clone(),
            path.starts_with(
                "private/official-content-authority/private/build-artifacts/projection-exporters/",
            ),
        )?;
    }
    for directory in &typed_official.directories {
        let path = if directory.is_empty() {
            "private/official-content-authority".to_owned()
        } else {
            format!("private/official-content-authority/{directory}")
        };
        expected.register_directory(&path)?;
    }
    Ok(expected)
}

pub(super) fn validate_public_tree_inventory(
    inventory: &PublicationTreeInventoryV3,
    manifest: &PublicationManifestV3,
    build: &BuildManifestV2,
) -> Result<()> {
    let mut expected_public = BTreeSet::from([format!(
        "manifests/builds/{}.json",
        manifest.build_manifest_sha256
    )]);
    for named in &build.viewer.engine.artifacts {
        expected_public.insert(build_artifact_object_path_v1(
            manifest.build_manifest_sha256,
            named,
        )?);
    }
    expected_public.extend(
        build
            .viewer
            .pages_shell
            .public_origin_artifacts
            .iter()
            .map(|artifact| artifact.path.clone()),
    );
    let actual_public = inventory_relative_files_v3(inventory, "cloudflare-public");
    ensure!(
        actual_public == expected_public,
        "Cloudflare public-static tree contains a missing or extra artifact"
    );

    let expected_signer = build
        .viewer
        .identity_signer
        .identity_signer_origin_artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<BTreeSet<_>>();
    let actual_signer = inventory_relative_files_v3(inventory, "cloudflare-identity-signer");
    ensure!(
        actual_signer == expected_signer,
        "identity-signer origin contains a missing or extra artifact"
    );
    Ok(())
}

pub(super) fn validate_materialized_datadir_binding(
    inventory: &mut PublicationTreeInventoryV3,
    manifest: &PublicationManifestV3,
) -> Result<()> {
    validate_inventory_artifact_v3(
        inventory,
        DATADIR_AUTHORITY_PATH,
        &manifest.datadir_release_authority,
    )?;
    validate_inventory_artifact_v3(
        inventory,
        DATADIR_DEPLOYMENT_RECEIPT_PATH,
        &manifest.datadir_deployment_receipt,
    )?;
    let authority: DatadirReleaseAuthorityV1 =
        load_inventory_canonical_document_v3(inventory, DATADIR_AUTHORITY_PATH)?;
    let receipt: DatadirDeploymentReceiptV1 =
        load_inventory_canonical_document_v3(inventory, DATADIR_DEPLOYMENT_RECEIPT_PATH)?;
    validate_datadir_binding(
        &authority,
        manifest.datadir_release_authority.sha256,
        &receipt,
    )?;

    validate_deployment_metadata_inventory(inventory)
}

pub(super) fn validate_deployment_metadata_inventory(
    inventory: &PublicationTreeInventoryV3,
) -> Result<()> {
    let expected = BTreeSet::from([
        "datadir-authority.json".to_owned(),
        "datadir-deployment.json".to_owned(),
        "exposure-v3.json".to_owned(),
    ]);
    let actual = inventory_relative_files_v3(inventory, "deployment");
    ensure!(
        actual == expected,
        "publication deployment metadata contains missing, extra, or datadir payload bytes"
    );
    Ok(())
}

pub(super) fn validate_public_privacy(
    inventory: &mut PublicationTreeInventoryV3,
    manifest: &PublicationManifestV3,
    matrix: &crate::plan_v3::OfficialProjectionAuthorityMatrixV3,
    projection_authority: &robin_run_protocol::OfficialProjectionAuthorityManifestV2,
    digests: &crate::OfficialContentDigestsV1,
) -> Result<()> {
    let mut forbidden_values = vec![
        matrix
            .projection_authority_manifest_sha256
            .to_string()
            .into_bytes(),
        matrix.canonical_digest()?.to_string().into_bytes(),
        matrix.execution_policy_sha256.to_string().into_bytes(),
        matrix.core_overlay_manifest_sha256.to_string().into_bytes(),
        projection_authority
            .projection_exporter
            .artifact
            .sha256
            .to_string()
            .into_bytes(),
        projection_authority
            .projection_exporter
            .artifact
            .media_type
            .as_bytes()
            .to_vec(),
        manifest
            .verifier_operator_config
            .sha256
            .to_string()
            .into_bytes(),
    ];
    forbidden_values.extend(
        manifest
            .campaign_states
            .iter()
            .map(|state| state.artifact.sha256.to_string().into_bytes()),
    );
    forbidden_values.extend(matrix.lanes.iter().flat_map(|lane| {
        [
            lane.source_tree_manifest_sha256.to_string().into_bytes(),
            lane.projection_receipt_sha256.to_string().into_bytes(),
            lane.execution_record_sha256.to_string().into_bytes(),
        ]
    }));
    let operator_config = format!(
        "private/verifier/operator-config/{}",
        manifest.verifier_operator_config.sha256
    );
    forbidden_values.push(operator_config.as_bytes().to_vec());
    forbidden_values.push(read_inventory_file_v3(
        inventory,
        &operator_config,
        manifest.verifier_operator_config.byte_length,
    )?);
    let mut campaign_artifacts = BTreeSet::new();
    for state in &manifest.campaign_states {
        if campaign_artifacts.insert(state.artifact.sha256) {
            let path = format!("private/campaign-states/{}", state.artifact.sha256);
            forbidden_values.push(path.as_bytes().to_vec());
            forbidden_values.push(read_inventory_file_v3(
                inventory,
                &path,
                state.artifact.byte_length,
            )?);
        }
    }
    forbidden_values.sort();
    forbidden_values.dedup();
    let backend_forbidden = forbidden_values.clone();
    let mut public_forbidden = forbidden_values;
    public_forbidden.extend(
        digests
            .full_content_manifest_sha256
            .iter()
            .map(|digest| digest.to_string().into_bytes()),
    );
    public_forbidden.push(
        digests
            .full_campaign_content_manifest_sha256
            .to_string()
            .into_bytes(),
    );

    scan_public_inventory_v3(
        inventory,
        "backend/manifests",
        &backend_forbidden,
        "backend manifests",
    )?;
    scan_public_inventory_v3(
        inventory,
        "cloudflare-public",
        &public_forbidden,
        "Cloudflare public-static",
    )?;
    scan_public_inventory_v3(
        inventory,
        "cloudflare-identity-signer",
        &public_forbidden,
        "identity signer",
    )?;
    Ok(())
}

pub(super) fn scan_public_inventory_v3(
    inventory: &mut PublicationTreeInventoryV3,
    root: &str,
    forbidden_values: &[Vec<u8>],
    label: &str,
) -> Result<()> {
    const PRIVATE_PATH_TERMS: &[&str] = &[
        "private",
        "projection-authority",
        "projection-exporter",
        "projection-receipt",
        "source-tree-manifest",
        "projection-execution",
        "verifier-source-binding",
        "campaign-state",
        "operator-config",
    ];
    let paths = inventory_relative_files_v3(inventory, root);
    for relative in paths {
        let folded = relative.to_ascii_lowercase();
        ensure!(
            !PRIVATE_PATH_TERMS.iter().any(|term| folded.contains(term)),
            "{label} path enters a private namespace: {relative}"
        );
        let path = format!("{root}/{relative}");
        let maximum = inventory
            .files
            .iter()
            .find(|file| file.path == path)
            .context("PublicationV3 public inventory path disappeared")?
            .artifact
            .byte_length;
        let bytes = read_inventory_file_v3(inventory, &path, maximum)?;
        for value in forbidden_values {
            ensure!(
                memchr::memmem::find(&bytes, value).is_none(),
                "{label} artifact {relative} contains a private authority value"
            );
        }
        if Path::new(&relative)
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            ensure!(
                u64::try_from(bytes.len())
                    .ok()
                    .is_some_and(|len| len <= MAX_DOCUMENT_BYTES),
                "{label} JSON exceeds the operator document bound"
            );
            let value: serde_json::Value = strict_json_from_slice(&bytes)?;
            let schema = public_json_schema(&relative, &value, label)?;
            reject_private_json_keys(&value, label, schema, &mut Vec::new())?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn scan_public_tree(
    root: &Path,
    forbidden_values: &[Vec<u8>],
    label: &str,
) -> Result<()> {
    const PRIVATE_PATH_TERMS: &[&str] = &[
        "private",
        "projection-authority",
        "projection-exporter",
        "projection-receipt",
        "source-tree-manifest",
        "projection-execution",
        "verifier-source-binding",
        "campaign-state",
        "operator-config",
    ];
    for (relative, absolute) in walk_regular_files(root)? {
        let relative = path_to_manifest(&relative)?;
        let folded = relative.to_ascii_lowercase();
        ensure!(
            !PRIVATE_PATH_TERMS.iter().any(|term| folded.contains(term)),
            "{label} path enters a private namespace: {relative}"
        );
        for value in forbidden_values {
            ensure!(
                !file_contains_bytes(&absolute, value)?,
                "{label} artifact {relative} contains a private authority value"
            );
        }
        if absolute
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let bytes = crate::read_regular_file_bounded(&absolute, MAX_DOCUMENT_BYTES)?;
            let value: serde_json::Value = strict_json_from_slice(&bytes)?;
            let schema = public_json_schema(&relative, &value, label)?;
            reject_private_json_keys(&value, label, schema, &mut Vec::new())?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicJsonSchema {
    Other,
    RulesetManifest,
    PublishedRuleset,
    CompetitionManifest,
}

pub(super) fn public_json_schema(
    relative: &str,
    value: &serde_json::Value,
    label: &str,
) -> Result<PublicJsonSchema> {
    let mut components = relative.split('/');
    let directory = components.next();
    let file = components.next();
    let exact_addressed_document = components.next().is_none() && file.is_some();
    let schema = match (directory, exact_addressed_document) {
        (Some("ruleset-manifests"), true) => PublicJsonSchema::RulesetManifest,
        (Some("published-rulesets"), true) => PublicJsonSchema::PublishedRuleset,
        (Some("competitions"), true) => PublicJsonSchema::CompetitionManifest,
        _ => PublicJsonSchema::Other,
    };
    match schema {
        PublicJsonSchema::RulesetManifest => {
            let document: RulesetManifestV1 = serde_json::from_value(value.clone())
                .with_context(|| format!("{label} contains a malformed ruleset manifest"))?;
            document.validate()?;
            let expected_file = format!("{}.json", document.canonical_digest()?);
            ensure!(
                file == Some(expected_file.as_str()),
                "{label} ruleset manifest path is not its lowercase canonical digest"
            );
            ensure!(
                serde_json::to_value(&document)? == *value,
                "{label} ruleset manifest differs from its exact public schema"
            );
        }
        PublicJsonSchema::PublishedRuleset => {
            let document: PublishedRulesetV1 = serde_json::from_value(value.clone())
                .with_context(|| format!("{label} contains a malformed published ruleset"))?;
            document.validate()?;
            let expected_file = format!("{}.json", document.ruleset_manifest_sha256);
            ensure!(
                file == Some(expected_file.as_str()),
                "{label} published ruleset path is not its lowercase ruleset digest"
            );
            ensure!(
                serde_json::to_value(&document)? == *value,
                "{label} published ruleset differs from its exact public schema"
            );
        }
        PublicJsonSchema::CompetitionManifest => {
            let document: CompetitionManifestV1 = serde_json::from_value(value.clone())
                .with_context(|| format!("{label} contains a malformed competition manifest"))?;
            document.validate()?;
            let expected_file = format!("{}.json", document.canonical_digest()?);
            ensure!(
                file == Some(expected_file.as_str()),
                "{label} competition manifest path is not its lowercase canonical digest"
            );
            ensure!(
                serde_json::to_value(&document)? == *value,
                "{label} competition manifest differs from its exact public schema"
            );
        }
        PublicJsonSchema::Other => {}
    }
    Ok(schema)
}

pub(super) fn reject_private_json_keys(
    value: &serde_json::Value,
    label: &str,
    schema: PublicJsonSchema,
    ancestors: &mut Vec<String>,
) -> Result<()> {
    const PRIVATE_KEYS: &[&str] = &[
        "private_request",
        "private_result",
        "session_genesis",
        "transcript",
        "participant_instance_id",
        "chain_id",
        "projection_authority",
        "projection_exporter",
        "starting_campaign",
        "terminal_private_campaign",
        "campaign_state",
        "verifier_operator_config",
    ];
    match value {
        serde_json::Value::Object(fields) => {
            for (key, child) in fields {
                let folded = key.to_ascii_lowercase();
                let allowed_campaign_requirement = key == "canonical_campaign_state"
                    && match schema {
                        PublicJsonSchema::RulesetManifest => ancestors.is_empty(),
                        PublicJsonSchema::PublishedRuleset => ancestors.as_slice() == ["manifest"],
                        PublicJsonSchema::CompetitionManifest => ancestors.is_empty(),
                        PublicJsonSchema::Other => false,
                    };
                if allowed_campaign_requirement {
                    let requirement: CanonicalCampaignStateRequirementV1 = serde_json::from_value(
                        child.clone(),
                    )
                    .with_context(|| {
                        format!(
                            "{label} canonical_campaign_state is not the exact public requirement"
                        )
                    })?;
                    requirement.validate()?;
                    ensure!(
                        serde_json::to_value(requirement)? == *child,
                        "{label} canonical_campaign_state contains non-public fields"
                    );
                }
                ensure!(
                    allowed_campaign_requirement
                        || !PRIVATE_KEYS.iter().any(|private| folded.contains(private)),
                    "{label} JSON contains private field {key:?}"
                );
                ancestors.push(key.clone());
                reject_private_json_keys(child, label, schema, ancestors)?;
                ancestors.pop();
            }
        }
        serde_json::Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                ancestors.push(index.to_string());
                reject_private_json_keys(child, label, schema, ancestors)?;
                ancestors.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn file_contains_bytes(path: &Path, needle: &[u8]) -> Result<bool> {
    use std::io::Read as _;

    ensure!(!needle.is_empty(), "privacy sentinel is empty");
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut carry = Vec::new();
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(false);
        }
        carry.extend_from_slice(&chunk[..count]);
        if memchr::memmem::find(&carry, needle).is_some() {
            return Ok(true);
        }
        let retained = needle.len().saturating_sub(1).min(carry.len());
        carry.drain(..carry.len() - retained);
    }
}

pub(super) fn load_one_inventory_addressed_document_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    directory: &str,
    digest: Digest32,
) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let documents = load_inventory_addressed_documents_v3(inventory, directory, &[digest])?;
    documents
        .into_iter()
        .next()
        .map(|(_, document)| document)
        .context("addressed PublicationV3 document is absent")
}

pub(super) fn load_inventory_addressed_documents_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    directory: &str,
    expected_digests: &[Digest32],
) -> Result<BTreeMap<Digest32, T>>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    ensure!(
        inventory_has_directory_v3(inventory, directory),
        "PublicationV3 addressed directory is absent: {directory}"
    );
    let expected_paths = expected_digests
        .iter()
        .map(|digest| format!("{digest}.json"))
        .collect::<BTreeSet<_>>();
    ensure!(
        inventory_relative_files_v3(inventory, directory) == expected_paths,
        "PublicationV3 addressed document directory inventory differs from its index"
    );
    expected_digests
        .iter()
        .map(|digest| {
            let path = format!("{directory}/{digest}.json");
            let document: T = load_inventory_document_v3(inventory, &path)?;
            ensure!(
                Digest32::digest_bytes(&canonical_json_bytes(&document)?) == *digest,
                "PublicationV3 addressed document digest differs from its path"
            );
            Ok((*digest, document))
        })
        .collect()
}

pub(super) fn load_inventory_published_rulesets_v3(
    inventory: &mut PublicationTreeInventoryV3,
    directory: &str,
    expected: &[PublishedRulesetArtifactV3],
) -> Result<BTreeMap<Digest32, PublishedRulesetV1>> {
    let expected_paths = expected
        .iter()
        .map(|entry| format!("{}.json", entry.ruleset_manifest_sha256))
        .collect::<BTreeSet<_>>();
    ensure!(
        inventory_has_directory_v3(inventory, directory)
            && inventory_relative_files_v3(inventory, directory) == expected_paths,
        "PublicationV3 published ruleset inventory differs from its index"
    );
    expected
        .iter()
        .map(|entry| {
            let path = format!("{directory}/{}.json", entry.ruleset_manifest_sha256);
            ensure!(
                inventory_artifact_v3(inventory, &path, &entry.artifact.media_type)?
                    == entry.artifact,
                "PublicationV3 published ruleset artifact is substituted"
            );
            let published: PublishedRulesetV1 = load_inventory_document_v3(inventory, &path)?;
            ensure!(
                published.ruleset_manifest_sha256 == entry.ruleset_manifest_sha256,
                "PublicationV3 published ruleset identity differs from its path"
            );
            Ok((entry.ruleset_manifest_sha256, published))
        })
        .collect()
}

#[cfg(test)]
pub(super) fn load_addressed_documents<T>(
    directory: &Path,
    expected_digests: &[Digest32],
) -> Result<BTreeMap<Digest32, T>>
where
    T: for<'de> Deserialize<'de> + Serialize + robin_run_protocol::Validate,
{
    validate_mount_root(directory)?;
    let expected_paths = expected_digests
        .iter()
        .map(|digest| format!("{digest}.json"))
        .collect::<BTreeSet<_>>();
    let actual_paths = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            validate_regular_file(&entry.path())?;
            entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF-8 addressed document path"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        actual_paths == expected_paths,
        "addressed document directory inventory differs from its index"
    );
    expected_digests
        .iter()
        .map(|digest| {
            let document: T =
                crate::load_canonical_document(&directory.join(format!("{digest}.json")))?;
            ensure!(
                Digest32::digest_bytes(&canonical_json_bytes(&document)?) == *digest,
                "addressed document canonical digest differs from its path"
            );
            Ok((*digest, document))
        })
        .collect()
}

pub(super) fn validate_inventory_artifact_v3(
    inventory: &PublicationTreeInventoryV3,
    path: &str,
    expected: &ArtifactRefV1,
) -> Result<()> {
    ensure!(
        inventory_artifact_v3(inventory, path, &expected.media_type)? == *expected,
        "materialized PublicationV3 artifact {path} differs from its identity"
    );
    Ok(())
}

pub(super) fn ensure_required_backend_layout(inventory: &PublicationTreeInventoryV3) -> Result<()> {
    for directory in [
        "builds",
        "content-manifests",
        "campaign-content-manifests",
        "rules-configs",
        "ruleset-manifests",
        "published-rulesets",
        "competitions",
        "policies",
    ] {
        ensure!(
            inventory_has_directory_v3(inventory, &format!("backend/manifests/{directory}")),
            "PublicationV3 omits backend manifest directory {directory}"
        );
    }
    ensure!(
        !inventory_has_directory_v3(inventory, "backend/manifests/rulesets"),
        "legacy monolithic rulesets compatibility directory is forbidden"
    );
    Ok(())
}

pub(super) fn ensure_no_full_public_leak(inventory: &mut PublicationTreeInventoryV3) -> Result<()> {
    let authority: crate::OfficialContentDigestsV1 = load_inventory_document_v3(
        inventory,
        "private/official-content-authority/official-content-digests.json",
    )?;
    for digest in authority.full_content_manifest_sha256 {
        ensure!(
            inventory.files.iter().all(|file| file.path
                != format!("cloudflare-public/manifests/content-manifests/{digest}.json")
                && !file
                    .path
                    .starts_with(&format!("cloudflare-public/content/{digest}/"))),
            "Full proprietary content leaked into Cloudflare public-static"
        );
    }
    ensure!(
        inventory.files.iter().all(|file| file.path
            != format!(
                "cloudflare-public/manifests/campaign-content-manifests/{}.json",
                authority.full_campaign_content_manifest_sha256
            )),
        "Full campaign catalog leaked into Cloudflare public-static"
    );
    Ok(())
}

pub(super) fn validate_transition(
    transition: &PublicationTransitionV3,
    candidate: &ValidatedPublicationV3,
) -> Result<()> {
    validate_transition_with(transition, candidate, || {})
}

pub(super) fn validate_transition_with<F>(
    transition: &PublicationTransitionV3,
    candidate: &ValidatedPublicationV3,
    after_validation: F,
) -> Result<()>
where
    F: FnOnce(),
{
    let previous = match transition {
        PublicationTransitionV3::Fresh => None,
        PublicationTransitionV3::Update { previous_release } => Some((
            validate_publication_v3_authority(previous_release)?,
            TransitionRule::Update,
        )),
        PublicationTransitionV3::StatusTransition { previous_release } => Some((
            validate_publication_v3_authority(previous_release)?,
            TransitionRule::StatusOnly,
        )),
        PublicationTransitionV3::Rollback { target_release } => Some((
            validate_publication_v3_authority(target_release)?,
            TransitionRule::Exact,
        )),
    };
    after_validation();
    if let Some((previous, rule)) = &previous {
        compare_transition_authorities(
            &previous.inventory.authority(),
            &candidate.inventory.authority(),
            *rule,
        )?;
        previous.ensure_live()?;
    }
    candidate.ensure_live()
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TransitionRule {
    Update,
    StatusOnly,
    Exact,
}

#[cfg(test)]
pub(super) fn compare_transition(
    previous: &Path,
    candidate: &Path,
    rule: TransitionRule,
) -> Result<()> {
    let previous_root = open_publication_root_v3(previous)?;
    let candidate_root = open_publication_root_v3(candidate)?;
    let previous = publication_tree_inventory_v3_from_fd(previous, &previous_root)?;
    let candidate = publication_tree_inventory_v3_from_fd(candidate, &candidate_root)?;
    compare_transition_authorities(&previous.authority(), &candidate.authority(), rule)
}

pub(super) fn compare_transition_authorities(
    previous: &PublicationTreeAuthorityV3,
    candidate: &PublicationTreeAuthorityV3,
    rule: TransitionRule,
) -> Result<()> {
    let previous_files = previous
        .files
        .iter()
        .map(|(path, artifact, _)| (path.clone(), artifact.clone()))
        .collect::<BTreeMap<_, _>>();
    let candidate_files = candidate
        .files
        .iter()
        .map(|(path, artifact, _)| (path.clone(), artifact.clone()))
        .collect::<BTreeMap<_, _>>();
    let previous_modes = previous
        .files
        .iter()
        .map(|(path, _, mode)| (path.clone(), *mode))
        .collect::<BTreeMap<_, _>>();
    let candidate_modes = candidate
        .files
        .iter()
        .map(|(path, _, mode)| (path.clone(), *mode))
        .collect::<BTreeMap<_, _>>();
    let previous_directories = &previous.directories;
    let candidate_directories = &candidate.directories;
    match rule {
        TransitionRule::Exact => {
            ensure!(
                previous_files == candidate_files,
                "rollback candidate is not byte-identical to its reviewed target"
            );
            ensure!(
                previous_modes == candidate_modes,
                "rollback candidate modes differ from its reviewed target"
            );
            ensure!(
                previous_directories == candidate_directories,
                "rollback candidate directories differ from its reviewed target"
            );
        }
        TransitionRule::StatusOnly => {
            let previous_immutable = immutable_transition_files(&previous_files);
            let candidate_immutable = immutable_transition_files(&candidate_files);
            ensure!(
                previous_immutable == candidate_immutable,
                "status transition changed an immutable publication file"
            );
            let previous_status = status_transition_files(&previous_files);
            let candidate_status = status_transition_files(&candidate_files);
            ensure!(
                previous_status.keys().eq(candidate_status.keys())
                    && previous_status != candidate_status,
                "status transition must change only existing published statuses"
            );
            ensure!(
                previous_modes == candidate_modes,
                "status transition changed publication modes"
            );
            ensure!(
                previous_directories == candidate_directories,
                "status transition changed publication directories"
            );
        }
        TransitionRule::Update => {
            let candidate_immutable = immutable_transition_files(&candidate_files);
            for (path, artifact) in immutable_transition_files(&previous_files) {
                ensure!(
                    candidate_immutable.get(path) == Some(&artifact),
                    "update removed or rewrote immutable file {path}"
                );
            }
            let candidate_status = status_transition_files(&candidate_files);
            for (path, artifact) in status_transition_files(&previous_files) {
                ensure!(
                    candidate_status.get(path) == Some(&artifact),
                    "update changed an existing status; use status_transition"
                );
            }
            for (path, mode) in &previous_modes {
                ensure!(
                    candidate_modes.get(path) == Some(mode),
                    "update removed or changed mode for existing path {path}"
                );
            }
            ensure!(
                previous_directories
                    .iter()
                    .all(|path| candidate_directories.binary_search(path).is_ok()),
                "update removed an existing publication directory"
            );
            ensure!(
                candidate_files.len() > previous_files.len(),
                "update adds no new immutable publication data"
            );
        }
    }
    Ok(())
}
