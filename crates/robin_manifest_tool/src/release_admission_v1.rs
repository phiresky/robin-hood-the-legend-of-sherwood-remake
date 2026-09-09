//! Deterministic authoring of production ruleset and competition admission.
//!
//! Operators select only real signing authorities and explicit competition
//! schedules. Build, content, policy, ruleset, campaign-state and competition
//! tuple identities are derived from already admitted canonical authorities.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use robin_engine::engine::RankedSimulationPolicy;
use robin_run_protocol::{
    ActiveTimeDefinitionV1, AnonymousParticipantPolicyV1, BoardCategoryV1, BoardMetricV1,
    BuildManifestV2, CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
    CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1, CampaignAggregationConsentPolicyV1,
    CampaignCompletionPolicyRequirementV1, CampaignRosterContinuityV1,
    CanonicalCampaignStateKindV1, CanonicalCampaignStateRequirementV1, CanonicalDocument as _,
    CanonicalStartPolicyV1, CanonicalValue, CompetitionManifestV1,
    CompetitionParticipantCompositionV1, CompetitionSeedPolicyV1, Digest32, FrameCountingPolicyV1,
    FullCampaignChainPolicyV1, FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1,
    ImmutablePolicyKindV1, ImmutablePolicyManifestV1, InputProvenanceEligibilityV1,
    LeaderboardSubjectV1, MAX_PARTICIPANT_INSTANCES_V1, MAX_REPLAY_SEATS_V1, MetricRankingPolicyV1,
    NamedParticipantPolicyV1, OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId,
    PaginationTieBreakV1, ParticipantEligibilityV1, PublicKey32, PublishedRulesetV1,
    RankedSimulationDifficultyV1, RankedSimulationPolicyV1, RulesConfigConstraintV1,
    RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1, RulesetOperationalStatusV1,
    RulesetSeedPolicyV1, RunCompositionPolicyV1, RunContentIdentityV1, ScoreAlgorithmV1,
    ScoreOverflowPolicyV1, TerminalResultPolicyV1, TickDurationV1, Validate as _,
    VisibleTiePolicyV1, canonical_json_bytes, official_achievement_policies_v1,
    official_full_campaign_completion_policy_v1,
};
use serde::{Deserialize, Serialize};

use crate::{
    MAX_DOCUMENT_BYTES, config_parent, ensure_absent_output, load_canonical_document,
    path_to_manifest, persist_staging, read_regular_file_bounded, resolve_path, staging_directory,
    strict_json_from_slice, validate_complete_ranked_rules_config_v1,
    validate_current_official_ranked_build_v2, walk_regular_files, write_bytes,
};

