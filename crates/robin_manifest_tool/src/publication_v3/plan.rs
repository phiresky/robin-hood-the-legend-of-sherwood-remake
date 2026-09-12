//! plan responsibilities of the admitted release pipeline.
use super::*;

pub(super) fn load_pinned_json_source<T>(source: &PinnedArtifactSourceV3) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    validate_pinned_source(source)?;
    ensure!(
        source.artifact.media_type == "application/json",
        "datadir metadata pin must use application/json"
    );
    let bytes = crate::read_regular_file_bounded(&source.source, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse datadir metadata {}", source.source.display()))?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "{} is not byte-for-byte canonical JSON",
        source.source.display()
    );
    Ok(document)
}

#[derive(Debug)]
pub(super) struct LoadedPublication {
    pub(super) plan: OperatorPublicationPlanV3,
    pub(super) authority: ValidatedOfficialContentV3,
    pub(super) build_draft: BuildDraftV2,
    pub(super) build_sha256: Digest32,
    pub(super) viewer_build_report_artifact: ArtifactRefV1,
    pub(super) rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1>,
    pub(super) policies: BTreeMap<Digest32, ImmutablePolicyManifestV1>,
    pub(super) published: BTreeMap<Digest32, PublishedRulesetV1>,
    pub(super) competitions: BTreeMap<Digest32, CompetitionManifestV1>,
    pub(super) campaign_states: Vec<CampaignStateArtifactV3>,
}

impl OperatorPublicationPlanV3 {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let bytes = crate::read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut plan: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse publication plan {}", path.display()))?;
        ensure!(
            plan.schema_version == PUBLICATION_PLAN_SCHEMA_VERSION,
            "unsupported publication plan schema"
        );
        let base = fs::canonicalize(config_parent(path)?)?;
        for path in [
            &mut plan.official_content_authority,
            &mut plan.build_draft_v2,
            &mut plan.viewer_build_report,
            &mut plan.datadir_release_authority.source,
            &mut plan.datadir_deployment_receipt.source,
            &mut plan.verifier_operator_config.source,
        ] {
            resolve_path(&base, path);
        }
        for state in &mut plan.campaign_states {
            resolve_path(&base, &mut state.source);
        }
        for path in plan
            .additional_rules_configs
            .iter_mut()
            .chain(&mut plan.policies)
            .chain(&mut plan.published_rulesets)
            .chain(&mut plan.competitions)
        {
            resolve_path(&base, path);
        }
        match &mut plan.transition {
            PublicationTransitionV3::Fresh => {}
            PublicationTransitionV3::Update { previous_release }
            | PublicationTransitionV3::StatusTransition { previous_release } => {
                resolve_path(&base, previous_release)
            }
            PublicationTransitionV3::Rollback { target_release } => {
                resolve_path(&base, target_release)
            }
        }
        Ok(plan)
    }
}

pub(super) fn load_publication(plan_path: &Path) -> Result<LoadedPublication> {
    let plan = OperatorPublicationPlanV3::load(plan_path)?;
    let authority = validate_official_content_v3(&plan.official_content_authority)?;
    let build_draft = BuildDraftV2::load(&plan.build_draft_v2)?;
    let authored_build = build_draft.author()?;
    ensure!(
        authored_build == authority.build,
        "fresh per-artifact build authoring differs from the plan-v3 BuildManifestV2"
    );
    validate_current_official_ranked_build_v2(&authored_build)?;
    let build_sha256 = authored_build.canonical_digest()?;
    let viewer_build_report: OfficialViewerBuildReportV2 =
        crate::load_canonical_document(&plan.viewer_build_report)?;
    viewer_build_report.validate_against(&authored_build)?;
    let viewer_build_report_artifact =
        artifact_from_file(&plan.viewer_build_report, "application/json")?;

    let datadir_release_authority: DatadirReleaseAuthorityV1 =
        load_pinned_json_source(&plan.datadir_release_authority)?;
    let datadir_deployment_receipt: DatadirDeploymentReceiptV1 =
        load_pinned_json_source(&plan.datadir_deployment_receipt)?;
    validate_datadir_binding(
        &datadir_release_authority,
        plan.datadir_release_authority.artifact.sha256,
        &datadir_deployment_receipt,
    )?;

    validate_pinned_source(&plan.verifier_operator_config)?;

    let mut rules_configs =
        BTreeMap::from([(authority.rules.canonical_digest()?, authority.rules.clone())]);
    for path in &plan.additional_rules_configs {
        let document: RulesConfigIdentityV1 = crate::load_canonical_document(path)?;
        validate_complete_ranked_rules_config(&document)?;
        let digest = document.canonical_digest()?;
        ensure!(
            rules_configs.insert(digest, document).is_none(),
            "duplicate rules config"
        );
    }

    let admitted_profiles =
        load_admitted_profile_managers_v1(&plan.official_content_authority, &authority)?;

    let mut campaign_states = plan
        .campaign_states
        .iter()
        .map(|state| {
            let artifact = CampaignStateArtifactV3 {
                edition: state.edition,
                kind: state.kind,
                rules_config_sha256: state.rules_config_sha256,
                artifact: state.artifact.clone(),
            };
            artifact.canonical_pin().validate()?;
            let rules_config = rules_configs
                .get(&state.rules_config_sha256)
                .context("campaign state references an absent rules config")?;
            validate_campaign_state_source(state, rules_config, &admitted_profiles)?;
            Ok(artifact)
        })
        .collect::<Result<Vec<_>>>()?;
    campaign_states.sort_by_key(CampaignStateArtifactV3::matrix_key);
    ensure!(
        campaign_state_matrix_is_exact(
            &campaign_states,
            &rules_configs.keys().copied().collect::<Vec<_>>(),
        ),
        "publication requires one exact Demo individual template and Full campaign genesis for every rules config"
    );
    let policies = load_documents(&plan.policies)?;
    let published = load_published(&plan.published_rulesets)?;
    let competitions = load_documents(&plan.competitions)?;
    let loaded = LoadedPublication {
        plan,
        authority,
        build_draft,
        build_sha256,
        viewer_build_report_artifact,
        rules_configs,
        policies,
        published,
        competitions,
        campaign_states,
    };
    validate_publication_closure(&loaded)?;
    Ok(loaded)
}

