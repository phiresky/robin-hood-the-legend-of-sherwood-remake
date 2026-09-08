//! Deterministic authoring for the private verifier-job catalog.
//!
//! The catalog is deliberately authored before a publication exists. Source
//! documents and verifier bundles therefore come from an operator staging
//! tree, while every path embedded in the resulting catalog names the exact
//! immutable production release selected by the public V2 build. Licensed raw
//! content is never read or copied by this module; templates can select only
//! the two fixed, separately installed Demo and Full roots.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use robin_run_protocol::{
    ArtifactRefV1, BoardMetricV1, CampaignContentManifestV1, CanonicalCampaignStatePinV1,
    CanonicalDocument as _, CompetitionManifestV1, ContentManifestV1, Digest32,
    ImmutablePolicyKindV1, ImmutablePolicyManifestV1, LeaderboardSubjectV1,
    OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1, OfficialContentEditionV1,
    OfficialContentSubjectV1, OpaqueId, PublishedRulesetV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
    RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1, RunContentIdentityV1,
    RunScopeKindV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, Validate as _,
    VerifierJobConfigCatalogV1, VerifierJobRouteV1, VerifierJobTemplateV1, VersionedBuildManifest,
    canonical_json_bytes, simulation_content_component_relative_path_v1,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const AUTHORING_PLAN_SCHEMA_VERSION_V1: u32 = 1;
const MAX_OPERATOR_DOCUMENT_BYTES: u64 = 128 * 1024 * 1024;
const RELEASES_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores/releases";
const DEMO_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/demo";
const FULL_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/full";

/// Source-only inputs used to author one production catalog.
///
/// `manifest_directory` has the same digest-addressed subdirectory layout as
/// `ServerConfig::manifest_directory`. `verifier_bundle_root` is the source
/// `verifier-bundles` tree emitted by official-content authoring. Campaign
/// files are listed explicitly so an omitted or unreferenced private state is
/// an authoring error instead of silently changing the server matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierCatalogAuthoringPlanV1 {
    pub schema_version: u32,
    pub source_commit: String,
    pub manifest_directory: PathBuf,
    pub verifier_bundle_root: PathBuf,
    pub campaign_state_sources: Vec<PathBuf>,
}