const RELEASE_ADMISSION_PLAN_SCHEMA_VERSION_V1: u32 = 1;
const RELEASE_ADMISSION_INDEX_SCHEMA_VERSION_V1: u32 = 1;
const MIN_COMPETITION_UNIX_MS: u64 = 1_577_836_800_000; // 2020-01-01T00:00:00Z
const MAX_COMPETITION_DURATION_MS: u64 = 366 * 24 * 60 * 60 * 1_000;
const MAX_COMPETITIONS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAdmissionAuthoringPlanV1 {
    pub schema_version: u32,
    pub build_manifest: PathBuf,
    pub official_content_authority: PathBuf,
    pub policy_inputs_directory: PathBuf,
    pub run_preflight_grant_public_key: PublicKey32,
    pub competition_run_grant_public_key: PublicKey32,
    /// Explicit, strictly ordered competition definitions. An empty array is
    /// valid, but omission is not.
    pub competitions: Vec<CompetitionAdmissionPlanV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionAdmissionPlanV1 {
    pub competition_id: OpaqueId,
    pub competition_version: u32,
    pub display_name: String,
    pub description: String,
    pub edition: OfficialContentEditionV1,
    pub ranked_simulation_policy: RankedSimulationPolicyV1,
    pub subject: LeaderboardSubjectV1,
    pub metric: BoardMetricV1,
    pub seed_policy: CompetitionSeedPolicyV1,
    pub participant_composition: CompetitionParticipantCompositionV1,
    pub starts_at_unix_ms: u64,
    pub ends_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAdmissionRulesetIndexEntryV1 {
    pub edition: OfficialContentEditionV1,
    pub ranked_simulation_policy: RankedSimulationPolicyV1,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub published_ruleset_sha256: Digest32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAdmissionCompetitionIndexEntryV1 {
    pub competition_id: OpaqueId,
    pub competition_version: u32,
    pub competition_manifest_sha256: Digest32,
}

/// Canonical closure index for publication-plan assembly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAdmissionIndexV1 {
    pub schema_version: u32,
    pub build_manifest_sha256: Digest32,
    pub official_content_digests_sha256: Digest32,
    pub projection_authority_matrix_sha256: Digest32,
    pub run_preflight_grant_public_key: PublicKey32,
    pub competition_run_grant_public_key: PublicKey32,
    /// Six entries in the fixed Standard/Original × Easy/Medium/Hard order.
    pub rulesets: Vec<ReleaseAdmissionRulesetIndexEntryV1>,
    /// Four entries in Input/Command/Submission/Verification order.
    pub policies: Vec<ImmutablePolicyIdentityV1>,
    pub competitions: Vec<ReleaseAdmissionCompetitionIndexEntryV1>,
}

#[derive(Debug, Clone)]
pub struct AuthoredReleaseAdmissionV1 {
    pub index: ReleaseAdmissionIndexV1,
    pub index_sha256: Digest32,
}

#[derive(Debug, Clone)]
struct LoadedRulesConfig {
    name: &'static str,
    document: RulesConfigIdentityV1,
    digest: Digest32,
}

#[derive(Debug, Clone)]
struct LoadedPolicy {
    name: &'static str,
    document: ImmutablePolicyManifestV1,
    identity: ImmutablePolicyIdentityV1,
}

#[derive(Debug, Clone)]
struct LoadedPolicyInputs {
    rules_configs: Vec<LoadedRulesConfig>,
    policies: Vec<LoadedPolicy>,
}

#[derive(Debug, Clone)]
struct AdmissionAuthorityFacts {
    build_manifest_sha256: Digest32,
    official_content_digests_sha256: Digest32,
    projection_authority_matrix_sha256: Digest32,
    content: BTreeMap<(OfficialContentEditionV1, OfficialContentSubjectV1), Digest32>,
    campaign: BTreeMap<OfficialContentEditionV1, Digest32>,
}

#[derive(Debug, Clone)]
struct PreparedAdmission {
    authority: AdmissionAuthorityFacts,
    policy_inputs: LoadedPolicyInputs,
}

#[derive(Debug, Clone)]
struct AuthoredAdmissionDocuments {
    index: ReleaseAdmissionIndexV1,
    rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1>,
    policies: BTreeMap<Digest32, ImmutablePolicyManifestV1>,
    published_rulesets: BTreeMap<Digest32, PublishedRulesetV1>,
    competitions: BTreeMap<Digest32, CompetitionManifestV1>,
}

/// Author an immutable, absent output tree atomically.
pub fn author_release_admission_v1(
    plan_path: &Path,
    output_directory: &Path,
) -> Result<AuthoredReleaseAdmissionV1> {
    ensure_absent_output(output_directory)?;
    let plan = load_plan(plan_path)?;
    let prepared = prepare_admission(&plan)?;
    let documents = author_documents(&plan, &prepared)?;
    let expected = output_tree(&documents)?;
    publish_output_tree(output_directory, &expected)?;
    validate_output_tree(output_directory, &expected)?;
    let index_sha256 = documents.index.canonical_digest()?;
    Ok(AuthoredReleaseAdmissionV1 {
        index: documents.index,
        index_sha256,
    })
}

/// Re-derive every document from the original authorities and require an
/// exact, no-extra output tree.
pub fn validate_release_admission_v1(
    plan_path: &Path,
    output_directory: &Path,
) -> Result<AuthoredReleaseAdmissionV1> {
    let plan = load_plan(plan_path)?;
    let prepared = prepare_admission(&plan)?;
    let documents = author_documents(&plan, &prepared)?;
    let expected = output_tree(&documents)?;
    validate_output_tree(output_directory, &expected)?;
    Ok(AuthoredReleaseAdmissionV1 {
        index_sha256: documents.index.canonical_digest()?,
        index: documents.index,
    })
}

impl robin_run_protocol::Validate for ReleaseAdmissionIndexV1 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != RELEASE_ADMISSION_INDEX_SCHEMA_VERSION_V1 {
            return Err(robin_run_protocol::ValidationError::SchemaVersion {
                document: "ReleaseAdmissionIndexV1",
                expected: RELEASE_ADMISSION_INDEX_SCHEMA_VERSION_V1,
                actual: self.schema_version,
            });
        }
        if self.build_manifest_sha256.is_zero()
            || self.official_content_digests_sha256.is_zero()
            || self.projection_authority_matrix_sha256.is_zero()
            || !plausible_public_key(self.run_preflight_grant_public_key)
            || !plausible_public_key(self.competition_run_grant_public_key)
            || self.run_preflight_grant_public_key == self.competition_run_grant_public_key
            || !matches!(self.rulesets.len(), 12 | 14)
            || self.policies.len() != 4
            || self.competitions.len() > MAX_COMPETITIONS
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "release_admission_index.authority",
            });
        }
        let expected_ruleset_order = ruleset_order();
        for (entry, (edition, policy)) in self.rulesets.iter().zip(expected_ruleset_order) {
            entry.ranked_simulation_policy.validate()?;
            if entry.edition != edition
                || entry.ranked_simulation_policy != policy
                || entry.rules_config_sha256.is_zero()
                || entry.ruleset_manifest_sha256.is_zero()
                || entry.published_ruleset_sha256.is_zero()
            {
                return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                    field: "release_admission_index.rulesets",
                });
            }
        }
        for (identity, expected_kind) in self.policies.iter().zip(policy_kind_order()) {
            identity.validate()?;
            if identity.kind != expected_kind {
                return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                    field: "release_admission_index.policies",
                });
            }
        }
        if !self.competitions.windows(2).all(|pair| {
            (&pair[0].competition_id, pair[0].competition_version)
                < (&pair[1].competition_id, pair[1].competition_version)
        }) || self.competitions.iter().any(|entry| {
            entry.competition_version == 0 || entry.competition_manifest_sha256.is_zero()
        }) {
            return Err(robin_run_protocol::ValidationError::NotCanonicalOrder {
                field: "release_admission_index.competitions",
            });
        }
        Ok(())
    }
}

fn load_plan(path: &Path) -> Result<ReleaseAdmissionAuthoringPlanV1> {
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let mut plan: ReleaseAdmissionAuthoringPlanV1 = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse release-admission plan {}", path.display()))?;
    ensure!(
        canonical_json_bytes(&plan)? == bytes,
        "release-admission plan is not byte-for-byte canonical JSON"
    );
    validate_plan(&plan)?;
    let base = config_parent(path)?;
    resolve_path(base, &mut plan.build_manifest);
    resolve_path(base, &mut plan.official_content_authority);
    resolve_path(base, &mut plan.policy_inputs_directory);
    Ok(plan)
}

fn validate_plan(plan: &ReleaseAdmissionAuthoringPlanV1) -> Result<()> {
    ensure!(
        plan.schema_version == RELEASE_ADMISSION_PLAN_SCHEMA_VERSION_V1,
        "unsupported release-admission plan schema {}",
        plan.schema_version
    );
    ensure_real_authority_key("run-preflight", plan.run_preflight_grant_public_key)?;
    ensure_real_authority_key(
        "competition-run-grant",
        plan.competition_run_grant_public_key,
    )?;
    ensure!(
        plan.run_preflight_grant_public_key != plan.competition_run_grant_public_key,
        "run-preflight and competition grants require distinct authority keys"
    );
    ensure!(
        plan.competitions.len() <= MAX_COMPETITIONS,
        "competition plan exceeds {MAX_COMPETITIONS} entries"
    );
    for competition in &plan.competitions {
        validate_competition_plan(competition)?;
    }
    ensure!(
        plan.competitions.windows(2).all(|pair| {
            (&pair[0].competition_id, pair[0].competition_version)
                < (&pair[1].competition_id, pair[1].competition_version)
        }),
        "competition plan must be strictly ordered by competition_id/version"
    );
    let mut ids = BTreeSet::new();
    ensure!(
        plan.competitions
            .iter()
            .all(|competition| ids.insert(competition.competition_id.clone())),
        "a release admits at most one version of each competition_id"
    );
    Ok(())
}

