//! Authoring and admission of operator-private canonical campaign templates.
//!
//! A ranked campaign template is not an opaque operator-provided blob. Its
//! bytes are the canonical `bitcode` encoding of the exact fresh
//! [`Campaign`] constructed from an edition's already-admitted profiles and
//! the difficulty carried by a complete, content-addressed rules
//! configuration. Production authoring accepts only a complete, validated
//! Plan-V3 official-content authority and recovers the exact typed profile set
//! admitted by both its loose and shipping projection lanes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use robin_engine::campaign::Campaign;
use robin_engine::engine::SimConfig;
use robin_engine::profiles::ProfileManager;
use robin_run_protocol::{
    ArtifactRefV1, CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1, CanonicalCampaignStateKindV1,
    CanonicalCampaignStateRequirementV1, CanonicalDocument as _, CanonicalValue, Digest32,
    OfficialContentEditionV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1, RulesConfigIdentityV1,
    SimulationContentComponentDocumentV1, SimulationContentComponentKindV1, Validate as _,
    canonical_json_bytes, simulation_content_component_relative_path_v1,
};
use serde::{Deserialize, Serialize};

use crate::plan_v3::{ValidatedOfficialContentV3, validate_official_content_v3};

/// Hard ceiling shared with the isolated verifier's campaign input boundary.
///
/// Authoring normally produces a much smaller object. Keeping the same
/// absolute ceiling here ensures an operator cannot ask the bitcode decoder
/// to allocate from an unbounded file merely because it is being admitted at
/// release time rather than run-verification time.
pub const MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1: usize = 64 * 1024 * 1024;
const MAX_PROFILES_COMPONENT_BYTES_V1: u64 = 128 * 1024 * 1024;
const CAMPAIGN_TEMPLATE_MATRIX_PLAN_SCHEMA_VERSION_V1: u32 = 1;
const CAMPAIGN_TEMPLATE_MATRIX_SCHEMA_VERSION_V1: u32 = 1;
const MAX_RULES_CONFIGS_PER_MATRIX_V1: usize = 64;
const CAMPAIGN_TEMPLATE_MATRIX_MANIFEST_V1: &str = "campaign-template-matrix-v1.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignTemplateMatrixPlanV1 {
    pub schema_version: u32,
    pub official_content_authority: PathBuf,
    pub rules_configs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignTemplateMatrixV1 {
    pub schema_version: u32,
    pub official_content_digests_sha256: Digest32,
    pub projection_authority_matrix_sha256: Digest32,
    pub entries: Vec<CampaignTemplateMatrixEntryV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignTemplateMatrixEntryV1 {
    pub rules_config_sha256: Digest32,
    pub requirement: CanonicalCampaignStateRequirementV1,
    pub path: String,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug)]
pub struct AdmittedProfileManagersV1 {
    pub demo: ProfileManager,
    pub full: ProfileManager,
}

impl AdmittedProfileManagersV1 {
    pub fn get(&self, edition: OfficialContentEditionV1) -> &ProfileManager {
        match edition {
            OfficialContentEditionV1::Demo => &self.demo,
            OfficialContentEditionV1::Full => &self.full,
        }
    }
}

/// Atomically author every Demo/Full template after validating the complete
/// Plan-V3 content authority exactly once.
pub fn author_campaign_template_matrix_v1(plan_path: &Path, output: &Path) -> Result<Digest32> {
    crate::ensure_absent_output(output)?;
    validate_normalized_output(output)?;
    let plan = load_matrix_plan_v1(plan_path)?;
    let authority = validate_official_content_v3(&plan.official_content_authority)?;
    let profiles = load_admitted_profile_managers_v1(&plan.official_content_authority, &authority)?;
    let rules = load_matrix_rules_v1(&plan.rules_configs)?;
    let matrix = expected_matrix_v1(&authority, &profiles, &rules)?;

    let staging = crate::staging_directory(output)?;
    for entry in &matrix.entries {
        let rules = &rules[&entry.rules_config_sha256];
        let (bytes, artifact) = author_canonical_campaign_template_v1(
            entry.requirement,
            rules,
            profiles.get(entry.requirement.edition),
        )?;
        ensure!(
            artifact == entry.artifact,
            "matrix artifact changed during authoring"
        );
        crate::write_bytes(&staging.path().join(&entry.path), &bytes)?;
    }
    crate::write_bytes(
        &staging.path().join(CAMPAIGN_TEMPLATE_MATRIX_MANIFEST_V1),
        &canonical_json_bytes(&matrix)?,
    )?;
    validate_matrix_tree_with_authority_v1(staging.path(), &matrix, &profiles, &rules)?;
    crate::persist_staging(staging, output)?;
    Ok(Digest32::digest_bytes(&canonical_json_bytes(&matrix)?))
}

