//! Deterministic operator tooling for verified-run release manifests.
//!
//! Ranked content identities are engine-owned simulation projections, not
//! installation/package inventories. This module requires independent native
//! and RHDDNA10 shipping exports and accepts a subject only when all eight canonical
//! component documents are byte-identical. Retail component documents are
//! materialized only below the private verifier-bundle tree; only DEMO
//! component objects are copied into the public static tree.

pub mod campaign_template_v1;
pub mod plan_v3;
pub mod publication_v3;
pub mod release_admission_v1;
pub mod sandbox_v3;
pub mod typed_js_authority;
pub mod verifier_catalog_v1;
pub mod vps_release_v2;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use goblin::elf::{Elf, header, program_header};
use robin_run_protocol::{
    ArtifactRefV1, BrowserIdentitySignerBuildIdentityV2, BrowserIdentitySignerBuildRecipeV2,
    BrowserIdentitySignerDeploymentPolicyV2, BrowserPagesArtifactV2,
    BrowserPagesShellBuildIdentityV2, BrowserPagesShellBuildRecipeV2, BrowserViewerBuildIdentityV2,
    BrowserViewerEngineBuildIdentityV2, BrowserViewerEngineBuildRecipeV2, BuildManifestV1,
    BuildManifestV2, BuildToolAuthorityDocumentV1, BuildToolAuthorityV1,
    CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1, CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
    CampaignContentEntryV1, CampaignContentManifestV1, CanonicalDocument as _, CanonicalValue,
    CompetitionManifestV1, ContentManifestV1, Digest32, ImmutablePolicyManifestV1,
    InputProvenanceEligibilityV1, NamedArtifactV1, NativeBuildPlatformV2, NativeLinkageV2,
    OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1, OfficialContentSubjectV1,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionExporterBuildIdentityV2, OfficialProjectionExporterPlatformV2,
    OfficialSimulationProjectionReceiptV1, OfficialSimulationProjectionReceiptV2,
    OfficialSourceTreeManifestV1, OfficialSourceTreeManifestV2, OfficialViewerBuildReportV2,
    PublishedRulesetV1, RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1,
    RunContentIdentityV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
    SimulationContentComponentDocumentV1, SimulationContentComponentKindV1,
    SimulationContentComponentV1, Validate as _, ValidationError, VerifierBuildIdentityV2,
    ViewerArtifactRoleV1, build_artifact_object_path_v1, canonical_json_bytes,
    demo_content_object_path_v1, official_achievement_policies_v1, official_content_subjects_v1,
    simulation_content_component_relative_path_v1,
};
use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::typed_js_authority::{
    JavaScriptBuildToolAuthorityDocumentV1, JavaScriptBuildToolRoleV1,
};

/// Shared fail-closed selector and exact materializer used by both the
/// projection exporter and operator verification tooling.
pub use robin_official_content as official_content_source;