/// Relevant, strictly decoded subset of one final server admission profile.
///
/// This mirrors the current server type rather than accepting an ad-hoc
/// catalog-specific profile. Fields not used as route keys are still decoded
/// so misspellings inside a profile cannot disappear into ignored TOML.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogAdmissionProfileV1 {
    id: String,
    content_subject: OfficialContentSubjectV1,
    mission_display_name: String,
    allowed_scopes: Vec<String>,
    build_manifest_id: String,
    content_manifest_id: String,
    campaign_content_manifest_id: Option<String>,
    config_id: String,
    ruleset_id: String,
    template_id: String,
    canonical_campaign_state: CanonicalCampaignStatePinV1,
    canonical_campaign_state_path: PathBuf,
    allowed_metrics: Vec<String>,
    ruleset_display_name: String,
    preset_id: String,
    preset_name: String,
    difficulty_id: String,
    difficulty_name: String,
    build_display_name: String,
    viewer_engine_build: String,
    viewer_available: bool,
    viewer_unavailable_reason: Option<String>,
    viewer_content_requirement: Option<CatalogViewerContentRequirementV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CatalogViewerContentRequirementV1 {
    BundledDemo,
    UserLocalRetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogCompetitionConfigV1 {
    manifest_sha256: String,
    admission_profile_id: String,
}

/// Only top-level fields that determine catalog authority are decoded here.
/// The complete server document remains validated by `robin_highscores` at
/// API/worker startup; admission-profile and competition tables themselves
/// are strict above.
#[derive(Debug, Deserialize)]
struct CatalogServerConfigV1 {
    manifest_directory: PathBuf,
    max_campaign_bytes: u64,
    admission_profiles: Vec<CatalogAdmissionProfileV1>,
    #[serde(default)]
    competitions: Vec<CatalogCompetitionConfigV1>,
}

#[derive(Debug)]
struct LoadedAuthorityV1 {
    build_digest: Digest32,
    build: VersionedBuildManifest,
    content: BTreeMap<Digest32, ContentManifestV1>,
    campaigns: BTreeMap<Digest32, CampaignContentManifestV1>,
    rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1>,
    rulesets: BTreeMap<Digest32, PublishedRulesetV1>,
    competitions: BTreeMap<Digest32, CompetitionManifestV1>,
    policies: BTreeMap<Digest32, ImmutablePolicyManifestV1>,
}

/// Author, validate, and write a canonical catalog without replacing an
/// existing output. Relative source paths in the plan are resolved against
/// the plan's directory; all resolved inputs must be canonical non-symlink
/// paths.
pub fn author_verifier_job_config_catalog_v1(
    plan_path: &Path,
    server_config_path: &Path,
    output: &Path,
) -> Result<ArtifactRefV1> {
    ensure!(!output.exists(), "catalog output already exists");
    let (plan, server) = load_authoring_inputs(plan_path, server_config_path)?;
    let catalog = build_verifier_job_config_catalog_v1(&plan, &server)?;
    let bytes = canonical_json_bytes(&catalog)?;
    ensure!(
        bytes.len() <= robin_run_protocol::MAX_VERIFIER_JOB_CONFIG_BYTES_V1,
        "verifier job catalog exceeds its protocol byte limit"
    );
    write_new_file(output, &bytes)?;
    Ok(ArtifactRefV1 {
        sha256: Digest32::digest_bytes(&bytes),
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".into(),
    })
}

/// Validate an existing canonical catalog against the complete deterministic
/// matrix re-derived from the same source plan and final server config.
pub fn validate_verifier_job_config_catalog_v1(
    plan_path: &Path,
    server_config_path: &Path,
    catalog_path: &Path,
) -> Result<ArtifactRefV1> {
    let (plan, server) = load_authoring_inputs(plan_path, server_config_path)?;
    let expected = build_verifier_job_config_catalog_v1(&plan, &server)?;
    let (actual, bytes): (VerifierJobConfigCatalogV1, Vec<u8>) =
        load_canonical_document(catalog_path)?;
    actual.validate()?;
    ensure!(
        actual == expected,
        "catalog does not exactly equal the server profile × scope × competition matrix"
    );
    Ok(ArtifactRefV1 {
        sha256: Digest32::digest_bytes(&bytes),
        byte_length: u64::try_from(bytes.len())?,
        media_type: "application/json".into(),
    })
}

fn load_authoring_inputs(
    plan_path: &Path,
    server_config_path: &Path,
) -> Result<(VerifierCatalogAuthoringPlanV1, CatalogServerConfigV1)> {
    let plan_bytes = read_bounded_regular_file(plan_path, MAX_OPERATOR_DOCUMENT_BYTES)?;
    reject_placeholders(&plan_bytes, "catalog authoring plan")?;
    let mut plan: VerifierCatalogAuthoringPlanV1 = serde_json::from_slice(&plan_bytes)
        .with_context(|| format!("parse catalog plan {}", plan_path.display()))?;
    ensure!(
        canonical_json_bytes(&plan)? == plan_bytes,
        "catalog authoring plan is not canonical JSON"
    );
    ensure!(
        plan.schema_version == AUTHORING_PLAN_SCHEMA_VERSION_V1,
        "unsupported catalog authoring plan schema"
    );
    validate_source_commit(&plan.source_commit)?;

    let plan_parent = plan_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let plan_parent = fs::canonicalize(plan_parent)?;
    plan.manifest_directory = resolve_canonical_input(&plan_parent, &plan.manifest_directory)?;
    plan.verifier_bundle_root = resolve_canonical_input(&plan_parent, &plan.verifier_bundle_root)?;
    for source in &mut plan.campaign_state_sources {
        *source = resolve_canonical_input(&plan_parent, source)?;
    }

    let server_bytes = read_bounded_regular_file(server_config_path, MAX_OPERATOR_DOCUMENT_BYTES)?;
    reject_placeholders(&server_bytes, "server config")?;
    let server: CatalogServerConfigV1 = toml::from_str(std::str::from_utf8(&server_bytes)?)
        .with_context(|| format!("parse server config {}", server_config_path.display()))?;
    Ok((plan, server))
}

fn build_verifier_job_config_catalog_v1(
    plan: &VerifierCatalogAuthoringPlanV1,
    server: &CatalogServerConfigV1,
) -> Result<VerifierJobConfigCatalogV1> {
    validate_source_commit(&plan.source_commit)?;
    let release_root = fixed_release_root(&plan.source_commit)?;
    ensure!(
        server.manifest_directory == release_root.join("config/manifests"),
        "server manifest_directory escapes or substitutes the immutable release"
    );
    ensure!(
        server.max_campaign_bytes > 0,
        "server max_campaign_bytes must be positive"
    );

    let authority = load_authority(&plan.manifest_directory, &plan.source_commit)?;
    validate_verifier_bundles(&plan.verifier_bundle_root, &authority.content)?;
    let campaign_sources =
        load_campaign_state_sources(&plan.campaign_state_sources, server.max_campaign_bytes)?;

    ensure!(
        !server.admission_profiles.is_empty(),
        "server has no admission profiles"
    );
    let mut profile_ids = BTreeSet::new();
    let mut competition_by_profile = BTreeMap::<String, Vec<Digest32>>::new();
    let mut configured_competitions = BTreeSet::new();
    for configured in &server.competitions {
        let digest = parse_digest(&configured.manifest_sha256, "competition manifest")?;
        ensure!(
            configured_competitions.insert(digest),
            "duplicate configured competition {digest}"
        );
        ensure!(
            authority.competitions.contains_key(&digest),
            "configured competition {digest} is absent from the manifest registry"
        );
        competition_by_profile
            .entry(configured.admission_profile_id.clone())
            .or_default()
            .push(digest);
    }
    for values in competition_by_profile.values_mut() {
        values.sort();
    }

    let mut entries = BTreeMap::<Vec<u8>, VerifierJobTemplateV1>::new();
    let mut referenced_campaign_states = BTreeSet::new();
    for profile in &server.admission_profiles {
        ensure!(
            profile_ids.insert(profile.id.clone()),
            "duplicate admission profile ID {}",
            profile.id
        );
        validate_profile_strings(profile)?;

        let build_digest = parse_digest(&profile.build_manifest_id, "build manifest")?;
        let content_digest = parse_digest(&profile.content_manifest_id, "content manifest")?;
        let rules_digest = parse_digest(&profile.config_id, "rules config")?;
        let ruleset_digest = parse_digest(&profile.ruleset_id, "ruleset manifest")?;
        ensure!(
            build_digest == authority.build_digest,
            "profile {} selects a substituted build",
            profile.id
        );
        let content = authority
            .content
            .get(&content_digest)
            .with_context(|| format!("profile {} references absent content", profile.id))?;
        let rules = authority
            .rules_configs
            .get(&rules_digest)
            .with_context(|| format!("profile {} references absent rules config", profile.id))?;
        let published = authority
            .rulesets
            .get(&ruleset_digest)
            .with_context(|| format!("profile {} references absent ruleset", profile.id))?;
        let ruleset = &published.manifest;
        ensure!(
            content.subject == profile.content_subject,
            "profile {} content subject differs from its manifest",
            profile.id
        );
        validate_profile_rules_tuple(
            profile,
            content_digest,
            content,
            rules_digest,
            rules,
            ruleset_digest,
            ruleset,
            &authority,
        )?;
        validate_profile_scopes(profile, content.edition, &content.subject, ruleset)?;
        validate_campaign_state_profile(
            profile,
            &release_root,
            rules_digest,
            content.edition,
            &campaign_sources,
        )?;
        referenced_campaign_states.insert(profile.canonical_campaign_state.artifact.sha256);

        let campaign_digest = match content.edition {
            OfficialContentEditionV1::Demo => {
                ensure!(
                    profile.campaign_content_manifest_id.is_none(),
                    "Demo profile {} unexpectedly selects a campaign catalog",
                    profile.id
                );
                None
            }
            OfficialContentEditionV1::Full => {
                let digest = parse_digest(
                    profile
                        .campaign_content_manifest_id
                        .as_deref()
                        .context("Full profile omits campaign content manifest")?,
                    "campaign content manifest",
                )?;
                let campaign = authority.campaigns.get(&digest).with_context(|| {
                    format!("profile {} references absent campaign catalog", profile.id)
                })?;
                ensure!(
                    campaign.edition == content.edition
                        && campaign.content_for(&content.subject) == Some(content_digest),
                    "profile {} campaign catalog does not contain its exact content",
                    profile.id
                );
                ensure!(
                    ruleset
                        .allowed_campaign_content_manifest_sha256
                        .binary_search(&digest)
                        .is_ok(),
                    "profile {} campaign catalog is not allowed by its ruleset",
                    profile.id
                );
                Some(digest)
            }
        };

        let competitions = competition_by_profile
            .get(&profile.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for digest in competitions {
            validate_competition_for_profile(
                authority
                    .competitions
                    .get(digest)
                    .expect("configured competition membership checked"),
                profile,
                content_digest,
                campaign_digest,
                rules_digest,
                ruleset_digest,
            )?;
        }
        let competition_matrix = std::iter::once(None)
            .chain(competitions.iter().copied().map(Some))
            .collect::<Vec<_>>();
        let scopes = canonical_scope_kinds(profile, content.edition, &content.subject)?;
        for scope_kind in scopes {
            for competition_digest in &competition_matrix {
                let route = VerifierJobRouteV1 {
                    schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
                    scope_kind,
                    content_edition: content.edition,
                    content_subject: content.subject.clone(),
                    build_manifest_sha256: authority.build_digest,
                    content_manifest_sha256: content_digest,
                    campaign_content_manifest_sha256: campaign_digest,
                    rules_config_sha256: rules_digest,
                    ruleset_manifest_sha256: ruleset_digest,
                    competition_manifest_sha256: *competition_digest,
                };
                let template = VerifierJobTemplateV1 {
                    schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
                    route: route.clone(),
                    content_catalog_root: fixed_content_catalog_root(&release_root, content_digest),
                    raw_content_root: fixed_raw_root(content.edition),
                    raw_content_edition: content.edition,
                    build_manifest: authority.build.clone(),
                    content_manifest: content.clone(),
                    campaign_content_manifest: campaign_digest
                        .map(|digest| authority.campaigns[&digest].clone()),
                    rules_config: rules.clone(),
                    ruleset_manifest: ruleset.clone(),
                    canonical_campaign_state: profile.canonical_campaign_state.clone(),
                    competition_manifest: competition_digest
                        .map(|digest| authority.competitions[&digest].clone()),
                };
                template.validate()?;
                insert_canonical_route(&mut entries, &route, template).with_context(|| {
                    format!(
                        "duplicate verifier route produced by profile {}",
                        profile.id
                    )
                })?;
            }
        }
    }
    for profile in competition_by_profile.keys() {
        ensure!(
            profile_ids.contains(profile),
            "competition references unknown admission profile {profile}"
        );
    }
    ensure!(
        referenced_campaign_states == campaign_sources.keys().copied().collect(),
        "campaign-state source set does not exactly equal profile state pins"
    );

    let catalog = VerifierJobConfigCatalogV1 {
        schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
        entries: entries.into_values().collect(),
    };
    catalog.validate()?;
    Ok(catalog)
}

#[allow(clippy::too_many_arguments)]
fn validate_profile_rules_tuple(
    profile: &CatalogAdmissionProfileV1,
    content_digest: Digest32,
    content: &ContentManifestV1,
    rules_digest: Digest32,
    rules: &RulesConfigIdentityV1,
    ruleset_digest: Digest32,
    ruleset: &RulesetManifestV1,
    authority: &LoadedAuthorityV1,
) -> Result<()> {
    robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(rules)?;
    ruleset.validate_ranked_simulation_policy(rules)?;
    let build_v2 = authority.build.require_public_v2()?;
    ensure!(
        rules.replay_schema_version == build_v2.replay_schema_version,
        "profile {} rules/build replay schemas differ",
        profile.id
    );
    ensure!(
        ruleset.rules_config_sha256 == rules_digest
            && ruleset.canonical_campaign_state == profile.canonical_campaign_state.requirement
            && ruleset
                .allowed_build_manifest_sha256
                .binary_search(&authority.build_digest)
                .is_ok()
            && ruleset
                .allowed_content_manifest_sha256
                .binary_search(&content_digest)
                .is_ok()
            && content.edition == profile.canonical_campaign_state.requirement.edition,
        "profile {} differs from its exact build/content/rules/campaign tuple",
        profile.id
    );
    ensure!(
        profile.ruleset_display_name == ruleset.display_name
            && profile.preset_id == ruleset.preset_id.as_str()
            && profile.preset_name == ruleset.preset_name
            && profile.difficulty_id == ruleset.difficulty_id.as_str()
            && profile.difficulty_name == ruleset.difficulty_name,
        "profile {} labels differ from ruleset {ruleset_digest}",
        profile.id
    );
    for (kind, identity) in [
        (
            ImmutablePolicyKindV1::InputProvenance,
            &ruleset.input_provenance_policy,
        ),
        (
            ImmutablePolicyKindV1::CommandAdmission,
            &ruleset.command_admission_policy,
        ),
        (
            ImmutablePolicyKindV1::SubmissionAdmission,
            &ruleset.submission_admission_policy,
        ),
        (
            ImmutablePolicyKindV1::Verification,
            &ruleset.verifier_policy,
        ),
    ] {
        let document = authority
            .policies
            .get(&identity.manifest_sha256)
            .with_context(|| format!("profile {} policy is absent", profile.id))?;
        ensure!(
            identity.kind == kind && document.kind == kind && identity.version == document.version,
            "profile {} policy identity is substituted",
            profile.id
        );
    }
    Ok(())
}

fn validate_profile_scopes(
    profile: &CatalogAdmissionProfileV1,
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
    ruleset: &RulesetManifestV1,
) -> Result<()> {
    let expected = expected_scopes(edition, subject)?;
    ensure!(
        profile
            .allowed_scopes
            .iter()
            .map(String::as_str)
            .eq(expected),
        "profile {} scopes do not match its exact official lane",
        profile.id
    );
    let required_boards: &[RulesetBoardScopeV1] = match edition {
        OfficialContentEditionV1::Demo => &[RulesetBoardScopeV1::IndividualLevel],
        OfficialContentEditionV1::Full => &[
            RulesetBoardScopeV1::CampaignMission,
            RulesetBoardScopeV1::FullCampaign,
        ],
    };
    ensure!(
        required_boards
            .iter()
            .all(|scope| ruleset.board_scopes.binary_search(scope).is_ok()),
        "profile {} scopes are absent from its immutable ruleset",
        profile.id
    );
    Ok(())
}

fn validate_campaign_state_profile(
    profile: &CatalogAdmissionProfileV1,
    release_root: &Path,
    rules_digest: Digest32,
    edition: OfficialContentEditionV1,
    sources: &BTreeMap<Digest32, ArtifactRefV1>,
) -> Result<()> {
    profile.canonical_campaign_state.validate()?;
    ensure!(
        profile
            .canonical_campaign_state
            .requirement
            .rules_config_sha256
            == rules_digest
            && profile.canonical_campaign_state.requirement.edition == edition,
        "profile {} campaign state differs from its rules or edition",
        profile.id
    );
    let artifact = &profile.canonical_campaign_state.artifact;
    ensure!(
        sources.get(&artifact.sha256) == Some(artifact),
        "profile {} campaign state bytes are absent or substituted",
        profile.id
    );
    ensure!(
        profile.canonical_campaign_state_path
            == release_root
                .join("private/campaign-states")
                .join(artifact.sha256.to_string()),
        "profile {} campaign-state path escapes or substitutes the release",
        profile.id
    );
    Ok(())
}

fn validate_competition_for_profile(
    competition: &CompetitionManifestV1,
    profile: &CatalogAdmissionProfileV1,
    content_digest: Digest32,
    campaign_digest: Option<Digest32>,
    rules_digest: Digest32,
    ruleset_digest: Digest32,
) -> Result<()> {
    competition.validate()?;
    let expected_content = match competition.subject {
        LeaderboardSubjectV1::Mission { .. } => RunContentIdentityV1::Mission {
            content_manifest_sha256: content_digest,
        },
        LeaderboardSubjectV1::FullCampaign => RunContentIdentityV1::FullCampaign {
            campaign_content_manifest_sha256: campaign_digest
                .context("full-campaign competition profile has no campaign catalog")?,
        },
    };
    ensure!(
        competition.content == expected_content
            && competition.rules_config_sha256 == rules_digest
            && competition.ruleset_manifest_sha256 == ruleset_digest
            && competition.canonical_campaign_state == profile.canonical_campaign_state.requirement,
        "competition {} differs from profile {}",
        competition.competition_id.as_str(),
        profile.id
    );
    let metric = match competition.metric {
        BoardMetricV1::OriginalScore => "original_score",
        BoardMetricV1::FastestSuccess => "fastest_success",
    };
    ensure!(
        profile
            .allowed_metrics
            .iter()
            .any(|allowed| allowed == metric),
        "competition {} metric is disabled by profile {}",
        competition.competition_id.as_str(),
        profile.id
    );
    Ok(())
}

fn validate_profile_strings(profile: &CatalogAdmissionProfileV1) -> Result<()> {
    OpaqueId::new(profile.template_id.clone())?;
    ensure!(
        !profile.id.is_empty()
            && !profile.mission_display_name.is_empty()
            && !profile.build_display_name.is_empty()
            && !profile.viewer_engine_build.is_empty(),
        "profile contains an empty required label"
    );
    ensure!(
        !profile.allowed_metrics.is_empty()
            && profile
                .allowed_metrics
                .iter()
                .all(|metric| { matches!(metric.as_str(), "original_score" | "fastest_success") })
            && profile
                .allowed_metrics
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == profile.allowed_metrics.len(),
        "profile {} metrics are invalid or repeated",
        profile.id
    );
    ensure!(
        profile.viewer_available == profile.viewer_unavailable_reason.is_none(),
        "profile {} viewer availability is inconsistent",
        profile.id
    );
    let expected_viewer = match profile.canonical_campaign_state.requirement.edition {
        OfficialContentEditionV1::Demo => CatalogViewerContentRequirementV1::BundledDemo,
        OfficialContentEditionV1::Full => CatalogViewerContentRequirementV1::UserLocalRetail,
    };
    ensure!(
        (!profile.viewer_available && profile.viewer_content_requirement.is_none())
            || profile.viewer_content_requirement == Some(expected_viewer),
        "profile {} viewer requirement differs from its edition",
        profile.id
    );
    Ok(())
}

fn canonical_scope_kinds(
    profile: &CatalogAdmissionProfileV1,
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
) -> Result<Vec<RunScopeKindV1>> {
    validate_exact_scope_strings(&profile.allowed_scopes, edition, subject)?;
    let mut scopes = profile
        .allowed_scopes
        .iter()
        .map(|scope| match scope.as_str() {
            "individual_level" => Ok(RunScopeKindV1::IndividualLevel),
            "campaign_genesis" | "campaign_continuation" => Ok(RunScopeKindV1::Campaign),
            _ => bail!("unknown admission scope {scope}"),
        })
        .collect::<Result<Vec<_>>>()?;
    scopes.sort_by_key(|scope| match scope {
        RunScopeKindV1::IndividualLevel => 0,
        RunScopeKindV1::Campaign => 1,
    });
    scopes.dedup();
    ensure!(!scopes.is_empty(), "profile has no verifier scope");
    Ok(scopes)
}

fn validate_exact_scope_strings(
    actual: &[String],
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
) -> Result<()> {
    let expected = expected_scopes(edition, subject)?;
    ensure!(
        actual.iter().map(String::as_str).eq(expected),
        "admission scopes are repeated, reordered, omitted, or outside the official lane"
    );
    Ok(())
}

fn expected_scopes(
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
) -> Result<impl Iterator<Item = &'static str>> {
    subject.validate()?;
    let values: &'static [&'static str] = match (edition, subject) {
        (OfficialContentEditionV1::Demo, OfficialContentSubjectV1::FieldMission { .. }) => {
            &["individual_level"]
        }
        (OfficialContentEditionV1::Full, OfficialContentSubjectV1::FieldMission { mission_id })
            if mission_id == OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1 =>
        {
            &["campaign_genesis", "campaign_continuation"]
        }
        (OfficialContentEditionV1::Full, _) => &["campaign_continuation"],
        (OfficialContentEditionV1::Demo, OfficialContentSubjectV1::Headquarters { .. }) => {
            bail!("Demo headquarters is not an official verifier lane")
        }
    };
    Ok(values.iter().copied())
}

