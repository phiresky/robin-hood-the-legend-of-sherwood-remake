//! Atomic authoring of the official V2 projection authority matrix.
//!
//! The historical plan accepted already-generated V1 receipts. This plan is
//! intentionally different: it selects and materializes the exact consumed
//! source closures, runs the BuildManifestV2-bound exporter itself in the
//! fixed sandbox, admits all four lanes, and only then publishes one immutable
//! result. A caller can never publish a successful Demo lane beside a failed
//! or missing Full lane.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use robin_assets::shipping_datadir::ShippingDatadir;
use robin_run_protocol::{
    ArtifactRefV1, BuildManifestV2, BuildToolAuthorityDocumentV1, CampaignContentEntryV1,
    CampaignContentManifestV1, CanonicalDocument as _, ContentManifestV1, Digest32,
    OfficialBuiltInOverlayKindV2, OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionExportReportV2, OfficialProjectionSourceFormatV1,
    OfficialSimulationProjectionReceiptV2, OfficialSourceFileV1, OfficialSourceTreeManifestV2,
    RulesConfigIdentityV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, Validate as _,
    canonical_json_bytes, demo_content_object_path_v1, official_content_subjects_v1,
    simulation_content_component_relative_path_v1, validate_official_projection_receipt_matrix_v2,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::official_content_source::{
    LooseSourceClosureInventory, ShippingSourceClosureInventory, inventory_loose_source_closure,
    inventory_shipping_source_closure, materialize_loose_source_closure,
    materialize_shipping_source_closure, shipping_projection_external_file_paths_v2,
    validate_shipping_projection_source_relative_paths_v2,
};
use crate::sandbox_v3::{
    SandboxRuntimeIdentityV1, SandboxedProjectionRequest, ValidatedSandboxProjection,
    run_projection_exporter_v2, validate_projection_output_v2,
};
use crate::{
    AuthoredContent, MAX_DOCUMENT_BYTES, artifact_from_bytes, artifact_from_file, config_parent,
    copy_artifact_exact, ensure_absent_output, loose_source_manifest_v2,
    make_verifier_bundles_read_only, path_to_manifest, persist_staging, read_mounted_projection,
    resolve_path, shipping_source_manifest_v2, staging_directory, strict_json_from_slice,
    validate_current_official_ranked_build_v2, validate_mount_root,
    validate_official_projection_rules_config_v1, validate_regular_file,
    validate_static_projection_exporter, walk_regular_files, write_bytes, write_canonical,
    write_digest_document, write_shared_bytes,
};

const PLAN_V3_SCHEMA_VERSION: u32 = 3;
const MATRIX_V3_SCHEMA_VERSION: u32 = 3;
const EXECUTION_RECORD_V3_SCHEMA_VERSION: u32 = 3;
const SOURCE_BINDING_V2_SCHEMA_VERSION: u32 = 2;
const JSON_MEDIA_TYPE: &str = "application/json";
const DIAGNOSTIC_MEDIA_TYPE: &str = "text/plain; charset=utf-8";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionPlanV3 {
    pub schema_version: u32,
    pub build_manifest: PathBuf,
    pub wasm_bindgen_cli_authority: PathBuf,
    pub binaryen_wasm_opt_authority: PathBuf,
    pub wabt_wasm_strip_authority: PathBuf,
    pub projection_authority_manifest: PathBuf,
    pub projection_exporter: PathBuf,
    pub rules_config: PathBuf,
    pub execution_policy: PathBuf,
    pub core_overlay_source_root: PathBuf,
    pub demo: EditionSourcePlanV3,
    pub full: EditionSourcePlanV3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditionSourcePlanV3 {
    pub edition: OfficialContentEditionV1,
    pub loose_source_root: PathBuf,
    pub shipping_source_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionStreamDigestV1 {
    pub sha256: Digest32,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionExecutionRecordV3 {
    pub schema_version: u32,
    pub edition: OfficialContentEditionV1,
    pub source_format: OfficialProjectionSourceFormatV1,
    /// Host safety provenance only. It is deliberately outside every content
    /// manifest and projection receipt identity.
    pub sandbox_runtime: SandboxRuntimeIdentityV1,
    pub report: OfficialProjectionExportReportV2,
    pub stdout: ArtifactRefV1,
    pub stderr: ProjectionStreamDigestV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionLaneAuthorityV3 {
    pub edition: OfficialContentEditionV1,
    pub source_format: OfficialProjectionSourceFormatV1,
    pub source_tree_manifest_sha256: Digest32,
    pub projection_receipt_sha256: Digest32,
    pub execution_record_sha256: Digest32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionAuthorityMatrixV3 {
    pub schema_version: u32,
    pub build_manifest_sha256: Digest32,
    pub projection_authority_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub execution_policy_sha256: Digest32,
    pub core_overlay_manifest_sha256: Digest32,
    pub lanes: Vec<OfficialProjectionLaneAuthorityV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierSourceBindingV2 {
    pub schema_version: u32,
    pub content_manifest_sha256: Digest32,
    pub edition: OfficialContentEditionV1,
    pub subject: robin_run_protocol::OfficialContentSubjectV1,
    pub build_manifest_sha256: Digest32,
    pub projection_authority_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub execution_policy_sha256: Digest32,
    pub core_overlay_manifest_sha256: Digest32,
    pub loose_projection_receipt_sha256: Digest32,
    pub loose_source_tree_manifest_sha256: Digest32,
    pub shipping_projection_receipt_sha256: Digest32,
    pub shipping_source_tree_manifest_sha256: Digest32,
}

impl robin_run_protocol::Validate for OfficialProjectionExecutionRecordV3 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != EXECUTION_RECORD_V3_SCHEMA_VERSION
            || self.sandbox_runtime.schema_version != 1
            || self.sandbox_runtime.bubblewrap.path != "/usr/bin/bwrap"
            || self.sandbox_runtime.prlimit.path != "/usr/bin/prlimit"
            || self.sandbox_runtime.bubblewrap.version.is_empty()
            || self.sandbox_runtime.prlimit.version.is_empty()
            || self.sandbox_runtime.limits
                != crate::sandbox_v3::SandboxResourceLimitsV1::OFFICIAL_V1
            || self.stdout.byte_length == 0
            || self.stdout.sha256.is_zero()
            || self.stderr.sha256.is_zero()
            || self.stderr.byte_length
                > crate::sandbox_v3::SandboxResourceLimitsV1::OFFICIAL_V1
                    .diagnostic_bytes_per_stream
            || self.report.edition != self.edition
            || self.report.source_format != self.source_format
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_projection_execution_record_v3",
            });
        }
        self.sandbox_runtime.bubblewrap.artifact.validate()?;
        self.sandbox_runtime.prlimit.artifact.validate()?;
        self.stdout.validate()?;
        self.report.validate()?;
        let report_bytes = canonical_json_bytes(&self.report).map_err(|_| {
            robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_projection_execution_record_v3.report_canonicalization",
            }
        })?;
        if artifact_from_bytes(&report_bytes, JSON_MEDIA_TYPE) != self.stdout {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_projection_execution_record_v3.stdout",
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for OfficialProjectionAuthorityMatrixV3 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != MATRIX_V3_SCHEMA_VERSION
            || self.build_manifest_sha256.is_zero()
            || self.projection_authority_manifest_sha256.is_zero()
            || self.rules_config_sha256.is_zero()
            || self.execution_policy_sha256.is_zero()
            || self.core_overlay_manifest_sha256.is_zero()
            || self.lanes.len() != 4
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_projection_authority_matrix_v3",
            });
        }
        let expected = official_lane_order();
        if self.lanes.iter().zip(expected).any(|(lane, expected)| {
            (lane.edition, lane.source_format) != expected
                || lane.source_tree_manifest_sha256.is_zero()
                || lane.projection_receipt_sha256.is_zero()
                || lane.execution_record_sha256.is_zero()
        }) {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "official_projection_authority_matrix_v3.lanes",
            });
        }
        if self
            .lanes
            .iter()
            .map(|lane| lane.source_tree_manifest_sha256)
            .collect::<BTreeSet<_>>()
            .len()
            != 4
            || self
                .lanes
                .iter()
                .map(|lane| lane.projection_receipt_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != 4
            || self
                .lanes
                .iter()
                .map(|lane| lane.execution_record_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != 4
        {
            return Err(robin_run_protocol::ValidationError::Duplicate {
                field: "official_projection_authority_matrix_v3.lane_authority",
                value: "digest".into(),
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for VerifierSourceBindingV2 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != SOURCE_BINDING_V2_SCHEMA_VERSION
            || [
                self.content_manifest_sha256,
                self.build_manifest_sha256,
                self.projection_authority_manifest_sha256,
                self.rules_config_sha256,
                self.execution_policy_sha256,
                self.core_overlay_manifest_sha256,
                self.loose_projection_receipt_sha256,
                self.loose_source_tree_manifest_sha256,
                self.shipping_projection_receipt_sha256,
                self.shipping_source_tree_manifest_sha256,
            ]
            .iter()
            .any(Digest32::is_zero)
        {
            return Err(robin_run_protocol::ValidationError::Zero {
                field: "verifier_source_binding_v2",
            });
        }
        self.subject.validate()
    }
}

#[derive(Debug)]
struct PreparedAuthority {
    build: BuildManifestV2,
    build_sha256: Digest32,
    wasm_bindgen_cli_authority: BuildToolAuthorityDocumentV1,
    binaryen_wasm_opt_authority: BuildToolAuthorityDocumentV1,
    wabt_wasm_strip_authority: BuildToolAuthorityDocumentV1,
    projection_authority: OfficialProjectionAuthorityManifestV2,
    projection_authority_sha256: Digest32,
    rules: RulesConfigIdentityV1,
    rules_sha256: Digest32,
    execution_policy: OfficialProjectionExecutionPolicyV1,
    execution_policy_sha256: Digest32,
    core_manifest: OfficialBuiltInOverlaySourceManifestV2,
    core_manifest_sha256: Digest32,
    exporter: ArtifactRefV1,
    central_exporter: PathBuf,
    sanitized_core_root: PathBuf,
}

#[derive(Debug)]
enum SourceInventory {
    Loose(LooseSourceClosureInventory),
    Shipping {
        references: Vec<String>,
        inventory: ShippingSourceClosureInventory,
    },
}

#[derive(Debug)]
struct PreparedSource {
    original_root: PathBuf,
    sanitized_root: PathBuf,
    manifest: OfficialSourceTreeManifestV2,
    inventory: SourceInventory,
}

#[derive(Debug)]
struct ValidatedLane {
    edition: OfficialContentEditionV1,
    source_format: OfficialProjectionSourceFormatV1,
    source_manifest: OfficialSourceTreeManifestV2,
    source_manifest_sha256: Digest32,
    projection: ValidatedSandboxProjection,
    receipt_sha256: Digest32,
    execution_record: OfficialProjectionExecutionRecordV3,
    execution_record_sha256: Digest32,
    output_root: PathBuf,
}

#[derive(Debug)]
struct AuthoredEditionV3 {
    edition: OfficialContentEditionV1,
    content: BTreeMap<Digest32, AuthoredContent>,
    campaign: CampaignContentManifestV1,
    loose_lane: usize,
    shipping_lane: usize,
}

/// Fully revalidated immutable output of `author-official-content-v3`.
/// Publication tooling consumes this typed view rather than trusting paths or
/// reauthoring receipts through the historical V1 route.
#[derive(Debug)]
pub struct ValidatedOfficialContentV3 {
    pub digests: crate::OfficialContentDigestsV1,
    pub matrix: OfficialProjectionAuthorityMatrixV3,
    pub build: BuildManifestV2,
    pub projection_authority: OfficialProjectionAuthorityManifestV2,
    pub rules: RulesConfigIdentityV1,
    pub execution_policy: OfficialProjectionExecutionPolicyV1,
    pub core_manifest: OfficialBuiltInOverlaySourceManifestV2,
    pub content: BTreeMap<Digest32, ContentManifestV1>,
    pub campaigns: BTreeMap<Digest32, CampaignContentManifestV1>,
}

/// Author the only currently authorized official content release path.
///
/// The output directory must not exist. Every source is re-inventoried after
/// its child exits, all four receipts are cross-validated as one matrix, and a
/// same-filesystem no-replace rename is the first externally visible success.
pub fn author_official_content_v3(
    plan_path: &Path,
    output_directory: &Path,
) -> Result<crate::OfficialContentDigestsV1> {
    ensure_absent_output(output_directory)?;
    validate_output_location(output_directory)?;
    let plan = OfficialProjectionPlanV3::load(plan_path)?;
    plan.validate(output_directory)?;

    let work = WritableTempDir::new(output_directory, ".robin-projection-v3-work-")?;
    let authority = prepare_authority(&plan, work.path())?;
    let lane_specs = [
        (
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
            plan.demo.loose_source_root.as_path(),
        ),
        (
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::ShippingDatadirV10,
            plan.demo.shipping_source_root.as_path(),
        ),
        (
            OfficialContentEditionV1::Full,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
            plan.full.loose_source_root.as_path(),
        ),
        (
            OfficialContentEditionV1::Full,
            OfficialProjectionSourceFormatV1::ShippingDatadirV10,
            plan.full.shipping_source_root.as_path(),
        ),
    ];
    let mut lanes = Vec::with_capacity(4);
    for (index, (edition, source_format, source_root)) in lane_specs.into_iter().enumerate() {
        let lane_root = work.path().join(format!("lane-{index}"));
        fs::create_dir(&lane_root)?;
        let prepared = prepare_source(
            edition,
            source_format,
            source_root,
            &lane_root.join("input"),
        )
        .with_context(|| format!("prepare {edition:?}/{source_format:?} source closure"))?;
        let lane = execute_lane(index, prepared, &authority, &lane_root)
            .with_context(|| format!("execute {edition:?}/{source_format:?} projection"))?;
        lanes.push(lane);
    }

    validate_authorities_after_all_lanes(&plan, &authority)?;
    validate_official_projection_receipt_matrix_v2(
        &lanes
            .iter()
            .map(|lane| lane.projection.receipt.clone())
            .collect::<Vec<_>>(),
    )?;
    let editions = author_equivalent_editions(&lanes)?;
    let digests = official_content_digests_v3(&editions)?;
    let matrix = authority_matrix(&authority, &lanes)?;

    let staging = staging_directory(output_directory)?;
    materialize_v3(
        staging.path(),
        &authority,
        &lanes,
        &editions,
        &digests,
        &matrix,
    )?;
    make_verifier_bundles_read_only(&staging.path().join("verifier-bundles"))?;
    let admitted = validate_official_content_v3(staging.path())?;
    ensure!(
        admitted.digests == digests && admitted.matrix == matrix,
        "materialized plan-v3 authority differs from the admitted transaction"
    );
    persist_staging(staging, output_directory)?;
    Ok(digests)
}

/// Revalidate a completed plan-v3 tree without consulting operator paths.
///
/// This is intentionally an exact-layout validator: an extra empty directory
/// fails just like an extra file. It is the admission boundary used by the V2
/// publication assembler and may also be run independently before mounting a
/// content authority into production.
pub fn validate_official_content_v3(root: &Path) -> Result<ValidatedOfficialContentV3> {
    validate_mount_root(root)?;
    let mut expected_files = BTreeSet::new();
    let digests: crate::OfficialContentDigestsV1 =
        load_expected_document(root, "official-content-digests.json", &mut expected_files)?;
    validate_digest_sidecar(
        root,
        "official-content-digests.sha256",
        digests.canonical_digest()?,
        &mut expected_files,
    )?;
    let matrix: OfficialProjectionAuthorityMatrixV3 = load_expected_document(
        root,
        "projection-authority-matrix-v3.json",
        &mut expected_files,
    )?;
    validate_digest_sidecar(
        root,
        "projection-authority-matrix-v3.sha256",
        matrix.canonical_digest()?,
        &mut expected_files,
    )?;

    let build_path = format!("manifests/builds-v2/{}.json", matrix.build_manifest_sha256);
    let build: BuildManifestV2 = load_expected_document(root, &build_path, &mut expected_files)?;
    validate_current_official_ranked_build_v2(&build)?;
    ensure!(
        build.canonical_digest()? == matrix.build_manifest_sha256,
        "BuildManifestV2 path digest mismatch"
    );
    let wasm_bindgen_authority_path = format!(
        "manifests/build-tool-authorities/{}.json",
        build.viewer.engine.wasm_bindgen_cli.authority_sha256
    );
    let wasm_bindgen_authority: BuildToolAuthorityDocumentV1 =
        load_expected_document(root, &wasm_bindgen_authority_path, &mut expected_files)?;
    let binaryen_authority_path = format!(
        "manifests/build-tool-authorities/{}.json",
        build.viewer.engine.binaryen_wasm_opt.authority_sha256
    );
    let binaryen_authority: BuildToolAuthorityDocumentV1 =
        load_expected_document(root, &binaryen_authority_path, &mut expected_files)?;
    let wabt_authority_path = format!(
        "manifests/build-tool-authorities/{}.json",
        build.viewer.engine.wabt_wasm_strip.authority_sha256
    );
    let wabt_authority: BuildToolAuthorityDocumentV1 =
        load_expected_document(root, &wabt_authority_path, &mut expected_files)?;
    build.validate_wasm_tool_authorities(
        &wasm_bindgen_authority,
        &binaryen_authority,
        &wabt_authority,
    )?;
    let projection_authority_path = format!(
        "private/projection-authority-manifests-v2/{}.json",
        matrix.projection_authority_manifest_sha256
    );
    let projection_authority: OfficialProjectionAuthorityManifestV2 =
        load_expected_document(root, &projection_authority_path, &mut expected_files)?;
    ensure!(
        projection_authority.canonical_digest()? == matrix.projection_authority_manifest_sha256,
        "private projection authority path digest mismatch"
    );
    projection_authority.validate_against(&build)?;
    let rules_path = format!(
        "manifests/rules-configs/{}.json",
        matrix.rules_config_sha256
    );
    let rules: RulesConfigIdentityV1 =
        load_expected_document(root, &rules_path, &mut expected_files)?;
    ensure!(
        rules.canonical_digest()? == matrix.rules_config_sha256,
        "projection rules path digest mismatch"
    );
    validate_official_projection_rules_config_v1(&rules)?;
    let execution_path = format!(
        "private/projection-execution-policies/{}.json",
        matrix.execution_policy_sha256
    );
    let execution_policy: OfficialProjectionExecutionPolicyV1 =
        load_expected_document(root, &execution_path, &mut expected_files)?;
    ensure!(
        execution_policy.canonical_digest()? == matrix.execution_policy_sha256
            && execution_policy.rules_config == rules,
        "projection execution policy path/rules mismatch"
    );
    let core_path = format!(
        "private/core-overlay-source-manifests-v2/{}.json",
        matrix.core_overlay_manifest_sha256
    );
    let core_manifest: OfficialBuiltInOverlaySourceManifestV2 =
        load_expected_document(root, &core_path, &mut expected_files)?;
    ensure!(
        core_manifest.canonical_digest()? == matrix.core_overlay_manifest_sha256,
        "core overlay manifest path digest mismatch"
    );
    let exporter_relative = format!(
        "private/build-artifacts/projection-exporters/{}",
        projection_authority.projection_exporter.artifact.sha256
    );
    expected_files.insert(exporter_relative.clone());
    validate_static_projection_exporter(
        &root.join(&exporter_relative),
        &projection_authority.projection_exporter.artifact,
    )?;

    let mut receipts = Vec::with_capacity(4);
    for lane in &matrix.lanes {
        let source_path = format!(
            "private/source-tree-manifests-v2/{}.json",
            lane.source_tree_manifest_sha256
        );
        let source: OfficialSourceTreeManifestV2 =
            load_expected_document(root, &source_path, &mut expected_files)?;
        ensure!(
            source.canonical_digest()? == lane.source_tree_manifest_sha256
                && source.edition == lane.edition
                && source.source_format == lane.source_format,
            "matrix source authority is substituted"
        );
        let receipt_path = format!(
            "private/projection-receipts-v2/{}.json",
            lane.projection_receipt_sha256
        );
        let receipt: OfficialSimulationProjectionReceiptV2 =
            load_expected_document(root, &receipt_path, &mut expected_files)?;
        ensure!(
            receipt.canonical_digest()? == lane.projection_receipt_sha256
                && receipt.edition == lane.edition
                && receipt.exporter.source_format == lane.source_format,
            "matrix projection receipt is substituted"
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
            load_expected_document(root, &record_relative, &mut expected_files)?;
        ensure!(
            record.canonical_digest()? == lane.execution_record_sha256,
            "matrix execution record is substituted"
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
            artifact_from_file(&root.join(&stdout_relative), JSON_MEDIA_TYPE)? == record.stdout,
            "execution stdout differs from its record"
        );
        let stderr_relative = format!("{execution_root}/stderr.log");
        expected_files.insert(stderr_relative.clone());
        let stderr = artifact_from_file(&root.join(&stderr_relative), DIAGNOSTIC_MEDIA_TYPE)?;
        ensure!(
            stderr.sha256 == record.stderr.sha256
                && stderr.byte_length == record.stderr.byte_length,
            "execution stderr differs from its record"
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
        .map(|manifest| Ok((manifest.canonical_digest()?, manifest)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let expected_content_digests = digests
        .demo_content_manifest_sha256
        .iter()
        .chain(&digests.full_content_manifest_sha256)
        .copied()
        .collect::<BTreeSet<_>>();
    ensure!(
        receipt_content.keys().copied().collect::<BTreeSet<_>>() == expected_content_digests,
        "official content digest index differs from the receipt matrix"
    );
    let mut content = BTreeMap::new();
    for digest in expected_content_digests {
        let relative = format!("manifests/content-manifests/{digest}.json");
        let manifest: ContentManifestV1 =
            load_expected_document(root, &relative, &mut expected_files)?;
        ensure!(
            manifest.canonical_digest()? == digest
                && receipt_content.get(&digest) == Some(&manifest),
            "backend content manifest differs from its receipt"
        );
        content.insert(digest, manifest);
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
            load_expected_document(root, &relative, &mut expected_files)?;
        ensure!(
            campaign.canonical_digest()? == digest && campaign.edition == edition,
            "campaign content authority is substituted"
        );
        let expected_entries = official_content_subjects_v1(edition)
            .into_iter()
            .map(|subject| {
                let (content_digest, _) = content
                    .iter()
                    .find(|(_, manifest)| {
                        manifest.edition == edition && manifest.subject == subject
                    })
                    .context("campaign subject has no exact content manifest")?;
                Ok(CampaignContentEntryV1 {
                    subject,
                    content_manifest_sha256: *content_digest,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            campaign.entries == expected_entries,
            "campaign catalog is not the exact authentic subject/content map"
        );
        campaigns.insert(digest, campaign);
    }

    for (digest, manifest) in &content {
        let edition_lanes = matrix
            .lanes
            .iter()
            .filter(|lane| lane.edition == manifest.edition)
            .collect::<Vec<_>>();
        ensure!(edition_lanes.len() == 2, "edition has an incomplete matrix");
        let (loose, shipping) =
            if edition_lanes[0].source_format == OfficialProjectionSourceFormatV1::LooseNativeV1 {
                (edition_lanes[0], edition_lanes[1])
            } else {
                (edition_lanes[1], edition_lanes[0])
            };
        let binding_relative = format!("private/verifier-source-bindings-v2/{digest}.json");
        let binding: VerifierSourceBindingV2 =
            load_expected_document(root, &binding_relative, &mut expected_files)?;
        let expected_binding = VerifierSourceBindingV2 {
            schema_version: SOURCE_BINDING_V2_SCHEMA_VERSION,
            content_manifest_sha256: *digest,
            edition: manifest.edition,
            subject: manifest.subject.clone(),
            build_manifest_sha256: matrix.build_manifest_sha256,
            projection_authority_manifest_sha256: matrix.projection_authority_manifest_sha256,
            rules_config_sha256: matrix.rules_config_sha256,
            execution_policy_sha256: matrix.execution_policy_sha256,
            core_overlay_manifest_sha256: matrix.core_overlay_manifest_sha256,
            loose_projection_receipt_sha256: loose.projection_receipt_sha256,
            loose_source_tree_manifest_sha256: loose.source_tree_manifest_sha256,
            shipping_projection_receipt_sha256: shipping.projection_receipt_sha256,
            shipping_source_tree_manifest_sha256: shipping.source_tree_manifest_sha256,
        };
        ensure!(
            binding == expected_binding,
            "verifier source binding is substituted"
        );
        let bundle_manifest = format!("verifier-bundles/{digest}/manifest.json");
        let bundled: ContentManifestV1 =
            load_expected_document(root, &bundle_manifest, &mut expected_files)?;
        ensure!(
            &bundled == manifest,
            "verifier bundle manifest is substituted"
        );
        for component in &manifest.components {
            let component_relative =
                simulation_content_component_relative_path_v1(&manifest.subject, component.kind)?;
            let bundle_relative = format!("verifier-bundles/{digest}/catalog/{component_relative}");
            expected_files.insert(bundle_relative.clone());
            ensure!(
                artifact_from_file(
                    &root.join(&bundle_relative),
                    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
                )? == component.artifact,
                "verifier bundle component differs from its manifest"
            );
        }
    }

    let demo_campaign = &campaigns[&digests.demo_campaign_content_manifest_sha256];
    let public_campaign = format!(
        "public/manifests/campaign-content-manifests/{}.json",
        digests.demo_campaign_content_manifest_sha256
    );
    let public_demo_campaign: CampaignContentManifestV1 =
        load_expected_document(root, &public_campaign, &mut expected_files)?;
    ensure!(
        &public_demo_campaign == demo_campaign,
        "public Demo campaign manifest is substituted"
    );
    for digest in &digests.demo_content_manifest_sha256 {
        let manifest = &content[digest];
        let public_manifest = format!("public/manifests/content-manifests/{digest}.json");
        let public_content: ContentManifestV1 =
            load_expected_document(root, &public_manifest, &mut expected_files)?;
        ensure!(
            &public_content == manifest,
            "public Demo content manifest is substituted"
        );
        for component in &manifest.components {
            let relative = format!(
                "public/{}",
                demo_content_object_path_v1(*digest, component)?
            );
            expected_files.insert(relative.clone());
            ensure!(
                artifact_from_file(
                    &root.join(&relative),
                    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
                )? == component.artifact,
                "public Demo component differs from its manifest"
            );
        }
    }
    validate_read_only_tree(&root.join("verifier-bundles"))?;
    validate_exact_tree_inventory(root, &expected_files)?;
    Ok(ValidatedOfficialContentV3 {
        digests,
        matrix,
        build,
        projection_authority,
        rules,
        execution_policy,
        core_manifest,
        content,
        campaigns,
    })
}

fn validate_read_only_tree(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for (_, file) in walk_regular_files(root)? {
            ensure!(
                fs::metadata(file)?.permissions().mode() & 0o222 == 0,
                "verifier bundle file is writable"
            );
        }
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            ensure!(
                fs::metadata(&directory)?.permissions().mode() & 0o222 == 0,
                "verifier bundle directory is writable"
            );
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(())
}

fn load_expected_document<T>(
    root: &Path,
    relative: &str,
    expected_files: &mut BTreeSet<String>,
) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    ensure!(
        expected_files.insert(relative.to_owned()),
        "plan-v3 layout repeats expected path {relative}"
    );
    crate::load_canonical_document(&root.join(relative))
}

fn validate_digest_sidecar(
    root: &Path,
    relative: &str,
    digest: Digest32,
    expected_files: &mut BTreeSet<String>,
) -> Result<()> {
    ensure!(
        expected_files.insert(relative.to_owned()),
        "plan-v3 layout repeats expected path {relative}"
    );
    let bytes = crate::read_regular_file_bounded(&root.join(relative), 64)?;
    ensure!(
        bytes == digest.to_string().as_bytes(),
        "digest sidecar {relative} differs from its canonical document"
    );
    Ok(())
}

fn validate_exact_tree_inventory(root: &Path, expected_files: &BTreeSet<String>) -> Result<()> {
    let actual_files = walk_regular_files(root)?
        .into_iter()
        .map(|(relative, _)| path_to_manifest(&relative))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        actual_files == *expected_files,
        "plan-v3 output contains a missing or extra file"
    );
    let mut expected_directories = BTreeSet::from([String::new()]);
    for file in expected_files {
        let path = Path::new(file);
        let mut parent = path.parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
    }
    let actual_directories = walk_directories(root)?;
    ensure!(
        actual_directories == expected_directories,
        "plan-v3 output contains an extra or missing directory"
    );
    Ok(())
}

fn walk_directories(root: &Path) -> Result<BTreeSet<String>> {
    let mut directories = BTreeSet::from([String::new()]);
    let mut pending = vec![(PathBuf::new(), root.to_path_buf())];
    while let Some((relative_root, absolute_root)) = pending.pop() {
        for entry in fs::read_dir(absolute_root)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "plan-v3 tree contains a forbidden symlink"
            );
            if metadata.is_dir() {
                let relative = relative_root.join(entry.file_name());
                directories.insert(path_to_manifest(&relative)?);
                pending.push((relative, entry.path()));
            } else {
                ensure!(metadata.is_file(), "plan-v3 tree has a special node");
            }
        }
    }
    Ok(directories)
}

impl OfficialProjectionPlanV3 {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = crate::read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?;
        let mut plan: Self = strict_json_from_slice(&bytes)
            .with_context(|| format!("parse plan-v3 {}", path.display()))?;
        ensure!(
            plan.schema_version == PLAN_V3_SCHEMA_VERSION,
            "unsupported official projection plan schema"
        );
        let base = fs::canonicalize(config_parent(path)?)?;
        for authority in [
            &mut plan.build_manifest,
            &mut plan.wasm_bindgen_cli_authority,
            &mut plan.binaryen_wasm_opt_authority,
            &mut plan.wabt_wasm_strip_authority,
            &mut plan.projection_authority_manifest,
            &mut plan.projection_exporter,
            &mut plan.rules_config,
            &mut plan.execution_policy,
            &mut plan.core_overlay_source_root,
            &mut plan.demo.loose_source_root,
            &mut plan.demo.shipping_source_root,
            &mut plan.full.loose_source_root,
            &mut plan.full.shipping_source_root,
        ] {
            resolve_path(&base, authority);
        }
        Ok(plan)
    }

    fn validate(&self, output: &Path) -> Result<()> {
        ensure!(
            self.demo.edition == OfficialContentEditionV1::Demo
                && self.full.edition == OfficialContentEditionV1::Full,
            "plan-v3 edition slots are substituted"
        );
        for root in [
            &self.core_overlay_source_root,
            &self.demo.loose_source_root,
            &self.demo.shipping_source_root,
            &self.full.loose_source_root,
            &self.full.shipping_source_root,
        ] {
            validate_mount_root(root)?;
        }
        for path in [
            &self.build_manifest,
            &self.wasm_bindgen_cli_authority,
            &self.binaryen_wasm_opt_authority,
            &self.wabt_wasm_strip_authority,
            &self.projection_authority_manifest,
            &self.projection_exporter,
            &self.rules_config,
            &self.execution_policy,
        ] {
            validate_regular_file(path)?;
            ensure!(
                fs::canonicalize(path)? == *path,
                "authority path is not normalized"
            );
        }
        let roots = [
            &self.core_overlay_source_root,
            &self.demo.loose_source_root,
            &self.demo.shipping_source_root,
            &self.full.loose_source_root,
            &self.full.shipping_source_root,
        ];
        ensure!(
            roots.iter().collect::<BTreeSet<_>>().len() == roots.len()
                && roots.iter().enumerate().all(|(left_index, left)| {
                    roots.iter().enumerate().all(|(right_index, right)| {
                        left_index == right_index
                            || (!left.starts_with(right) && !right.starts_with(left))
                    })
                }),
            "all official source/core roots must be distinct and non-overlapping"
        );
        let output_parent = fs::canonicalize(
            output
                .parent()
                .context("official projection output has no parent")?,
        )?;
        ensure!(
            roots
                .iter()
                .all(|root| !output_parent.starts_with(root) && !root.starts_with(&output_parent)),
            "output parent must not overlap a source or core authority"
        );
        Ok(())
    }
}

fn validate_output_location(output: &Path) -> Result<()> {
    ensure!(output.is_absolute(), "plan-v3 output must be absolute");
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("plan-v3 output has no parent")?;
    fs::create_dir_all(parent)?;
    ensure!(
        fs::canonicalize(parent)? == parent,
        "plan-v3 output parent must be normalized"
    );
    Ok(())
}

fn prepare_authority(plan: &OfficialProjectionPlanV3, work: &Path) -> Result<PreparedAuthority> {
    let build: BuildManifestV2 = crate::load_canonical_document(&plan.build_manifest)?;
    validate_current_official_ranked_build_v2(&build)?;
    let build_sha256 = build.canonical_digest()?;
    let wasm_bindgen_cli_authority =
        crate::load_build_tool_authority(&plan.wasm_bindgen_cli_authority)?;
    let binaryen_wasm_opt_authority =
        crate::load_build_tool_authority(&plan.binaryen_wasm_opt_authority)?;
    let wabt_wasm_strip_authority =
        crate::load_build_tool_authority(&plan.wabt_wasm_strip_authority)?;
    build.validate_wasm_tool_authorities(
        &wasm_bindgen_cli_authority,
        &binaryen_wasm_opt_authority,
        &wabt_wasm_strip_authority,
    )?;
    let projection_authority: OfficialProjectionAuthorityManifestV2 =
        crate::load_canonical_document(&plan.projection_authority_manifest)?;
    projection_authority.validate_against(&build)?;
    let projection_authority_sha256 = projection_authority.canonical_digest()?;
    validate_static_projection_exporter(
        &plan.projection_exporter,
        &projection_authority.projection_exporter.artifact,
    )?;
    let rules: RulesConfigIdentityV1 = crate::load_canonical_document(&plan.rules_config)?;
    validate_official_projection_rules_config_v1(&rules)?;
    let rules_sha256 = rules.canonical_digest()?;
    let execution_policy: OfficialProjectionExecutionPolicyV1 =
        crate::load_canonical_document(&plan.execution_policy)?;
    ensure!(
        execution_policy.rules_config == rules,
        "execution policy does not embed the exact canonical rules config"
    );
    let execution_policy_sha256 = execution_policy.canonical_digest()?;

    let core_manifest = inventory_core_overlay(&plan.core_overlay_source_root)?;
    let core_manifest_sha256 = core_manifest.canonical_digest()?;
    let sanitized_core_root = work.join("core");
    materialize_core_overlay(
        &plan.core_overlay_source_root,
        &sanitized_core_root,
        &core_manifest,
    )?;

    let central_exporter = work.join("projection-exporter");
    copy_artifact_exact(
        &plan.projection_exporter,
        &central_exporter,
        &projection_authority.projection_exporter.artifact,
    )?;
    make_executable_read_only(&central_exporter)?;
    validate_static_projection_exporter(
        &central_exporter,
        &projection_authority.projection_exporter.artifact,
    )?;
    Ok(PreparedAuthority {
        build,
        build_sha256,
        wasm_bindgen_cli_authority,
        binaryen_wasm_opt_authority,
        wabt_wasm_strip_authority,
        projection_authority,
        projection_authority_sha256,
        rules,
        rules_sha256,
        execution_policy,
        execution_policy_sha256,
        core_manifest,
        core_manifest_sha256,
        exporter: artifact_from_file(
            &central_exporter,
            robin_run_protocol::OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
        )?,
        central_exporter,
        sanitized_core_root,
    })
}

fn prepare_source(
    edition: OfficialContentEditionV1,
    source_format: OfficialProjectionSourceFormatV1,
    original_root: &Path,
    sanitized_root: &Path,
) -> Result<PreparedSource> {
    match source_format {
        OfficialProjectionSourceFormatV1::LooseNativeV1 => {
            let inventory = inventory_loose_source_closure(
                original_root,
                crate::official_resource_locale_root(edition),
            )?;
            let manifest = loose_source_manifest_v2(edition, &inventory)?;
            materialize_loose_source_closure(original_root, sanitized_root, &inventory)?;
            let sanitized = inventory_loose_source_closure(
                sanitized_root,
                crate::official_resource_locale_root(edition),
            )?;
            ensure!(sanitized == inventory, "loose sanitized closure changed");
            Ok(PreparedSource {
                original_root: original_root.to_path_buf(),
                sanitized_root: sanitized_root.to_path_buf(),
                manifest,
                inventory: SourceInventory::Loose(inventory),
            })
        }
        OfficialProjectionSourceFormatV1::ShippingDatadirV10 => {
            let datadir_path = resolve_shipping_datadir(original_root)?;
            let datadir = ShippingDatadir::load_from_file(&datadir_path)?;
            validate_shipping_locale(&datadir, edition)?;
            let references = shipping_projection_external_file_paths_v2(&datadir)?;
            let inventory = inventory_shipping_source_closure(original_root, &references)?;
            validate_shipping_projection_source_relative_paths_v2(
                &datadir,
                &inventory
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect::<Vec<_>>(),
            )?;
            let manifest = shipping_source_manifest_v2(edition, &inventory)?;
            materialize_shipping_source_closure(
                original_root,
                sanitized_root,
                &references,
                &inventory,
            )?;
            validate_sanitized_shipping(sanitized_root, edition, &references, &inventory)?;
            Ok(PreparedSource {
                original_root: original_root.to_path_buf(),
                sanitized_root: sanitized_root.to_path_buf(),
                manifest,
                inventory: SourceInventory::Shipping {
                    references,
                    inventory,
                },
            })
        }
    }
}

fn execute_lane(
    index: usize,
    prepared: PreparedSource,
    authority: &PreparedAuthority,
    lane_root: &Path,
) -> Result<ValidatedLane> {
    let edition = prepared.manifest.edition;
    let source_format = prepared.manifest.source_format;
    let authority_root = lane_root.join("authority");
    let output_root = lane_root.join("output");
    fs::create_dir(&authority_root)?;
    fs::create_dir(&output_root)?;
    write_canonical(
        &authority_root.join("build-manifest.json"),
        &authority.build,
    )?;
    write_canonical(
        &authority_root.join("projection-authority-manifest.json"),
        &authority.projection_authority,
    )?;
    write_canonical(
        &authority_root.join("core-overlay-manifest.json"),
        &authority.core_manifest,
    )?;
    write_canonical(
        &authority_root.join("execution-policy.json"),
        &authority.execution_policy,
    )?;
    fs::hard_link(&authority.central_exporter, authority_root.join("exporter"))?;
    write_canonical(&authority_root.join("rules-config.json"), &authority.rules)?;
    write_canonical(
        &authority_root.join("source-manifest.json"),
        &prepared.manifest,
    )?;
    validate_static_projection_exporter(&authority_root.join("exporter"), &authority.exporter)?;

    let request = SandboxedProjectionRequest {
        authority_root,
        source_root: prepared.sanitized_root.clone(),
        core_overlay_root: authority.sanitized_core_root.clone(),
        output_root: output_root.clone(),
        edition,
        source_format,
        expected_exporter: authority.exporter.clone(),
    };
    let child = run_projection_exporter_v2(&request)?;
    let validated = validate_projection_output_v2(
        &request,
        child,
        &authority.build,
        &authority.projection_authority,
        &authority.rules,
        &prepared.manifest,
        &authority.core_manifest,
    )?;
    validate_source_after_execution(&prepared)?;
    ensure!(
        inventory_core_overlay(&authority.sanitized_core_root)? == authority.core_manifest,
        "sanitized core overlay changed during export"
    );

    let receipt_sha256 = validated.receipt.canonical_digest()?;
    let record = execution_record(validated.report.clone(), &validated)?;
    let execution_record_sha256 = record.canonical_digest()?;
    ensure!(
        index < 4,
        "internal lane index is outside the fixed four-lane matrix"
    );
    make_tree_writable(&prepared.sanitized_root)?;
    fs::remove_dir_all(&prepared.sanitized_root)?;
    Ok(ValidatedLane {
        edition,
        source_format,
        source_manifest_sha256: prepared.manifest.canonical_digest()?,
        source_manifest: prepared.manifest,
        projection: validated,
        receipt_sha256,
        execution_record: record,
        execution_record_sha256,
        output_root,
    })
}

fn execution_record(
    report: OfficialProjectionExportReportV2,
    projection: &ValidatedSandboxProjection,
) -> Result<OfficialProjectionExecutionRecordV3> {
    let record = OfficialProjectionExecutionRecordV3 {
        schema_version: EXECUTION_RECORD_V3_SCHEMA_VERSION,
        edition: report.edition,
        source_format: report.source_format,
        sandbox_runtime: projection.runtime.clone(),
        report,
        stdout: artifact_from_bytes(&projection.stdout, JSON_MEDIA_TYPE),
        stderr: ProjectionStreamDigestV1 {
            sha256: Digest32::digest_bytes(&projection.stderr),
            byte_length: u64::try_from(projection.stderr.len())?,
        },
    };
    record.validate()?;
    Ok(record)
}

fn validate_source_after_execution(prepared: &PreparedSource) -> Result<()> {
    match &prepared.inventory {
        SourceInventory::Loose(expected) => {
            ensure!(
                inventory_loose_source_closure(
                    &prepared.original_root,
                    &expected.resource_locale_root
                )? == *expected,
                "original loose source changed during projection"
            );
            ensure!(
                inventory_loose_source_closure(
                    &prepared.sanitized_root,
                    &expected.resource_locale_root
                )? == *expected,
                "sanitized loose source changed during projection"
            );
        }
        SourceInventory::Shipping {
            references,
            inventory,
        } => {
            let original_datadir = ShippingDatadir::load_from_file(&resolve_shipping_datadir(
                &prepared.original_root,
            )?)?;
            ensure!(
                shipping_projection_external_file_paths_v2(&original_datadir)? == *references,
                "shipping decoded reference union changed during projection"
            );
            ensure!(
                inventory_shipping_source_closure(&prepared.original_root, references)?
                    == *inventory,
                "original shipping source changed during projection"
            );
            validate_sanitized_shipping(
                &prepared.sanitized_root,
                prepared.manifest.edition,
                references,
                inventory,
            )?;
        }
    }
    Ok(())
}

fn validate_authorities_after_all_lanes(
    plan: &OfficialProjectionPlanV3,
    authority: &PreparedAuthority,
) -> Result<()> {
    ensure!(
        crate::load_canonical_document::<BuildManifestV2>(&plan.build_manifest)? == authority.build
            && crate::load_build_tool_authority(&plan.wasm_bindgen_cli_authority)?
                == authority.wasm_bindgen_cli_authority
            && crate::load_build_tool_authority(&plan.binaryen_wasm_opt_authority)?
                == authority.binaryen_wasm_opt_authority
            && crate::load_build_tool_authority(&plan.wabt_wasm_strip_authority)?
                == authority.wabt_wasm_strip_authority
            && crate::load_canonical_document::<OfficialProjectionAuthorityManifestV2>(
                &plan.projection_authority_manifest
            )? == authority.projection_authority
            && crate::load_canonical_document::<RulesConfigIdentityV1>(&plan.rules_config)?
                == authority.rules
            && crate::load_canonical_document::<OfficialProjectionExecutionPolicyV1>(
                &plan.execution_policy
            )? == authority.execution_policy,
        "canonical projection authorities changed during the four-lane run"
    );
    validate_static_projection_exporter(&plan.projection_exporter, &authority.exporter)?;
    validate_static_projection_exporter(&authority.central_exporter, &authority.exporter)?;
    ensure!(
        inventory_core_overlay(&plan.core_overlay_source_root)? == authority.core_manifest
            && inventory_core_overlay(&authority.sanitized_core_root)? == authority.core_manifest,
        "core overlay changed during the four-lane run"
    );
    Ok(())
}

fn author_equivalent_editions(lanes: &[ValidatedLane]) -> Result<[AuthoredEditionV3; 2]> {
    ensure!(
        lanes.len() == 4
            && lanes
                .iter()
                .zip(official_lane_order())
                .all(|(lane, expected)| (lane.edition, lane.source_format) == expected),
        "validated lanes are not in the fixed matrix order"
    );
    Ok([
        author_equivalent_edition(&lanes[0], &lanes[1], 0, 1)?,
        author_equivalent_edition(&lanes[2], &lanes[3], 2, 3)?,
    ])
}

fn author_equivalent_edition(
    loose: &ValidatedLane,
    shipping: &ValidatedLane,
    loose_lane: usize,
    shipping_lane: usize,
) -> Result<AuthoredEditionV3> {
    ensure!(
        loose.edition == shipping.edition
            && loose.source_format == OfficialProjectionSourceFormatV1::LooseNativeV1
            && shipping.source_format == OfficialProjectionSourceFormatV1::ShippingDatadirV10
            && loose.projection.content == shipping.projection.content,
        "loose and shipping content manifest sets differ"
    );
    let mut content = BTreeMap::new();
    let mut entries = Vec::new();
    for manifest in &loose.projection.content {
        let mut components = BTreeMap::new();
        for component in &manifest.components {
            let relative =
                simulation_content_component_relative_path_v1(&manifest.subject, component.kind)?;
            let loose_bytes = read_mounted_projection(
                &loose.output_root.join("catalog"),
                Path::new(&relative),
                component,
                "plan-v3 loose",
            )?;
            let shipping_bytes = read_mounted_projection(
                &shipping.output_root.join("catalog"),
                Path::new(&relative),
                component,
                "plan-v3 shipping",
            )?;
            ensure!(
                loose_bytes == shipping_bytes,
                "loose/shipping canonical component bytes differ for {:?}/{:?}",
                manifest.subject,
                component.kind
            );
            ensure!(
                components.insert(component.kind, loose_bytes).is_none(),
                "duplicate simulation component kind"
            );
        }
        let digest = manifest.canonical_digest()?;
        entries.push(CampaignContentEntryV1 {
            subject: manifest.subject.clone(),
            content_manifest_sha256: digest,
        });
        ensure!(
            content
                .insert(
                    digest,
                    AuthoredContent {
                        manifest: manifest.clone(),
                        components,
                    },
                )
                .is_none(),
            "two subjects produced one content digest"
        );
    }
    let campaign = CampaignContentManifestV1 {
        schema_version: 1,
        edition: loose.edition,
        entries,
    };
    campaign.validate()?;
    Ok(AuthoredEditionV3 {
        edition: loose.edition,
        content,
        campaign,
        loose_lane,
        shipping_lane,
    })
}

fn official_content_digests_v3(
    editions: &[AuthoredEditionV3; 2],
) -> Result<crate::OfficialContentDigestsV1> {
    let digests = crate::OfficialContentDigestsV1 {
        schema_version: 1,
        demo_content_manifest_sha256: editions[0].content.keys().copied().collect(),
        full_content_manifest_sha256: editions[1].content.keys().copied().collect(),
        demo_campaign_content_manifest_sha256: editions[0].campaign.canonical_digest()?,
        full_campaign_content_manifest_sha256: editions[1].campaign.canonical_digest()?,
    };
    digests.validate()?;
    Ok(digests)
}

fn authority_matrix(
    authority: &PreparedAuthority,
    lanes: &[ValidatedLane],
) -> Result<OfficialProjectionAuthorityMatrixV3> {
    let matrix = OfficialProjectionAuthorityMatrixV3 {
        schema_version: MATRIX_V3_SCHEMA_VERSION,
        build_manifest_sha256: authority.build_sha256,
        projection_authority_manifest_sha256: authority.projection_authority_sha256,
        rules_config_sha256: authority.rules_sha256,
        execution_policy_sha256: authority.execution_policy_sha256,
        core_overlay_manifest_sha256: authority.core_manifest_sha256,
        lanes: lanes
            .iter()
            .map(|lane| OfficialProjectionLaneAuthorityV3 {
                edition: lane.edition,
                source_format: lane.source_format,
                source_tree_manifest_sha256: lane.source_manifest_sha256,
                projection_receipt_sha256: lane.receipt_sha256,
                execution_record_sha256: lane.execution_record_sha256,
            })
            .collect(),
    };
    matrix.validate()?;
    Ok(matrix)
}

fn materialize_v3(
    root: &Path,
    authority: &PreparedAuthority,
    lanes: &[ValidatedLane],
    editions: &[AuthoredEditionV3; 2],
    digests: &crate::OfficialContentDigestsV1,
    matrix: &OfficialProjectionAuthorityMatrixV3,
) -> Result<()> {
    write_digest_document(
        root,
        "manifests/builds-v2",
        authority.build_sha256,
        &authority.build,
    )?;
    write_digest_document(
        root,
        "manifests/build-tool-authorities",
        authority.wasm_bindgen_cli_authority.canonical_digest()?,
        &authority.wasm_bindgen_cli_authority,
    )?;
    write_digest_document(
        root,
        "manifests/build-tool-authorities",
        authority.binaryen_wasm_opt_authority.canonical_digest()?,
        &authority.binaryen_wasm_opt_authority,
    )?;
    write_digest_document(
        root,
        "manifests/build-tool-authorities",
        authority.wabt_wasm_strip_authority.canonical_digest()?,
        &authority.wabt_wasm_strip_authority,
    )?;
    write_digest_document(
        root,
        "private/projection-authority-manifests-v2",
        authority.projection_authority_sha256,
        &authority.projection_authority,
    )?;
    write_digest_document(
        root,
        "manifests/rules-configs",
        authority.rules_sha256,
        &authority.rules,
    )?;
    write_digest_document(
        root,
        "private/projection-execution-policies",
        authority.execution_policy_sha256,
        &authority.execution_policy,
    )?;
    write_digest_document(
        root,
        "private/core-overlay-source-manifests-v2",
        authority.core_manifest_sha256,
        &authority.core_manifest,
    )?;
    let published_exporter = root
        .join("private/build-artifacts/projection-exporters")
        .join(authority.exporter.sha256.to_string());
    copy_artifact_exact(
        &authority.central_exporter,
        &published_exporter,
        &authority.exporter,
    )?;
    make_executable_read_only(&published_exporter)?;
    validate_static_projection_exporter(&published_exporter, &authority.exporter)?;

    for lane in lanes {
        write_digest_document(
            root,
            "private/source-tree-manifests-v2",
            lane.source_manifest_sha256,
            &lane.source_manifest,
        )?;
        write_digest_document(
            root,
            "private/projection-receipts-v2",
            lane.receipt_sha256,
            &lane.projection.receipt,
        )?;
        let execution = root
            .join("private/projection-executions")
            .join(lane.receipt_sha256.to_string());
        write_canonical(&execution.join("record.json"), &lane.execution_record)?;
        write_bytes(&execution.join("stdout.json"), &lane.projection.stdout)?;
        write_bytes(&execution.join("stderr.log"), &lane.projection.stderr)?;
        ensure!(
            artifact_from_file(&execution.join("stdout.json"), JSON_MEDIA_TYPE)?
                == lane.execution_record.stdout
                && artifact_from_file(&execution.join("stderr.log"), DIAGNOSTIC_MEDIA_TYPE)?.sha256
                    == lane.execution_record.stderr.sha256,
            "materialized execution streams differ from their private record"
        );
    }

    for edition in editions {
        let loose = &lanes[edition.loose_lane];
        let shipping = &lanes[edition.shipping_lane];
        let campaign_digest = edition.campaign.canonical_digest()?;
        write_digest_document(
            root,
            "manifests/campaign-content-manifests",
            campaign_digest,
            &edition.campaign,
        )?;
        if edition.edition == OfficialContentEditionV1::Demo {
            write_digest_document(
                &root.join("public"),
                "manifests/campaign-content-manifests",
                campaign_digest,
                &edition.campaign,
            )?;
        }
        for (content_digest, authored) in &edition.content {
            let binding = VerifierSourceBindingV2 {
                schema_version: SOURCE_BINDING_V2_SCHEMA_VERSION,
                content_manifest_sha256: *content_digest,
                edition: edition.edition,
                subject: authored.manifest.subject.clone(),
                build_manifest_sha256: authority.build_sha256,
                projection_authority_manifest_sha256: authority.projection_authority_sha256,
                rules_config_sha256: authority.rules_sha256,
                execution_policy_sha256: authority.execution_policy_sha256,
                core_overlay_manifest_sha256: authority.core_manifest_sha256,
                loose_projection_receipt_sha256: loose.receipt_sha256,
                loose_source_tree_manifest_sha256: loose.source_manifest_sha256,
                shipping_projection_receipt_sha256: shipping.receipt_sha256,
                shipping_source_tree_manifest_sha256: shipping.source_manifest_sha256,
            };
            write_canonical(
                &root
                    .join("private/verifier-source-bindings-v2")
                    .join(format!("{content_digest}.json")),
                &binding,
            )?;
            write_digest_document(
                root,
                "manifests/content-manifests",
                *content_digest,
                &authored.manifest,
            )?;
            if edition.edition == OfficialContentEditionV1::Demo {
                write_digest_document(
                    &root.join("public"),
                    "manifests/content-manifests",
                    *content_digest,
                    &authored.manifest,
                )?;
            }
            let bundle = root
                .join("verifier-bundles")
                .join(content_digest.to_string());
            write_canonical(&bundle.join("manifest.json"), &authored.manifest)?;
            for component in &authored.manifest.components {
                let bytes = authored
                    .components
                    .get(&component.kind)
                    .context("authored component disappeared")?;
                let relative = simulation_content_component_relative_path_v1(
                    &authored.manifest.subject,
                    component.kind,
                )?;
                write_bytes(&bundle.join("catalog").join(relative), bytes)?;
                if edition.edition == OfficialContentEditionV1::Demo {
                    let public = demo_content_object_path_v1(*content_digest, component)?;
                    write_shared_bytes(&root.join("public").join(public), bytes)?;
                }
            }
        }
    }
    write_canonical(&root.join("official-content-digests.json"), digests)?;
    write_bytes(
        &root.join("official-content-digests.sha256"),
        digests.canonical_digest()?.to_string().as_bytes(),
    )?;
    write_canonical(&root.join("projection-authority-matrix-v3.json"), matrix)?;
    write_bytes(
        &root.join("projection-authority-matrix-v3.sha256"),
        matrix.canonical_digest()?.to_string().as_bytes(),
    )?;
    Ok(())
}

fn inventory_core_overlay(root: &Path) -> Result<OfficialBuiltInOverlaySourceManifestV2> {
    validate_mount_root(root)?;
    let mut files = Vec::new();
    for (relative, absolute) in walk_regular_files(root)? {
        let path = path_to_manifest(&relative)?;
        if !path
            .split('/')
            .next()
            .is_some_and(|component| component.eq_ignore_ascii_case("Data"))
        {
            continue;
        }
        let artifact = artifact_from_file(&absolute, "application/octet-stream")?;
        files.push(OfficialSourceFileV1 {
            path,
            sha256: artifact.sha256,
            byte_length: artifact.byte_length,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = OfficialBuiltInOverlaySourceManifestV2 {
        schema_version: 2,
        kind: OfficialBuiltInOverlayKindV2::CoreDatadirV1,
        files,
    };
    manifest.validate()?;
    Ok(manifest)
}

fn materialize_core_overlay(
    source: &Path,
    destination: &Path,
    expected: &OfficialBuiltInOverlaySourceManifestV2,
) -> Result<()> {
    ensure!(!destination.exists(), "sanitized core destination exists");
    fs::create_dir(destination)?;
    for file in &expected.files {
        let artifact = ArtifactRefV1 {
            sha256: file.sha256,
            byte_length: file.byte_length,
            media_type: "application/octet-stream".into(),
        };
        copy_artifact_exact(
            &source.join(&file.path),
            &destination.join(&file.path),
            &artifact,
        )?;
    }
    ensure!(
        inventory_core_overlay(destination)? == *expected,
        "sanitized core overlay differs from its inventory"
    );
    make_verifier_bundles_read_only(destination)?;
    Ok(())
}

fn validate_shipping_locale(
    datadir: &ShippingDatadir,
    edition: OfficialContentEditionV1,
) -> Result<()> {
    let expected = crate::official_resource_locale_root(edition);
    let locale = datadir
        .locale(expected)?
        .with_context(|| format!("shipping archive has no exact LCID {expected}"))?;
    ensure!(
        locale.source_lcid.as_deref() == Some(expected),
        "shipping locale is not bound to exact source LCID {expected}"
    );
    Ok(())
}

fn validate_sanitized_shipping(
    root: &Path,
    edition: OfficialContentEditionV1,
    references: &[String],
    expected: &ShippingSourceClosureInventory,
) -> Result<()> {
    let inventory = inventory_shipping_source_closure(root, references)?;
    ensure!(&inventory == expected, "sanitized shipping closure changed");
    let datadir = ShippingDatadir::load_from_file(&resolve_shipping_datadir(root)?)?;
    validate_shipping_locale(&datadir, edition)?;
    validate_shipping_projection_source_relative_paths_v2(
        &datadir,
        &inventory
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>(),
    )?;
    ensure!(
        shipping_projection_external_file_paths_v2(&datadir)? == references,
        "sanitized shipping archive decodes a different reference union"
    );
    Ok(())
}

fn resolve_shipping_datadir(root: &Path) -> Result<PathBuf> {
    let data = unique_casefold_child(root, "Data")?
        .context("shipping source has no unique Data directory")?;
    ensure!(
        fs::symlink_metadata(&data)?.is_dir(),
        "shipping Data path is not a directory"
    );
    let datadir = unique_casefold_child(&data, "datadir.bin")?
        .context("shipping source has no unique Data/datadir.bin")?;
    validate_regular_file(&datadir)?;
    Ok(datadir)
}

fn unique_casefold_child(parent: &Path, name: &str) -> Result<Option<PathBuf>> {
    let mut matches = fs::read_dir(parent)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    matches.sort();
    ensure!(
        matches.len() <= 1,
        "filesystem has ambiguous case-folded {name} entries"
    );
    Ok(matches.pop())
}

fn make_executable_read_only(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o555))?;
    }
    Ok(())
}

fn make_tree_writable(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if !root.exists() {
            return Ok(());
        }
        let mut directories = vec![root.to_path_buf()];
        for (_, file) in walk_regular_files(root)? {
            fs::set_permissions(file, fs::Permissions::from_mode(0o600))?;
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
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for directory in directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn official_lane_order() -> [(OfficialContentEditionV1, OfficialProjectionSourceFormatV1); 4] {
    [
        (
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
        ),
        (
            OfficialContentEditionV1::Demo,
            OfficialProjectionSourceFormatV1::ShippingDatadirV10,
        ),
        (
            OfficialContentEditionV1::Full,
            OfficialProjectionSourceFormatV1::LooseNativeV1,
        ),
        (
            OfficialContentEditionV1::Full,
            OfficialProjectionSourceFormatV1::ShippingDatadirV10,
        ),
    ]
}

struct WritableTempDir(tempfile::TempDir);

impl WritableTempDir {
    fn new(output: &Path, prefix: &str) -> Result<Self> {
        let parent = output.parent().context("output has no parent")?;
        Ok(Self(
            tempfile::Builder::new().prefix(prefix).tempdir_in(parent)?,
        ))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for WritableTempDir {
    fn drop(&mut self) {
        let _ = make_tree_writable(self.0.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_rejects_missing_or_substituted_lane() {
        let digest = Digest32::digest_bytes(b"authority");
        let mut matrix = OfficialProjectionAuthorityMatrixV3 {
            schema_version: 3,
            build_manifest_sha256: digest,
            projection_authority_manifest_sha256: digest,
            rules_config_sha256: digest,
            execution_policy_sha256: digest,
            core_overlay_manifest_sha256: digest,
            lanes: official_lane_order()
                .into_iter()
                .enumerate()
                .map(
                    |(index, (edition, source_format))| OfficialProjectionLaneAuthorityV3 {
                        edition,
                        source_format,
                        source_tree_manifest_sha256: Digest32::digest_bytes(
                            format!("source-{index}").as_bytes(),
                        ),
                        projection_receipt_sha256: Digest32::digest_bytes(
                            format!("receipt-{index}").as_bytes(),
                        ),
                        execution_record_sha256: Digest32::digest_bytes(
                            format!("record-{index}").as_bytes(),
                        ),
                    },
                )
                .collect(),
        };
        matrix.validate().unwrap();
        matrix.lanes.pop();
        assert!(matrix.validate().is_err());
        matrix.lanes.push(OfficialProjectionLaneAuthorityV3 {
            edition: OfficialContentEditionV1::Full,
            source_format: OfficialProjectionSourceFormatV1::LooseNativeV1,
            source_tree_manifest_sha256: Digest32::digest_bytes(b"source-replacement"),
            projection_receipt_sha256: Digest32::digest_bytes(b"receipt-replacement"),
            execution_record_sha256: Digest32::digest_bytes(b"record-replacement"),
        });
        assert!(matrix.validate().is_err());
    }

    #[test]
    fn core_inventory_excludes_non_vfs_repository_files_and_rejects_mutable_data() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("Data/Interface")).unwrap();
        fs::write(root.path().join("README.md"), b"not mounted").unwrap();
        fs::write(root.path().join("Data/Interface/fixed.json"), b"{}").unwrap();
        let manifest = inventory_core_overlay(&fs::canonicalize(root.path()).unwrap()).unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].path, "Data/Interface/fixed.json");
        fs::create_dir_all(root.path().join("Data/cache")).unwrap();
        fs::write(root.path().join("Data/cache/host.json"), b"{}").unwrap();
        assert!(inventory_core_overlay(&fs::canonicalize(root.path()).unwrap()).is_err());
    }

    #[test]
    fn invalid_plan_cannot_create_an_output() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("release");
        let plan = root.path().join("plan.json");
        fs::write(&plan, b"{\"schema_version\":3}").unwrap();
        assert!(author_official_content_v3(&plan, &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn plan_requires_an_explicit_wasm_bindgen_authority_path() {
        let mut value = serde_json::json!({
            "schema_version": 3,
            "build_manifest": "build.json",
            "wasm_bindgen_cli_authority": "wasm-bindgen.json",
            "binaryen_wasm_opt_authority": "binaryen.json",
            "wabt_wasm_strip_authority": "wabt.json",
            "projection_authority_manifest": "projection-authority.json",
            "projection_exporter": "projection-exporter",
            "rules_config": "rules.json",
            "execution_policy": "execution-policy.json",
            "core_overlay_source_root": "core",
            "demo": {
                "edition": "demo",
                "loose_source_root": "demo-loose",
                "shipping_source_root": "demo-shipping"
            },
            "full": {
                "edition": "full",
                "loose_source_root": "full-loose",
                "shipping_source_root": "full-shipping"
            }
        });
        let plan: OfficialProjectionPlanV3 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            plan.wasm_bindgen_cli_authority,
            PathBuf::from("wasm-bindgen.json")
        );

        value
            .as_object_mut()
            .unwrap()
            .remove("wasm_bindgen_cli_authority");
        assert!(serde_json::from_value::<OfficialProjectionPlanV3>(value).is_err());
    }
}