const PLAN_SCHEMA_VERSION: u32 = 1;
const PROJECTION_PLAN_SCHEMA_VERSION: u32 = 2;
const RELEASE_LOCK_SCHEMA_VERSION: u32 = 2;
const MAX_DOCUMENT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PROJECTION_EXPORTER_BYTES: u64 = 512 * 1024 * 1024;
const OFFICIAL_DEMO_RESOURCE_LOCALE_ROOT_V1: &str = "1033";
const OFFICIAL_FULL_RESOURCE_LOCALE_ROOT_V1: &str = "2047";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionPlanV1 {
    pub schema_version: u32,
    pub demo: EditionProjectionPlanV1,
    pub full: EditionProjectionPlanV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditionProjectionPlanV1 {
    pub edition: OfficialContentEditionV1,
    /// Exact supervisor-approved raw source mount. It is inventoried but never
    /// copied into an operator release.
    pub native_source_root: PathBuf,
    pub native_source_tree_manifest: PathBuf,
    pub native_projection_receipt: PathBuf,
    /// Engine-exported loose/native projection mount.
    pub native_projection_root: PathBuf,
    pub shipping_source_root: PathBuf,
    pub shipping_source_tree_manifest: PathBuf,
    pub shipping_projection_receipt: PathBuf,
    /// Engine-exported RHDDNA10/browser shipping projection mount.
    pub shipping_projection_root: PathBuf,
}

pub use robin_run_protocol::{
    OfficialProjectionExporterIdentityV1 as ProjectionExporterIdentityV1,
    OfficialProjectionSourceFormatV1 as ProjectionSourceFormatV1,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildDraftV1 {
    pub schema_version: u32,
    pub source_commit: String,
    pub cargo_lock: PathBuf,
    pub target_triple: String,
    pub cargo_profile: String,
    #[serde(default)]
    pub cargo_features: Vec<String>,
    pub replay_schema_version: u32,
    pub save_schema_version: u32,
    pub network_protocol_version: u32,
    pub verifier: BuildArtifactSourceV1,
    pub viewer_artifacts: Vec<ViewerArtifactSourceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildDraftV2 {
    pub schema_version: u32,
    pub source_commit: String,
    pub cargo_lock: PathBuf,
    pub replay_schema_version: u32,
    pub save_schema_version: u32,
    pub network_protocol_version: u32,
    pub verifier: PathBuf,
    pub viewer_engine_artifacts: Vec<ViewerArtifactSourceV1>,
    pub pages_shell_artifacts: Vec<BrowserArtifactSourceV2>,
    pub identity_signer_artifacts: Vec<BrowserArtifactSourceV2>,
    pub rust_toolchain_authority: PathBuf,
    pub wasm_bindgen_cli: BuildToolSourceV2,
    pub binaryen_wasm_opt: BuildToolSourceV2,
    pub wabt_wasm_strip: BuildToolSourceV2,
    pub node: BuildToolSourceV2,
    pub pnpm: BuildToolSourceV2,
    pub package_json: PathBuf,
    pub pnpm_lock: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionAuthorityDraftV2 {
    pub schema_version: u32,
    pub public_build_manifest: PathBuf,
    pub projection_exporter: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildToolSourceV2 {
    pub version: String,
    pub authority_document: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserArtifactSourceV2 {
    pub source: PathBuf,
    pub published_path: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildArtifactSourceV1 {
    pub source: PathBuf,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerArtifactSourceV1 {
    pub source: PathBuf,
    pub published_path: String,
    pub role: ViewerArtifactRoleV1,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorReleasePlanV1 {
    pub schema_version: u32,
    pub official_projection_plan: PathBuf,
    pub builds: Vec<ReleaseBuildV1>,
    pub rules_configs: Vec<PathBuf>,
    pub policies: Vec<PathBuf>,
    /// Canonical `PublishedRulesetV1` documents. The embedded immutable
    /// manifest and mutable publication status are emitted separately.
    pub published_rulesets: Vec<PathBuf>,
    #[serde(default)]
    pub competitions: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseBuildV1 {
    pub draft: PathBuf,
    /// Pinned canonical manifest which must equal fresh artifact authoring.
    pub expected_manifest: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialContentDigestsV1 {
    pub schema_version: u32,
    pub demo_content_manifest_sha256: Vec<Digest32>,
    pub full_content_manifest_sha256: Vec<Digest32>,
    pub demo_campaign_content_manifest_sha256: Digest32,
    pub full_campaign_content_manifest_sha256: Digest32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierSourceBindingV1 {
    pub schema_version: u32,
    pub content_manifest_sha256: Digest32,
    pub official_projection_plan_sha256: Digest32,
    pub edition: OfficialContentEditionV1,
    pub subject: OfficialContentSubjectV1,
    pub native_projection_receipt_sha256: Digest32,
    pub native_source_tree_manifest_sha256: Digest32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseFileExposureV1 {
    BackendManifest,
    OperatorPrivate,
    PublicStatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFileV1 {
    pub path: String,
    pub artifact: ArtifactRefV1,
    pub exposure: ReleaseFileExposureV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorReleaseLockV1 {
    pub schema_version: u32,
    pub source_plan_sha256: Digest32,
    pub official_projection_plan_sha256: Digest32,
    pub official_content: OfficialContentDigestsV1,
    pub projection_receipt_sha256: Vec<Digest32>,
    pub source_tree_manifest_sha256: Vec<Digest32>,
    pub build_manifest_sha256: Vec<Digest32>,
    pub rules_config_sha256: Vec<Digest32>,
    pub policy_manifest_sha256: Vec<Digest32>,
    pub ruleset_manifest_sha256: Vec<Digest32>,
    pub competition_manifest_sha256: Vec<Digest32>,
    /// Complete file inventory excluding this lock and its digest sidecar.
    pub files: Vec<ReleaseFileV1>,
}

impl robin_run_protocol::Validate for OfficialContentDigestsV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != 1
            || self.demo_content_manifest_sha256.len()
                != official_content_subjects_v1(OfficialContentEditionV1::Demo).len()
            || self.full_content_manifest_sha256.len()
                != official_content_subjects_v1(OfficialContentEditionV1::Full).len()
            || self.demo_campaign_content_manifest_sha256.is_zero()
            || self.full_campaign_content_manifest_sha256.is_zero()
            || self.demo_campaign_content_manifest_sha256
                == self.full_campaign_content_manifest_sha256
            || !strict_nonzero_digests(&self.demo_content_manifest_sha256)
            || !strict_nonzero_digests(&self.full_content_manifest_sha256)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_content_digests",
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for VerifierSourceBindingV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != 1
            || self.content_manifest_sha256.is_zero()
            || self.official_projection_plan_sha256.is_zero()
            || self.native_projection_receipt_sha256.is_zero()
            || self.native_source_tree_manifest_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "verifier_source_binding",
            });
        }
        self.subject.validate()
    }
}

impl robin_run_protocol::Validate for OperatorReleaseLockV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != RELEASE_LOCK_SCHEMA_VERSION
            || self.source_plan_sha256.is_zero()
            || self.official_projection_plan_sha256.is_zero()
            || !strict_nonzero_digests(&self.projection_receipt_sha256)
            || !strict_nonzero_digests(&self.source_tree_manifest_sha256)
            || !strict_nonzero_digests(&self.build_manifest_sha256)
            || !strict_nonzero_digests(&self.rules_config_sha256)
            || !strict_nonzero_digests(&self.policy_manifest_sha256)
            || !strict_nonzero_digests(&self.ruleset_manifest_sha256)
            || (!self.competition_manifest_sha256.is_empty()
                && !strict_nonzero_digests(&self.competition_manifest_sha256))
            || self.files.is_empty()
            || !self
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            || self.files.iter().any(|file| {
                file.path.is_empty()
                    || file.path.starts_with('/')
                    || file
                        .path
                        .split('/')
                        .any(|part| part.is_empty() || part == "." || part == "..")
                    || file.artifact.validate().is_err()
            })
        {
            return Err(ValidationError::ClaimMismatch {
                field: "operator_release_lock",
            });
        }
        self.official_content.validate()
    }
}

fn strict_nonzero_digests(values: &[Digest32]) -> bool {
    !values.is_empty()
        && values.iter().all(|digest| !digest.is_zero())
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

#[derive(Debug, Clone)]
pub struct AuthoredDocument {
    pub digest: Digest32,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    Build,
    BuildV2,
    BuildToolAuthority,
    JavascriptBuildToolAuthority,
    ViewerBuildReportV2,
    ProjectionAuthorityV2,
    Content,
    CampaignContent,
    SimulationComponent,
    SourceTreeManifest,
    SourceTreeManifestV2,
    ProjectionReceipt,
    ProjectionReceiptV2,
    BuiltInOverlaySourceManifestV2,
    ProjectionExecutionPolicy,
    VerifierSourceBinding,
    VerifierSourceBindingV2,
    ProjectionExecutionRecordV3,
    ProjectionAuthorityMatrixV3,
    RulesConfig,
    RulesetManifest,
    PublishedRuleset,
    Competition,
    Policy,
    ReleaseLock,
}

impl std::str::FromStr for DocumentKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "build" => Ok(Self::Build),
            "build-v2" => Ok(Self::BuildV2),
            "build-tool-authority" => Ok(Self::BuildToolAuthority),
            "javascript-build-tool-authority" => Ok(Self::JavascriptBuildToolAuthority),
            "viewer-build-report-v2" => Ok(Self::ViewerBuildReportV2),
            "projection-authority-v2" => Ok(Self::ProjectionAuthorityV2),
            "content" => Ok(Self::Content),
            "campaign-content" => Ok(Self::CampaignContent),
            "simulation-component" => Ok(Self::SimulationComponent),
            "source-tree-manifest" => Ok(Self::SourceTreeManifest),
            "source-tree-manifest-v2" => Ok(Self::SourceTreeManifestV2),
            "projection-receipt" => Ok(Self::ProjectionReceipt),
            "projection-receipt-v2" => Ok(Self::ProjectionReceiptV2),
            "built-in-overlay-source-manifest-v2" => Ok(Self::BuiltInOverlaySourceManifestV2),
            "projection-execution-policy" => Ok(Self::ProjectionExecutionPolicy),
            "verifier-source-binding" => Ok(Self::VerifierSourceBinding),
            "verifier-source-binding-v2" => Ok(Self::VerifierSourceBindingV2),
            "projection-execution-record-v3" => Ok(Self::ProjectionExecutionRecordV3),
            "projection-authority-matrix-v3" => Ok(Self::ProjectionAuthorityMatrixV3),
            "rules-config" => Ok(Self::RulesConfig),
            "ruleset-manifest" => Ok(Self::RulesetManifest),
            "published-ruleset" => Ok(Self::PublishedRuleset),
            "competition" => Ok(Self::Competition),
            "policy" => Ok(Self::Policy),
            "release-lock" => Ok(Self::ReleaseLock),
            _ => bail!("unknown document kind {value:?}"),
        }
    }
}

#[derive(Debug)]
struct AuthoredContent {
    manifest: ContentManifestV1,
    components: BTreeMap<SimulationContentComponentKindV1, Vec<u8>>,
}

#[derive(Debug)]
struct AuthoredEdition {
    edition: OfficialContentEditionV1,
    content: BTreeMap<Digest32, AuthoredContent>,
    campaign: CampaignContentManifestV1,
    native_source: ValidatedProjectionSource,
    shipping_source: ValidatedProjectionSource,
}

#[derive(Debug)]
struct ValidatedProjectionSource {
    source_tree_manifest: OfficialSourceTreeManifestV1,
    source_tree_manifest_sha256: Digest32,
    receipt: OfficialSimulationProjectionReceiptV1,
    receipt_sha256: Digest32,
}

#[derive(Debug)]
struct LoadedBuild {
    manifest: BuildManifestV1,
    draft: BuildDraftV1,
}

#[derive(Debug)]
struct LoadedRelease {
    source_plan_sha256: Digest32,
    official_projection_plan_sha256: Digest32,
    editions: [AuthoredEdition; 2],
    builds: BTreeMap<Digest32, LoadedBuild>,
    rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1>,
    policies: BTreeMap<Digest32, ImmutablePolicyManifestV1>,
    ruleset_manifests: BTreeMap<Digest32, RulesetManifestV1>,
    published_rulesets: BTreeMap<Digest32, PublishedRulesetV1>,
    competitions: BTreeMap<Digest32, CompetitionManifestV1>,
}

#[cfg(test)]
fn required_component_kinds() -> [SimulationContentComponentKindV1; 8] {
    [
        SimulationContentComponentKindV1::Profiles,
        SimulationContentComponentKindV1::LoadedLevel,
        SimulationContentComponentKindV1::MissionScripts,
        SimulationContentComponentKindV1::SpriteSimulationMetadata,
        SimulationContentComponentKindV1::MapGeometryMetadata,
        SimulationContentComponentKindV1::LocalizedDeterministicText,
        SimulationContentComponentKindV1::SoundDurationTables,
        SimulationContentComponentKindV1::InterfaceSimulationMetadata,
    ]
}

impl OfficialProjectionPlanV1 {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut plan: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse projection plan {}", path.display()))?;
        ensure!(
            plan.schema_version == PROJECTION_PLAN_SCHEMA_VERSION,
            "unsupported projection-plan schema {}",
            plan.schema_version
        );
        let base = config_parent(path)?;
        plan.demo.resolve_roots(base);
        plan.full.resolve_roots(base);
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.demo.edition == OfficialContentEditionV1::Demo,
            "demo plan is not typed as DEMO"
        );
        ensure!(
            self.full.edition == OfficialContentEditionV1::Full,
            "full plan is not typed as FULL"
        );
        self.demo.validate()?;
        self.full.validate()?;
        let canonical_roots = [
            &self.demo.native_source_root,
            &self.demo.native_projection_root,
            &self.demo.shipping_source_root,
            &self.demo.shipping_projection_root,
            &self.full.native_source_root,
            &self.full.native_projection_root,
            &self.full.shipping_source_root,
            &self.full.shipping_projection_root,
        ]
        .into_iter()
        .map(fs::canonicalize)
        .collect::<std::io::Result<Vec<_>>>()?;
        ensure!(
            canonical_roots.iter().collect::<BTreeSet<_>>().len() == 8
                && canonical_roots
                    .iter()
                    .enumerate()
                    .all(|(left_index, left)| {
                        canonical_roots
                            .iter()
                            .enumerate()
                            .all(|(right_index, right)| {
                                left_index == right_index
                                    || (!left.starts_with(right) && !right.starts_with(left))
                            })
                    }),
            "raw and exported DEMO/FULL native/RHDDNA10 sources must be eight distinct mounts"
        );
        Ok(())
    }
}

impl EditionProjectionPlanV1 {
    fn resolve_roots(&mut self, base: &Path) {
        resolve_path(base, &mut self.native_source_root);
        resolve_path(base, &mut self.native_source_tree_manifest);
        resolve_path(base, &mut self.native_projection_receipt);
        resolve_path(base, &mut self.native_projection_root);
        resolve_path(base, &mut self.shipping_source_root);
        resolve_path(base, &mut self.shipping_source_tree_manifest);
        resolve_path(base, &mut self.shipping_projection_receipt);
        resolve_path(base, &mut self.shipping_projection_root);
    }

    fn validate(&self) -> Result<()> {
        validate_mount_root(&self.native_projection_root)
            .with_context(|| format!("validate {:?} native projection root", self.edition))?;
        validate_mount_root(&self.shipping_projection_root)
            .with_context(|| format!("validate {:?} shipping projection root", self.edition))?;
        ensure!(
            self.native_projection_root != self.shipping_projection_root,
            "{:?} native and shipping sources must be independently mounted",
            self.edition
        );
        validate_mount_root(&self.native_source_root)
            .with_context(|| format!("validate {:?} native raw source mount", self.edition))?;
        validate_mount_root(&self.shipping_source_root)
            .with_context(|| format!("validate {:?} shipping raw source mount", self.edition))?;
        Ok(())
    }
}

impl BuildDraftV1 {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut draft: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse build draft {}", path.display()))?;
        ensure!(
            draft.schema_version == PLAN_SCHEMA_VERSION,
            "unsupported build-draft schema {}",
            draft.schema_version
        );
        let base = config_parent(path)?;
        resolve_path(base, &mut draft.cargo_lock);
        resolve_path(base, &mut draft.verifier.source);
        for artifact in &mut draft.viewer_artifacts {
            resolve_path(base, &mut artifact.source);
        }
        Ok(draft)
    }

    pub fn author(&self) -> Result<BuildManifestV1> {
        let mut cargo_features = self.cargo_features.clone();
        cargo_features.sort();
        ensure!(
            cargo_features.windows(2).all(|pair| pair[0] != pair[1]),
            "build draft repeats a Cargo feature"
        );
        let mut viewer_artifacts = self
            .viewer_artifacts
            .iter()
            .map(|source| {
                Ok(NamedArtifactV1 {
                    path: source.published_path.clone(),
                    role: source.role.clone(),
                    artifact: artifact_from_file(&source.source, &source.media_type)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        viewer_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = BuildManifestV1 {
            schema_version: 1,
            source_commit: self.source_commit.clone(),
            cargo_lock_sha256: hash_regular_file(&self.cargo_lock)?,
            target_triple: self.target_triple.clone(),
            cargo_profile: self.cargo_profile.clone(),
            cargo_features,
            replay_schema_version: self.replay_schema_version,
            save_schema_version: self.save_schema_version,
            network_protocol_version: self.network_protocol_version,
            verifier: artifact_from_file(&self.verifier.source, &self.verifier.media_type)?,
            viewer_artifacts,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

impl BuildDraftV2 {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut draft: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse V2 build draft {}", path.display()))?;
        ensure!(
            draft.schema_version == 2,
            "unsupported V2 build-draft schema"
        );
        let base = config_parent(path)?;
        resolve_path(base, &mut draft.cargo_lock);
        resolve_path(base, &mut draft.verifier);
        resolve_path(base, &mut draft.rust_toolchain_authority);
        resolve_path(base, &mut draft.package_json);
        resolve_path(base, &mut draft.pnpm_lock);
        for tool in [
            &mut draft.wasm_bindgen_cli,
            &mut draft.binaryen_wasm_opt,
            &mut draft.wabt_wasm_strip,
            &mut draft.node,
            &mut draft.pnpm,
        ] {
            resolve_path(base, &mut tool.authority_document);
        }
        for artifact in &mut draft.viewer_engine_artifacts {
            resolve_path(base, &mut artifact.source);
        }
        for artifact in draft
            .pages_shell_artifacts
            .iter_mut()
            .chain(&mut draft.identity_signer_artifacts)
        {
            resolve_path(base, &mut artifact.source);
        }
        Ok(draft)
    }

    pub fn author(&self) -> Result<BuildManifestV2> {
        let mut viewer_engine_artifacts = self
            .viewer_engine_artifacts
            .iter()
            .map(|source| {
                Ok(NamedArtifactV1 {
                    path: source.published_path.clone(),
                    role: source.role.clone(),
                    artifact: artifact_from_file(&source.source, &source.media_type)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        viewer_engine_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
        let pages_shell_artifacts = author_browser_artifacts(&self.pages_shell_artifacts)?;
        let identity_signer_artifacts = author_browser_artifacts(&self.identity_signer_artifacts)?;
        let wasm_bindgen_authority =
            load_build_tool_authority(&self.wasm_bindgen_cli.authority_document)?;
        let binaryen_authority =
            load_build_tool_authority(&self.binaryen_wasm_opt.authority_document)?;
        let wabt_authority = load_build_tool_authority(&self.wabt_wasm_strip.authority_document)?;
        let node = author_javascript_build_tool(&self.node, JavaScriptBuildToolRoleV1::Node)?;
        let pnpm = author_javascript_build_tool(&self.pnpm, JavaScriptBuildToolRoleV1::Pnpm)?;
        let rust_toolchain: robin_run_protocol::RustToolchainAuthorityV1 =
            load_canonical_authority_document(&self.rust_toolchain_authority)?;
        let rust_toolchain_sha256 = rust_toolchain.canonical_digest()?;
        let verifier = artifact_from_file(
            &self.verifier,
            robin_run_protocol::RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2,
        )?;
        validate_static_ranked_executable(
            &self.verifier,
            &verifier,
            robin_run_protocol::RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2,
            "ranked replay verifier",
        )?;
        let manifest = BuildManifestV2 {
            schema_version: 2,
            source_commit: self.source_commit.clone(),
            cargo_lock_sha256: hash_regular_file(&self.cargo_lock)?,
            replay_schema_version: self.replay_schema_version,
            save_schema_version: self.save_schema_version,
            network_protocol_version: self.network_protocol_version,
            verifier: VerifierBuildIdentityV2 {
                platform: NativeBuildPlatformV2::X86_64UnknownLinuxMusl,
                target_triple: "x86_64-unknown-linux-musl".into(),
                cargo_profile: "release".into(),
                cargo_features: vec![],
                cargo_package: "robin_replay_verifier".into(),
                cargo_binary: "robin-replay-verifier".into(),
                linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
                artifact: verifier,
            },
            viewer: BrowserViewerBuildIdentityV2 {
                engine: BrowserViewerEngineBuildIdentityV2 {
                    target_triple: "wasm32-unknown-unknown".into(),
                    cargo_profile: "wasm-release".into(),
                    cargo_features: vec!["audio".into()],
                    cargo_package: "robin_rs".into(),
                    cargo_binary: "robin".into(),
                    recipe: BrowserViewerEngineBuildRecipeV2::WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1,
                    rust_toolchain: rust_toolchain.clone(),
                    rust_toolchain_sha256,
                    wasm_bindgen_cli: author_build_tool(&self.wasm_bindgen_cli)?,
                    binaryen_wasm_opt: author_build_tool(&self.binaryen_wasm_opt)?,
                    wabt_wasm_strip: author_build_tool(&self.wabt_wasm_strip)?,
                    artifacts: viewer_engine_artifacts,
                },
                pages_shell: BrowserPagesShellBuildIdentityV2 {
                    recipe: BrowserPagesShellBuildRecipeV2::PnpmFrozenLockfileViteStaticShellV1,
                    node,
                    pnpm,
                    package_json_sha256: hash_regular_file(&self.package_json)?,
                    pnpm_lock_sha256: hash_regular_file(&self.pnpm_lock)?,
                    public_origin_artifacts: pages_shell_artifacts,
                },
                identity_signer: BrowserIdentitySignerBuildIdentityV2 {
                    target_triple: "wasm32-unknown-unknown".into(),
                    cargo_profile: "wasm-release".into(),
                    cargo_features: vec!["identity-signer-bridge".into()],
                    cargo_package: "robin_identity_signer".into(),
                    cargo_binary: "leaderboard_identity_bridge".into(),
                    recipe: BrowserIdentitySignerBuildRecipeV2::WasmBindgenWebSeparateOriginBridgeV1,
                    deployment_policy: BrowserIdentitySignerDeploymentPolicyV2::SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1,
                    rust_toolchain,
                    rust_toolchain_sha256,
                    wasm_bindgen_cli: author_build_tool(&self.wasm_bindgen_cli)?,
                    identity_signer_origin_artifacts: identity_signer_artifacts,
                },
            },
        };
        manifest.validate()?;
        manifest.validate_wasm_tool_authorities(
            &wasm_bindgen_authority,
            &binaryen_authority,
            &wabt_authority,
        )?;
        Ok(manifest)
    }
}

impl ProjectionAuthorityDraftV2 {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut draft: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse projection authority draft {}", path.display()))?;
        ensure!(
            draft.schema_version == 2,
            "unsupported authority draft schema"
        );
        let base = config_parent(path)?;
        resolve_path(base, &mut draft.public_build_manifest);
        resolve_path(base, &mut draft.projection_exporter);
        Ok(draft)
    }

    pub fn author(&self) -> Result<OfficialProjectionAuthorityManifestV2> {
        let public: BuildManifestV2 = load_canonical_document(&self.public_build_manifest)?;
        let artifact = artifact_from_file(
            &self.projection_exporter,
            robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        )?;
        validate_static_projection_exporter(&self.projection_exporter, &artifact)?;
        let authority = OfficialProjectionAuthorityManifestV2 {
            schema_version: 2,
            public_build_manifest_sha256: public.canonical_digest()?,
            source_commit: public.source_commit.clone(),
            cargo_lock_sha256: public.cargo_lock_sha256,
            replay_schema_version: public.replay_schema_version,
            save_schema_version: public.save_schema_version,
            network_protocol_version: public.network_protocol_version,
            projection_exporter: OfficialProjectionExporterBuildIdentityV2 {
                platform: OfficialProjectionExporterPlatformV2::X86_64UnknownLinuxMusl,
                target_triple: "x86_64-unknown-linux-musl".into(),
                cargo_profile: "release".into(),
                cargo_features: vec!["projection-export".into()],
                cargo_package: "robin_rs".into(),
                cargo_example: "export_simulation_content".into(),
                linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
                exporter_version: robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_VERSION_V2,
                simulation_content_projection_schema_version:
                    robin_run_protocol::OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1,
                artifact,
            },
        };
        authority.validate_against(&public)?;
        Ok(authority)
    }
}

fn author_browser_artifacts(
    sources: &[BrowserArtifactSourceV2],
) -> Result<Vec<BrowserPagesArtifactV2>> {
    let mut artifacts = sources
        .iter()
        .map(|source| {
            Ok(BrowserPagesArtifactV2 {
                path: source.published_path.clone(),
                artifact: artifact_from_file(&source.source, &source.media_type)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn author_build_tool(source: &BuildToolSourceV2) -> Result<BuildToolAuthorityV1> {
    let canonical = canonical_build_tool_authority_bytes(&source.authority_document)?;
    Ok(BuildToolAuthorityV1 {
        version: source.version.clone(),
        authority_sha256: Digest32::digest_bytes(&canonical),
    })
}

fn author_javascript_build_tool(
    source: &BuildToolSourceV2,
    expected_role: JavaScriptBuildToolRoleV1,
) -> Result<BuildToolAuthorityV1> {
    let authority: JavaScriptBuildToolAuthorityDocumentV1 =
        load_canonical_authority_document(&source.authority_document)?;
    ensure!(
        authority.role == expected_role && authority.version == source.version,
        "JavaScript build-tool source does not match its exact typed authority"
    );
    Ok(authority.build_tool_binding()?)
}

fn load_canonical_authority_document<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let authority: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical authority {}", path.display()))?;
    authority.validate()?;
    ensure!(
        canonical_authority_presentation(&bytes, &canonical_json_bytes(&authority)?),
        "{} is valid but not a canonical authority presentation",
        path.display()
    );
    Ok(authority)
}

pub(crate) fn load_build_tool_authority(path: &Path) -> Result<BuildToolAuthorityDocumentV1> {
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let authority: BuildToolAuthorityDocumentV1 = strict_json_from_slice(&bytes)?;
    authority.validate()?;
    ensure!(
        canonical_authority_presentation(&bytes, &canonical_json_bytes(&authority)?),
        "build-tool authority document is not canonical JSON"
    );
    Ok(authority)
}

fn canonical_build_tool_authority_bytes(path: &Path) -> Result<Vec<u8>> {
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let authority: CanonicalValue = strict_json_from_slice(&bytes)?;
    authority.validate_depth(128)?;
    let canonical = canonical_json_bytes(&authority)?;
    ensure!(
        canonical_authority_presentation(&bytes, &canonical),
        "build-tool authority document is not canonical JSON"
    );
    Ok(canonical)
}

fn canonical_authority_presentation(bytes: &[u8], canonical: &[u8]) -> bool {
    bytes == canonical
        || bytes
            .strip_suffix(b"\n")
            .is_some_and(|without_newline| without_newline == canonical)
}

impl OperatorReleasePlanV1 {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut plan: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse release plan {}", path.display()))?;
        ensure!(
            plan.schema_version == PLAN_SCHEMA_VERSION,
            "unsupported release-plan schema {}",
            plan.schema_version
        );
        let base = config_parent(path)?;
        resolve_path(base, &mut plan.official_projection_plan);
        for build in &mut plan.builds {
            resolve_path(base, &mut build.draft);
            resolve_path(base, &mut build.expected_manifest);
        }
        for path in plan
            .rules_configs
            .iter_mut()
            .chain(&mut plan.policies)
            .chain(&mut plan.published_rulesets)
            .chain(&mut plan.competitions)
        {
            resolve_path(base, path);
        }
        ensure!(!plan.builds.is_empty(), "release has no build manifests");
        ensure!(
            !plan.rules_configs.is_empty(),
            "release has no rules configs"
        );
        ensure!(!plan.policies.is_empty(), "release has no policy documents");
        ensure!(
            !plan.published_rulesets.is_empty(),
            "release has no published rulesets"
        );
        Ok(plan)
    }
}

pub fn hash_file(path: &Path) -> Result<ArtifactRefV1> {
    artifact_from_file(path, "application/octet-stream")
}

/// Independently decode and round-trip the complete engine `SimConfig` bound
/// by the official execution policy. Missing fields, unknown fields, defaults,
/// and non-Medium difficulty all fail before the exporter is launched.
pub fn validate_official_projection_rules_config_v1(
    rules_config: &RulesConfigIdentityV1,
) -> Result<()> {
    let sim_config = decode_complete_ranked_rules_config_v1(rules_config)?;
    ensure!(
        sim_config.difficulty == robin_engine::player_profile::DifficultyLevel::Medium,
        "official projection SimConfig must use Medium difficulty"
    );
    Ok(())
}

/// Independently decode and round-trip an arbitrary current-schema ranked rules
/// configuration. Unlike the projection execution-policy validator, this does
/// not impose Medium difficulty: distinct immutable official rulesets may
/// intentionally rank other complete engine configurations.
pub fn validate_complete_ranked_rules_config_v1(
    rules_config: &RulesConfigIdentityV1,
) -> Result<()> {
    let _ = decode_complete_ranked_rules_config_v1(rules_config)?;
    Ok(())
}

/// Admit only the exact build-format tuple emitted by the current ranked
/// client. Historical public V2 manifests remain readable, but cannot become
/// a new official projection or deployment authority.
pub(crate) fn validate_current_official_ranked_build_v2(build: &BuildManifestV2) -> Result<()> {
    build.validate()?;
    ensure!(
        build.replay_schema_version == CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1
            && build.save_schema_version == CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1
            && build.network_protocol_version == robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        "official publication build does not match the current replay/save/network schema tuple"
    );
    Ok(())
}

fn decode_complete_ranked_rules_config_v1(
    rules_config: &RulesConfigIdentityV1,
) -> Result<robin_engine::engine::SimConfig> {
    rules_config.validate()?;
    ensure!(
        rules_config.replay_schema_version == CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        "official ranked rules require the current replay schema"
    );
    let canonical_input = CanonicalValue::Object(rules_config.sim_config.clone());
    let sim_config: robin_engine::engine::SimConfig = serde_json::from_value(
        serde_json::to_value(&canonical_input).context("encode canonical SimConfig")?,
    )
    .context("decode complete official SimConfig")?;
    sim_config
        .validate()
        .context("validate official SimConfig")?;
    let canonical_round_trip: CanonicalValue = serde_json::from_value(
        serde_json::to_value(sim_config).context("encode engine SimConfig")?,
    )
    .context("canonicalize engine SimConfig")?;
    ensure!(
        canonical_round_trip == canonical_input,
        "official SimConfig contains missing, unknown, defaulted, or noncanonical fields"
    );
    let policy = robin_engine::engine::RankedSimulationPolicy::from_config(
        rules_config.ranked_simulation_policy,
        sim_config,
    )
    .context("decode ranked simulation policy")?;
    policy
        .validate_config(sim_config)
        .context("rules SimConfig differs from its ranked simulation policy")?;
    Ok(sim_config)
}

/// Adapt the shared loader-derived loose inventory into the canonical V2
/// protocol document. This is intentionally a typed conversion: an all-tree
/// or differently versioned inventory has no accepted fallback.
pub fn loose_source_manifest_v2(
    edition: OfficialContentEditionV1,
    inventory: &official_content_source::LooseSourceClosureInventory,
) -> Result<OfficialSourceTreeManifestV2> {
    ensure!(
        inventory.policy == official_content_source::LOOSE_NATIVE_SOURCE_CLOSURE_V2,
        "loose source inventory uses an unsupported closure policy"
    );
    ensure!(
        inventory.resource_locale_root == official_resource_locale_root(edition),
        "loose source inventory LCID does not match its typed edition"
    );
    let manifest = OfficialSourceTreeManifestV2 {
        schema_version: 2,
        edition,
        source_format: ProjectionSourceFormatV1::LooseNativeV1,
        closure_kind:
            robin_run_protocol::OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1,
        files: source_closure_files_v2(&inventory.files),
    };
    manifest.validate()?;
    Ok(manifest)
}

/// Adapt the exact RHDDNA10 archive-and-referenced-split inventory into its
/// canonical V2 authority. The decoded reference union is established by
/// `robin_assets` before this conversion; arbitrary subsets are not accepted
/// by the plan-v3 shipping lane.
pub fn shipping_source_manifest_v2(
    edition: OfficialContentEditionV1,
    inventory: &official_content_source::ShippingSourceClosureInventory,
) -> Result<OfficialSourceTreeManifestV2> {
    ensure!(
        inventory.policy == official_content_source::SHIPPING_DATADIR_SOURCE_CLOSURE_V2,
        "shipping source inventory uses an unsupported closure policy"
    );
    let manifest = OfficialSourceTreeManifestV2 {
        schema_version: 2,
        edition,
        source_format: ProjectionSourceFormatV1::ShippingDatadirV10,
        closure_kind: robin_run_protocol::OfficialSourceClosureKindV2::
            ShippingDatadirV10ArchiveAndReferencedSplitsV1,
        files: source_closure_files_v2(&inventory.files),
    };
    manifest.validate()?;
    Ok(manifest)
}

fn source_closure_files_v2(
    files: &[official_content_source::SourceClosureFile],
) -> Vec<robin_run_protocol::OfficialSourceFileV1> {
    files
        .iter()
        .map(|file| robin_run_protocol::OfficialSourceFileV1 {
            path: file.path.clone(),
            sha256: Digest32::from_bytes(file.sha256),
            byte_length: file.byte_length,
        })
        .collect()
}

/// Re-hash and inspect the exact executable authorized by `BuildManifestV2`.
///
/// Official projection authoring deliberately has no host-runtime manifest:
/// its Linux exporter must therefore be a static x86-64 ELF. A dynamic ELF,
/// script, wrong architecture, non-executable file, or artifact substitution
/// is rejected before bubblewrap is invoked.
pub fn validate_static_projection_exporter(path: &Path, expected: &ArtifactRefV1) -> Result<()> {
    validate_static_ranked_executable(
        path,
        expected,
        robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        "projection exporter",
    )
}

fn validate_static_ranked_executable(
    path: &Path,
    expected: &ArtifactRefV1,
    expected_media_type: &str,
    label: &str,
) -> Result<()> {
    ensure!(
        expected.media_type == expected_media_type,
        "{label} has the wrong media type"
    );
    let metadata = validate_regular_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        ensure!(
            metadata.permissions().mode() & 0o111 != 0,
            "{label} is not executable"
        );
    }
    let bytes = read_regular_file_bounded(path, MAX_PROJECTION_EXPORTER_BYTES)?;
    ensure!(
        artifact_from_bytes(&bytes, &expected.media_type) == *expected,
        "{label} bytes differ from its manifest identity"
    );
    let elf =
        Elf::parse(&bytes).with_context(|| format!("{label} is not a valid ELF executable"))?;
    ensure!(elf.is_64, "{label} is not a 64-bit ELF");
    ensure!(elf.little_endian, "{label} is not little-endian");
    ensure!(
        elf.header.e_machine == header::EM_X86_64,
        "{label} is not an x86-64 ELF"
    );
    ensure!(
        matches!(elf.header.e_type, header::ET_EXEC | header::ET_DYN),
        "{label} is not an executable ELF"
    );
    ensure!(elf.entry != 0, "{label} has no entry point");
    ensure!(
        elf.interpreter.is_none()
            && elf
                .program_headers
                .iter()
                .all(|header| header.p_type != program_header::PT_INTERP),
        "{label} has a dynamic program interpreter"
    );
    ensure!(
        elf.libraries.is_empty(),
        "{label} declares DT_NEEDED host libraries: {:?}",
        elf.libraries
    );
    ensure!(
        elf.program_headers.iter().any(|program| {
            program.p_type == program_header::PT_LOAD
                && program.p_flags & program_header::PF_X != 0
                && program.p_filesz != 0
        }),
        "{label} has no executable load segment"
    );
    // Re-hash through a second file descriptor after parsing. This makes a
    // concurrent replacement fail rather than authorizing the parsed bytes
    // while launching different bytes.
    ensure!(
        artifact_from_file(path, &expected.media_type)? == *expected,
        "{label} changed while it was inspected"
    );
    Ok(())
}

pub fn author_build(draft_path: &Path, output: &Path) -> Result<AuthoredDocument> {
    write_authored_document(output, &BuildDraftV1::load(draft_path)?.author()?)
}

pub fn author_build_v2(draft_path: &Path, output: &Path) -> Result<AuthoredDocument> {
    write_authored_document(output, &BuildDraftV2::load(draft_path)?.author()?)
}

pub fn author_projection_authority_v2(
    draft_path: &Path,
    output: &Path,
) -> Result<AuthoredDocument> {
    write_authored_document(
        output,
        &ProjectionAuthorityDraftV2::load(draft_path)?.author()?,
    )
}

pub fn author_viewer_build_report_v2(
    public_build_manifest: &Path,
    output: &Path,
) -> Result<AuthoredDocument> {
    let build: BuildManifestV2 = load_canonical_document(public_build_manifest)?;
    let report = OfficialViewerBuildReportV2::from_public_build(&build)?;
    report.validate_against(&build)?;
    write_authored_document(output, &report)
}

pub fn canonicalize_document(
    kind: DocumentKind,
    input: &Path,
    output: &Path,
) -> Result<AuthoredDocument> {
    match kind {
        DocumentKind::Build => canonicalize_typed::<BuildManifestV1>(input, output),
        DocumentKind::BuildV2 => canonicalize_typed::<BuildManifestV2>(input, output),
        DocumentKind::BuildToolAuthority => {
            canonicalize_typed::<BuildToolAuthorityDocumentV1>(input, output)
        }
        DocumentKind::JavascriptBuildToolAuthority => {
            canonicalize_typed::<JavaScriptBuildToolAuthorityDocumentV1>(input, output)
        }
        DocumentKind::ViewerBuildReportV2 => {
            canonicalize_typed::<OfficialViewerBuildReportV2>(input, output)
        }
        DocumentKind::ProjectionAuthorityV2 => {
            canonicalize_typed::<OfficialProjectionAuthorityManifestV2>(input, output)
        }
        DocumentKind::Content => canonicalize_typed::<ContentManifestV1>(input, output),
        DocumentKind::CampaignContent => {
            canonicalize_typed::<CampaignContentManifestV1>(input, output)
        }
        DocumentKind::SimulationComponent => {
            let document = SimulationContentComponentDocumentV1::from_bitcode(
                &read_regular_file_bounded(input, MAX_DOCUMENT_BYTES)?,
            )?;
            let canonical_bytes = document.bitcode_bytes()?;
            let digest = Digest32::digest_bytes(&canonical_bytes);
            write_bytes(output, &canonical_bytes)?;
            Ok(AuthoredDocument {
                digest,
                canonical_bytes,
            })
        }
        DocumentKind::SourceTreeManifest => {
            canonicalize_typed::<OfficialSourceTreeManifestV1>(input, output)
        }
        DocumentKind::SourceTreeManifestV2 => {
            canonicalize_typed::<OfficialSourceTreeManifestV2>(input, output)
        }
        DocumentKind::ProjectionReceipt => {
            canonicalize_typed::<OfficialSimulationProjectionReceiptV1>(input, output)
        }
        DocumentKind::ProjectionReceiptV2 => {
            canonicalize_typed::<OfficialSimulationProjectionReceiptV2>(input, output)
        }
        DocumentKind::BuiltInOverlaySourceManifestV2 => {
            canonicalize_typed::<OfficialBuiltInOverlaySourceManifestV2>(input, output)
        }
        DocumentKind::ProjectionExecutionPolicy => {
            canonicalize_typed::<OfficialProjectionExecutionPolicyV1>(input, output)
        }
        DocumentKind::VerifierSourceBinding => {
            canonicalize_typed::<VerifierSourceBindingV1>(input, output)
        }
        DocumentKind::VerifierSourceBindingV2 => {
            canonicalize_typed::<plan_v3::VerifierSourceBindingV2>(input, output)
        }
        DocumentKind::ProjectionExecutionRecordV3 => {
            canonicalize_typed::<plan_v3::OfficialProjectionExecutionRecordV3>(input, output)
        }
        DocumentKind::ProjectionAuthorityMatrixV3 => {
            canonicalize_typed::<plan_v3::OfficialProjectionAuthorityMatrixV3>(input, output)
        }
        DocumentKind::RulesConfig => canonicalize_typed::<RulesConfigIdentityV1>(input, output),
        DocumentKind::RulesetManifest => canonicalize_typed::<RulesetManifestV1>(input, output),
        DocumentKind::PublishedRuleset => canonicalize_typed::<PublishedRulesetV1>(input, output),
        DocumentKind::Competition => canonicalize_typed::<CompetitionManifestV1>(input, output),
        DocumentKind::Policy => canonicalize_typed::<ImmutablePolicyManifestV1>(input, output),
        DocumentKind::ReleaseLock => canonicalize_typed::<OperatorReleaseLockV1>(input, output),
    }
}

pub fn validate_document(kind: DocumentKind, input: &Path) -> Result<Digest32> {
    match kind {
        DocumentKind::Build => validate_typed::<BuildManifestV1>(input),
        DocumentKind::BuildV2 => validate_typed::<BuildManifestV2>(input),
        DocumentKind::BuildToolAuthority => {
            Ok(load_build_tool_authority(input)?.canonical_digest()?)
        }
        DocumentKind::JavascriptBuildToolAuthority => Ok(load_canonical_authority_document::<
            JavaScriptBuildToolAuthorityDocumentV1,
        >(input)?
        .canonical_digest()?),
        DocumentKind::ViewerBuildReportV2 => validate_typed::<OfficialViewerBuildReportV2>(input),
        DocumentKind::ProjectionAuthorityV2 => {
            validate_typed::<OfficialProjectionAuthorityManifestV2>(input)
        }
        DocumentKind::Content => validate_typed::<ContentManifestV1>(input),
        DocumentKind::CampaignContent => validate_typed::<CampaignContentManifestV1>(input),
        DocumentKind::SimulationComponent => {
            let bytes = read_regular_file_bounded(input, MAX_DOCUMENT_BYTES)?;
            SimulationContentComponentDocumentV1::from_bitcode(&bytes)?;
            Ok(Digest32::digest_bytes(bytes))
        }
        DocumentKind::SourceTreeManifest => validate_typed::<OfficialSourceTreeManifestV1>(input),
        DocumentKind::SourceTreeManifestV2 => validate_typed::<OfficialSourceTreeManifestV2>(input),
        DocumentKind::ProjectionReceipt => {
            validate_typed::<OfficialSimulationProjectionReceiptV1>(input)
        }
        DocumentKind::ProjectionReceiptV2 => {
            validate_typed::<OfficialSimulationProjectionReceiptV2>(input)
        }
        DocumentKind::BuiltInOverlaySourceManifestV2 => {
            validate_typed::<OfficialBuiltInOverlaySourceManifestV2>(input)
        }
        DocumentKind::ProjectionExecutionPolicy => {
            validate_typed::<OfficialProjectionExecutionPolicyV1>(input)
        }
        DocumentKind::VerifierSourceBinding => validate_typed::<VerifierSourceBindingV1>(input),
        DocumentKind::VerifierSourceBindingV2 => {
            validate_typed::<plan_v3::VerifierSourceBindingV2>(input)
        }
        DocumentKind::ProjectionExecutionRecordV3 => {
            validate_typed::<plan_v3::OfficialProjectionExecutionRecordV3>(input)
        }
        DocumentKind::ProjectionAuthorityMatrixV3 => {
            validate_typed::<plan_v3::OfficialProjectionAuthorityMatrixV3>(input)
        }
        DocumentKind::RulesConfig => validate_typed::<RulesConfigIdentityV1>(input),
        DocumentKind::RulesetManifest => validate_typed::<RulesetManifestV1>(input),
        DocumentKind::PublishedRuleset => validate_typed::<PublishedRulesetV1>(input),
        DocumentKind::Competition => validate_typed::<CompetitionManifestV1>(input),
        DocumentKind::Policy => validate_typed::<ImmutablePolicyManifestV1>(input),
        DocumentKind::ReleaseLock => validate_typed::<OperatorReleaseLockV1>(input),
    }
}

pub fn author_official_content(
    plan_path: &Path,
    output_directory: &Path,
) -> Result<OfficialContentDigestsV1> {
    ensure_absent_output(output_directory)?;
    let plan = OfficialProjectionPlanV1::load(plan_path)?;
    let editions = author_editions(&plan)?;
    let digests = official_content_digests(&editions)?;
    let projection_plan_sha256 = canonical_config_digest::<OfficialProjectionPlanV1>(plan_path)?;
    let staging = staging_directory(output_directory)?;
    materialize_content(staging.path(), &editions, projection_plan_sha256)?;
    write_canonical(
        &staging.path().join("official-content-digests.json"),
        &digests,
    )?;
    write_bytes(
        &staging.path().join("projection-plan.sha256"),
        projection_plan_sha256.to_string().as_bytes(),
    )?;
    make_verifier_bundles_read_only(&staging.path().join("verifier-bundles"))?;
    validate_verifier_bundle_layout(staging.path(), &editions)?;
    persist_staging(staging, output_directory)?;
    Ok(digests)
}

fn author_editions(plan: &OfficialProjectionPlanV1) -> Result<[AuthoredEdition; 2]> {
    Ok([
        author_edition(&plan.demo).context("author exact DEMO projections")?,
        author_edition(&plan.full).context("author exact FULL projections")?,
    ])
}

fn author_edition(plan: &EditionProjectionPlanV1) -> Result<AuthoredEdition> {
    let native_exporter = ProjectionExporterIdentityV1 {
        exporter_version: 1,
        source_format: ProjectionSourceFormatV1::LooseNativeV1,
    };
    let shipping_exporter = ProjectionExporterIdentityV1 {
        exporter_version: 1,
        source_format: ProjectionSourceFormatV1::ShippingDatadirV10,
    };
    let native_source = validate_projection_source(
        plan.edition,
        &plan.native_source_root,
        &plan.native_source_tree_manifest,
        &plan.native_projection_receipt,
        native_exporter,
    )
    .context("validate loose/native source receipt")?;
    let shipping_source = validate_projection_source(
        plan.edition,
        &plan.shipping_source_root,
        &plan.shipping_source_tree_manifest,
        &plan.shipping_projection_receipt,
        shipping_exporter,
    )
    .context("validate RHDDNA10 source receipt")?;
    ensure!(
        native_source.receipt.subjects == shipping_source.receipt.subjects,
        "native and RHDDNA10 receipts do not bind identical content manifests"
    );
    validate_projection_catalog_root(&plan.native_projection_root, &native_source.receipt)?;
    validate_projection_catalog_root(&plan.shipping_projection_root, &shipping_source.receipt)?;
    let mut content = BTreeMap::new();
    let mut entries = Vec::with_capacity(native_source.receipt.subjects.len());
    for subject in &native_source.receipt.subjects {
        let authored = author_subject(plan, &subject.content_manifest).with_context(|| {
            format!(
                "author projection for {:?}",
                subject.content_manifest.subject
            )
        })?;
        ensure!(
            authored.manifest == subject.content_manifest,
            "authored projection differs from its source-bound content manifest"
        );
        let digest = authored.manifest.canonical_digest()?;
        entries.push(CampaignContentEntryV1 {
            subject: authored.manifest.subject.clone(),
            content_manifest_sha256: digest,
        });
        ensure!(
            content.insert(digest, authored).is_none(),
            "two {:?} subjects produced one ambiguous content identity",
            plan.edition
        );
    }
    let campaign = CampaignContentManifestV1 {
        schema_version: 1,
        edition: plan.edition,
        entries,
    };
    campaign.validate()?;
    Ok(AuthoredEdition {
        edition: plan.edition,
        content,
        campaign,
        native_source,
        shipping_source,
    })
}

fn validate_projection_catalog_root(
    root: &Path,
    receipt: &OfficialSimulationProjectionReceiptV1,
) -> Result<()> {
    let expected = receipt
        .subjects
        .iter()
        .flat_map(|subject| {
            subject.content_manifest.components.iter().map(|component| {
                simulation_content_component_relative_path_v1(
                    &subject.content_manifest.subject,
                    component.kind,
                )
                .map(PathBuf::from)
            })
        })
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let actual = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, _)| relative)
        .collect::<BTreeSet<_>>();
    ensure!(
        actual == expected,
        "projection catalog contains a missing, extra, or misnamed component file"
    );
    Ok(())
}

fn validate_projection_source(
    edition: OfficialContentEditionV1,
    source_root: &Path,
    source_tree_manifest_path: &Path,
    receipt_path: &Path,
    expected_exporter: ProjectionExporterIdentityV1,
) -> Result<ValidatedProjectionSource> {
    let source_tree_manifest: OfficialSourceTreeManifestV1 =
        load_canonical_document(source_tree_manifest_path)?;
    ensure!(
        source_tree_manifest.edition == edition
            && source_tree_manifest.source_format == expected_exporter.source_format,
        "source-tree manifest edition/lane differs from the projection plan"
    );
    validate_raw_source_tree(source_root, &source_tree_manifest)?;
    let source_tree_manifest_sha256 = source_tree_manifest.canonical_digest()?;

    let receipt: OfficialSimulationProjectionReceiptV1 = load_canonical_document(receipt_path)?;
    ensure!(
        receipt.edition == edition
            && receipt.exporter == expected_exporter
            && receipt.source_tree_manifest_sha256 == source_tree_manifest_sha256
            && usize::try_from(receipt.source_file_count).ok()
                == Some(source_tree_manifest.files.len()),
        "projection receipt does not bind the exact exporter and raw source inventory"
    );
    validate_official_resource_locale_root(edition, &receipt)?;
    if expected_exporter.source_format == ProjectionSourceFormatV1::LooseNativeV1 {
        validate_native_resource_locale_inventory(edition, &source_tree_manifest)?;
    }
    let receipt_sha256 = receipt.canonical_digest()?;
    Ok(ValidatedProjectionSource {
        source_tree_manifest,
        source_tree_manifest_sha256,
        receipt,
        receipt_sha256,
    })
}

fn official_resource_locale_root(edition: OfficialContentEditionV1) -> &'static str {
    match edition {
        OfficialContentEditionV1::Demo => OFFICIAL_DEMO_RESOURCE_LOCALE_ROOT_V1,
        OfficialContentEditionV1::Full => OFFICIAL_FULL_RESOURCE_LOCALE_ROOT_V1,
    }
}

fn validate_official_resource_locale_root(
    edition: OfficialContentEditionV1,
    receipt: &OfficialSimulationProjectionReceiptV1,
) -> Result<()> {
    let expected = official_resource_locale_root(edition);
    ensure!(
        receipt
            .subjects
            .iter()
            .all(|subject| { subject.content_manifest.resource_locale_root.as_str() == expected }),
        "official {edition:?} projection must bind exact resource locale root {expected}"
    );
    Ok(())
}

fn validate_native_resource_locale_inventory(
    edition: OfficialContentEditionV1,
    source: &OfficialSourceTreeManifestV1,
) -> Result<()> {
    let expected = official_resource_locale_root(edition);
    let numeric_roots = source
        .files
        .iter()
        .filter_map(|file| file.path.split('/').next())
        .filter(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        numeric_roots == BTreeSet::from([expected]),
        "approved loose/native {edition:?} inventory must contain only exact resource locale root {expected}"
    );
    let expected_level_resource = format!("{expected}/data/text/level.res");
    let matching_level_resources = source
        .files
        .iter()
        .filter(|file| file.path.eq_ignore_ascii_case(&expected_level_resource))
        .count();
    ensure!(
        matching_level_resources == 1,
        "approved loose/native {edition:?} inventory must contain exactly one {expected}/Data/Text/Level.res locale authority"
    );
    Ok(())
}

fn validate_raw_source_tree(
    source_root: &Path,
    expected: &OfficialSourceTreeManifestV1,
) -> Result<()> {
    validate_mount_root(source_root)?;
    let actual = walk_regular_files(source_root)?
        .into_iter()
        .map(|(relative, absolute)| {
            let (sha256, byte_length) = hash_stable_source_file(&absolute)?;
            Ok(robin_run_protocol::OfficialSourceFileV1 {
                path: path_to_manifest(&relative)?,
                sha256,
                byte_length,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        actual == expected.files,
        "approved raw source mount inventory differs from its canonical manifest"
    );
    Ok(())
}

fn hash_stable_source_file(path: &Path) -> Result<(Digest32, u64)> {
    validate_regular_file(path)?;
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    ensure!(before.is_file(), "source inventory entry is not a file");
    let sha256 = Digest32::digest_reader(&mut file)?;
    let after = file.metadata()?;
    ensure!(
        before.len() == after.len()
            && match (before.modified(), after.modified()) {
                (Ok(before), Ok(after)) => before == after,
                _ => true,
            },
        "source file changed while it was hashed: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let current = fs::symlink_metadata(path)?;
        ensure!(
            !current.file_type().is_symlink()
                && current.dev() == after.dev()
                && current.ino() == after.ino(),
            "source file was replaced while it was hashed: {}",
            path.display()
        );
    }
    Ok((sha256, after.len()))
}

fn author_subject(
    edition: &EditionProjectionPlanV1,
    manifest: &ContentManifestV1,
) -> Result<AuthoredContent> {
    let mut components = BTreeMap::new();
    for component in &manifest.components {
        let relative =
            simulation_content_component_relative_path_v1(&manifest.subject, component.kind)?;
        let native_bytes = read_mounted_projection(
            &edition.native_projection_root,
            Path::new(&relative),
            component,
            "native",
        )?;
        let shipping_bytes = read_mounted_projection(
            &edition.shipping_projection_root,
            Path::new(&relative),
            component,
            "RHDDNA10 shipping",
        )?;
        ensure!(
            native_bytes == shipping_bytes,
            "native and RHDDNA10 shipping projections differ for {:?}; browser/native ranked reconstruction is not equivalent",
            component.kind
        );
        ensure!(
            components.insert(component.kind, native_bytes).is_none(),
            "duplicate component kind"
        );
    }
    manifest.validate()?;
    Ok(AuthoredContent {
        manifest: manifest.clone(),
        components,
    })
}

fn read_mounted_projection(
    root: &Path,
    relative: &Path,
    component: &SimulationContentComponentV1,
    source_name: &str,
) -> Result<Vec<u8>> {
    let path = resolve_mounted_file(root, relative)?;
    let bytes = read_regular_file_bounded(&path, MAX_DOCUMENT_BYTES)?;
    let document: SimulationContentComponentDocumentV1 =
        SimulationContentComponentDocumentV1::from_bitcode(&bytes)
            .with_context(|| format!("parse {source_name} component {}", path.display()))?;
    document.validate()?;
    ensure!(
        document.kind == component.kind
            && document.component_schema_version == component.component_schema_version,
        "{source_name} component declaration does not match its document"
    );
    let canonical = document.bitcode_bytes()?;
    ensure!(
        bytes == canonical,
        "{source_name} projection {} is not byte-for-byte canonical bitcode",
        path.display()
    );
    ensure!(
        Digest32::digest_bytes(&bytes) == component.artifact.sha256
            && u64::try_from(bytes.len()).ok() == Some(component.artifact.byte_length)
            && component.artifact.media_type == SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
        "{source_name} component bytes differ from their receipt artifact"
    );
    Ok(bytes)
}

fn official_content_digests(editions: &[AuthoredEdition; 2]) -> Result<OfficialContentDigestsV1> {
    ensure!(
        editions[0].edition == OfficialContentEditionV1::Demo
            && editions[1].edition == OfficialContentEditionV1::Full,
        "internal edition order is not DEMO then FULL"
    );
    let demo_catalog = editions[0].campaign.canonical_digest()?;
    let full_catalog = editions[1].campaign.canonical_digest()?;
    ensure!(
        demo_catalog != full_catalog,
        "DEMO and FULL catalogs collide"
    );
    Ok(OfficialContentDigestsV1 {
        schema_version: 1,
        demo_content_manifest_sha256: editions[0].content.keys().copied().collect(),
        full_content_manifest_sha256: editions[1].content.keys().copied().collect(),
        demo_campaign_content_manifest_sha256: demo_catalog,
        full_campaign_content_manifest_sha256: full_catalog,
    })
}

fn materialize_content(
    root: &Path,
    editions: &[AuthoredEdition; 2],
    official_projection_plan_sha256: Digest32,
) -> Result<()> {
    for edition in editions {
        for source in [&edition.native_source, &edition.shipping_source] {
            write_digest_document(
                root,
                "private/source-tree-manifests",
                source.source_tree_manifest_sha256,
                &source.source_tree_manifest,
            )?;
            write_digest_document(
                root,
                "private/projection-receipts",
                source.receipt_sha256,
                &source.receipt,
            )?;
        }
        let campaign_digest = edition.campaign.canonical_digest()?;
        write_digest_document(
            root,
            "manifests/campaign-content-manifests",
            campaign_digest,
            &edition.campaign,
        )?;
        // Catalogs and static manifest refs contain no retail payload and are
        // safe for clients that reconstruct FULL from user-local content.
        write_digest_document(
            &root.join("public"),
            "manifests/campaign-content-manifests",
            campaign_digest,
            &edition.campaign,
        )?;
        for (content_digest, authored) in &edition.content {
            let binding = VerifierSourceBindingV1 {
                schema_version: 1,
                content_manifest_sha256: *content_digest,
                official_projection_plan_sha256,
                edition: authored.manifest.edition,
                subject: authored.manifest.subject.clone(),
                native_projection_receipt_sha256: edition.native_source.receipt_sha256,
                native_source_tree_manifest_sha256: edition
                    .native_source
                    .source_tree_manifest_sha256,
            };
            write_canonical(
                &root
                    .join("private/verifier-source-bindings")
                    .join(format!("{content_digest}.json")),
                &binding,
            )?;
            write_digest_document(
                root,
                "manifests/content-manifests",
                *content_digest,
                &authored.manifest,
            )?;
            write_digest_document(
                &root.join("public"),
                "manifests/content-manifests",
                *content_digest,
                &authored.manifest,
            )?;
            let bundle = root
                .join("verifier-bundles")
                .join(content_digest.to_string());
            write_canonical(&bundle.join("manifest.json"), &authored.manifest)?;
            for component in &authored.manifest.components {
                let bytes = authored.components.get(&component.kind).ok_or_else(|| {
                    anyhow::anyhow!("missing authored component {:?}", component.kind)
                })?;
                ensure!(
                    Digest32::digest_bytes(bytes) == component.artifact.sha256
                        && u64::try_from(bytes.len()).ok() == Some(component.artifact.byte_length),
                    "component bytes changed after authoring"
                );
                let relative = simulation_content_component_relative_path_v1(
                    &authored.manifest.subject,
                    component.kind,
                )?;
                write_bytes(&bundle.join("catalog").join(relative), bytes)?;
                if edition.edition == OfficialContentEditionV1::Demo {
                    let relative = demo_content_object_path_v1(*content_digest, component)?;
                    write_shared_bytes(&root.join("public").join(relative), bytes)?;
                }
            }
        }
    }
    Ok(())
}

pub fn assemble_release(plan_path: &Path, output_directory: &Path) -> Result<Digest32> {
    ensure_absent_output(output_directory)?;
    let loaded = load_release(plan_path)?;
    validate_release_closure(&loaded)?;
    let staging = staging_directory(output_directory)?;
    materialize_release(staging.path(), &loaded)?;
    let lock = release_lock(staging.path(), &loaded)?;
    let lock_bytes = lock.canonical_bytes()?;
    let lock_digest = Digest32::digest_bytes(&lock_bytes);
    write_bytes(&staging.path().join("release-lock.json"), &lock_bytes)?;
    write_bytes(
        &staging.path().join("release-lock.sha256"),
        lock_digest.to_string().as_bytes(),
    )?;
    make_verifier_bundles_read_only(&staging.path().join("verifier-bundles"))?;
    validate_verifier_bundle_layout(staging.path(), &loaded.editions)?;
    persist_staging(staging, output_directory)?;
    Ok(lock_digest)
}

pub fn validate_release(plan_path: &Path, release_directory: &Path) -> Result<Digest32> {
    validate_mount_root(release_directory).context("validate release directory")?;
    let loaded = load_release(plan_path)?;
    validate_release_closure(&loaded)?;
    let expected = tempfile::Builder::new()
        .prefix("robin-manifestctl-validate-")
        .tempdir()
        .context("create validation tree")?;
    materialize_release(expected.path(), &loaded)?;
    let lock = release_lock(expected.path(), &loaded)?;
    let lock_bytes = lock.canonical_bytes()?;
    let digest = Digest32::digest_bytes(&lock_bytes);
    write_bytes(&expected.path().join("release-lock.json"), &lock_bytes)?;
    write_bytes(
        &expected.path().join("release-lock.sha256"),
        digest.to_string().as_bytes(),
    )?;
    compare_trees(expected.path(), release_directory)?;
    validate_verifier_bundle_layout(release_directory, &loaded.editions)?;
    Ok(digest)
}

fn validate_verifier_bundle_layout(root: &Path, editions: &[AuthoredEdition; 2]) -> Result<()> {
    for edition in editions {
        for (content_digest, authored) in &edition.content {
            let bundle = root
                .join("verifier-bundles")
                .join(content_digest.to_string());
            let root_entries = fs::read_dir(&bundle)?.collect::<std::io::Result<Vec<_>>>()?;
            let root_names = root_entries
                .iter()
                .map(|entry| {
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("verifier bundle filename is not UTF-8"))
                })
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                root_names == BTreeSet::from(["catalog".to_owned(), "manifest.json".to_owned()]),
                "verifier bundle must contain only manifest.json and catalog/"
            );
            let manifest: ContentManifestV1 =
                load_canonical_document(&bundle.join("manifest.json"))?;
            ensure!(
                manifest == authored.manifest && manifest.canonical_digest()? == *content_digest,
                "verifier bundle manifest does not match its content address"
            );
            let expected_paths = authored
                .manifest
                .components
                .iter()
                .map(|component| {
                    simulation_content_component_relative_path_v1(
                        &authored.manifest.subject,
                        component.kind,
                    )
                })
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            let actual_paths = walk_regular_files(&bundle.join("catalog"))?
                .into_iter()
                .map(|(relative, _)| path_to_manifest(&relative))
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                actual_paths == expected_paths,
                "verifier catalog must contain exactly the eight canonical component paths"
            );
            for component in &authored.manifest.components {
                let relative = simulation_content_component_relative_path_v1(
                    &authored.manifest.subject,
                    component.kind,
                )?;
                let path = bundle.join("catalog").join(relative);
                ensure!(
                    artifact_from_file(&path, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1)?
                        == component.artifact,
                    "verifier component bytes do not match manifest"
                );
                let document = SimulationContentComponentDocumentV1::from_bitcode(
                    &read_regular_file_bounded(&path, MAX_DOCUMENT_BYTES)?,
                )?;
                ensure!(
                    document.kind == component.kind
                        && document.component_schema_version == component.component_schema_version,
                    "verifier component document does not match its typed reference"
                );
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                ensure!(
                    fs::metadata(&bundle)?.permissions().mode() & 0o222 == 0
                        && walk_regular_files(&bundle)?.iter().all(|(_, file)| {
                            fs::metadata(file)
                                .is_ok_and(|metadata| metadata.permissions().mode() & 0o222 == 0)
                        }),
                    "verifier bundle is not read-only"
                );
            }
        }
    }
    Ok(())
}

fn load_release(plan_path: &Path) -> Result<LoadedRelease> {
    let source_plan_sha256 = canonical_config_digest::<OperatorReleasePlanV1>(plan_path)?;
    let plan = OperatorReleasePlanV1::load(plan_path)?;
    let official_projection_plan_sha256 =
        canonical_config_digest::<OfficialProjectionPlanV1>(&plan.official_projection_plan)?;
    let projections = OfficialProjectionPlanV1::load(&plan.official_projection_plan)?;
    let editions = author_editions(&projections)?;

    let mut builds = BTreeMap::new();
    for source in &plan.builds {
        let draft = BuildDraftV1::load(&source.draft)?;
        let authored = draft.author()?;
        let expected: BuildManifestV1 = load_canonical_document(&source.expected_manifest)?;
        ensure!(
            authored == expected,
            "fresh build/artifact identity does not match pinned manifest {}",
            source.expected_manifest.display()
        );
        let digest = expected.canonical_digest()?;
        ensure!(
            builds
                .insert(
                    digest,
                    LoadedBuild {
                        manifest: expected,
                        draft,
                    },
                )
                .is_none(),
            "duplicate build manifest {digest}"
        );
    }
    let rules_configs = load_canonical_documents(&plan.rules_configs, |document| {
        let document: RulesConfigIdentityV1 = document;
        Ok((document.canonical_digest()?, document))
    })?;
    let policies = load_canonical_documents(&plan.policies, |document| {
        let document: ImmutablePolicyManifestV1 = document;
        Ok((document.canonical_digest()?, document))
    })?;
    let published_rulesets = load_canonical_documents(&plan.published_rulesets, |document| {
        let document: PublishedRulesetV1 = document;
        Ok((document.ruleset_manifest_sha256, document))
    })?;
    let ruleset_manifests = published_rulesets
        .iter()
        .map(|(digest, published)| (*digest, published.manifest.clone()))
        .collect();
    let competitions = load_canonical_documents(&plan.competitions, |document| {
        let document: CompetitionManifestV1 = document;
        Ok((document.canonical_digest()?, document))
    })?;
    Ok(LoadedRelease {
        source_plan_sha256,
        official_projection_plan_sha256,
        editions,
        builds,
        rules_configs,
        policies,
        ruleset_manifests,
        published_rulesets,
        competitions,
    })
}

fn validate_release_closure(loaded: &LoadedRelease) -> Result<()> {
    let content = loaded
        .editions
        .iter()
        .flat_map(|edition| {
            edition
                .content
                .iter()
                .map(|(digest, content)| (*digest, &content.manifest))
        })
        .collect::<BTreeMap<_, _>>();
    let catalogs = loaded
        .editions
        .iter()
        .map(|edition| Ok((edition.campaign.canonical_digest()?, &edition.campaign)))
        .collect::<Result<BTreeMap<_, _>>>()?;

    for (digest, published) in &loaded.published_rulesets {
        published.validate()?;
        ensure!(
            published.manifest.canonical_digest()? == *digest,
            "published ruleset does not cross-bind its immutable manifest"
        );
        let ruleset = &published.manifest;
        validate_official_ranked_input_provenance(
            ruleset.input_provenance_eligibility,
            &ruleset.replay_schema_versions,
        )
        .with_context(|| format!("ruleset {digest} uses a historical input provenance lane"))?;
        ensure!(
            ruleset.achievement_policies == official_achievement_policies_v1(),
            "official ruleset {digest} does not pin the exact four Required achievement policies"
        );
        ensure!(
            loaded
                .rules_configs
                .contains_key(&ruleset.rules_config_sha256),
            "ruleset {digest} references an absent rules config"
        );
        ensure!(
            ruleset
                .allowed_build_manifest_sha256
                .iter()
                .all(|candidate| loaded.builds.contains_key(candidate)),
            "ruleset {digest} references an absent build"
        );
        ensure!(
            ruleset
                .allowed_content_manifest_sha256
                .iter()
                .all(|candidate| content.contains_key(candidate)),
            "ruleset {digest} references an absent official content manifest"
        );
        let allowed_editions = ruleset
            .allowed_content_manifest_sha256
            .iter()
            .map(|candidate| content[candidate].edition)
            .collect::<BTreeSet<_>>();
        ensure!(
            allowed_editions.len() == 1,
            "ruleset {digest} must target exactly one official edition"
        );
        let edition = *allowed_editions.iter().next().expect("one edition checked");
        let authored_edition = loaded
            .editions
            .iter()
            .find(|authored| authored.edition == edition)
            .expect("both official editions are always authored");
        ensure!(
            ruleset.allowed_content_manifest_sha256
                == authored_edition.content.keys().copied().collect::<Vec<_>>(),
            "ruleset {digest} must allow the complete exact {:?} content matrix",
            edition
        );
        ensure!(
            ruleset
                .allowed_campaign_content_manifest_sha256
                .iter()
                .all(|candidate| catalogs.contains_key(candidate)),
            "ruleset {digest} references an absent official campaign catalog"
        );
        let advertises_full_campaign = ruleset
            .board_scopes
            .binary_search(&RulesetBoardScopeV1::FullCampaign)
            .is_ok();
        match edition {
            OfficialContentEditionV1::Demo => ensure!(
                !advertises_full_campaign
                    && ruleset.allowed_campaign_content_manifest_sha256.is_empty(),
                "DEMO ruleset {digest} must not advertise FullCampaign or import FULL HQ content"
            ),
            OfficialContentEditionV1::Full if advertises_full_campaign => ensure!(
                ruleset.allowed_campaign_content_manifest_sha256
                    == vec![authored_edition.campaign.canonical_digest()?],
                "FULL ruleset {digest} must bind the exact official campaign catalog"
            ),
            OfficialContentEditionV1::Full => ensure!(
                ruleset.allowed_campaign_content_manifest_sha256.is_empty(),
                "non-FullCampaign ruleset {digest} has a campaign catalog"
            ),
        }
        for identity in [
            &ruleset.input_provenance_policy,
            &ruleset.command_admission_policy,
            &ruleset.submission_admission_policy,
            &ruleset.verifier_policy,
        ] {
            let policy = loaded
                .policies
                .get(&identity.manifest_sha256)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "ruleset {digest} references absent policy {:?}",
                        identity.kind
                    )
                })?;
            ensure!(
                policy.kind == identity.kind && policy.version == identity.version,
                "ruleset {digest} policy identity does not match policy document"
            );
        }
        for build_digest in &ruleset.allowed_build_manifest_sha256 {
            let build = &loaded
                .builds
                .get(build_digest)
                .expect("membership checked")
                .manifest;
            let config = loaded
                .rules_configs
                .get(&ruleset.rules_config_sha256)
                .expect("membership checked");
            ensure!(
                ruleset
                    .replay_schema_versions
                    .binary_search(&build.replay_schema_version)
                    .is_ok()
                    && ruleset
                        .network_protocol_versions
                        .binary_search(&build.network_protocol_version)
                        .is_ok()
                    && config.replay_schema_version == build.replay_schema_version
                    && build.replay_schema_version == CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
                "ruleset {digest} build/config schema closure is inconsistent"
            );
        }
        for catalog_digest in &ruleset.allowed_campaign_content_manifest_sha256 {
            let catalog = catalogs.get(catalog_digest).expect("membership checked");
            for entry in &catalog.entries {
                let manifest = content
                    .get(&entry.content_manifest_sha256)
                    .ok_or_else(|| anyhow::anyhow!("catalog constituent is absent"))?;
                ensure!(
                    manifest.edition == catalog.edition && manifest.subject == entry.subject,
                    "campaign catalog edition/subject does not match constituent manifest"
                );
            }
        }
    }

    for (digest, competition) in &loaded.competitions {
        competition.validate()?;
        let ruleset = loaded
            .ruleset_manifests
            .get(&competition.ruleset_manifest_sha256)
            .ok_or_else(|| anyhow::anyhow!("competition {digest} references absent ruleset"))?;
        ensure!(
            competition.rules_config_sha256 == ruleset.rules_config_sha256,
            "competition {digest} rules config differs from ruleset"
        );
        let content_exists = match competition.content {
            RunContentIdentityV1::Mission {
                content_manifest_sha256,
            } => {
                content.contains_key(&content_manifest_sha256)
                    && ruleset
                        .allowed_content_manifest_sha256
                        .binary_search(&content_manifest_sha256)
                        .is_ok()
            }
            RunContentIdentityV1::FullCampaign {
                campaign_content_manifest_sha256,
            } => {
                catalogs.contains_key(&campaign_content_manifest_sha256)
                    && ruleset
                        .allowed_campaign_content_manifest_sha256
                        .binary_search(&campaign_content_manifest_sha256)
                        .is_ok()
                    && ruleset
                        .board_scopes
                        .binary_search(&RulesetBoardScopeV1::FullCampaign)
                        .is_ok()
            }
        };
        ensure!(
            content_exists,
            "competition {digest} content tuple is not allowed"
        );
    }
    Ok(())
}

fn validate_official_ranked_input_provenance(
    eligibility: InputProvenanceEligibilityV1,
    replay_schema_versions: &[u32],
) -> Result<()> {
    ensure!(
        eligibility == InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly
            && replay_schema_versions == [CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1],
        "official ranked releases require one current-schema canonical replay"
    );
    Ok(())
}

fn materialize_release(root: &Path, loaded: &LoadedRelease) -> Result<()> {
    materialize_content(
        root,
        &loaded.editions,
        loaded.official_projection_plan_sha256,
    )?;
    for (digest, build) in &loaded.builds {
        write_digest_document(root, "manifests/builds", *digest, &build.manifest)?;
        copy_artifact_exact(
            &build.draft.verifier.source,
            &root
                .join("private/build-artifacts")
                .join(build.manifest.verifier.sha256.to_string()),
            &build.manifest.verifier,
        )?;
        let sources = build
            .draft
            .viewer_artifacts
            .iter()
            .map(|source| (source.published_path.as_str(), source))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            sources.len() == build.draft.viewer_artifacts.len(),
            "build draft repeats a viewer published path"
        );
        for artifact in &build.manifest.viewer_artifacts {
            let source = sources
                .get(artifact.path.as_str())
                .ok_or_else(|| anyhow::anyhow!("viewer artifact source disappeared"))?;
            let relative = build_artifact_object_path_v1(*digest, artifact)?;
            copy_artifact_exact(
                &source.source,
                &root.join("public").join(relative),
                &artifact.artifact,
            )?;
        }
    }
    for (digest, document) in &loaded.rules_configs {
        write_digest_document(root, "manifests/rules-configs", *digest, document)?;
    }
    for (digest, document) in &loaded.policies {
        write_digest_document(root, "manifests/policies", *digest, document)?;
    }
    for (digest, document) in &loaded.ruleset_manifests {
        write_digest_document(root, "manifests/ruleset-manifests", *digest, document)?;
    }
    for (digest, document) in &loaded.published_rulesets {
        write_digest_document(root, "manifests/published-rulesets", *digest, document)?;
    }
    for (digest, document) in &loaded.competitions {
        write_digest_document(root, "manifests/competitions", *digest, document)?;
    }
    Ok(())
}

fn release_lock(root: &Path, loaded: &LoadedRelease) -> Result<OperatorReleaseLockV1> {
    let mut files = Vec::new();
    for (relative, absolute) in walk_regular_files(root)? {
        let path = path_to_manifest(&relative)?;
        let exposure = if path.starts_with("public/") {
            ReleaseFileExposureV1::PublicStatic
        } else if path.starts_with("private/") || path.starts_with("verifier-bundles/") {
            ReleaseFileExposureV1::OperatorPrivate
        } else {
            ReleaseFileExposureV1::BackendManifest
        };
        files.push(ReleaseFileV1 {
            path,
            artifact: artifact_from_file(&absolute, "application/octet-stream")?,
            exposure,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    ensure!(
        files.windows(2).all(|pair| pair[0].path < pair[1].path),
        "release file inventory is not unique"
    );
    let projection_receipt_sha256 = loaded
        .editions
        .iter()
        .flat_map(|edition| {
            [
                edition.native_source.receipt_sha256,
                edition.shipping_source.receipt_sha256,
            ]
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let source_tree_manifest_sha256 = loaded
        .editions
        .iter()
        .flat_map(|edition| {
            [
                edition.native_source.source_tree_manifest_sha256,
                edition.shipping_source.source_tree_manifest_sha256,
            ]
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(OperatorReleaseLockV1 {
        schema_version: RELEASE_LOCK_SCHEMA_VERSION,
        source_plan_sha256: loaded.source_plan_sha256,
        official_projection_plan_sha256: loaded.official_projection_plan_sha256,
        official_content: official_content_digests(&loaded.editions)?,
        projection_receipt_sha256,
        source_tree_manifest_sha256,
        build_manifest_sha256: loaded.builds.keys().copied().collect(),
        rules_config_sha256: loaded.rules_configs.keys().copied().collect(),
        policy_manifest_sha256: loaded.policies.keys().copied().collect(),
        ruleset_manifest_sha256: loaded.ruleset_manifests.keys().copied().collect(),
        competition_manifest_sha256: loaded.competitions.keys().copied().collect(),
        files,
    })
}

fn compare_trees(expected: &Path, actual: &Path) -> Result<()> {
    let expected_files = walk_regular_files(expected)?
        .into_iter()
        .map(|(relative, absolute)| Ok((path_to_manifest(&relative)?, absolute)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let actual_files = walk_regular_files(actual)?
        .into_iter()
        .map(|(relative, absolute)| Ok((path_to_manifest(&relative)?, absolute)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    ensure!(
        expected_files.keys().eq(actual_files.keys()),
        "release tree contains a missing or extra file"
    );
    for (path, expected_file) in expected_files {
        let actual_file = actual_files.get(&path).expect("key sets checked");
        let expected_artifact = artifact_from_file(&expected_file, "application/octet-stream")?;
        let actual_artifact = artifact_from_file(actual_file, "application/octet-stream")?;
        ensure!(
            expected_artifact == actual_artifact,
            "release file {path} differs from deterministic regeneration"
        );
    }
    Ok(())
}

fn load_canonical_documents<T, F>(
    paths: &[PathBuf],
    mut identity: F,
) -> Result<BTreeMap<Digest32, T>>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
    F: FnMut(T) -> Result<(Digest32, T)>,
{
    let mut documents = BTreeMap::new();
    for path in paths {
        let document: T = load_canonical_document(path)?;
        let (digest, document) = identity(document)?;
        ensure!(
            documents.insert(digest, document).is_none(),
            "duplicate canonical document digest {digest}"
        );
    }
    Ok(documents)
}

fn load_canonical_document<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical document {}", path.display()))?;
    document.validate()?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "{} is valid but not byte-for-byte canonical JSON",
        path.display()
    );
    Ok(document)
}

fn canonical_config_digest<T>(path: &Path) -> Result<Digest32>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse operator config {}", path.display()))?;
    Ok(Digest32::digest_bytes(canonical_json_bytes(&document)?))
}

fn canonicalize_typed<T>(input: &Path, output: &Path) -> Result<AuthoredDocument>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let bytes = read_regular_file_bounded(input, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse document {}", input.display()))?;
    write_authored_document(output, &document)
}

fn validate_typed<T>(input: &Path) -> Result<Digest32>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let document: T = load_canonical_document(input)?;
    Ok(Digest32::digest_bytes(canonical_json_bytes(&document)?))
}

fn write_authored_document<T>(output: &Path, document: &T) -> Result<AuthoredDocument>
where
    T: Serialize + robin_run_protocol::Validate,
{
    document.validate()?;
    let canonical_bytes = canonical_json_bytes(document)?;
    let digest = Digest32::digest_bytes(&canonical_bytes);
    write_bytes(output, &canonical_bytes)?;
    Ok(AuthoredDocument {
        digest,
        canonical_bytes,
    })
}

fn write_digest_document<T>(root: &Path, kind: &str, digest: Digest32, document: &T) -> Result<()>
where
    T: Serialize + robin_run_protocol::Validate,
{
    document.validate()?;
    let bytes = canonical_json_bytes(document)?;
    ensure!(
        Digest32::digest_bytes(&bytes) == digest || kind.ends_with("published-rulesets"),
        "immutable document path digest does not match canonical bytes"
    );
    // PublishedRulesetV1 is intentionally addressed by its embedded immutable
    // ruleset digest; its mutable operational status is never immutable-cache
    // content despite sharing the lookup key.
    write_bytes(&root.join(kind).join(format!("{digest}.json")), &bytes)
}

fn write_canonical<T>(path: &Path, document: &T) -> Result<()>
where
    T: Serialize + robin_run_protocol::Validate,
{
    document.validate()?;
    write_bytes(path, &canonical_json_bytes(document)?)
}

fn artifact_from_file(path: &Path, media_type: &str) -> Result<ArtifactRefV1> {
    ensure!(
        !media_type.trim().is_empty(),
        "artifact media type must not be empty"
    );
    let metadata = validate_regular_file(path)?;
    let reader = BufReader::new(File::open(path)?);
    Ok(ArtifactRefV1 {
        sha256: Digest32::digest_reader(reader)?,
        byte_length: metadata.len(),
        media_type: media_type.into(),
    })
}

fn artifact_from_bytes(bytes: &[u8], media_type: &str) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: bytes.len() as u64,
        media_type: media_type.into(),
    }
}

fn hash_regular_file(path: &Path) -> Result<Digest32> {
    Ok(artifact_from_file(path, "application/octet-stream")?.sha256)
}

fn copy_artifact_exact(source: &Path, output: &Path, expected: &ArtifactRefV1) -> Result<()> {
    ensure!(
        artifact_from_file(source, &expected.media_type)? == *expected,
        "artifact source {} does not match its manifest identity",
        source.display()
    );
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let input = File::open(source)?;
    let mut input = BufReader::new(input);
    let output_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .with_context(|| format!("create artifact {}", output.display()))?;
    let mut output_file = BufWriter::new(output_file);
    std::io::copy(&mut input, &mut output_file)?;
    output_file.flush()?;
    output_file.get_ref().sync_all()?;
    ensure!(
        artifact_from_file(output, &expected.media_type)? == *expected,
        "copied artifact failed post-write digest verification"
    );
    Ok(())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writer.write_all(bytes)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn write_shared_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        ensure!(
            read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)? == bytes,
            "content-addressed object collision at {}",
            path.display()
        );
        return Ok(());
    }
    write_bytes(path, bytes)
}

fn validate_mount_root(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "mount must be an absolute path");
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("required mount {} is absent", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "required mount {} must be a non-symlink directory",
        path.display()
    );
    ensure!(
        fs::canonicalize(path)? == path,
        "required mount {} must be a normalized absolute path",
        path.display()
    );
    Ok(())
}

fn validate_regular_file(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("required file {} is absent", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "{} must be a regular non-symlink file",
        path.display()
    );
    Ok(metadata)
}

fn validate_relative_source_path(path: &Path) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "source path is empty");
    ensure!(!path.is_absolute(), "source path must be relative");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "source path contains '.', '..', a prefix, or a root"
    );
    path_to_manifest(path)?;
    Ok(())
}

fn resolve_mounted_file(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_relative_source_path(relative)?;
    let canonical_root = fs::canonicalize(root)?;
    let candidate = root.join(relative);
    validate_regular_file(&candidate)?;
    let canonical_candidate = fs::canonicalize(&candidate)?;
    ensure!(
        canonical_candidate.starts_with(&canonical_root),
        "mounted projection path escapes its declared root"
    );
    Ok(candidate)
}

fn read_regular_file_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let metadata = validate_regular_file(path)?;
    ensure!(
        metadata.len() <= maximum,
        "{} exceeds the {} byte operator-document limit",
        path.display(),
        maximum
    );
    let capacity = usize::try_from(metadata.len()).context("file length does not fit usize")?;
    let mut bytes = Vec::with_capacity(capacity);
    File::open(path)?.read_to_end(&mut bytes)?;
    ensure!(
        u64::try_from(bytes.len()).ok() == Some(metadata.len()),
        "{} changed while it was read",
        path.display()
    );
    Ok(bytes)
}

fn walk_regular_files(root: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut pending = vec![(PathBuf::new(), root.to_path_buf())];
    let mut files = Vec::new();
    while let Some((relative_root, absolute_root)) = pending.pop() {
        let mut entries = fs::read_dir(&absolute_root)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "tree contains forbidden symlink {}",
                entry.path().display()
            );
            let relative = relative_root.join(entry.file_name());
            if metadata.is_dir() {
                pending.push((relative, entry.path()));
            } else {
                ensure!(
                    metadata.is_file(),
                    "tree contains non-regular entry {}",
                    entry.path().display()
                );
                files.push((relative, entry.path()));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn path_to_manifest(path: &Path) -> Result<String> {
    validate_relative_source_path_shallow(path)?;
    let mut output = String::new();
    for (index, component) in path.components().enumerate() {
        let Component::Normal(component) = component else {
            bail!("path is not canonical relative")
        };
        let component = component
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("path is not UTF-8"))?;
        ensure!(
            !component.contains(['/', '\\']) && !component.is_empty(),
            "path component is invalid"
        );
        if index != 0 {
            output.push('/');
        }
        output.push_str(component);
    }
    ensure!(!output.is_empty(), "path is empty");
    Ok(output)
}

fn validate_relative_source_path_shallow(path: &Path) -> Result<()> {
    ensure!(!path.is_absolute(), "path is absolute");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "path is not canonical relative"
    );
    Ok(())
}

fn resolve_path(base: &Path, path: &mut PathBuf) {
    if path.is_relative() {
        *path = base.join(&*path);
    }
}

fn config_parent(path: &Path) -> Result<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("operator config path has no parent"))
}

fn ensure_absent_output(path: &Path) -> Result<()> {
    ensure!(
        !path.exists(),
        "output {} already exists; releases are immutable and never overwritten",
        path.display()
    );
    Ok(())
}

fn staging_directory(output: &Path) -> Result<tempfile::TempDir> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("output path has no parent"))?;
    fs::create_dir_all(parent)?;
    tempfile::Builder::new()
        .prefix(".robin-manifestctl-")
        .tempdir_in(parent)
        .context("create same-filesystem staging directory")
}

fn persist_staging(staging: tempfile::TempDir, output: &Path) -> Result<()> {
    sync_directory_tree(staging.path())?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging.path(),
        rustix::fs::CWD,
        output,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .with_context(|| {
        format!(
            "atomically publish {} as {}",
            staging.path().display(),
            output.display()
        )
    })?;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        ensure_absent_output(output)?;
        fs::rename(staging.path(), output).with_context(|| {
            format!(
                "atomically publish {} as {}",
                staging.path().display(),
                output.display()
            )
        })?;
    }
    if let Some(parent) = output.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn sync_directory_tree(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
                directories.push(entry.path());
            }
        }
    }
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
    for directory in directories {
        File::open(directory)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn make_verifier_bundles_read_only(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if !root.exists() {
        return Ok(());
    }
    let mut directories = vec![root.to_path_buf()];
    for (_, file) in walk_regular_files(root)? {
        fs::set_permissions(&file, fs::Permissions::from_mode(0o444))?;
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
                directories.push(entry.path());
            }
        }
    }
    directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
    for directory in directories {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o555))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_verifier_bundles_read_only(root: &Path) -> Result<()> {
    for (_, file) in walk_regular_files(root)? {
        let mut permissions = fs::metadata(&file)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(file, permissions)?;
    }
    Ok(())
}