fn insert_canonical_route<T>(
    entries: &mut BTreeMap<Vec<u8>, T>,
    route: &VerifierJobRouteV1,
    value: T,
) -> Result<()> {
    route.validate()?;
    let key = canonical_json_bytes(route)?;
    ensure!(
        entries.insert(key, value).is_none(),
        "duplicate canonical verifier route"
    );
    Ok(())
}

fn load_authority(root: &Path, source_commit: &str) -> Result<LoadedAuthorityV1> {
    ensure_canonical_directory(root, "manifest directory")?;
    let builds = load_document_directory(root, "builds", |document: &VersionedBuildManifest| {
        document.validate()?;
        Ok(document.canonical_digest()?)
    })?;
    ensure!(
        builds.len() == 1,
        "registry must contain one exact public build"
    );
    let (build_digest, build) = builds.into_iter().next().expect("length checked");
    let public_v2 = build.require_public_v2()?;
    ensure!(
        public_v2.source_commit == source_commit,
        "plan source commit differs from BuildManifestV2"
    );

    let content =
        load_document_directory(root, "content-manifests", |document: &ContentManifestV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        })?;
    let campaigns = load_document_directory(
        root,
        "campaign-content-manifests",
        |document: &CampaignContentManifestV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        },
    )?;
    let rules_configs =
        load_document_directory(root, "rules-configs", |document: &RulesConfigIdentityV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        })?;
    let immutable_rulesets: BTreeMap<Digest32, RulesetManifestV1> =
        load_document_directory(root, "ruleset-manifests", |document: &RulesetManifestV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        })?;
    let rulesets = load_document_directory(
        root,
        "published-rulesets",
        |document: &PublishedRulesetV1| {
            document.validate()?;
            Ok(document.ruleset_manifest_sha256)
        },
    )?;
    ensure!(
        immutable_rulesets.len() == rulesets.len()
            && rulesets.iter().all(|(digest, published)| {
                immutable_rulesets.get(digest) == Some(&published.manifest)
            }),
        "immutable and published ruleset registries differ"
    );
    let competitions =
        load_document_directory(root, "competitions", |document: &CompetitionManifestV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        })?;
    let policies =
        load_document_directory(root, "policies", |document: &ImmutablePolicyManifestV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        })?;
    validate_campaign_catalogs(&content, &campaigns)?;
    Ok(LoadedAuthorityV1 {
        build_digest,
        build,
        content,
        campaigns,
        rules_configs,
        rulesets,
        competitions,
        policies,
    })
}

