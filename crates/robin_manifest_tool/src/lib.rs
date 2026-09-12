//! Deterministic operator tooling for verified-run release manifests.
//!
//! Ranked content identities are engine-owned simulation projections, not
//! installation/package inventories. This module requires independent native
//! and RHDDNA10 shipping exports and accepts a subject only when all eight canonical
//! component documents are byte-identical. Retail component documents are
//! materialized only below the private verifier-bundle tree; only DEMO
//! component objects are copied into the public static tree.

mod fs_util;
use fs_util::{read_regular_file_bounded, validate_regular_file};
pub mod campaign_template_v1;
pub mod plan_v3;
pub mod publication_v3;
pub mod release_admission_v1;
pub mod sandbox_v3;
pub mod typed_js_authority;
pub mod verifier_catalog_v1;
#[cfg(target_os = "linux")]
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
    BrowserViewerEngineBuildIdentityV2, BrowserViewerEngineBuildRecipeV2, BuildManifestV2,
    BuildToolAuthorityDocumentV1, BuildToolAuthorityV1, CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
    CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1, CampaignContentManifestV1, CanonicalDocument as _,
    CanonicalValue, CompetitionManifestV1, ContentManifestV1, Digest32, ImmutablePolicyManifestV1,
    NamedArtifactV1, NativeBuildPlatformV2, NativeLinkageV2,
    OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1, OfficialContentSubjectV1,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionExporterBuildIdentityV2, OfficialProjectionExporterPlatformV2,
    OfficialSimulationProjectionReceiptV2, OfficialSourceTreeManifestV2,
    OfficialViewerBuildReportV2, PublishedRulesetV1, RulesConfigIdentityV1, RulesetManifestV1,
    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, SimulationContentComponentDocumentV1,
    SimulationContentComponentKindV1, SimulationContentComponentV1, Validate as _, ValidationError,
    VerifierBuildIdentityV2, ViewerArtifactRoleV1, canonical_json_bytes,
    official_content_subjects_v1,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::typed_js_authority::{
    JavaScriptBuildToolAuthorityDocumentV1, JavaScriptBuildToolRoleV1,
};

/// Shared fail-closed selector and exact materializer used by both the
/// projection exporter and operator verification tooling.
pub use robin_official_content as official_content_source;
const MAX_DOCUMENT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PROJECTION_EXPORTER_BYTES: u64 = 512 * 1024 * 1024;
const OFFICIAL_DEMO_RESOURCE_LOCALE_ROOT_V1: &str = "1033";
const OFFICIAL_FULL_RESOURCE_LOCALE_ROOT_V1: &str = "2047";

pub use robin_run_protocol::OfficialProjectionSourceFormatV1 as ProjectionSourceFormatV1;

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
pub struct ViewerArtifactSourceV1 {
    pub source: PathBuf,
    pub published_path: String,
    pub role: ViewerArtifactRoleV1,
    pub media_type: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum DocumentKind {
    #[value(name = "build-v2")]
    BuildV2,
    BuildToolAuthority,
    JavascriptBuildToolAuthority,
    #[value(name = "viewer-build-report-v2")]
    ViewerBuildReportV2,
    #[value(name = "projection-authority-v2")]
    ProjectionAuthorityV2,
    Content,
    CampaignContent,
    SimulationComponent,
    #[value(name = "source-tree-manifest-v2")]
    SourceTreeManifestV2,
    #[value(name = "projection-receipt-v2")]
    ProjectionReceiptV2,
    #[value(name = "built-in-overlay-source-manifest-v2")]
    BuiltInOverlaySourceManifestV2,
    ProjectionExecutionPolicy,
    #[value(name = "verifier-source-binding-v2")]
    VerifierSourceBindingV2,
    #[value(name = "projection-execution-record-v3")]
    ProjectionExecutionRecordV3,
    #[value(name = "projection-authority-matrix-v3")]
    ProjectionAuthorityMatrixV3,
    RulesConfig,
    RulesetManifest,
    PublishedRuleset,
    Competition,
    Policy,
}

#[derive(Debug)]
struct AuthoredContent {
    manifest: ContentManifestV1,
    components: BTreeMap<SimulationContentComponentKindV1, Vec<u8>>,
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
        DocumentKind::SourceTreeManifestV2 => {
            canonicalize_typed::<OfficialSourceTreeManifestV2>(input, output)
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
    }
}

pub fn validate_document(kind: DocumentKind, input: &Path) -> Result<Digest32> {
    match kind {
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
        DocumentKind::SourceTreeManifestV2 => validate_typed::<OfficialSourceTreeManifestV2>(input),
        DocumentKind::ProjectionReceiptV2 => {
            validate_typed::<OfficialSimulationProjectionReceiptV2>(input)
        }
        DocumentKind::BuiltInOverlaySourceManifestV2 => {
            validate_typed::<OfficialBuiltInOverlaySourceManifestV2>(input)
        }
        DocumentKind::ProjectionExecutionPolicy => {
            validate_typed::<OfficialProjectionExecutionPolicyV1>(input)
        }
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
    }
}

fn official_resource_locale_root(edition: OfficialContentEditionV1) -> &'static str {
    match edition {
        OfficialContentEditionV1::Demo => OFFICIAL_DEMO_RESOURCE_LOCALE_ROOT_V1,
        OfficialContentEditionV1::Full => OFFICIAL_FULL_RESOURCE_LOCALE_ROOT_V1,
    }
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

fn load_canonical_document<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let (document, _): (T, _) = fs_util::load_canonical_bytes(path, MAX_DOCUMENT_BYTES)?;
    document.validate()?;
    Ok(document)
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
    ensure!(
        fs_util::valid_relative_path(path.to_str().context("path is not UTF-8")?),
        "path is not canonical relative"
    );
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

fn strict_json_from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(robin_run_protocol::strict_json::from_slice(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_kinds_advertise_live_stages_and_reject_retired_authority() {
        use clap::ValueEnum as _;
        for retired in [
            "build",
            "source-tree-manifest",
            "projection-receipt",
            "release-lock",
            "verifier-source-binding",
        ] {
            assert!(
                DocumentKind::from_str(retired, false).is_err(),
                "retired authority {retired}"
            );
        }
        for live in [
            "build-v2",
            "source-tree-manifest-v2",
            "projection-receipt-v2",
            "projection-authority-matrix-v3",
        ] {
            assert!(
                DocumentKind::from_str(live, false).is_ok(),
                "live authority {live}"
            );
        }
    }

    #[test]
    fn strict_json_rejects_duplicate_keys() {
        for duplicate in [
            br#"{"schema_version":1,"schema_version":1}"#.as_slice(),
            br#"{"nested":[{"key":null,"key":true}]}"#.as_slice(),
        ] {
            let error = strict_json_from_slice::<serde_json::Value>(duplicate).unwrap_err();
            assert!(matches!(
                error.downcast_ref::<robin_run_protocol::strict_json::StrictJsonError>(),
                Some(robin_run_protocol::strict_json::StrictJsonError::DuplicateKey(_))
            ));
        }
        assert!(strict_json_from_slice::<serde_json::Value>(b"null true").is_err());
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