fn validate_competition_plan(plan: &CompetitionAdmissionPlanV1) -> Result<()> {
    ensure!(plan.competition_version > 0, "competition version is zero");
    reject_placeholder_text("competition display name", &plan.display_name)?;
    reject_placeholder_text("competition description", &plan.description)?;
    reject_placeholder_text("competition id", plan.competition_id.as_str())?;
    plan.subject.validate()?;
    plan.ranked_simulation_policy.validate()?;
    ensure!(
        matches!(plan.seed_policy, CompetitionSeedPolicyV1::Pinned { .. }),
        "official competitions require a server-pinned seed"
    );
    ensure!(
        plan.starts_at_unix_ms >= MIN_COMPETITION_UNIX_MS
            && plan.ends_at_unix_ms > plan.starts_at_unix_ms
            && plan.ends_at_unix_ms - plan.starts_at_unix_ms <= MAX_COMPETITION_DURATION_MS,
        "competition schedule must use real timestamps and last at most 366 days"
    );
    let expected_category = match plan.edition {
        OfficialContentEditionV1::Demo => BoardCategoryV1::IndividualLevel,
        OfficialContentEditionV1::Full => BoardCategoryV1::Campaign,
    };
    match &plan.subject {
        LeaderboardSubjectV1::Mission { category, .. } => {
            ensure!(
                *category == expected_category,
                "competition mission category does not match its edition ruleset"
            );
        }
        LeaderboardSubjectV1::FullCampaign => {
            ensure!(
                plan.edition == OfficialContentEditionV1::Full,
                "Demo cannot offer a full-campaign competition"
            );
        }
    }
    Ok(())
}

fn prepare_admission(plan: &ReleaseAdmissionAuthoringPlanV1) -> Result<PreparedAdmission> {
    let build: BuildManifestV2 = load_canonical_document(&plan.build_manifest)?;
    validate_current_official_ranked_build_v2(&build)?;
    ensure!(
        build.replay_schema_version == CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1
            && build.network_protocol_version == CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
        "build does not use the current ranked replay/network tuple"
    );
    let authority = crate::plan_v3::validate_official_content_v3(&plan.official_content_authority)?;
    ensure!(
        authority.build == build,
        "selected build differs from the exact plan-v3 content authority build"
    );
    let build_manifest_sha256 = build.canonical_digest()?;
    ensure!(
        authority.matrix.build_manifest_sha256 == build_manifest_sha256,
        "plan-v3 matrix build binding is substituted"
    );
    let policy_inputs = load_policy_inputs(
        &plan.policy_inputs_directory,
        &authority.rules,
        &authority.execution_policy,
    )?;

    let mut content = BTreeMap::new();
    for (digest, manifest) in &authority.content {
        ensure!(
            content
                .insert((manifest.edition, manifest.subject.clone()), *digest)
                .is_none(),
            "official content repeats an edition/subject tuple"
        );
    }
    let mut campaign = BTreeMap::new();
    for (digest, manifest) in &authority.campaigns {
        ensure!(
            campaign.insert(manifest.edition, *digest).is_none(),
            "official content repeats an edition campaign catalog"
        );
    }
    ensure!(
        campaign.len() == 2,
        "official content authority must contain Demo and Full catalogs"
    );
    Ok(PreparedAdmission {
        authority: AdmissionAuthorityFacts {
            build_manifest_sha256,
            official_content_digests_sha256: authority.digests.canonical_digest()?,
            projection_authority_matrix_sha256: authority.matrix.canonical_digest()?,
            content,
            campaign,
        },
        policy_inputs,
    })
}

fn load_policy_inputs(
    root: &Path,
    projection_rules: &RulesConfigIdentityV1,
    projection_execution_policy: &robin_run_protocol::OfficialProjectionExecutionPolicyV1,
) -> Result<LoadedPolicyInputs> {
    let root_metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect policy input root {}", root.display()))?;
    ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "policy input root must be a real directory"
    );
    let expected_paths = expected_policy_input_paths();
    let actual_paths = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, _)| path_to_manifest(&relative))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        actual_paths == expected_paths,
        "policy input tree contains a missing or extra file"
    );

    let loaded_projection: robin_run_protocol::OfficialProjectionExecutionPolicyV1 =
        load_canonical_document(&root.join("projection-execution-policy.json"))?;
    ensure!(
        &loaded_projection == projection_execution_policy,
        "policy inputs projection execution policy differs from plan-v3 authority"
    );

    let mut rules_configs = Vec::with_capacity(6);
    for (name, identity) in rules_config_specs() {
        let path = root.join("rules-configs").join(format!("{name}.json"));
        let document: RulesConfigIdentityV1 = load_canonical_document(&path)?;
        validate_complete_ranked_rules_config_v1(&document)?;
        ensure!(
            document == official_rules_config(identity)?,
            "rules config {name} differs from its source-derived current policy"
        );
        if name == "standard-medium" {
            ensure!(
                &document == projection_rules,
                "Standard/Medium policy input differs from plan-v3 projection rules"
            );
        }
        rules_configs.push(LoadedRulesConfig {
            name,
            digest: document.canonical_digest()?,
            document,
        });
    }

    let mut policies = Vec::with_capacity(4);
    for (name, kind) in policy_specs() {
        let path = root.join("policies").join(format!("{name}.json"));
        let document: ImmutablePolicyManifestV1 = load_canonical_document(&path)?;
        ensure!(
            document == official_policy(kind),
            "immutable policy {name} differs from the exact current admission policy"
        );
        let identity = ImmutablePolicyIdentityV1 {
            kind,
            version: document.version,
            manifest_sha256: document.canonical_digest()?,
        };
        identity.validate()?;
        policies.push(LoadedPolicy {
            name,
            document,
            identity,
        });
    }
    let loaded = LoadedPolicyInputs {
        rules_configs,
        policies,
    };
    validate_loaded_policy_inputs(&loaded)?;
    Ok(loaded)
}

fn validate_loaded_policy_inputs(inputs: &LoadedPolicyInputs) -> Result<()> {
    let expected_rules = rules_config_specs();
    ensure!(
        inputs.rules_configs.len() == expected_rules.len(),
        "policy inputs omit or substitute an authored ranked rules config"
    );
    for (loaded, (name, identity)) in inputs.rules_configs.iter().zip(expected_rules) {
        ensure!(
            loaded.name == name
                && loaded.document.ranked_simulation_policy == identity
                && loaded.digest == loaded.document.canonical_digest()?,
            "policy inputs omit or substitute ranked rules config {name}"
        );
    }
    let expected_policies = policy_specs();
    ensure!(
        inputs.policies.len() == expected_policies.len(),
        "policy inputs omit or substitute one of the four immutable policies"
    );
    for (loaded, (name, kind)) in inputs.policies.iter().zip(expected_policies) {
        ensure!(
            loaded.name == name
                && loaded.document == official_policy(kind)
                && loaded.identity.kind == kind
                && loaded.identity.version == loaded.document.version
                && loaded.identity.manifest_sha256 == loaded.document.canonical_digest()?,
            "policy inputs omit or substitute immutable policy {name}"
        );
    }
    Ok(())
}