fn validate_campaign_catalogs(
    content: &BTreeMap<Digest32, ContentManifestV1>,
    campaigns: &BTreeMap<Digest32, CampaignContentManifestV1>,
) -> Result<()> {
    ensure!(!content.is_empty(), "content registry is empty");
    ensure!(
        campaigns.len() == 2,
        "registry must contain Demo and Full catalogs"
    );
    for edition in [
        OfficialContentEditionV1::Demo,
        OfficialContentEditionV1::Full,
    ] {
        let matching = campaigns
            .values()
            .filter(|campaign| campaign.edition == edition)
            .collect::<Vec<_>>();
        ensure!(
            matching.len() == 1,
            "registry must contain one exact {edition:?} campaign catalog"
        );
        let campaign = matching[0];
        let expected = content
            .iter()
            .filter(|(_, manifest)| manifest.edition == edition)
            .map(|(digest, manifest)| (manifest.subject.clone(), *digest))
            .collect::<BTreeMap<_, _>>();
        let actual = campaign
            .entries
            .iter()
            .map(|entry| (entry.subject.clone(), entry.content_manifest_sha256))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            !expected.is_empty() && actual.len() == campaign.entries.len() && actual == expected,
            "{edition:?} campaign catalog does not exactly cover its content registry"
        );
    }
    Ok(())
}