pub(super) fn validate_complete_ranked_rules_config(config: &RulesConfigIdentityV1) -> Result<()> {
    validate_complete_ranked_rules_config_v1(config)
}

pub(super) fn load_documents<T>(paths: &[PathBuf]) -> Result<BTreeMap<Digest32, T>>
where
    T: for<'de> Deserialize<'de> + Serialize + robin_run_protocol::Validate,
{
    let mut documents = BTreeMap::new();
    for path in paths {
        let document: T = crate::load_canonical_document(path)?;
        let digest = Digest32::digest_bytes(&canonical_json_bytes(&document)?);
        ensure!(
            documents.insert(digest, document).is_none(),
            "duplicate canonical document {digest}"
        );
    }
    Ok(documents)
}

pub(super) fn load_published(paths: &[PathBuf]) -> Result<BTreeMap<Digest32, PublishedRulesetV1>> {
    let mut documents = BTreeMap::new();
    for path in paths {
        let document: PublishedRulesetV1 = crate::load_canonical_document(path)?;
        ensure!(
            documents
                .insert(document.ruleset_manifest_sha256, document)
                .is_none(),
            "duplicate published ruleset identity"
        );
    }
    Ok(documents)
}

pub(super) fn validate_pinned_source(source: &PinnedArtifactSourceV3) -> Result<()> {
    validate_regular_file(&source.source)?;
    source.artifact.validate()?;
    ensure!(
        artifact_from_file(&source.source, &source.artifact.media_type)? == source.artifact,
        "pinned source differs from its artifact identity"
    );
    Ok(())
}

pub(super) fn validate_campaign_state_source(
    source: &CampaignStateSourceV3,
    rules_config: &RulesConfigIdentityV1,
    profiles: &AdmittedProfileManagersV1,
) -> Result<()> {
    validate_complete_ranked_rules_config(rules_config)?;
    ensure!(
        rules_config.canonical_digest()? == source.rules_config_sha256,
        "campaign state rules identity differs from its canonical rules document"
    );
    ensure!(
        source.artifact.media_type == RANKED_CAMPAIGN_MEDIA_TYPE_V1,
        "campaign state uses an unsupported media type"
    );
    ensure!(
        source.artifact.byte_length > 0
            && source.artifact.byte_length
                <= crate::campaign_template_v1::MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1 as u64,
        "campaign state exceeds the canonical template boundary"
    );
    validate_regular_file(&source.source)?;
    ensure!(
        artifact_from_file(&source.source, RANKED_CAMPAIGN_MEDIA_TYPE_V1)? == source.artifact,
        "campaign state source differs from its exact artifact pin"
    );
    let bytes = crate::read_regular_file_bounded(
        &source.source,
        crate::campaign_template_v1::MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1 as u64,
    )?;
    let requirement = CanonicalCampaignStateRequirementV1 {
        edition: source.edition,
        kind: match source.kind {
            CampaignStateKindV3::IndividualTemplate => {
                CanonicalCampaignStateKindV1::IndividualTemplate
            }
            CampaignStateKindV3::FullCampaignGenesis => {
                CanonicalCampaignStateKindV1::FullCampaignGenesis
            }
        },
        rules_config_sha256: source.rules_config_sha256,
    };
    let artifact = validate_canonical_campaign_template_v1(
        &bytes,
        requirement,
        rules_config,
        profiles.get(source.edition),
    )?;
    ensure!(
        artifact == source.artifact,
        "campaign state artifact differs from exact authority-derived template"
    );
    Ok(())
}