fn author_documents(
    plan: &ReleaseAdmissionAuthoringPlanV1,
    prepared: &PreparedAdmission,
) -> Result<AuthoredAdmissionDocuments> {
    validate_plan(plan)?;
    validate_loaded_policy_inputs(&prepared.policy_inputs)?;
    let policy_identities = prepared
        .policy_inputs
        .policies
        .iter()
        .map(|policy| policy.identity.clone())
        .collect::<Vec<_>>();
    let mut published_rulesets = BTreeMap::new();
    let mut ruleset_index = Vec::with_capacity(14);
    for (edition, simulation_policy) in ruleset_order() {
        let config = prepared
            .policy_inputs
            .rules_configs
            .iter()
            .find(|config| config.document.ranked_simulation_policy == simulation_policy)
            .context("one of the seven rules configs is absent")?;
        let manifest = official_ruleset(
            edition,
            config,
            &prepared.authority,
            &policy_identities,
            plan.run_preflight_grant_public_key,
        )?;
        manifest.validate_ranked_simulation_policy(&config.document)?;
        let ruleset_manifest_sha256 = manifest.canonical_digest()?;
        let published = PublishedRulesetV1 {
            schema_version: 1,
            ruleset_manifest_sha256,
            manifest,
            operational_status: RulesetOperationalStatusV1::Active,
        };
        published.validate()?;
        let published_ruleset_sha256 = Digest32::digest_bytes(canonical_json_bytes(&published)?);
        ensure!(
            published_rulesets
                .insert(ruleset_manifest_sha256, published)
                .is_none(),
            "two official rulesets produced one immutable digest"
        );
        ruleset_index.push(ReleaseAdmissionRulesetIndexEntryV1 {
            edition,
            ranked_simulation_policy: simulation_policy,
            rules_config_sha256: config.digest,
            ruleset_manifest_sha256,
            published_ruleset_sha256,
        });
    }
    ensure!(
        published_rulesets.len() == 14,
        "official release admission must author exactly 14 rulesets"
    );

    let mut competitions = BTreeMap::new();
    let mut competition_index = Vec::with_capacity(plan.competitions.len());
    for competition_plan in &plan.competitions {
        let ruleset_entry = ruleset_index
            .iter()
            .find(|entry| {
                entry.edition == competition_plan.edition
                    && entry.ranked_simulation_policy == competition_plan.ranked_simulation_policy
            })
            .context("competition does not select an authored official ruleset")?;
        let ruleset = published_rulesets
            .get(&ruleset_entry.ruleset_manifest_sha256)
            .expect("ruleset index is internal authority");
        let competition = official_competition(
            competition_plan,
            ruleset,
            &prepared.authority,
            plan.competition_run_grant_public_key,
        )?;
        let digest = competition.canonical_digest()?;
        ensure!(
            competitions.insert(digest, competition).is_none(),
            "two competition plans produced one manifest digest"
        );
        competition_index.push(ReleaseAdmissionCompetitionIndexEntryV1 {
            competition_id: competition_plan.competition_id.clone(),
            competition_version: competition_plan.competition_version,
            competition_manifest_sha256: digest,
        });
    }

    let index = ReleaseAdmissionIndexV1 {
        schema_version: RELEASE_ADMISSION_INDEX_SCHEMA_VERSION_V1,
        build_manifest_sha256: prepared.authority.build_manifest_sha256,
        official_content_digests_sha256: prepared.authority.official_content_digests_sha256,
        projection_authority_matrix_sha256: prepared.authority.projection_authority_matrix_sha256,
        run_preflight_grant_public_key: plan.run_preflight_grant_public_key,
        competition_run_grant_public_key: plan.competition_run_grant_public_key,
        rulesets: ruleset_index,
        policies: policy_identities,
        competitions: competition_index,
    };
    index.validate()?;
    Ok(AuthoredAdmissionDocuments {
        index,
        rules_configs: prepared
            .policy_inputs
            .rules_configs
            .iter()
            .map(|config| (config.digest, config.document.clone()))
            .collect(),
        policies: prepared
            .policy_inputs
            .policies
            .iter()
            .map(|policy| (policy.identity.manifest_sha256, policy.document.clone()))
            .collect(),
        published_rulesets,
        competitions,
    })
}