fn load_document_directory<T, F>(
    root: &Path,
    kind: &str,
    mut identity: F,
) -> Result<BTreeMap<Digest32, T>>
where
    T: DeserializeOwned + Serialize,
    F: FnMut(&T) -> Result<Digest32>,
{
    let directory = root.join(kind);
    ensure_canonical_directory(&directory, kind)?;
    let mut documents = BTreeMap::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "{kind} contains a non-regular or symlink entry"
        );
        let filename = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("{kind} filename is not UTF-8"))?;
        let stem = filename
            .strip_suffix(".json")
            .context("manifest filename does not end in .json")?;
        let expected = parse_digest(stem, "manifest filename")?;
        let (document, _): (T, Vec<u8>) = load_canonical_document(&path)?;
        let actual = identity(&document)?;
        ensure!(
            actual == expected,
            "manifest {filename} differs from its digest filename"
        );
        ensure!(
            documents.insert(actual, document).is_none(),
            "duplicate manifest digest {actual}"
        );
    }
    Ok(documents)
}

fn load_campaign_state_sources(
    sources: &[PathBuf],
    max_campaign_bytes: u64,
) -> Result<BTreeMap<Digest32, ArtifactRefV1>> {
    ensure!(!sources.is_empty(), "campaign-state source list is empty");
    let mut artifacts = BTreeMap::new();
    for source in sources {
        let bytes = read_bounded_regular_file(source, max_campaign_bytes)
            .with_context(|| format!("read campaign state {}", source.display()))?;
        let artifact = ArtifactRefV1 {
            sha256: Digest32::digest_bytes(&bytes),
            byte_length: u64::try_from(bytes.len())?,
            media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
        };
        artifact.validate()?;
        ensure!(
            artifacts.insert(artifact.sha256, artifact).is_none(),
            "duplicate campaign-state source bytes"
        );
    }
    Ok(artifacts)
}