/// Revalidate one immutable matrix while hashing the complete Plan-V3
/// authority once, never once per template.
pub fn validate_campaign_template_matrix_v1(plan_path: &Path, root: &Path) -> Result<Digest32> {
    let plan = load_matrix_plan_v1(plan_path)?;
    let authority = validate_official_content_v3(&plan.official_content_authority)?;
    let profiles = load_admitted_profile_managers_v1(&plan.official_content_authority, &authority)?;
    let rules = load_matrix_rules_v1(&plan.rules_configs)?;
    let expected = expected_matrix_v1(&authority, &profiles, &rules)?;
    validate_matrix_tree_with_authority_v1(root, &expected, &profiles, &rules)?;
    Ok(Digest32::digest_bytes(&canonical_json_bytes(&expected)?))
}

fn campaign_requirement_v1(
    edition: OfficialContentEditionV1,
    rules_config: &RulesConfigIdentityV1,
) -> Result<CanonicalCampaignStateRequirementV1> {
    Ok(CanonicalCampaignStateRequirementV1 {
        edition,
        kind: match edition {
            OfficialContentEditionV1::Demo => CanonicalCampaignStateKindV1::IndividualTemplate,
            OfficialContentEditionV1::Full => CanonicalCampaignStateKindV1::FullCampaignGenesis,
        },
        rules_config_sha256: rules_config.canonical_digest()?,
    })
}

fn load_rules_config_v1(path: &Path) -> Result<RulesConfigIdentityV1> {
    let bytes = crate::read_regular_file_bounded(path, crate::MAX_DOCUMENT_BYTES)?;
    let rules: RulesConfigIdentityV1 = crate::strict_json_from_slice(&bytes)
        .context("decode campaign-template rules configuration")?;
    rules
        .validate()
        .context("validate campaign-template rules configuration")?;
    ensure!(
        canonical_json_bytes(&rules)? == bytes,
        "campaign-template rules configuration is not byte-for-byte canonical JSON"
    );
    Ok(rules)
}

/// Extract both exact typed profile catalogs from a fully validated Plan-V3
/// authority. Every subject in an edition must name and contain the same
/// Profiles artifact. Shipping provenance remains transitively bound by the
/// authority's mandatory loose/shipping equivalence matrix and source
/// bindings; no standalone component path is accepted.
pub fn load_admitted_profile_managers_v1(
    authority_root: &Path,
    authority: &ValidatedOfficialContentV3,
) -> Result<AdmittedProfileManagersV1> {
    load_admitted_profile_managers_from_catalog_v1(
        &authority_root.join("verifier-bundles"),
        &authority.content,
        &authority.digests.demo_content_manifest_sha256,
        &authority.digests.full_content_manifest_sha256,
    )
}

/// Internal selector over the catalog already owned by a typed, fully
/// validated authority. It is deliberately not public: accepting a standalone
/// manifest map and bundle root would weaken the provenance boundary.
fn load_admitted_profile_managers_from_catalog_v1(
    bundle_root: &Path,
    content: &BTreeMap<Digest32, robin_run_protocol::ContentManifestV1>,
    demo_content_digests: &[Digest32],
    full_content_digests: &[Digest32],
) -> Result<AdmittedProfileManagersV1> {
    Ok(AdmittedProfileManagersV1 {
        demo: load_edition_profiles_from_catalog_v1(
            bundle_root,
            content,
            demo_content_digests,
            OfficialContentEditionV1::Demo,
        )?,
        full: load_edition_profiles_from_catalog_v1(
            bundle_root,
            content,
            full_content_digests,
            OfficialContentEditionV1::Full,
        )?,
    })
}