fn official_ruleset(
    edition: OfficialContentEditionV1,
    config: &LoadedRulesConfig,
    authority: &AdmissionAuthorityFacts,
    policies: &[ImmutablePolicyIdentityV1],
    run_preflight_grant_public_key: PublicKey32,
) -> Result<RulesetManifestV1> {
    ensure!(
        policies.len() == 4,
        "ruleset requires exactly four policies"
    );
    let policy = config.document.ranked_simulation_policy;
    let mut content = authority
        .content
        .iter()
        .filter_map(|((candidate_edition, _), digest)| {
            (*candidate_edition == edition).then_some(*digest)
        })
        .collect::<Vec<_>>();
    content.sort_unstable();
    ensure!(!content.is_empty(), "ruleset edition content is absent");
    let campaign_digest = *authority
        .campaign
        .get(&edition)
        .context("ruleset edition campaign catalog is absent")?;
    let (board_scopes, allowed_campaign, campaign_completion, state_kind) = match edition {
        OfficialContentEditionV1::Demo => (
            vec![RulesetBoardScopeV1::IndividualLevel],
            vec![],
            CampaignCompletionPolicyRequirementV1::NotOffered,
            CanonicalCampaignStateKindV1::IndividualTemplate,
        ),
        OfficialContentEditionV1::Full => (
            vec![
                RulesetBoardScopeV1::IndividualLevel,
                RulesetBoardScopeV1::CampaignMission,
                RulesetBoardScopeV1::FullCampaign,
            ],
            vec![campaign_digest],
            CampaignCompletionPolicyRequirementV1::Required(
                official_full_campaign_completion_policy_v1(),
            ),
            CanonicalCampaignStateKindV1::FullCampaignGenesis,
        ),
    };
    let edition_name = match edition {
        OfficialContentEditionV1::Demo => "Demo",
        OfficialContentEditionV1::Full => "Full",
    };
    let any = policy.preset == robin_run_protocol::RankedSimulationPresetV1::Custom;
    let mut manifest = RulesetManifestV1 {
        schema_version: 1,
        display_name: format!(
            "{edition_name} / {} / {}",
            policy.preset.preset_name(),
            policy.difficulty.difficulty_name()
        ),
        preset_id: OpaqueId::new(policy.preset.preset_id())?,
        preset_name: policy.preset.preset_name().into(),
        difficulty_id: OpaqueId::new(policy.difficulty.difficulty_id())?,
        difficulty_name: policy.difficulty.difficulty_name().into(),
        rules_config_sha256: config.digest,
        rules_config_constraint: RulesConfigConstraintV1::ExactCanonicalDigestOnly,
        allowed_build_manifest_sha256: vec![authority.build_manifest_sha256],
        allowed_content_manifest_sha256: content,
        allowed_campaign_content_manifest_sha256: allowed_campaign,
        board_scopes,
        campaign_completion_policy: campaign_completion,
        metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        metric_ranking: vec![
            MetricRankingPolicyV1::OriginalScoreDescending,
            MetricRankingPolicyV1::FastestSuccessAscending,
        ],
        achievement_policies: official_achievement_policies_v1(),
        canonical_start_policy:
            CanonicalStartPolicyV1::RulesConfigBoundMissionSetupAndVerifiedPredecessor,
        canonical_campaign_state: CanonicalCampaignStateRequirementV1 {
            edition,
            kind: state_kind,
            rules_config_sha256: config.digest,
        },
        run_preflight_grant_public_key,
        full_campaign_chain_policy:
            FullCampaignChainPolicyV1::CanonicalGenesisEveryFieldAndHeadquartersSessionIndependentCompletion,
        campaign_roster_continuity: CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets,
        campaign_aggregation_consent_policy:
            CampaignAggregationConsentPolicyV1::EveryAuthenticatedKeyFinalCosignsEachSession,
        participant_eligibility: ParticipantEligibilityV1 {
            allow_single_player: true,
            allow_multiplayer: true,
            named_policy:
                NamedParticipantPolicyV1::HostGenesisGuestTransportJoinAttestationAndFinalCosign,
            anonymous_policy: AnonymousParticipantPolicyV1::AllowedAuthenticatedButPubliclyRedacted,
            minimum_max_concurrent_players: 1,
            maximum_max_concurrent_players: MAX_REPLAY_SEATS_V1,
            maximum_participant_instances: MAX_PARTICIPANT_INSTANCES_V1,
        },
        replay_schema_versions: vec![CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1],
        network_protocol_versions: vec![CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1],
        input_provenance_policy: policies[0].clone(),
        command_admission_policy: policies[1].clone(),
        submission_admission_policy: policies[2].clone(),
        verifier_policy: policies[3].clone(),
        input_provenance_eligibility:
            InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
        terminal_result_policy: TerminalResultPolicyV1::IndependentlyReachedWonOnly,
        score_algorithm: ScoreAlgorithmV1::OriginalMissionAttemptWrappingSubtotalCampaignDeltaV1,
        score_overflow_policy: ScoreOverflowPolicyV1::RejectCampaignOrAggregateOverflow,
        visible_tie_policy: VisibleTiePolicyV1::EqualPrimaryMetricSharesRank,
        pagination_tie_break:
            PaginationTieBreakV1::AcceptedSequenceThenVerificationTimeThenRunIdOnly,
        tick_duration: TickDurationV1 {
            numerator_micros: 50_000,
            denominator: 1,
        },
        active_time_definition: ActiveTimeDefinitionV1::SuccessfulSimulationTicks,
        frame_counting_policy:
            FrameCountingPolicyV1::ZeroBasedEventsBeforeExclusiveReplayFrameCount,
        full_campaign_time_aggregation:
            FullCampaignTimeAggregationV1::CheckedSumEveryVerifiedFieldAndHeadquartersSession,
        run_composition_policy:
            RunCompositionPolicyV1::MissionSingleReplayFullCampaignOrderedSessionsNoSyntheticReplay,
        main_board_seed_policy: RulesetSeedPolicyV1::Open,
        competition_seed_policy: RulesetSeedPolicyV1::ServerPinned,
        allow_save_creation: true,
        allow_autosave: true,
        allow_state_load: false,
        allow_mission_restart: false,
    };
    if any {
        manifest.display_name = format!("{edition_name} / Any ruleset");
        manifest.preset_id = OpaqueId::new("any")?;
        manifest.preset_name = "Any ruleset".into();
        manifest.difficulty_id = OpaqueId::new("any")?;
        manifest.difficulty_name = "Any difficulty".into();
        manifest.rules_config_constraint = RulesConfigConstraintV1::AnyCanonicalSimConfig;
    }
    manifest.validate()?;
    Ok(manifest)
}