fn validate_verifier_bundles(
    root: &Path,
    content: &BTreeMap<Digest32, ContentManifestV1>,
) -> Result<()> {
    ensure_canonical_directory(root, "verifier bundle root")?;
    let actual_children = fs::read_dir(root)?
        .map(|entry| {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "verifier bundle root contains a non-directory or symlink"
            );
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("verifier bundle name is not UTF-8"))?;
            parse_digest(&name, "verifier bundle directory")
        })
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        actual_children == content.keys().copied().collect(),
        "verifier bundle set does not exactly equal content manifests"
    );
    for (digest, manifest) in content {
        let bundle = root.join(digest.to_string());
        let (bundled_manifest, _): (ContentManifestV1, Vec<u8>) =
            load_canonical_document(&bundle.join("manifest.json"))?;
        ensure!(
            &bundled_manifest == manifest,
            "verifier bundle {digest} embeds a substituted manifest"
        );
        let mut expected_files = BTreeSet::from([PathBuf::from("manifest.json")]);
        for component in &manifest.components {
            let relative = PathBuf::from("catalog").join(
                simulation_content_component_relative_path_v1(&manifest.subject, component.kind)?,
            );
            let bytes =
                read_bounded_regular_file(&bundle.join(&relative), component.artifact.byte_length)?;
            ensure!(
                bytes.len() as u64 == component.artifact.byte_length
                    && Digest32::digest_bytes(&bytes) == component.artifact.sha256
                    && component.artifact.media_type == SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
                "verifier bundle {digest} component is substituted"
            );
            expected_files.insert(relative);
        }
        let (actual_files, actual_directories) = tree_inventory(&bundle)?;
        ensure!(
            actual_files == expected_files,
            "verifier bundle {digest} has missing or extra files"
        );
        ensure!(
            actual_directories == parent_directories(&expected_files),
            "verifier bundle {digest} has missing or extra directories"
        );
    }
    Ok(())
}