fn load_edition_profiles_from_catalog_v1(
    bundle_root: &Path,
    content: &BTreeMap<Digest32, robin_run_protocol::ContentManifestV1>,
    expected_digests: &[Digest32],
    edition: OfficialContentEditionV1,
) -> Result<ProfileManager> {
    ensure!(
        !expected_digests.is_empty(),
        "official {edition:?} authority has no content subjects"
    );

    let mut admitted_artifact = None;
    let mut admitted_document: Option<SimulationContentComponentDocumentV1> = None;
    for content_digest in expected_digests {
        let manifest = content
            .get(content_digest)
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
                "official {edition:?} subjects disagree about their Profiles artifact"
            );
        } else {
            admitted_artifact = Some(component.artifact.clone());
        }

        let relative = simulation_content_component_relative_path_v1(
            &manifest.subject,
            SimulationContentComponentKindV1::Profiles,
        )?;
        let path = bundle_root
            .join(content_digest.to_string())
            .join("catalog")
            .join(relative);
        let bytes = crate::read_regular_file_bounded(&path, MAX_PROFILES_COMPONENT_BYTES_V1)?;
        ensure!(
            crate::artifact_from_bytes(
                &bytes,
                robin_run_protocol::SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
            ) == component.artifact,
            "authenticated Profiles component differs from its content manifest"
        );
        let document: SimulationContentComponentDocumentV1 = crate::strict_json_from_slice(&bytes)
            .context("decode authenticated Profiles component")?;
        document
            .validate()
            .context("validate authenticated Profiles component")?;
        ensure!(
            document.kind == SimulationContentComponentKindV1::Profiles
                && document.canonical_bytes()? == bytes,
            "authenticated Profiles component is not canonical typed Profiles"
        );
        if let Some(expected) = &admitted_document {
            ensure!(
                expected == &document,
                "official {edition:?} Profiles component bytes are not identical"
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

fn load_matrix_plan_v1(path: &Path) -> Result<CampaignTemplateMatrixPlanV1> {
    let bytes = crate::read_regular_file_bounded(path, crate::MAX_DOCUMENT_BYTES)?;
    let mut plan: CampaignTemplateMatrixPlanV1 = crate::strict_json_from_slice(&bytes)
        .with_context(|| format!("parse campaign-template matrix plan {}", path.display()))?;
    ensure!(
        canonical_json_bytes(&plan)? == bytes,
        "campaign-template matrix plan is not canonical JSON"
    );
    ensure!(
        plan.schema_version == CAMPAIGN_TEMPLATE_MATRIX_PLAN_SCHEMA_VERSION_V1,
        "unsupported campaign-template matrix plan schema"
    );
    ensure!(
        !plan.rules_configs.is_empty()
            && plan.rules_configs.len() <= MAX_RULES_CONFIGS_PER_MATRIX_V1,
        "campaign-template matrix requires 1..={MAX_RULES_CONFIGS_PER_MATRIX_V1} rules configs"
    );
    ensure!(
        plan.rules_configs.windows(2).all(|pair| pair[0] < pair[1]),
        "campaign-template matrix rules paths must be sorted and unique"
    );

    let base = fs::canonicalize(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .context("campaign-template matrix plan has no parent")?,
    )?;
    crate::resolve_path(&base, &mut plan.official_content_authority);
    for rules in &mut plan.rules_configs {
        crate::resolve_path(&base, rules);
    }
    crate::validate_mount_root(&plan.official_content_authority)?;
    for rules in &plan.rules_configs {
        validate_regular_file(rules, "rules configuration")?;
        ensure!(
            rules.is_absolute() && fs::canonicalize(rules)? == *rules,
            "rules configuration path is not normalized"
        );
    }
    Ok(plan)
}

fn load_matrix_rules_v1(paths: &[PathBuf]) -> Result<BTreeMap<Digest32, RulesConfigIdentityV1>> {
    let mut rules = BTreeMap::new();
    for path in paths {
        let document = load_rules_config_v1(path)?;
        decode_complete_rules_config(&document)?;
        let digest = document.canonical_digest()?;
        ensure!(
            rules.insert(digest, document).is_none(),
            "campaign-template matrix repeats one canonical rules config"
        );
    }
    Ok(rules)
}

fn expected_matrix_v1(
    authority: &ValidatedOfficialContentV3,
    profiles: &AdmittedProfileManagersV1,
    rules: &BTreeMap<Digest32, RulesConfigIdentityV1>,
) -> Result<CampaignTemplateMatrixV1> {
    let mut entries = Vec::with_capacity(rules.len() * 2);
    for (rules_digest, rules_config) in rules {
        for edition in [
            OfficialContentEditionV1::Demo,
            OfficialContentEditionV1::Full,
        ] {
            let requirement = campaign_requirement_v1(edition, rules_config)?;
            ensure!(
                requirement.rules_config_sha256 == *rules_digest,
                "campaign requirement rules digest changed"
            );
            let (_, artifact) = author_canonical_campaign_template_v1(
                requirement,
                rules_config,
                profiles.get(edition),
            )?;
            let edition_path = match edition {
                OfficialContentEditionV1::Demo => "demo",
                OfficialContentEditionV1::Full => "full",
            };
            entries.push(CampaignTemplateMatrixEntryV1 {
                rules_config_sha256: *rules_digest,
                requirement,
                path: format!("templates/{rules_digest}/{edition_path}.bitcode"),
                artifact,
            });
        }
    }
    let matrix = CampaignTemplateMatrixV1 {
        schema_version: CAMPAIGN_TEMPLATE_MATRIX_SCHEMA_VERSION_V1,
        official_content_digests_sha256: authority.digests.canonical_digest()?,
        projection_authority_matrix_sha256: authority.matrix.canonical_digest()?,
        entries,
    };
    validate_matrix_document_v1(&matrix)?;
    Ok(matrix)
}

fn validate_matrix_document_v1(matrix: &CampaignTemplateMatrixV1) -> Result<()> {
    ensure!(
        matrix.schema_version == CAMPAIGN_TEMPLATE_MATRIX_SCHEMA_VERSION_V1
            && !matrix.official_content_digests_sha256.is_zero()
            && !matrix.projection_authority_matrix_sha256.is_zero()
            && !matrix.entries.is_empty()
            && matrix.entries.len() <= MAX_RULES_CONFIGS_PER_MATRIX_V1 * 2
            && matrix.entries.len().is_multiple_of(2),
        "invalid campaign-template matrix header"
    );
    let mut expected_order = None;
    let mut paths = BTreeSet::new();
    for entry in &matrix.entries {
        entry.requirement.validate()?;
        entry.artifact.validate()?;
        ensure!(
            entry.rules_config_sha256 == entry.requirement.rules_config_sha256
                && entry.artifact.media_type == RANKED_CAMPAIGN_MEDIA_TYPE_V1,
            "campaign-template matrix entry has inconsistent identities"
        );
        let edition_order = match entry.requirement.edition {
            OfficialContentEditionV1::Demo => 0_u8,
            OfficialContentEditionV1::Full => 1_u8,
        };
        let key = (entry.rules_config_sha256, edition_order);
        ensure!(
            expected_order
                .as_ref()
                .is_none_or(|previous| previous < &key),
            "campaign-template matrix entries are not sorted and unique"
        );
        expected_order = Some(key);
        let expected_kind = match entry.requirement.edition {
            OfficialContentEditionV1::Demo => CanonicalCampaignStateKindV1::IndividualTemplate,
            OfficialContentEditionV1::Full => CanonicalCampaignStateKindV1::FullCampaignGenesis,
        };
        let edition_path = if edition_order == 0 { "demo" } else { "full" };
        ensure!(
            entry.requirement.kind == expected_kind
                && entry.path
                    == format!(
                        "templates/{}/{edition_path}.bitcode",
                        entry.rules_config_sha256
                    )
                && paths.insert(entry.path.clone()),
            "campaign-template matrix entry has a substituted kind or path"
        );
        validate_relative_matrix_path_v1(&entry.path)?;
    }
    for pair in matrix.entries.chunks_exact(2) {
        ensure!(
            pair[0].rules_config_sha256 == pair[1].rules_config_sha256
                && pair[0].requirement.edition == OfficialContentEditionV1::Demo
                && pair[1].requirement.edition == OfficialContentEditionV1::Full,
            "campaign-template matrix is missing an edition pair"
        );
    }
    Ok(())
}

fn validate_matrix_tree_with_authority_v1(
    root: &Path,
    expected: &CampaignTemplateMatrixV1,
    profiles: &AdmittedProfileManagersV1,
    rules: &BTreeMap<Digest32, RulesConfigIdentityV1>,
) -> Result<()> {
    crate::validate_mount_root(root)?;
    let matrix_path = root.join(CAMPAIGN_TEMPLATE_MATRIX_MANIFEST_V1);
    let matrix_bytes = crate::read_regular_file_bounded(&matrix_path, crate::MAX_DOCUMENT_BYTES)?;
    let actual: CampaignTemplateMatrixV1 = crate::strict_json_from_slice(&matrix_bytes)
        .context("decode campaign-template matrix manifest")?;
    validate_matrix_document_v1(&actual)?;
    ensure!(
        canonical_json_bytes(&actual)? == matrix_bytes && actual == *expected,
        "campaign-template matrix manifest differs from exact authority derivation"
    );

    let mut expected_files = BTreeSet::from([CAMPAIGN_TEMPLATE_MATRIX_MANIFEST_V1.to_owned()]);
    for entry in &actual.entries {
        let path = root.join(&entry.path);
        validate_regular_file(&path, "campaign template")?;
        let bytes = crate::read_regular_file_bounded(
            &path,
            MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1 as u64,
        )?;
        let rules = rules
            .get(&entry.rules_config_sha256)
            .context("campaign-template matrix references absent rules")?;
        let artifact = validate_canonical_campaign_template_v1(
            &bytes,
            entry.requirement,
            rules,
            profiles.get(entry.requirement.edition),
        )?;
        ensure!(
            artifact == entry.artifact,
            "campaign template differs from its exact matrix artifact"
        );
        expected_files.insert(entry.path.clone());
    }
    let actual_files = crate::walk_regular_files(root)?
        .into_iter()
        .map(|(relative, _)| manifest_path_v1(&relative))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        actual_files == expected_files,
        "campaign-template matrix contains a missing or extra file"
    );
    validate_matrix_directories_v1(root, &expected_files)
}

fn validate_relative_matrix_path_v1(path: &str) -> Result<()> {
    let path = Path::new(path);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "campaign-template matrix path is not canonical relative"
    );
    Ok(())
}