fn official_competition(
    plan: &CompetitionAdmissionPlanV1,
    ruleset: &PublishedRulesetV1,
    authority: &AdmissionAuthorityFacts,
    competition_run_grant_public_key: PublicKey32,
) -> Result<CompetitionManifestV1> {
    validate_competition_plan(plan)?;
    ruleset.validate()?;
    ensure!(
        ruleset.manifest.canonical_campaign_state.edition == plan.edition
            && ruleset.manifest.rules_config_sha256
                == ruleset
                    .manifest
                    .canonical_campaign_state
                    .rules_config_sha256,
        "competition selected a ruleset from another edition/config"
    );
    ensure!(
        ruleset.manifest.metrics.binary_search(&plan.metric).is_ok(),
        "competition metric is not offered by its ruleset"
    );
    let (content, required_scope) = match &plan.subject {
        LeaderboardSubjectV1::Mission {
            mission_id,
            category,
        } => {
            let digest = authority
                .content
                .get(&(
                    plan.edition,
                    OfficialContentSubjectV1::FieldMission {
                        mission_id: mission_id.clone(),
                    },
                ))
                .copied()
                .context("competition mission is absent from official content authority")?;
            let scope = match category {
                BoardCategoryV1::IndividualLevel => RulesetBoardScopeV1::IndividualLevel,
                BoardCategoryV1::Campaign => RulesetBoardScopeV1::CampaignMission,
            };
            (
                RunContentIdentityV1::Mission {
                    content_manifest_sha256: digest,
                },
                scope,
            )
        }
        LeaderboardSubjectV1::FullCampaign => (
            RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256: *authority
                    .campaign
                    .get(&OfficialContentEditionV1::Full)
                    .context("Full campaign catalog is absent")?,
            },
            RulesetBoardScopeV1::FullCampaign,
        ),
    };
    ensure!(
        ruleset
            .manifest
            .board_scopes
            .binary_search(&required_scope)
            .is_ok(),
        "competition subject scope is not offered by its ruleset"
    );
    let content_allowed = match content {
        RunContentIdentityV1::Mission {
            content_manifest_sha256,
        } => ruleset
            .manifest
            .allowed_content_manifest_sha256
            .binary_search(&content_manifest_sha256)
            .is_ok(),
        RunContentIdentityV1::FullCampaign {
            campaign_content_manifest_sha256,
        } => ruleset
            .manifest
            .allowed_campaign_content_manifest_sha256
            .binary_search(&campaign_content_manifest_sha256)
            .is_ok(),
    };
    ensure!(
        content_allowed,
        "competition content is outside its ruleset"
    );
    let players = plan.participant_composition.max_concurrent_players();
    ensure!(
        players
            >= ruleset
                .manifest
                .participant_eligibility
                .minimum_max_concurrent_players
            && players
                <= ruleset
                    .manifest
                    .participant_eligibility
                    .maximum_max_concurrent_players,
        "competition participant composition is outside its ruleset"
    );
    let competition = CompetitionManifestV1 {
        schema_version: 1,
        competition_id: plan.competition_id.clone(),
        competition_version: plan.competition_version,
        display_name: plan.display_name.clone(),
        description: plan.description.clone(),
        subject: plan.subject.clone(),
        metric: plan.metric,
        rules_config_sha256: ruleset.manifest.rules_config_sha256,
        ruleset_manifest_sha256: ruleset.ruleset_manifest_sha256,
        canonical_campaign_state: ruleset.manifest.canonical_campaign_state,
        content,
        seed_policy: plan.seed_policy,
        participant_composition: plan.participant_composition,
        competition_run_grant_public_key,
        starts_at_unix_ms: plan.starts_at_unix_ms,
        ends_at_unix_ms: plan.ends_at_unix_ms,
    };
    competition.validate()?;
    Ok(competition)
}

fn output_tree(documents: &AuthoredAdmissionDocuments) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    insert_output(
        &mut files,
        "release-admission-index-v1.json".into(),
        &documents.index,
    )?;
    for (digest, document) in &documents.rules_configs {
        insert_output(&mut files, format!("rules-configs/{digest}.json"), document)?;
    }
    for (digest, document) in &documents.policies {
        insert_output(&mut files, format!("policies/{digest}.json"), document)?;
    }
    for (digest, document) in &documents.published_rulesets {
        insert_output(
            &mut files,
            format!("published-rulesets/{digest}.json"),
            document,
        )?;
    }
    for (digest, document) in &documents.competitions {
        insert_output(&mut files, format!("competitions/{digest}.json"), document)?;
    }
    Ok(files)
}

fn insert_output<T: Serialize + robin_run_protocol::Validate>(
    files: &mut BTreeMap<String, Vec<u8>>,
    path: String,
    document: &T,
) -> Result<()> {
    document.validate()?;
    ensure!(
        files
            .insert(path.clone(), canonical_json_bytes(document)?)
            .is_none(),
        "duplicate output path {path}"
    );
    Ok(())
}

fn publish_output_tree(output: &Path, expected: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    ensure_absent_output(output)?;
    let staging = staging_directory(output)?;
    for (relative, bytes) in expected {
        write_bytes(&staging.path().join(relative), bytes)?;
    }
    validate_output_tree(staging.path(), expected)?;
    persist_staging(staging, output)
}

fn validate_output_tree(root: &Path, expected: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect release-admission output {}", root.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "release-admission output must be a real directory"
    );
    let actual = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, absolute)| {
            let relative = path_to_manifest(&relative)?;
            let bytes = read_regular_file_bounded(&absolute, MAX_DOCUMENT_BYTES)?;
            Ok((relative, bytes))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    ensure!(actual == *expected, "release-admission output tree differs");
    Ok(())
}

fn expected_policy_input_paths() -> Vec<String> {
    let mut paths = vec!["projection-execution-policy.json".into()];
    paths.extend(policy_specs().map(|(name, _)| format!("policies/{name}.json")));
    paths.extend(rules_config_specs().map(|(name, _)| format!("rules-configs/{name}.json")));
    paths.sort();
    paths
}

fn rules_config_specs() -> [(&'static str, RankedSimulationPolicyV1); 7] {
    [
        (
            "standard-easy",
            RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Easy),
        ),
        (
            "standard-medium",
            RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
        ),
        (
            "standard-hard",
            RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Hard),
        ),
        (
            "original-parity-easy",
            RankedSimulationPolicyV1::original_parity(RankedSimulationDifficultyV1::Easy),
        ),
        (
            "original-parity-medium",
            RankedSimulationPolicyV1::original_parity(RankedSimulationDifficultyV1::Medium),
        ),
        (
            "original-parity-hard",
            RankedSimulationPolicyV1::original_parity(RankedSimulationDifficultyV1::Hard),
        ),
        (
            "any",
            RankedSimulationPolicyV1 {
                version: 1,
                preset: robin_run_protocol::RankedSimulationPresetV1::Custom,
                difficulty: RankedSimulationDifficultyV1::Medium,
            },
        ),
    ]
}

fn ruleset_order() -> [(OfficialContentEditionV1, RankedSimulationPolicyV1); 14] {
    let specs = rules_config_specs();
    std::array::from_fn(|index| {
        let edition = if index < 7 {
            OfficialContentEditionV1::Demo
        } else {
            OfficialContentEditionV1::Full
        };
        (edition, specs[index % 7].1)
    })
}