fn tree_inventory(root: &Path) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "authority tree contains a symlink"
            );
            let relative = path.strip_prefix(root)?.to_path_buf();
            ensure_safe_relative_path(&relative)?;
            if metadata.is_dir() {
                ensure!(
                    directories.insert(relative),
                    "authority tree repeats a directory"
                );
                pending.push(path);
            } else if metadata.is_file() {
                ensure!(files.insert(relative), "authority tree repeats a file");
            } else {
                bail!("authority tree contains a special filesystem node");
            }
        }
    }
    Ok((files, directories))
}

fn parent_directories(files: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    let mut directories = BTreeSet::new();
    for file in files {
        let mut parent = file.parent();
        while let Some(value) = parent.filter(|value| !value.as_os_str().is_empty()) {
            directories.insert(value.to_path_buf());
            parent = value.parent();
        }
    }
    directories
}

fn fixed_release_root(source_commit: &str) -> Result<PathBuf> {
    validate_source_commit(source_commit)?;
    Ok(Path::new(RELEASES_ROOT).join(source_commit))
}

fn fixed_content_catalog_root(release_root: &Path, content_digest: Digest32) -> PathBuf {
    release_root
        .join("private/verifier-bundles")
        .join(content_digest.to_string())
        .join("catalog")
}

fn fixed_raw_root(edition: OfficialContentEditionV1) -> PathBuf {
    PathBuf::from(match edition {
        OfficialContentEditionV1::Demo => DEMO_RAW_CONTENT_ROOT,
        OfficialContentEditionV1::Full => FULL_RAW_CONTENT_ROOT,
    })
}

fn validate_source_commit(value: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && value.bytes().any(|byte| byte != b'0'),
        "source commit must be a nonzero full lowercase 40-character Git object ID"
    );
    Ok(())
}

fn parse_digest(value: &str, field: &str) -> Result<Digest32> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{field} must be 64 lowercase hexadecimal digits"
    );
    let digest = value.parse::<Digest32>()?;
    ensure!(!digest.is_zero(), "{field} must not be zero");
    ensure!(digest.to_string() == value, "{field} is not canonical");
    Ok(digest)
}

fn resolve_canonical_input(base: &Path, configured: &Path) -> Result<PathBuf> {
    ensure!(
        !configured.as_os_str().is_empty(),
        "operator input path is empty"
    );
    let candidate = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        base.join(configured)
    };
    ensure_normalized_absolute(&candidate)?;
    let canonical = fs::canonicalize(&candidate)?;
    ensure!(
        canonical == candidate,
        "operator input path contains a symlink or noncanonical component: {}",
        candidate.display()
    );
    Ok(canonical)
}

fn ensure_canonical_directory(path: &Path, label: &str) -> Result<()> {
    ensure_normalized_absolute(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} is not a non-symlink directory"
    );
    ensure!(
        fs::canonicalize(path)? == path,
        "{label} contains a symlinked path component"
    );
    Ok(())
}

fn ensure_normalized_absolute(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "path is not absolute: {}",
        path.display()
    );
    for component in path.components() {
        ensure!(
            matches!(component, Component::RootDir | Component::Normal(_)),
            "path contains a noncanonical or escaping component: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_safe_relative_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty() && !path.is_absolute(),
        "authority relative path is empty or absolute"
    );
    for component in path.components() {
        ensure!(
            matches!(component, Component::Normal(_)),
            "authority relative path escapes its root"
        );
    }
    Ok(())
}

fn read_bounded_regular_file(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "operator input is not a regular non-symlink file: {}",
        path.display()
    );
    ensure!(
        metadata.len() > 0 && metadata.len() <= maximum,
        "operator input is empty or exceeds its byte limit: {}",
        path.display()
    );
    let mut file = fs::File::open(path)?;
    ensure!(file.metadata()?.is_file(), "operator input changed type");
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    std::io::Read::by_ref(&mut file)
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= maximum,
        "operator input changed length while reading"
    );
    Ok(bytes)
}

fn load_canonical_document<T>(path: &Path) -> Result<(T, Vec<u8>)>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = read_bounded_regular_file(path, MAX_OPERATOR_DOCUMENT_BYTES)?;
    let document: T = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse canonical document {}", path.display()))?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "document is not canonical JSON: {}",
        path.display()
    );
    Ok((document, bytes))
}