fn manifest_path_v1(path: &Path) -> Result<String> {
    let mut output = String::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            bail!("matrix path is not canonical relative")
        };
        let component = component.to_str().context("matrix path is not UTF-8")?;
        if !output.is_empty() {
            output.push('/');
        }
        output.push_str(component);
    }
    ensure!(!output.is_empty(), "matrix path is empty");
    Ok(output)
}

fn validate_matrix_directories_v1(root: &Path, files: &BTreeSet<String>) -> Result<()> {
    let expected = files
        .iter()
        .flat_map(|file| {
            let path = Path::new(file);
            path.ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    let mut pending = vec![(PathBuf::new(), root.to_path_buf())];
    while let Some((relative, absolute)) = pending.pop() {
        for entry in fs::read_dir(absolute)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let child = relative.join(entry.file_name());
                actual.insert(child.clone());
                pending.push((child, entry.path()));
            }
        }
    }
    ensure!(
        actual == expected,
        "campaign-template matrix has extra or missing directories"
    );
    Ok(())
}

fn validate_normalized_output(output: &Path) -> Result<()> {
    ensure!(
        output.is_absolute(),
        "campaign-template matrix output must be absolute"
    );
    let parent = output
        .parent()
        .context("campaign-template matrix output has no parent")?;
    fs::create_dir_all(parent)?;
    ensure!(
        fs::canonicalize(parent)? == parent,
        "campaign-template matrix output parent is not normalized"
    );
    Ok(())
}