fn policy_specs() -> [(&'static str, ImmutablePolicyKindV1); 4] {
    [
        ("input-provenance", ImmutablePolicyKindV1::InputProvenance),
        ("command-admission", ImmutablePolicyKindV1::CommandAdmission),
        (
            "submission-admission",
            ImmutablePolicyKindV1::SubmissionAdmission,
        ),
        ("verification", ImmutablePolicyKindV1::Verification),
    ]
}

fn policy_kind_order() -> [ImmutablePolicyKindV1; 4] {
    policy_specs().map(|(_, kind)| kind)
}

fn official_rules_config(identity: RankedSimulationPolicyV1) -> Result<RulesConfigIdentityV1> {
    let policy = if identity.preset == robin_run_protocol::RankedSimulationPresetV1::Custom {
        RankedSimulationPolicy::from_config(
            identity,
            RankedSimulationPolicy::standard_medium().expected_config(),
        )?
    } else {
        RankedSimulationPolicy::from_identity(identity)?
    };
    let sim_config = match CanonicalValue::from_serializable(&policy.expected_config())? {
        CanonicalValue::Object(values) => values,
        _ => bail!("SimConfig did not serialize as an object"),
    };
    let document = RulesConfigIdentityV1 {
        schema_version: 1,
        replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        ranked_simulation_policy: identity,
        sim_config,
        rules: BTreeMap::from([
            (
                "canonical_replay_encoding".into(),
                CanonicalValue::String("compact_bitcode".into()),
            ),
            ("ranked".into(), CanonicalValue::Bool(true)),
            (
                "requires_complete_replay".into(),
                CanonicalValue::Bool(true),
            ),
            (
                "requires_server_resimulation".into(),
                CanonicalValue::Bool(true),
            ),
            (
                "terminal_result".into(),
                CanonicalValue::String("won".into()),
            ),
        ]),
    };
    document.validate()?;
    validate_complete_ranked_rules_config_v1(&document)?;
    Ok(document)
}

fn official_policy(kind: ImmutablePolicyKindV1) -> ImmutablePolicyManifestV1 {
    let string = |value: &str| CanonicalValue::String(value.into());
    let rules = match kind {
        ImmutablePolicyKindV1::InputProvenance => BTreeMap::from([
            ("accepted_encoding".into(), string("compact_bitcode")),
            (
                "canonical_replay_schema_only".into(),
                CanonicalValue::Bool(true),
            ),
            ("console_input_allowed".into(), CanonicalValue::Bool(false)),
            (
                "http_step_input_allowed".into(),
                CanonicalValue::Bool(false),
            ),
            ("playback_input_allowed".into(), CanonicalValue::Bool(false)),
        ]),
        ImmutablePolicyKindV1::CommandAdmission => BTreeMap::from([
            (
                "developer_commands_allowed".into(),
                CanonicalValue::Bool(false),
            ),
            (
                "mission_restart_allowed".into(),
                CanonicalValue::Bool(false),
            ),
            ("save_creation_allowed".into(), CanonicalValue::Bool(false)),
            ("state_load_allowed".into(), CanonicalValue::Bool(false)),
        ]),
        ImmutablePolicyKindV1::SubmissionAdmission => BTreeMap::from([
            (
                "complete_replay_required".into(),
                CanonicalValue::Bool(true),
            ),
            (
                "multiplayer_participant_cosignatures_required".into(),
                CanonicalValue::Bool(true),
            ),
            ("terminal_result".into(), string("won")),
        ]),
        ImmutablePolicyKindV1::Verification => BTreeMap::from([
            (
                "exact_build_content_rules_campaign_required".into(),
                CanonicalValue::Bool(true),
            ),
            (
                "isolated_server_resimulation_required".into(),
                CanonicalValue::Bool(true),
            ),
            (
                "metrics_recomputed_server_side".into(),
                CanonicalValue::Bool(true),
            ),
        ]),
    };
    ImmutablePolicyManifestV1 {
        schema_version: 1,
        kind,
        version: 1,
        rules,
    }
}

fn ensure_real_authority_key(label: &str, key: PublicKey32) -> Result<()> {
    ensure!(
        plausible_public_key(key),
        "{label} public key is zero or an obvious repeated-byte test key"
    );
    Ok(())
}

fn plausible_public_key(key: PublicKey32) -> bool {
    !key.is_zero() && key.as_bytes().windows(2).any(|pair| pair[0] != pair[1])
}

fn reject_placeholder_text(label: &str, value: &str) -> Result<()> {
    let lower = value.to_ascii_lowercase();
    ensure!(!value.trim().is_empty(), "{label} is empty");
    ensure!(
        ![
            "placeholder",
            "changeme",
            "todo",
            "example.invalid",
            "<",
            ">"
        ]
        .iter()
        .any(|needle| lower.contains(needle)),
        "{label} contains a placeholder marker"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{SimulationSeed64, official_content_subjects_v1};

    fn key(offset: u8) -> PublicKey32 {
        PublicKey32::from_bytes(std::array::from_fn(|index| {
            offset.wrapping_add(index as u8)
        }))
    }

    fn test_authority() -> AdmissionAuthorityFacts {
        let mut content = BTreeMap::new();
        for edition in [
            OfficialContentEditionV1::Demo,
            OfficialContentEditionV1::Full,
        ] {
            for subject in official_content_subjects_v1(edition) {
                let label = format!("{edition:?}/{subject:?}");
                content.insert((edition, subject), Digest32::digest_bytes(label));
            }
        }
        AdmissionAuthorityFacts {
            build_manifest_sha256: Digest32::digest_bytes(b"build"),
            official_content_digests_sha256: Digest32::digest_bytes(b"content-digests"),
            projection_authority_matrix_sha256: Digest32::digest_bytes(b"projection-matrix"),
            content,
            campaign: BTreeMap::from([
                (
                    OfficialContentEditionV1::Demo,
                    Digest32::digest_bytes(b"demo-campaign"),
                ),
                (
                    OfficialContentEditionV1::Full,
                    Digest32::digest_bytes(b"full-campaign"),
                ),
            ]),
        }
    }

    fn test_policy_inputs() -> LoadedPolicyInputs {
        LoadedPolicyInputs {
            rules_configs: rules_config_specs()
                .into_iter()
                .map(|(name, identity)| {
                    let document = official_rules_config(identity).unwrap();
                    LoadedRulesConfig {
                        name,
                        digest: document.canonical_digest().unwrap(),
                        document,
                    }
                })
                .collect(),
            policies: policy_specs()
                .into_iter()
                .map(|(name, kind)| {
                    let document = official_policy(kind);
                    let identity = ImmutablePolicyIdentityV1 {
                        kind,
                        version: document.version,
                        manifest_sha256: document.canonical_digest().unwrap(),
                    };
                    LoadedPolicy {
                        name,
                        document,
                        identity,
                    }
                })
                .collect(),
        }
    }

    fn mission_plan() -> CompetitionAdmissionPlanV1 {
        let mission_id = official_content_subjects_v1(OfficialContentEditionV1::Demo)
            .into_iter()
            .find_map(|subject| match subject {
                OfficialContentSubjectV1::FieldMission { mission_id } => Some(mission_id),
                OfficialContentSubjectV1::Headquarters { .. } => None,
            })
            .unwrap();
        CompetitionAdmissionPlanV1 {
            competition_id: OpaqueId::new("demo-speed-week-2026").unwrap(),
            competition_version: 1,
            display_name: "Demo Speed Week 2026".into(),
            description: "Fastest verified completion of the Demo mission.".into(),
            edition: OfficialContentEditionV1::Demo,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(
                RankedSimulationDifficultyV1::Medium,
            ),
            subject: LeaderboardSubjectV1::Mission {
                mission_id,
                category: BoardCategoryV1::IndividualLevel,
            },
            metric: BoardMetricV1::FastestSuccess,
            seed_policy: CompetitionSeedPolicyV1::Pinned {
                simulation_seed: SimulationSeed64::new(42),
            },
            participant_composition: CompetitionParticipantCompositionV1::SinglePlayer,
            starts_at_unix_ms: 1_800_000_000_000,
            ends_at_unix_ms: 1_800_604_800_000,
        }
    }

    fn test_plan() -> ReleaseAdmissionAuthoringPlanV1 {
        ReleaseAdmissionAuthoringPlanV1 {
            schema_version: 1,
            build_manifest: "build.json".into(),
            official_content_authority: "content".into(),
            policy_inputs_directory: "policies".into(),
            run_preflight_grant_public_key: key(11),
            competition_run_grant_public_key: key(91),
            competitions: vec![mission_plan()],
        }
    }

    fn prepared() -> PreparedAdmission {
        PreparedAdmission {
            authority: test_authority(),
            policy_inputs: test_policy_inputs(),
        }
    }

    #[test]
    fn authors_preset_and_open_rulesets_for_both_editions() {
        let authored = author_documents(&test_plan(), &prepared()).unwrap();
        assert_eq!(authored.published_rulesets.len(), 14);
        assert_eq!(authored.index.rulesets.len(), 14);
        for published in authored.published_rulesets.values() {
            published.validate().unwrap();
            assert_eq!(
                published.manifest.run_preflight_grant_public_key,
                test_plan().run_preflight_grant_public_key
            );
            match published.manifest.canonical_campaign_state.edition {
                OfficialContentEditionV1::Demo => {
                    assert_eq!(
                        published.manifest.board_scopes,
                        [RulesetBoardScopeV1::IndividualLevel]
                    );
                    assert_eq!(
                        published.manifest.canonical_campaign_state.kind,
                        CanonicalCampaignStateKindV1::IndividualTemplate
                    );
                }
                OfficialContentEditionV1::Full => {
                    assert_eq!(
                        published.manifest.board_scopes,
                        [
                            RulesetBoardScopeV1::IndividualLevel,
                            RulesetBoardScopeV1::CampaignMission,
                            RulesetBoardScopeV1::FullCampaign,
                        ]
                    );
                    assert_eq!(
                        published.manifest.canonical_campaign_state.kind,
                        CanonicalCampaignStateKindV1::FullCampaignGenesis
                    );
                }
            }
        }
    }

    #[test]
    fn competition_tuple_and_both_keys_are_derived_and_bound() {
        let plan = test_plan();
        let authored = author_documents(&plan, &prepared()).unwrap();
        let competition = authored.competitions.values().next().unwrap();
        assert_eq!(
            competition.competition_run_grant_public_key,
            plan.competition_run_grant_public_key
        );
        let ruleset = &authored.published_rulesets[&competition.ruleset_manifest_sha256];
        assert_eq!(
            ruleset.manifest.run_preflight_grant_public_key,
            plan.run_preflight_grant_public_key
        );
        assert_eq!(
            competition.rules_config_sha256,
            ruleset.manifest.rules_config_sha256
        );
        assert_eq!(
            competition.canonical_campaign_state,
            ruleset.manifest.canonical_campaign_state
        );
        assert_eq!(
            competition.content,
            RunContentIdentityV1::Mission {
                content_manifest_sha256: prepared().authority.content[&(
                    OfficialContentEditionV1::Demo,
                    OfficialContentSubjectV1::FieldMission {
                        mission_id: match &competition.subject {
                            LeaderboardSubjectV1::Mission { mission_id, .. } => mission_id.clone(),
                            LeaderboardSubjectV1::FullCampaign => unreachable!(),
                        },
                    },
                )],
            }
        );
    }

    #[test]
    fn omission_and_substitution_fail_closed() {
        let mut missing = test_policy_inputs();
        missing.rules_configs.pop();
        assert!(validate_loaded_policy_inputs(&missing).is_err());

        let mut substituted = test_policy_inputs();
        substituted.policies[0]
            .document
            .rules
            .insert("console_input_allowed".into(), CanonicalValue::Bool(true));
        assert!(validate_loaded_policy_inputs(&substituted).is_err());

        let mut wrong_key = test_plan();
        wrong_key.run_preflight_grant_public_key = PublicKey32::from_bytes([7; 32]);
        assert!(validate_plan(&wrong_key).is_err());

        let mut wrong_subject = test_plan();
        let LeaderboardSubjectV1::Mission { category, .. } =
            &mut wrong_subject.competitions[0].subject
        else {
            unreachable!()
        };
        *category = BoardCategoryV1::Campaign;
        assert!(validate_plan(&wrong_subject).is_err());
    }

    #[test]
    fn output_is_deterministic_and_exact() {
        let plan = test_plan();
        let first = output_tree(&author_documents(&plan, &prepared()).unwrap()).unwrap();
        let second = output_tree(&author_documents(&plan, &prepared()).unwrap()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first
                .keys()
                .filter(|path| path.starts_with("published-rulesets/"))
                .count(),
            14
        );
        assert_eq!(
            first
                .keys()
                .filter(|path| path.starts_with("rules-configs/"))
                .count(),
            7
        );
        assert_eq!(
            first
                .keys()
                .filter(|path| path.starts_with("policies/"))
                .count(),
            4
        );
        assert_eq!(
            first
                .keys()
                .filter(|path| path.starts_with("competitions/"))
                .count(),
            1
        );
    }

    #[test]
    fn plan_requires_explicit_canonical_competition_order_and_real_dates() {
        let mut plan = test_plan();
        let mut second = mission_plan();
        second.competition_id = OpaqueId::new("another-competition").unwrap();
        plan.competitions.push(second);
        assert!(validate_plan(&plan).is_err());
        plan.competitions.sort_by(|left, right| {
            (&left.competition_id, left.competition_version)
                .cmp(&(&right.competition_id, right.competition_version))
        });
        assert!(validate_plan(&plan).is_ok());
        plan.competitions[0].starts_at_unix_ms = 1;
        assert!(validate_plan(&plan).is_err());
    }
}