fn reject_placeholders(bytes: &[u8], label: &str) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    let lowercase = text.to_ascii_lowercase();
    ensure!(
        !lowercase.contains("placeholder")
            && !lowercase.contains("changeme")
            && !lowercase.contains("example.invalid"),
        "{label} contains a placeholder value"
    );
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure!(parent.is_dir(), "catalog output parent does not exist");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create catalog {}", path.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> Digest32 {
        Digest32::from_bytes([byte; 32])
    }

    fn route(subject: &str, competition: Option<Digest32>) -> VerifierJobRouteV1 {
        VerifierJobRouteV1 {
            schema_version: robin_run_protocol::SCHEMA_VERSION_V1,
            scope_kind: RunScopeKindV1::IndividualLevel,
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: subject.into(),
            },
            build_manifest_sha256: digest(1),
            content_manifest_sha256: Digest32::digest_bytes(subject.as_bytes()),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: digest(3),
            ruleset_manifest_sha256: digest(4),
            competition_manifest_sha256: competition,
        }
    }

    #[test]
    fn routes_are_keyed_by_canonical_bytes_and_duplicates_fail_closed() -> Result<()> {
        let mut entries = BTreeMap::new();
        let later = route("z", Some(digest(9)));
        let earlier = route("a", None);
        insert_canonical_route(&mut entries, &later, "later")?;
        insert_canonical_route(&mut entries, &earlier, "earlier")?;
        let keys = entries.keys().cloned().collect::<Vec<_>>();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(insert_canonical_route(&mut entries, &earlier, "duplicate").is_err());
        Ok(())
    }

    #[test]
    fn official_scope_strings_are_exact_and_campaign_phases_collapse() -> Result<()> {
        let genesis = OfficialContentSubjectV1::FieldMission {
            mission_id: OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1.into(),
        };
        let scopes = vec!["campaign_genesis".into(), "campaign_continuation".into()];
        validate_exact_scope_strings(&scopes, OfficialContentEditionV1::Full, &genesis)?;
        let profile = CatalogAdmissionProfileV1 {
            id: "full-genesis".into(),
            content_subject: genesis.clone(),
            mission_display_name: "Genesis".into(),
            allowed_scopes: scopes,
            build_manifest_id: digest(1).to_string(),
            content_manifest_id: digest(2).to_string(),
            campaign_content_manifest_id: Some(digest(3).to_string()),
            config_id: digest(4).to_string(),
            ruleset_id: digest(5).to_string(),
            template_id: "full-genesis".into(),
            canonical_campaign_state: serde_json::from_value(serde_json::json!({
                "requirement": {
                    "edition": "full",
                    "kind": "full_campaign_genesis",
                    "rules_config_sha256": digest(4),
                },
                "artifact": {
                    "sha256": digest(6),
                    "byte_length": 1,
                    "media_type": RANKED_CAMPAIGN_MEDIA_TYPE_V1,
                }
            }))?,
            canonical_campaign_state_path: PathBuf::from("/unused"),
            allowed_metrics: vec!["original_score".into()],
            ruleset_display_name: "rules".into(),
            preset_id: "preset".into(),
            preset_name: "Preset".into(),
            difficulty_id: "medium".into(),
            difficulty_name: "Medium".into(),
            build_display_name: "build".into(),
            viewer_engine_build: "viewer".into(),
            viewer_available: true,
            viewer_unavailable_reason: None,
            viewer_content_requirement: Some(CatalogViewerContentRequirementV1::UserLocalRetail),
        };
        assert_eq!(
            canonical_scope_kinds(&profile, OfficialContentEditionV1::Full, &genesis)?,
            [RunScopeKindV1::Campaign]
        );
        for invalid in [
            vec!["campaign_continuation".into(), "campaign_genesis".into()],
            vec!["campaign_genesis".into()],
            vec![
                "campaign_genesis".into(),
                "campaign_continuation".into(),
                "campaign_continuation".into(),
            ],
        ] {
            assert!(
                validate_exact_scope_strings(&invalid, OfficialContentEditionV1::Full, &genesis)
                    .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn production_paths_and_identities_reject_escape_and_placeholder_values() {
        let commit = "a".repeat(40);
        let release = fixed_release_root(&commit).unwrap();
        assert_eq!(
            release,
            PathBuf::from(format!(
                "/home/robinhood/.local/opt/robin-highscores/releases/{commit}"
            ))
        );
        assert_eq!(
            fixed_content_catalog_root(&release, digest(7)),
            release
                .join("private/verifier-bundles")
                .join(digest(7).to_string())
                .join("catalog")
        );
        assert_eq!(
            fixed_raw_root(OfficialContentEditionV1::Demo),
            PathBuf::from("/home/robinhood/.local/share/robin-highscores/raw-content/demo")
        );
        assert_eq!(
            fixed_raw_root(OfficialContentEditionV1::Full),
            PathBuf::from("/home/robinhood/.local/share/robin-highscores/raw-content/full")
        );
        assert!(validate_source_commit(&"0".repeat(40)).is_err());
        assert!(validate_source_commit(&"A".repeat(40)).is_err());
        assert!(parse_digest(&"0".repeat(64), "test").is_err());
        assert!(ensure_safe_relative_path(Path::new("a/../b")).is_err());
        assert!(reject_placeholders(b"token = 'CHANGEme'", "test").is_err());
    }

    #[test]
    fn campaign_sources_are_complete_unique_and_byte_bounded() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        fs::write(&first, b"first-state")?;
        fs::write(&second, b"second-state")?;
        let artifacts = load_campaign_state_sources(&[first.clone(), second], 64)?;
        assert_eq!(artifacts.len(), 2);
        assert!(load_campaign_state_sources(&[first.clone(), first], 64).is_err());
        assert!(load_campaign_state_sources(&[], 64).is_err());
        Ok(())
    }

    #[test]
    fn tree_inventory_rejects_extra_directories_and_symlinks() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path();
        fs::create_dir_all(root.join("catalog/mission"))?;
        fs::write(root.join("manifest.json"), b"manifest")?;
        fs::write(root.join("catalog/mission/component.json"), b"component")?;
        let (files, directories) = tree_inventory(root)?;
        assert_eq!(
            files,
            BTreeSet::from([
                PathBuf::from("manifest.json"),
                PathBuf::from("catalog/mission/component.json"),
            ])
        );
        assert_eq!(directories, parent_directories(&files));
        fs::create_dir(root.join("extra"))?;
        let (_, directories) = tree_inventory(root)?;
        assert_ne!(directories, parent_directories(&files));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("manifest.json", root.join("link"))?;
            assert!(tree_inventory(root).is_err());
        }
        Ok(())
    }
}