#[derive(Debug)]
struct StrictJsonValue(serde_json::Value);

impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJsonValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .map(StrictJsonValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(StrictJsonValue(value.to_owned().into()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictJsonValue>()? {
            values.push(value.0);
        }
        Ok(StrictJsonValue(serde_json::Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            ensure_serde(
                !values.contains_key(&key),
                format!("duplicate JSON object key {key:?}"),
            )?;
            values.insert(key, map.next_value::<StrictJsonValue>()?.0);
        }
        Ok(StrictJsonValue(serde_json::Value::Object(values)))
    }
}

fn ensure_serde<E: serde::de::Error>(condition: bool, message: String) -> Result<(), E> {
    if condition {
        Ok(())
    } else {
        Err(E::custom(message))
    }
}

fn strict_json_from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJsonValue::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(serde_json::from_value(value.0)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        CanonicalValue, ContentClosureKindV1, SimulationSpeechTimingSourceV1,
        official_content_manifest_name_v1,
    };

    struct ProjectionFixture {
        _source_native_demo: tempfile::TempDir,
        _native_demo: tempfile::TempDir,
        _source_shipping_demo: tempfile::TempDir,
        _shipping_demo: tempfile::TempDir,
        _source_native_full: tempfile::TempDir,
        _native_full: tempfile::TempDir,
        _source_shipping_full: tempfile::TempDir,
        _shipping_full: tempfile::TempDir,
        config: tempfile::TempDir,
        plan: OfficialProjectionPlanV1,
        plan_path: PathBuf,
    }

    impl ProjectionFixture {
        fn new() -> Result<Self> {
            let source_native_demo = tempfile::tempdir()?;
            let native_demo = tempfile::tempdir()?;
            let source_shipping_demo = tempfile::tempdir()?;
            let shipping_demo = tempfile::tempdir()?;
            let source_native_full = tempfile::tempdir()?;
            let native_full = tempfile::tempdir()?;
            let source_shipping_full = tempfile::tempdir()?;
            let shipping_full = tempfile::tempdir()?;
            let config = tempfile::tempdir()?;
            let demo = edition_plan(
                OfficialContentEditionV1::Demo,
                source_native_demo.path(),
                native_demo.path(),
                source_shipping_demo.path(),
                shipping_demo.path(),
                config.path(),
            )?;
            let full = edition_plan(
                OfficialContentEditionV1::Full,
                source_native_full.path(),
                native_full.path(),
                source_shipping_full.path(),
                shipping_full.path(),
                config.path(),
            )?;
            let plan = OfficialProjectionPlanV1 {
                schema_version: PROJECTION_PLAN_SCHEMA_VERSION,
                demo,
                full,
            };
            let plan_path = config.path().join("official-projections.json");
            write_plan(&plan_path, &plan)?;
            Ok(Self {
                _source_native_demo: source_native_demo,
                _native_demo: native_demo,
                _source_shipping_demo: source_shipping_demo,
                _shipping_demo: shipping_demo,
                _source_native_full: source_native_full,
                _native_full: native_full,
                _source_shipping_full: source_shipping_full,
                _shipping_full: shipping_full,
                config,
                plan,
                plan_path,
            })
        }

        fn rewrite_plan(&self) -> Result<()> {
            fs::write(&self.plan_path, serde_json::to_vec(&self.plan)?)?;
            Ok(())
        }
    }

    fn write_plan(path: &Path, plan: &OfficialProjectionPlanV1) -> Result<()> {
        fs::write(path, serde_json::to_vec(plan)?)?;
        Ok(())
    }

    fn edition_plan(
        edition: OfficialContentEditionV1,
        native_source_root: &Path,
        native_root: &Path,
        shipping_source_root: &Path,
        shipping_root: &Path,
        config_root: &Path,
    ) -> Result<EditionProjectionPlanV1> {
        let authored_subjects = official_content_subjects_v1(edition)
            .into_iter()
            .map(|subject| {
                let mut component_references = Vec::new();
                for kind in required_component_kinds() {
                    let relative = PathBuf::from(simulation_content_component_relative_path_v1(
                        &subject, kind,
                    )?);
                    let document = SimulationContentComponentDocumentV1 {
                        schema_version: 1,
                        kind,
                        component_schema_version: 1,
                        payload: CanonicalValue::String(format!(
                            "{edition:?}:{}:{kind:?}",
                            subject.mission_id()
                        )),
                    };
                    let bytes = document.bitcode_bytes()?;
                    component_references.push(SimulationContentComponentV1 {
                        kind,
                        component_schema_version: 1,
                        artifact: ArtifactRefV1 {
                            sha256: Digest32::digest_bytes(&bytes),
                            byte_length: u64::try_from(bytes.len())?,
                            media_type: SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1.into(),
                        },
                    });
                    for root in [native_root, shipping_root] {
                        let path = root.join(&relative);
                        fs::create_dir_all(path.parent().expect("component has a parent"))?;
                        fs::write(path, &bytes)?;
                    }
                }
                let name = official_content_manifest_name_v1(edition, &subject);
                let content_manifest = ContentManifestV1 {
                    schema_version: 1,
                    name: name.clone(),
                    edition,
                    subject: subject.clone(),
                    closure: ContentClosureKindV1::StaticPreparedMissionContentProjection,
                    projection_schema_version: 2,
                    resource_locale_root: robin_run_protocol::ResourceLocaleRootV1::new(
                        match edition {
                            OfficialContentEditionV1::Demo => "1033",
                            OfficialContentEditionV1::Full => "2047",
                        },
                    )?,
                    speech_timing: SimulationSpeechTimingSourceV1::BaseInstallation,
                    components: component_references,
                };
                Ok(robin_run_protocol::OfficialProjectionSubjectReceiptV1 { content_manifest })
            })
            .collect::<Result<Vec<_>>>()?;
        let (native_source_tree_manifest, native_source_tree_manifest_path) =
            write_source_tree_manifest(
                edition,
                ProjectionSourceFormatV1::LooseNativeV1,
                native_source_root,
                config_root,
            )?;
        let (shipping_source_tree_manifest, shipping_source_tree_manifest_path) =
            write_source_tree_manifest(
                edition,
                ProjectionSourceFormatV1::ShippingDatadirV10,
                shipping_source_root,
                config_root,
            )?;
        let native_exporter = ProjectionExporterIdentityV1 {
            exporter_version: 1,
            source_format: ProjectionSourceFormatV1::LooseNativeV1,
        };
        let shipping_exporter = ProjectionExporterIdentityV1 {
            exporter_version: 1,
            source_format: ProjectionSourceFormatV1::ShippingDatadirV10,
        };
        let native_projection_receipt = write_projection_receipt(
            edition,
            native_exporter,
            &native_source_tree_manifest,
            &authored_subjects,
            config_root,
        )?;
        let shipping_projection_receipt = write_projection_receipt(
            edition,
            shipping_exporter,
            &shipping_source_tree_manifest,
            &authored_subjects,
            config_root,
        )?;
        Ok(EditionProjectionPlanV1 {
            edition,
            native_source_root: native_source_root.to_path_buf(),
            native_source_tree_manifest: native_source_tree_manifest_path,
            native_projection_receipt,
            native_projection_root: native_root.to_path_buf(),
            shipping_source_root: shipping_source_root.to_path_buf(),
            shipping_source_tree_manifest: shipping_source_tree_manifest_path,
            shipping_projection_receipt,
            shipping_projection_root: shipping_root.to_path_buf(),
        })
    }

    fn write_source_tree_manifest(
        edition: OfficialContentEditionV1,
        source_format: ProjectionSourceFormatV1,
        source_root: &Path,
        config_root: &Path,
    ) -> Result<(OfficialSourceTreeManifestV1, PathBuf)> {
        let relative = match source_format {
            ProjectionSourceFormatV1::LooseNativeV1 => PathBuf::from(format!(
                "{}/Data/Text/Level.res",
                official_resource_locale_root(edition)
            )),
            ProjectionSourceFormatV1::ShippingDatadirV10 => PathBuf::from("Data/source.marker"),
        };
        let marker = source_root.join(&relative);
        fs::create_dir_all(marker.parent().unwrap())?;
        fs::write(&marker, format!("{edition:?}:{source_format:?}"))?;
        let bytes = fs::read(&marker)?;
        let manifest = OfficialSourceTreeManifestV1 {
            schema_version: 1,
            edition,
            source_format,
            files: vec![robin_run_protocol::OfficialSourceFileV1 {
                path: path_to_manifest(&relative)?,
                sha256: Digest32::digest_bytes(&bytes),
                byte_length: u64::try_from(bytes.len())?,
            }],
        };
        let path = config_root.join(format!("{edition:?}-{source_format:?}-source-tree.json"));
        fs::write(&path, manifest.canonical_bytes()?)?;
        Ok((manifest, path))
    }

    fn write_projection_receipt(
        edition: OfficialContentEditionV1,
        exporter: ProjectionExporterIdentityV1,
        source_tree_manifest: &OfficialSourceTreeManifestV1,
        subjects: &[robin_run_protocol::OfficialProjectionSubjectReceiptV1],
        config_root: &Path,
    ) -> Result<PathBuf> {
        let receipt = OfficialSimulationProjectionReceiptV1 {
            schema_version: 1,
            exporter,
            edition,
            source_tree_manifest_sha256: source_tree_manifest.canonical_digest()?,
            source_file_count: u32::try_from(source_tree_manifest.files.len())?,
            subjects: subjects.to_vec(),
        };
        let path = config_root.join(format!(
            "{edition:?}-{:?}-projection-receipt.json",
            exporter.source_format
        ));
        fs::write(&path, receipt.canonical_bytes()?)?;
        Ok(path)
    }

    #[cfg(unix)]
    fn make_tree_writable(root: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let mut directories = vec![root.to_path_buf()];
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            if !directory.exists() {
                continue;
            }
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push(entry.path());
                    directories.push(entry.path());
                } else {
                    fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o644))?;
                }
            }
        }
        drop(directories);
        Ok(())
    }

    #[cfg(not(unix))]
    fn make_tree_writable(root: &Path) -> Result<()> {
        for (_, file) in walk_regular_files(root)? {
            let mut permissions = fs::metadata(&file)?.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(file, permissions)?;
        }
        Ok(())
    }

    #[test]
    fn official_content_is_reproducible_private_for_full_and_exact_for_verifier() -> Result<()> {
        let fixture = ProjectionFixture::new()?;
        let first = fixture.config.path().join("release-a");
        let second = fixture.config.path().join("release-b");
        let first_digests = author_official_content(&fixture.plan_path, &first)?;
        let second_digests = author_official_content(&fixture.plan_path, &second)?;
        assert_eq!(first_digests, second_digests);
        assert_eq!(first_digests.demo_content_manifest_sha256.len(), 1);
        assert_eq!(first_digests.full_content_manifest_sha256.len(), 39);
        compare_trees(&first, &second)?;

        let public_content = first.join("public/content");
        let public_content_ids =
            fs::read_dir(public_content)?.collect::<std::io::Result<Vec<_>>>()?;
        assert_eq!(
            public_content_ids.len(),
            1,
            "only DEMO payload may be public"
        );
        assert_eq!(
            public_content_ids[0].file_name().to_string_lossy(),
            first_digests.demo_content_manifest_sha256[0].to_string()
        );
        let bundle_count = fs::read_dir(first.join("verifier-bundles"))?
            .collect::<std::io::Result<Vec<_>>>()?
            .len();
        assert_eq!(bundle_count, 40);
        assert_eq!(
            fs::read_dir(first.join("private/projection-receipts"))?
                .collect::<std::io::Result<Vec<_>>>()?
                .len(),
            4
        );
        assert_eq!(
            fs::read_dir(first.join("private/source-tree-manifests"))?
                .collect::<std::io::Result<Vec<_>>>()?
                .len(),
            4
        );
        assert_eq!(
            fs::read_dir(first.join("private/verifier-source-bindings"))?
                .collect::<std::io::Result<Vec<_>>>()?
                .len(),
            40
        );
        assert!(
            walk_regular_files(&first.join("public"))?
                .iter()
                .all(|(path, _)| !path.to_string_lossy().contains("source-tree")
                    && !path.to_string_lossy().contains("projection-receipt"))
        );

        assert!(author_official_content(&fixture.plan_path, &first).is_err());
        fs::write(second.join("unexpected-extra"), b"not part of the release")?;
        assert!(compare_trees(&first, &second).is_err());

        make_tree_writable(&first)?;
        make_tree_writable(&second)?;
        Ok(())
    }

    #[test]
    fn projection_mismatch_missing_mount_and_unknown_exporter_fail_closed() -> Result<()> {
        let mismatch = ProjectionFixture::new()?;
        let subject = official_content_subjects_v1(OfficialContentEditionV1::Full)
            .into_iter()
            .next()
            .unwrap();
        let kind = SimulationContentComponentKindV1::Profiles;
        let relative = simulation_content_component_relative_path_v1(&subject, kind)?;
        let path = mismatch.plan.full.shipping_projection_root.join(relative);
        let changed = SimulationContentComponentDocumentV1 {
            schema_version: 1,
            kind,
            component_schema_version: 1,
            payload: CanonicalValue::String("transcoded simulation mismatch".into()),
        };
        fs::write(path, changed.bitcode_bytes()?)?;
        assert!(
            author_official_content(
                &mismatch.plan_path,
                &mismatch.config.path().join("mismatch-output")
            )
            .is_err()
        );

        let mut missing = ProjectionFixture::new()?;
        missing.plan.full.shipping_projection_root =
            missing.config.path().join("absent-full-shipping-export");
        missing.rewrite_plan()?;
        assert!(OfficialProjectionPlanV1::load(&missing.plan_path).is_err());

        let unsupported = ProjectionFixture::new()?;
        let receipt_path = &unsupported.plan.full.shipping_projection_receipt;
        assert!(
            validate_document(DocumentKind::ProjectionReceiptV2, receipt_path).is_err(),
            "a V1 all-tree receipt must never decode as an official V2 authority"
        );
        let mut receipt: OfficialSimulationProjectionReceiptV1 =
            serde_json::from_slice(&fs::read(receipt_path)?)?;
        receipt.exporter.exporter_version = 9;
        fs::write(receipt_path, canonical_json_bytes(&receipt)?)?;
        assert!(
            author_official_content(
                &unsupported.plan_path,
                &unsupported.config.path().join("unsupported-output")
            )
            .is_err()
        );

        let wrong_locale = ProjectionFixture::new()?;
        let receipt_path = &wrong_locale.plan.full.shipping_projection_receipt;
        let mut receipt: OfficialSimulationProjectionReceiptV1 =
            serde_json::from_slice(&fs::read(receipt_path)?)?;
        for subject in &mut receipt.subjects {
            subject.content_manifest.resource_locale_root =
                robin_run_protocol::ResourceLocaleRootV1::new(
                    OFFICIAL_DEMO_RESOURCE_LOCALE_ROOT_V1,
                )?;
        }
        fs::write(receipt_path, receipt.canonical_bytes()?)?;
        assert!(
            author_official_content(
                &wrong_locale.plan_path,
                &wrong_locale.config.path().join("wrong-locale-output")
            )
            .is_err()
        );

        let wrong_inventory = OfficialSourceTreeManifestV1 {
            schema_version: 1,
            edition: OfficialContentEditionV1::Demo,
            source_format: ProjectionSourceFormatV1::LooseNativeV1,
            files: vec![robin_run_protocol::OfficialSourceFileV1 {
                path: "2047/Data/Text/Level.res".into(),
                sha256: Digest32::from_bytes([91; 32]),
                byte_length: 1,
            }],
        };
        assert!(
            validate_native_resource_locale_inventory(
                OfficialContentEditionV1::Demo,
                &wrong_inventory
            )
            .is_err()
        );

        let raw_drift = ProjectionFixture::new()?;
        fs::write(
            raw_drift
                .plan
                .full
                .native_source_root
                .join(OFFICIAL_FULL_RESOURCE_LOCALE_ROOT_V1)
                .join("Data/Text/Level.res"),
            b"source bytes changed after receipt",
        )?;
        assert!(
            author_official_content(
                &raw_drift.plan_path,
                &raw_drift.config.path().join("raw-drift-output")
            )
            .is_err()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let symlinked = ProjectionFixture::new()?;
            symlink(
                symlinked.config.path(),
                symlinked
                    .plan
                    .demo
                    .native_source_root
                    .join("forbidden-link"),
            )?;
            assert!(
                author_official_content(
                    &symlinked.plan_path,
                    &symlinked.config.path().join("symlink-output")
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn strict_json_rejects_duplicate_keys() {
        let duplicate = br#"{"schema_version":1,"schema_version":1}"#;
        assert!(strict_json_from_slice::<serde_json::Value>(duplicate).is_err());
    }

    #[test]
    fn browser_engine_and_signer_workflows_use_the_pinned_wasm_bindgen_package() {
        let runtime = include_str!("../../../.github/workflows/build-static-runtime.yml");
        let static_origins = include_str!("../../../.github/workflows/deploy-static-workers.yml");
        for (name, workflow) in [
            ("browser runtime", runtime),
            ("identity signer", static_origins),
        ] {
            assert!(
                workflow.contains("scripts/install_pinned_wasm_bindgen.sh"),
                "{name} workflow bypasses the accepted wasm-bindgen package authority"
            );
            assert!(
                !workflow.contains("cargo install wasm-bindgen-cli"),
                "{name} workflow derives wasm-bindgen from an unbound Cargo install"
            );
        }
        assert!(runtime.contains("steps.wasm-tools.outputs.wasm-bindgen"));
        assert!(static_origins.contains("steps.wasm-bindgen.outputs.executable"));
    }

    #[test]
    fn official_release_rejects_historical_or_mixed_input_provenance() {
        assert!(
            validate_official_ranked_input_provenance(
                InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
                &[CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1]
            )
            .is_ok()
        );
        for versions in [
            vec![CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1.saturating_sub(1)],
            vec![
                CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1.saturating_sub(1),
                CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ],
        ] {
            assert!(
                validate_official_ranked_input_provenance(
                    InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
                    &versions
                )
                .is_err()
            );
        }
    }

    #[test]
    fn build_authoring_hashes_exact_bytes_and_never_overwrites() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cargo_lock = root.path().join("Cargo.lock");
        let verifier = root.path().join("verifier");
        let viewer = root.path().join("viewer.wasm");
        let entry = root.path().join("entry.js");
        fs::write(&cargo_lock, b"lock-v1")?;
        fs::write(&verifier, b"verifier-v1")?;
        fs::write(&viewer, b"viewer-v1")?;
        fs::write(&entry, b"export function main() {}")?;
        let draft = BuildDraftV1 {
            schema_version: 1,
            source_commit: "a".repeat(40),
            cargo_lock: cargo_lock.clone(),
            target_triple: "x86_64-unknown-linux-gnu".into(),
            cargo_profile: "release".into(),
            cargo_features: vec!["replay".into()],
            replay_schema_version: 20,
            save_schema_version: 1,
            network_protocol_version: 1,
            verifier: BuildArtifactSourceV1 {
                source: verifier.clone(),
                media_type: "application/octet-stream".into(),
            },
            viewer_artifacts: vec![
                ViewerArtifactSourceV1 {
                    source: entry,
                    published_path: "viewer/entry.js".into(),
                    role: ViewerArtifactRoleV1::EntryJavaScript,
                    media_type: "text/javascript".into(),
                },
                ViewerArtifactSourceV1 {
                    source: viewer,
                    published_path: "viewer/robin.wasm".into(),
                    role: ViewerArtifactRoleV1::WebAssembly,
                    media_type: "application/wasm".into(),
                },
            ],
        };
        let draft_path = root.path().join("build-draft.json");
        fs::write(&draft_path, serde_json::to_vec(&draft)?)?;
        let output = root.path().join("build-manifest.json");
        let first = author_build(&draft_path, &output)?;
        assert!(
            validate_document(DocumentKind::BuildV2, &output).is_err(),
            "a historical BuildManifestV1 must never authorize official V2 projection"
        );
        assert!(author_build(&draft_path, &output).is_err());
        fs::write(verifier, b"verifier-v2")?;
        let changed = BuildDraftV1::load(&draft_path)?
            .author()?
            .canonical_digest()?;
        assert_ne!(first.digest, changed);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn build_v2_authoring_splits_public_build_and_private_exporter() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir()?;
        for (name, bytes) in [
            ("Cargo.lock", b"lock-v2".as_slice()),
            ("viewer.wasm", b"viewer-v2".as_slice()),
            ("entry.js", b"export function main() {}".as_slice()),
            ("index.html", b"<!doctype html>public static".as_slice()),
            ("signer.html", b"<!doctype html>signer".as_slice()),
            ("signer.js", b"export function sign() {}".as_slice()),
            ("signer.wasm", b"signer-v2".as_slice()),
            ("package.json", b"{}".as_slice()),
            ("pnpm-lock.yaml", b"lockfileVersion: '9.0'".as_slice()),
        ] {
            fs::write(root.path().join(name), bytes)?;
        }
        fs::write(
            root.path().join("wasm-bindgen-authority.json"),
            include_bytes!("../../../.github/tool-authorities/wasm-bindgen-cli-v0.2.127.json"),
        )?;
        fs::write(
            root.path().join("binaryen-authority.json"),
            include_bytes!("../../../.github/tool-authorities/binaryen-wasm-opt-v132.json"),
        )?;
        fs::write(
            root.path().join("wabt-authority.json"),
            include_bytes!("../../../.github/tool-authorities/wabt-wasm-strip-v1.0.41.json"),
        )?;
        fs::write(
            root.path().join("node-authority.json"),
            canonical_json_bytes(&crate::typed_js_authority::official_node_authority_v1())?,
        )?;
        fs::write(
            root.path().join("pnpm-authority.json"),
            canonical_json_bytes(&crate::typed_js_authority::official_pnpm_authority_v1())?,
        )?;
        let rust_toolchain = robin_run_protocol::RustToolchainAuthorityV1 {
            schema_version: 1,
            channel: "nightly-2026-08-25".into(),
            components: vec!["rust-src".into(), "rustc-codegen-cranelift-preview".into()],
            targets: vec!["wasm32-unknown-unknown".into()],
        };
        fs::write(
            root.path().join("rust-toolchain-authority.json"),
            canonical_json_bytes(&rust_toolchain)?,
        )?;
        let verifier = root.path().join("verifier");
        fs::write(&verifier, minimal_static_x86_64_elf())?;
        fs::set_permissions(&verifier, fs::Permissions::from_mode(0o500))?;
        let exporter = root.path().join("projection-exporter");
        fs::write(&exporter, minimal_static_x86_64_elf())?;
        fs::set_permissions(&exporter, fs::Permissions::from_mode(0o500))?;
        let javascript_tool = |version: &str, authority: &str| BuildToolSourceV2 {
            version: version.into(),
            authority_document: root.path().join(authority),
        };
        let official_tool = |version: &str, authority: &str| BuildToolSourceV2 {
            version: version.into(),
            authority_document: root.path().join(authority),
        };
        let draft = BuildDraftV2 {
            schema_version: 2,
            source_commit: "b".repeat(40),
            cargo_lock: root.path().join("Cargo.lock"),
            replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            save_schema_version: CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1.saturating_sub(1),
            network_protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION
                .saturating_sub(1),
            verifier: verifier.clone(),
            viewer_engine_artifacts: vec![
                ViewerArtifactSourceV1 {
                    source: root.path().join("entry.js"),
                    published_path: "viewer/robin.js".into(),
                    role: ViewerArtifactRoleV1::EntryJavaScript,
                    media_type: "text/javascript".into(),
                },
                ViewerArtifactSourceV1 {
                    source: root.path().join("viewer.wasm"),
                    published_path: "viewer/robin_bg.wasm".into(),
                    role: ViewerArtifactRoleV1::WebAssembly,
                    media_type: "application/wasm".into(),
                },
            ],
            pages_shell_artifacts: vec![BrowserArtifactSourceV2 {
                source: root.path().join("index.html"),
                published_path: "index.html".into(),
                media_type: "text/html".into(),
            }],
            identity_signer_artifacts: vec![
                BrowserArtifactSourceV2 {
                    source: root.path().join("signer.html"),
                    published_path: "identity-signer/index.html".into(),
                    media_type: "text/html".into(),
                },
                BrowserArtifactSourceV2 {
                    source: root.path().join("signer.js"),
                    published_path: "identity-signer/bridge/leaderboard_identity_bridge.js".into(),
                    media_type: "text/javascript".into(),
                },
                BrowserArtifactSourceV2 {
                    source: root.path().join("signer.wasm"),
                    published_path: "identity-signer/bridge/leaderboard_identity_bridge_bg.wasm"
                        .into(),
                    media_type: "application/wasm".into(),
                },
            ],
            rust_toolchain_authority: root.path().join("rust-toolchain-authority.json"),
            wasm_bindgen_cli: official_tool("0.2.127", "wasm-bindgen-authority.json"),
            binaryen_wasm_opt: official_tool("version_132", "binaryen-authority.json"),
            wabt_wasm_strip: official_tool("1.0.41", "wabt-authority.json"),
            node: javascript_tool("24.19.0", "node-authority.json"),
            pnpm: javascript_tool("12.3.4", "pnpm-authority.json"),
            package_json: root.path().join("package.json"),
            pnpm_lock: root.path().join("pnpm-lock.yaml"),
        };
        let draft_path = root.path().join("build-v2-draft.json");
        fs::write(&draft_path, serde_json::to_vec(&draft)?)?;
        let output = root.path().join("build-v2.json");
        let authored = author_build_v2(&draft_path, &output)?;
        let manifest: BuildManifestV2 = serde_json::from_slice(&authored.canonical_bytes)?;
        assert_eq!(
            manifest.viewer.identity_signer.cargo_package,
            "robin_identity_signer"
        );
        let wasm_bindgen_authority =
            load_build_tool_authority(&root.path().join("wasm-bindgen-authority.json"))?;
        assert_eq!(
            manifest.viewer.engine.wasm_bindgen_cli.authority_sha256,
            wasm_bindgen_authority.canonical_digest()?
        );
        assert_eq!(
            manifest.viewer.engine.wasm_bindgen_cli,
            manifest.viewer.identity_signer.wasm_bindgen_cli,
            "engine and isolated signer must share one exact wasm-bindgen authority"
        );
        ensure!(
            validate_current_official_ranked_build_v2(&manifest).is_err(),
            "stale save/network build tuple became an official release authority"
        );
        let mut current_draft = draft.clone();
        current_draft.save_schema_version = CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1;
        current_draft.network_protocol_version = robin_engine::multiplayer::NET_PROTOCOL_VERSION;
        validate_current_official_ranked_build_v2(&current_draft.author()?)?;
        assert_eq!(
            manifest.verifier.artifact.sha256,
            hash_regular_file(&verifier)?
        );
        assert!(!String::from_utf8(authored.canonical_bytes)?.contains("projection_exporter"));
        let viewer_report_path = root.path().join("viewer-build-report-v2.json");
        let viewer_report = author_viewer_build_report_v2(&output, &viewer_report_path)?;
        let decoded_report: OfficialViewerBuildReportV2 =
            serde_json::from_slice(&viewer_report.canonical_bytes)?;
        decoded_report.validate_against(&manifest)?;
        assert!(author_viewer_build_report_v2(&output, &viewer_report_path).is_err());
        assert!(author_build_v2(&draft_path, &output).is_err());

        let mut missing_wasm_bindgen_field = serde_json::to_value(&draft)?;
        missing_wasm_bindgen_field
            .as_object_mut()
            .unwrap()
            .remove("wasm_bindgen_cli");
        assert!(
            strict_json_from_slice::<BuildDraftV2>(&serde_json::to_vec(
                &missing_wasm_bindgen_field
            )?)
            .is_err(),
            "V2 build draft accepted an omitted wasm-bindgen authority"
        );

        let mut substituted_wasm_bindgen = draft.clone();
        substituted_wasm_bindgen.wasm_bindgen_cli.authority_document =
            root.path().join("binaryen-authority.json");
        assert!(
            substituted_wasm_bindgen.author().is_err(),
            "public build accepted Binaryen in the wasm-bindgen authority slot"
        );

        let mut moving_wasm_bindgen = draft.clone();
        moving_wasm_bindgen.wasm_bindgen_cli.version = "0.2.128".into();
        assert!(
            moving_wasm_bindgen.author().is_err(),
            "public build accepted a non-pinned wasm-bindgen version"
        );

        let mut absent_wasm_bindgen = draft.clone();
        absent_wasm_bindgen.wasm_bindgen_cli.authority_document =
            root.path().join("absent-wasm-bindgen-authority.json");
        assert!(
            absent_wasm_bindgen.author().is_err(),
            "public build accepted an absent wasm-bindgen authority path"
        );

        let mut tampered_wasm_bindgen_authority = wasm_bindgen_authority.clone();
        tampered_wasm_bindgen_authority.distribution.sha256 = Digest32::digest_bytes(b"tampered");
        let tampered_wasm_bindgen_path = root.path().join("tampered-wasm-bindgen-authority.json");
        fs::write(
            &tampered_wasm_bindgen_path,
            canonical_json_bytes(&tampered_wasm_bindgen_authority)?,
        )?;
        let mut tampered_wasm_bindgen = draft.clone();
        tampered_wasm_bindgen.wasm_bindgen_cli.authority_document = tampered_wasm_bindgen_path;
        assert!(
            tampered_wasm_bindgen.author().is_err(),
            "public build accepted a wasm-bindgen distribution digest substitution"
        );

        let mut swapped_tools = draft.clone();
        std::mem::swap(
            &mut swapped_tools.binaryen_wasm_opt.authority_document,
            &mut swapped_tools.wabt_wasm_strip.authority_document,
        );
        assert!(
            swapped_tools.author().is_err(),
            "public build accepted swapped Binaryen/WABT authorities"
        );

        let authority_draft = ProjectionAuthorityDraftV2 {
            schema_version: 2,
            public_build_manifest: output,
            projection_exporter: exporter.clone(),
        };
        let authority = authority_draft.author()?;
        authority.validate_against(&manifest)?;
        assert_eq!(
            authority.projection_exporter.artifact.sha256,
            hash_regular_file(&exporter)?
        );
        Ok(())
    }

    #[cfg(unix)]
    fn minimal_static_x86_64_elf() -> Vec<u8> {
        const ELF_HEADER_BYTES: usize = 64;
        const PROGRAM_HEADER_BYTES: usize = 56;
        let file_size = (ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES + 1) as u64;
        let mut bytes = vec![0_u8; file_size as usize];
        bytes[..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        bytes[16..18].copy_from_slice(&header::ET_EXEC.to_le_bytes());
        bytes[18..20].copy_from_slice(&header::EM_X86_64.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x40_0078_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&(ELF_HEADER_BYTES as u64).to_le_bytes());
        bytes[52..54].copy_from_slice(&(ELF_HEADER_BYTES as u16).to_le_bytes());
        bytes[54..56].copy_from_slice(&(PROGRAM_HEADER_BYTES as u16).to_le_bytes());
        bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
        bytes[58..60].copy_from_slice(&64_u16.to_le_bytes());

        let program = ELF_HEADER_BYTES;
        bytes[program..program + 4].copy_from_slice(&program_header::PT_LOAD.to_le_bytes());
        bytes[program + 4..program + 8]
            .copy_from_slice(&(program_header::PF_R | program_header::PF_X).to_le_bytes());
        bytes[program + 16..program + 24].copy_from_slice(&0x40_0000_u64.to_le_bytes());
        bytes[program + 24..program + 32].copy_from_slice(&0x40_0000_u64.to_le_bytes());
        bytes[program + 32..program + 40].copy_from_slice(&file_size.to_le_bytes());
        bytes[program + 40..program + 48].copy_from_slice(&file_size.to_le_bytes());
        bytes[program + 48..program + 56].copy_from_slice(&0x1000_u64.to_le_bytes());
        bytes[file_size as usize - 1] = 0xc3; // x86-64 `ret`; never executed by this test.
        bytes
    }

    #[cfg(unix)]
    #[test]
    fn static_exporter_gate_accepts_only_exact_static_x86_64_elf() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir()?;
        let exporter = root.path().join("projection-exporter");
        let bytes = minimal_static_x86_64_elf();
        fs::write(&exporter, &bytes)?;
        fs::set_permissions(&exporter, fs::Permissions::from_mode(0o500))?;
        let expected = artifact_from_file(
            &exporter,
            robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        )?;
        validate_static_projection_exporter(&exporter, &expected)?;

        let mut substituted = expected.clone();
        substituted.sha256 = Digest32::digest_bytes(b"different exporter");
        assert!(validate_static_projection_exporter(&exporter, &substituted).is_err());

        fs::set_permissions(&exporter, fs::Permissions::from_mode(0o400))?;
        assert!(validate_static_projection_exporter(&exporter, &expected).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn static_exporter_gate_rejects_dynamic_host_binary_and_symlink() -> Result<()> {
        use std::os::unix::fs::symlink;

        let dynamic = std::env::current_exe()?;
        let expected = artifact_from_file(
            &dynamic,
            robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        )?;
        assert!(validate_static_projection_exporter(&dynamic, &expected).is_err());

        let root = tempfile::tempdir()?;
        let link = root.path().join("exporter-link");
        symlink(&dynamic, &link)?;
        assert!(validate_static_projection_exporter(&link, &expected).is_err());
        Ok(())
    }

    #[test]
    fn source_inventory_adapters_are_typed_v2_only() -> Result<()> {
        use official_content_source::{
            LOOSE_NATIVE_SOURCE_CLOSURE_V2, LooseSourceClosureInventory,
            SHIPPING_DATADIR_SOURCE_CLOSURE_V2, ShippingSourceClosureInventory, SourceClosureFile,
        };

        let source_file = |path: &str, byte: u8| SourceClosureFile {
            path: path.into(),
            sha256: [byte; 32],
            byte_length: u64::from(byte),
        };
        let loose = LooseSourceClosureInventory {
            policy: LOOSE_NATIVE_SOURCE_CLOSURE_V2.into(),
            resource_locale_root: "1033".into(),
            files: vec![
                source_file("1033/Data/Text/Level.res", 1),
                source_file("Data/Levels/Dem_Lei_MP.rhm", 2),
            ],
        };
        let loose_manifest = loose_source_manifest_v2(OfficialContentEditionV1::Demo, &loose)?;
        assert_eq!(loose_manifest.schema_version, 2);
        assert!(loose_source_manifest_v2(OfficialContentEditionV1::Full, &loose).is_err());
        let mut v1_policy = loose.clone();
        v1_policy.policy = "whole_tree_v1".into();
        assert!(loose_source_manifest_v2(OfficialContentEditionV1::Demo, &v1_policy).is_err());

        let shipping = ShippingSourceClosureInventory {
            policy: SHIPPING_DATADIR_SOURCE_CLOSURE_V2.into(),
            files: vec![
                source_file("Data/Characters/Soldier B02.rhs", 3),
                source_file("Data/datadir.bin", 4),
            ],
        };
        let shipping_manifest =
            shipping_source_manifest_v2(OfficialContentEditionV1::Full, &shipping)?;
        assert_eq!(shipping_manifest.schema_version, 2);
        let mut arbitrary_subset = shipping.clone();
        arbitrary_subset.files.remove(1);
        assert!(
            shipping_source_manifest_v2(OfficialContentEditionV1::Full, &arbitrary_subset).is_err()
        );
        Ok(())
    }

    #[test]
    fn operator_rules_gate_requires_complete_exact_medium_sim_config() -> Result<()> {
        let canonical: CanonicalValue = serde_json::from_value(serde_json::to_value(
            robin_engine::engine::SimConfig::standard_ranked(
                robin_engine::player_profile::DifficultyLevel::Medium,
            ),
        )?)?;
        let CanonicalValue::Object(sim_config) = canonical else {
            bail!("SimConfig did not canonicalize to an object")
        };
        let baseline = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: robin_run_protocol::RankedSimulationPolicyV1::standard(
                robin_run_protocol::RankedSimulationDifficultyV1::Medium,
            ),
            sim_config,
            rules: BTreeMap::from([("ranked".into(), CanonicalValue::Bool(true))]),
        };
        validate_official_projection_rules_config_v1(&baseline)?;

        let mut missing = baseline.clone();
        missing.sim_config.remove("script_enabled");
        assert!(validate_official_projection_rules_config_v1(&missing).is_err());
        let mut unknown = baseline.clone();
        unknown
            .sim_config
            .insert("host_preference".into(), CanonicalValue::Bool(true));
        assert!(validate_official_projection_rules_config_v1(&unknown).is_err());
        let mut hard = baseline;
        hard.sim_config
            .insert("difficulty".into(), CanonicalValue::String("Hard".into()));
        assert!(validate_official_projection_rules_config_v1(&hard).is_err());
        Ok(())
    }
}