fn validate_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{label} is not a regular non-symlink file: {}",
        path.display()
    );
    Ok(())
}

/// Produce the only canonical fresh-campaign bytes accepted for this
/// edition/kind/rules tuple.
///
/// `profiles` must be the exact profile catalog loaded from the admitted
/// official source for `requirement.edition`. The template intentionally does
/// not select a mission, create a restart snapshot, assign a campaign-history
/// run id, or apply any menu/profile state: those are later authenticated run
/// setup operations, not genesis state.
pub fn author_canonical_campaign_template_v1(
    requirement: CanonicalCampaignStateRequirementV1,
    rules_config: &RulesConfigIdentityV1,
    profiles: &ProfileManager,
) -> Result<(Vec<u8>, ArtifactRefV1)> {
    let sim_config = validate_template_authority(requirement, rules_config, profiles)?;
    let campaign = Campaign::from_profiles(profiles, sim_config.difficulty);
    campaign
        .validate_history_schema()
        .map_err(anyhow::Error::msg)
        .context("fresh campaign has invalid history storage")?;

    let bytes = bitcode::encode(&campaign);
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1,
        "authored campaign template exceeds the verifier campaign-input boundary"
    );
    let artifact = campaign_template_artifact(&bytes)?;
    Ok((bytes, artifact))
}

/// Decode and admit campaign-template bytes against an exact typed authority.
///
/// Admission is deliberately stronger than successful bitcode decoding:
///
/// * re-encoding must reproduce every input byte, rejecting trailing or
///   alternative opaque encodings;
/// * decoded history storage must be internally valid; and
/// * the bytes must exactly equal a fresh campaign independently rebuilt from
///   the admitted profiles and rules difficulty.
///
/// Consequently, a valid save, continuation, selected-mission checkpoint, or
/// otherwise well-formed `Campaign` cannot be substituted for a genesis
/// template.
pub fn validate_canonical_campaign_template_v1(
    bytes: &[u8],
    requirement: CanonicalCampaignStateRequirementV1,
    rules_config: &RulesConfigIdentityV1,
    profiles: &ProfileManager,
) -> Result<ArtifactRefV1> {
    ensure!(!bytes.is_empty(), "campaign template is empty");
    ensure!(
        bytes.len() <= MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1,
        "campaign template exceeds the verifier campaign-input boundary"
    );

    let decoded: Campaign =
        bitcode::decode(bytes).context("decode campaign template as current Campaign bitcode")?;
    decoded
        .validate_history_schema()
        .map_err(anyhow::Error::msg)
        .context("campaign template has invalid history storage")?;
    ensure!(
        bitcode::encode(&decoded) == bytes,
        "campaign template is not the canonical current Campaign bitcode encoding"
    );

    let (expected, expected_artifact) =
        author_canonical_campaign_template_v1(requirement, rules_config, profiles)?;
    ensure!(
        bytes == expected,
        "campaign template differs from the exact fresh campaign for its edition and rules"
    );
    debug_assert_eq!(campaign_template_artifact(bytes)?, expected_artifact);
    Ok(expected_artifact)
}

fn validate_template_authority(
    requirement: CanonicalCampaignStateRequirementV1,
    rules_config: &RulesConfigIdentityV1,
    profiles: &ProfileManager,
) -> Result<SimConfig> {
    requirement
        .validate()
        .context("validate canonical campaign-state requirement")?;
    let expected_kind = match requirement.edition {
        OfficialContentEditionV1::Demo => CanonicalCampaignStateKindV1::IndividualTemplate,
        OfficialContentEditionV1::Full => CanonicalCampaignStateKindV1::FullCampaignGenesis,
    };
    ensure!(
        requirement.kind == expected_kind,
        "canonical campaign template kind does not match its official edition"
    );

    let sim_config = decode_complete_rules_config(rules_config)?;
    ensure!(
        requirement.rules_config_sha256 == rules_config.canonical_digest()?,
        "canonical campaign template requirement names a different rules configuration"
    );

    // The exact fully authenticated official profile catalog is authoritative
    // here. In particular, real shipping data can contain duplicate localized
    // `Robin des bois` names; `Campaign::from_profiles` deliberately uses the
    // first match and retains its retail-order fallback. Guard only the shapes
    // that would panic or produce an unusable campaign.
    ensure!(
        profiles.characters.len() > 1,
        "official campaign template profile catalog has fewer than two characters"
    );
    ensure!(
        !profiles.missions.is_empty(),
        "official campaign template profile catalog has no missions"
    );
    Ok(sim_config)
}

/// Decode the complete current-schema `SimConfig` without accepting serde
/// defaults or unknown fields. This mirrors the manifest tool's general
/// ranked-rules gate locally so campaign-template admission cannot be used
/// independently with a partial rules document.
fn decode_complete_rules_config(rules_config: &RulesConfigIdentityV1) -> Result<SimConfig> {
    rules_config
        .validate()
        .context("validate campaign-template rules configuration")?;
    ensure!(
        rules_config.replay_schema_version == CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        "campaign templates require the current ranked replay schema"
    );

    let canonical_input = CanonicalValue::Object(rules_config.sim_config.clone());
    let sim_config: SimConfig = serde_json::from_value(
        serde_json::to_value(&canonical_input).context("encode canonical SimConfig")?,
    )
    .context("decode complete campaign-template SimConfig")?;
    sim_config
        .validate()
        .context("validate campaign-template SimConfig")?;
    let canonical_round_trip: CanonicalValue = serde_json::from_value(
        serde_json::to_value(sim_config).context("encode engine SimConfig")?,
    )
    .context("canonicalize engine SimConfig")?;
    ensure!(
        canonical_round_trip == canonical_input,
        "campaign-template SimConfig contains missing, unknown, defaulted, or noncanonical fields"
    );
    let policy = robin_engine::engine::RankedSimulationPolicy::from_identity(
        rules_config.ranked_simulation_policy,
    )
    .context("decode campaign-template ranked simulation policy")?;
    policy
        .validate_config(sim_config)
        .context("campaign-template SimConfig differs from its ranked simulation policy")?;
    Ok(sim_config)
}

fn campaign_template_artifact(bytes: &[u8]) -> Result<ArtifactRefV1> {
    let byte_length = u64::try_from(bytes.len()).context("campaign template length exceeds u64")?;
    let artifact = ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length,
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
    };
    artifact
        .validate()
        .context("validate authored campaign-template artifact")?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use anyhow::Result;
    use robin_engine::campaign::Campaign;
    use robin_engine::engine::SimConfig;
    use robin_engine::player_profile::DifficultyLevel;
    use robin_engine::profiles::{CharacterProfile, MissionProfile, ProfileManager};
    use robin_run_protocol::{
        CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1, CanonicalCampaignStateKindV1,
        CanonicalCampaignStateRequirementV1, CanonicalDocument as _, CanonicalValue,
        OfficialContentEditionV1, RANKED_CAMPAIGN_MEDIA_TYPE_V1, RankedSimulationDifficultyV1,
        RankedSimulationPolicyV1, RulesConfigIdentityV1,
    };

    use super::{
        MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1, author_canonical_campaign_template_v1,
        validate_canonical_campaign_template_v1,
    };

    fn profiles() -> ProfileManager {
        let mut profiles = ProfileManager::new();
        for (index, name) in ["Robin des villes", "Robin des bois", "Petit Jean"]
            .into_iter()
            .enumerate()
        {
            profiles.characters.push(CharacterProfile {
                index: index as u32,
                profile_name: name.to_owned(),
                action_max_ammo: [10; robin_engine::profiles::NUMBER_OF_PC_ACTIONS],
                ..Default::default()
            });
        }
        profiles.missions.push(MissionProfile::default());
        profiles
    }

    fn rules_config(difficulty: DifficultyLevel) -> Result<RulesConfigIdentityV1> {
        let protocol_difficulty = match difficulty {
            DifficultyLevel::Easy => RankedSimulationDifficultyV1::Easy,
            DifficultyLevel::Medium => RankedSimulationDifficultyV1::Medium,
            DifficultyLevel::Hard => RankedSimulationDifficultyV1::Hard,
            DifficultyLevel::Legendary | DifficultyLevel::Custom(_) => {
                panic!("test helper only authors protocol V1 difficulties")
            }
        };
        let CanonicalValue::Object(sim_config) = serde_json::from_value(serde_json::to_value(
            SimConfig::standard_ranked(difficulty),
        )?)?
        else {
            panic!("SimConfig must canonicalize to an object")
        };
        Ok(RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(protocol_difficulty),
            sim_config,
            rules: BTreeMap::from([("ranked".into(), CanonicalValue::Bool(true))]),
        })
    }

    fn requirement(
        edition: OfficialContentEditionV1,
        rules: &RulesConfigIdentityV1,
    ) -> Result<CanonicalCampaignStateRequirementV1> {
        Ok(CanonicalCampaignStateRequirementV1 {
            edition,
            kind: match edition {
                OfficialContentEditionV1::Demo => CanonicalCampaignStateKindV1::IndividualTemplate,
                OfficialContentEditionV1::Full => CanonicalCampaignStateKindV1::FullCampaignGenesis,
            },
            rules_config_sha256: rules.canonical_digest()?,
        })
    }

    #[test]
    fn demo_template_is_exact_fresh_campaign_for_rules_difficulty() -> Result<()> {
        let profiles = profiles();
        let rules = rules_config(DifficultyLevel::Easy)?;
        let requirement = requirement(OfficialContentEditionV1::Demo, &rules)?;
        let (bytes, artifact) =
            author_canonical_campaign_template_v1(requirement, &rules, &profiles)?;

        assert_eq!(
            bytes,
            bitcode::encode(&Campaign::from_profiles(&profiles, DifficultyLevel::Easy))
        );
        assert_eq!(artifact.media_type, RANKED_CAMPAIGN_MEDIA_TYPE_V1);
        assert_eq!(artifact.byte_length, bytes.len() as u64);
        assert_eq!(
            validate_canonical_campaign_template_v1(&bytes, requirement, &rules, &profiles)?,
            artifact
        );
        Ok(())
    }

    #[test]
    fn full_template_requires_full_campaign_genesis_kind() -> Result<()> {
        let profiles = profiles();
        let rules = rules_config(DifficultyLevel::Medium)?;
        let full = requirement(OfficialContentEditionV1::Full, &rules)?;
        author_canonical_campaign_template_v1(full, &rules, &profiles)?;

        let wrong = CanonicalCampaignStateRequirementV1 {
            kind: CanonicalCampaignStateKindV1::IndividualTemplate,
            ..full
        };
        assert!(author_canonical_campaign_template_v1(wrong, &rules, &profiles).is_err());
        Ok(())
    }

    #[test]
    fn demo_template_requires_individual_kind() -> Result<()> {
        let profiles = profiles();
        let rules = rules_config(DifficultyLevel::Medium)?;
        let demo = requirement(OfficialContentEditionV1::Demo, &rules)?;
        let wrong = CanonicalCampaignStateRequirementV1 {
            kind: CanonicalCampaignStateKindV1::FullCampaignGenesis,
            ..demo
        };
        assert!(author_canonical_campaign_template_v1(wrong, &rules, &profiles).is_err());
        Ok(())
    }

    #[test]
    fn substituted_rules_digest_is_rejected() -> Result<()> {
        let profiles = profiles();
        let easy = rules_config(DifficultyLevel::Easy)?;
        let hard = rules_config(DifficultyLevel::Hard)?;
        let requirement = requirement(OfficialContentEditionV1::Demo, &easy)?;
        assert!(author_canonical_campaign_template_v1(requirement, &hard, &profiles).is_err());
        Ok(())
    }

    #[test]
    fn partial_sim_config_is_rejected_instead_of_defaulted() -> Result<()> {
        let profiles = profiles();
        let mut rules = rules_config(DifficultyLevel::Medium)?;
        rules.sim_config.remove("script_enabled");
        let requirement = CanonicalCampaignStateRequirementV1 {
            edition: OfficialContentEditionV1::Demo,
            kind: CanonicalCampaignStateKindV1::IndividualTemplate,
            rules_config_sha256: rules.canonical_digest()?,
        };
        assert!(author_canonical_campaign_template_v1(requirement, &rules, &profiles).is_err());
        Ok(())
    }

    #[test]
    fn arbitrary_and_oversized_bytes_fail_before_admission() -> Result<()> {
        let profiles = profiles();
        let rules = rules_config(DifficultyLevel::Medium)?;
        let requirement = requirement(OfficialContentEditionV1::Demo, &rules)?;
        assert!(
            validate_canonical_campaign_template_v1(
                b"opaque operator bytes",
                requirement,
                &rules,
                &profiles
            )
            .is_err()
        );
        assert!(
            validate_canonical_campaign_template_v1(
                &vec![0; MAX_CANONICAL_CAMPAIGN_TEMPLATE_BYTES_V1 + 1],
                requirement,
                &rules,
                &profiles
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn well_formed_non_genesis_campaign_is_rejected() -> Result<()> {
        let profiles = profiles();
        let rules = rules_config(DifficultyLevel::Medium)?;
        let requirement = requirement(OfficialContentEditionV1::Full, &rules)?;
        let mut campaign = Campaign::from_profiles(&profiles, DifficultyLevel::Medium);
        campaign.ares = 0;
        let bytes = bitcode::encode(&campaign);
        assert!(
            validate_canonical_campaign_template_v1(&bytes, requirement, &rules, &profiles)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn unusable_official_profile_catalog_is_rejected_without_campaign_panic() -> Result<()> {
        let rules = rules_config(DifficultyLevel::Medium)?;
        let requirement = requirement(OfficialContentEditionV1::Demo, &rules)?;
        assert!(
            author_canonical_campaign_template_v1(requirement, &rules, &ProfileManager::new())
                .is_err()
        );

        let mut one_character = profiles();
        one_character.characters.truncate(1);
        assert!(
            author_canonical_campaign_template_v1(requirement, &rules, &one_character).is_err()
        );

        let mut no_missions = profiles();
        no_missions.missions.clear();
        assert!(author_canonical_campaign_template_v1(requirement, &rules, &no_missions).is_err());
        Ok(())
    }
}
