//! Atomic PublicationV3 deployment assembled from an admitted Plan-V3 authority.
//!
//! This layer never regenerates a projection receipt. It validates the exact
//! immutable authority in place, closes rules/policies/competitions over it,
//! copies only Demo-safe assets into the Cloudflare public-static origin, and
//! publishes one complete backend, private-verifier, public-static, and
//! isolated-signer tree with a no-replace rename.

use std::collections::{BTreeMap, BTreeSet};

mod persistence;
mod topology;
mod topology_inventory;
pub use persistence::{
    CloudflareMaterializationInstalledButParentSyncFailed,
    CloudflareMaterializationPersistenceStateUncertain, PublicationInstalledButParentSyncFailed,
};
use persistence::{
    PinnedPublicationStagingV3, PublicationPersistenceOutcome,
    PublicationPersistenceStateUncertain, create_pinned_publication_staging_v3,
    discard_failed_publication_staging, installed_publication_durability_error,
    persist_publication_staging,
};
#[cfg(test)]
use persistence::{discard_failed_publication_staging_with, persist_publication_staging_with};
use std::fs;
use std::io::{BufReader, Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use topology::{ExpectedPublicationTopologyV3, valid_publication_relative_path_v3};

use anyhow::{Context as _, Result, ensure};
use goblin::elf::{Elf, header, program_header};
use robin_run_protocol::{
    ArtifactRefV1, BuildManifestV2, BuildToolAuthorityDocumentV1,
    CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1, CampaignCompletionPolicyRequirementV1,
    CampaignContentManifestV1, CanonicalCampaignStateKindV1, CanonicalCampaignStatePinV1,
    CanonicalCampaignStateRequirementV1, CanonicalDocument as _, CompetitionManifestV1,
    ContentManifestV1, Digest32, ImmutablePolicyManifestV1, InputProvenanceEligibilityV1,
    OfficialBuiltInOverlaySourceManifestV2, OfficialContentEditionV1,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionSourceFormatV1, OfficialSimulationProjectionReceiptV2,
    OfficialSourceTreeManifestV2, OfficialViewerBuildReportV2, PublishedRulesetV1,
    RANKED_CAMPAIGN_MEDIA_TYPE_V1, RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1,
    RunContentIdentityV1, SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1,
    SimulationContentComponentDocumentV1, SimulationContentComponentKindV1, Validate as _,
    build_artifact_object_path_v1, canonical_json_bytes, demo_content_object_path_v1,
    official_achievement_policies_v1, official_content_subjects_v1,
    official_full_campaign_completion_policy_v1, simulation_content_component_relative_path_v1,
    validate_official_projection_receipt_matrix_v2,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::campaign_template_v1::{
    AdmittedProfileManagersV1, load_admitted_profile_managers_v1,
    validate_canonical_campaign_template_v1,
};
use crate::plan_v3::{
    OfficialProjectionAuthorityMatrixV3, OfficialProjectionExecutionRecordV3,
    ValidatedOfficialContentV3, VerifierSourceBindingV2, validate_official_content_v3,
};
use crate::{
    BuildDraftV2, MAX_DOCUMENT_BYTES, ReleaseFileExposureV1, ReleaseFileV1, artifact_from_file,
    config_parent, copy_artifact_exact, make_verifier_bundles_read_only, path_to_manifest,
    resolve_path, strict_json_from_slice, validate_complete_ranked_rules_config_v1,
    validate_current_official_ranked_build_v2, validate_mount_root,
    validate_official_projection_rules_config_v1, validate_regular_file, walk_regular_files,
    write_bytes, write_canonical, write_digest_document,
};

const PUBLICATION_PLAN_SCHEMA_VERSION: u32 = 3;
const PUBLICATION_MANIFEST_SCHEMA_VERSION: u32 = 3;
const PUBLICATION_LOCK_SCHEMA_VERSION: u32 = 3;
const BACKEND_PUBLICATION_SCHEMA_VERSION: u32 = 3;
const DEPLOYMENT_EXPOSURE_SCHEMA_VERSION: u32 = 3;
const CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION: u32 = 1;
const CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH: &str =
    "cloudflare-publication-materialization-v1.json";
const CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH: &str =
    "cloudflare-publication-materialization-v1.sha256";
const DATADIR_RELEASE_SCHEMA_VERSION: u32 = 1;
const DATADIR_AUTHORITY_PATH: &str = "deployment/datadir-authority.json";
const DATADIR_DEPLOYMENT_RECEIPT_PATH: &str = "deployment/datadir-deployment.json";
const DATADIR_WORKER_NAME: &str = "robinhood-datadir-assets";
const DATADIR_ROUTE_PATTERN: &str = "robinhood.phiresky.xyz/datadirs/*";
const DATADIR_PUBLIC_ROOT_URL: &str = "https://robinhood.phiresky.xyz/datadirs/";
const DEMO_CONTENT_MANIFEST_URL: &str =
    "https://robinhood.phiresky.xyz/datadirs/demo-leicester/robinhood-web-content.json";
const DEMO_DATADIR_URL: &str =
    "https://robinhood.phiresky.xyz/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst";
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_PUBLICATION_TREE_ENTRIES: usize = 1_048_576;
const MAX_PUBLICATION_TREE_DEPTH: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorPublicationPlanV3 {
    pub schema_version: u32,
    pub official_content_authority: PathBuf,
    /// Freshly reauthors the BuildManifestV2 and supplies exact local verifier
    /// and viewer files. It must equal the authority build byte-for-byte.
    pub build_draft_v2: PathBuf,
    /// Producer report for the exact engine, public-static, and signer
    /// closures. The
    /// operator still independently hashes and inventories every file.
    pub viewer_build_report: PathBuf,
    /// Canonical authority for the independently assembled Demo datadir
    /// Worker corpus. The normal publication contains this metadata but no
    /// datadir payload bytes.
    pub datadir_release_authority: PinnedArtifactSourceV3,
    /// Canonical proof of the exact Cloudflare Worker version that serves the
    /// authority above.
    pub datadir_deployment_receipt: PinnedArtifactSourceV3,
    pub verifier_operator_config: PinnedArtifactSourceV3,
    pub campaign_states: Vec<CampaignStateSourceV3>,
    #[serde(default)]
    pub additional_rules_configs: Vec<PathBuf>,
    pub policies: Vec<PathBuf>,
    pub published_rulesets: Vec<PathBuf>,
    #[serde(default)]
    pub competitions: Vec<PathBuf>,
    pub transition: PublicationTransitionV3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedArtifactSourceV3 {
    pub source: PathBuf,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignStateKindV3 {
    IndividualTemplate,
    FullCampaignGenesis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignStateSourceV3 {
    pub edition: OfficialContentEditionV1,
    pub kind: CampaignStateKindV3,
    pub rules_config_sha256: Digest32,
    pub source: PathBuf,
    pub artifact: ArtifactRefV1,
}

/// Exact Demo identity shared by the standalone datadir authority and its
/// post-deployment receipt. There is intentionally no Full-edition member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatadirDemoAuthorityV1 {
    pub content_manifest_url: String,
    pub content_manifest_sha256: Digest32,
    pub datadir_url: String,
    pub datadir_sha256: Digest32,
    pub datadir_byte_length: u64,
    pub native_content_sha256: Digest32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatadirReleaseAuthorityV1 {
    pub schema_version: u32,
    /// Producer provenance for this independently deployed immutable corpus.
    /// It is intentionally not the consuming publication's source commit.
    pub source_commit: String,
    /// Producer provenance paired with `source_commit`; the authority pin and
    /// deployment receipt bind this V1 document without coupling it to a
    /// later consuming BuildManifestV2.
    pub cargo_lock_sha256: Digest32,
    pub inventory_sha256: Digest32,
    pub worker_name: String,
    pub route_pattern: String,
    pub public_root_url: String,
    pub demo: DatadirDemoAuthorityV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatadirDeploymentReceiptV1 {
    pub schema_version: u32,
    pub authority_sha256: Digest32,
    pub inventory_sha256: Digest32,
    pub source_commit: String,
    pub worker_name: String,
    pub worker_version_id: String,
    pub route_pattern: String,
    pub public_root_url: String,
    pub demo: DatadirDemoAuthorityV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicationTransitionV3 {
    Fresh,
    Update { previous_release: PathBuf },
    StatusTransition { previous_release: PathBuf },
    Rollback { target_release: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedRulesetArtifactV3 {
    pub ruleset_manifest_sha256: Digest32,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignStateArtifactV3 {
    pub edition: OfficialContentEditionV1,
    pub kind: CampaignStateKindV3,
    pub rules_config_sha256: Digest32,
    pub artifact: ArtifactRefV1,
}

impl CampaignStateKindV3 {
    const fn canonical(self) -> CanonicalCampaignStateKindV1 {
        match self {
            Self::IndividualTemplate => CanonicalCampaignStateKindV1::IndividualTemplate,
            Self::FullCampaignGenesis => CanonicalCampaignStateKindV1::FullCampaignGenesis,
        }
    }
}

impl CampaignStateArtifactV3 {
    fn requirement(&self) -> CanonicalCampaignStateRequirementV1 {
        CanonicalCampaignStateRequirementV1 {
            edition: self.edition,
            kind: self.kind.canonical(),
            rules_config_sha256: self.rules_config_sha256,
        }
    }

    fn canonical_pin(&self) -> CanonicalCampaignStatePinV1 {
        CanonicalCampaignStatePinV1 {
            requirement: self.requirement(),
            artifact: self.artifact.clone(),
        }
    }

    const fn matrix_key(&self) -> (Digest32, OfficialContentEditionV1, CampaignStateKindV3) {
        (self.rules_config_sha256, self.edition, self.kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationManifestV3 {
    pub schema_version: u32,
    pub projection_authority_matrix_sha256: Digest32,
    pub official_content_digests_sha256: Digest32,
    pub build_manifest_sha256: Digest32,
    pub viewer_build_report: ArtifactRefV1,
    pub datadir_release_authority: ArtifactRefV1,
    pub datadir_deployment_receipt: ArtifactRefV1,
    pub verifier_operator_config: ArtifactRefV1,
    pub campaign_states: Vec<CampaignStateArtifactV3>,
    pub rules_config_sha256: Vec<Digest32>,
    pub policy_manifest_sha256: Vec<Digest32>,
    pub ruleset_manifest_sha256: Vec<Digest32>,
    pub published_rulesets: Vec<PublishedRulesetArtifactV3>,
    pub competition_manifest_sha256: Vec<Digest32>,
    pub public_static_files: Vec<PublicStaticFileArtifactV3>,
    pub identity_signer_files: Vec<PublicStaticFileArtifactV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicStaticFileArtifactV3 {
    pub published_path: String,
    pub artifact: ArtifactRefV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationLockV3 {
    pub schema_version: u32,
    pub publication_manifest_sha256: Digest32,
    /// Complete inventory excluding this lock and its sidecar.
    pub files: Vec<ReleaseFileV1>,
    /// Complete directory inventory, including the `.` publication root and
    /// every empty directory. Each entry binds its exact Unix mode.
    pub directories: Vec<PublicationDirectoryV3>,
    /// Exact Unix mode for every entry in `files`. The operator publication
    /// contract is Linux-only and mode changes are therefore part of an
    /// update/rollback decision even though they are not content identity.
    pub file_modes: Vec<PublicationFileModeV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationDirectoryV3 {
    pub path: String,
    pub unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationFileModeV3 {
    pub path: String,
    pub unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareMaterializedFileV1 {
    pub path: String,
    pub artifact: ArtifactRefV1,
    pub unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareMaterializedTreeInventoryV1 {
    pub files: Vec<CloudflareMaterializedFileV1>,
    pub directories: Vec<PublicationDirectoryV3>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudflareMaterializationOriginV1 {
    Public,
    IdentitySigner,
    DeploymentAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareMaterializedOriginInventoryV1 {
    pub schema_version: u32,
    pub origin: CloudflareMaterializationOriginV1,
    pub root: String,
    pub files: Vec<CloudflareMaterializedFileV1>,
    pub directories: Vec<PublicationDirectoryV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareMaterializedOriginAuthorityV1 {
    pub origin: CloudflareMaterializationOriginV1,
    pub root: String,
    pub inventory_path: String,
    pub inventory: ArtifactRefV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflarePublicationMaterializationV1 {
    pub schema_version: u32,
    pub publication_schema_version: u32,
    pub source_commit: String,
    pub source_tree_sha1: String,
    pub cargo_lock_sha256: Digest32,
    pub publication_manifest_sha256: Digest32,
    pub publication_lock_sha256: Digest32,
    pub origins: Vec<CloudflareMaterializedOriginAuthorityV1>,
    /// Exact output closure after all copied payloads and origin inventory
    /// documents exist, but before this self-referential receipt and its
    /// digest sidecar are added.
    pub output_inventory: CloudflareMaterializedTreeInventoryV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendPublicationV3 {
    pub schema_version: u32,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Vec<Digest32>,
    pub campaign_content_manifest_sha256: Vec<Digest32>,
    pub rules_config_sha256: Vec<Digest32>,
    pub ruleset_manifest_sha256: Vec<Digest32>,
    pub competition_manifest_sha256: Vec<Digest32>,
    pub policy_manifest_sha256: Vec<Digest32>,
    pub verifier_program: ArtifactRefV1,
    pub verifier_operator_config: ArtifactRefV1,
    pub campaign_states: Vec<CampaignStateArtifactV3>,
}

/// Machine-checkable filesystem and origin boundary for a published release.
///
/// This document deliberately does not configure the backend itself. The
/// backend receives only `backend/manifests` as its public manifest registry;
/// its publication summary and all verifier inputs remain operator-private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentExposureV3 {
    pub schema_version: u32,
    pub public_origin: String,
    pub identity_signer_origin: String,
    pub public_static_root: String,
    pub identity_signer_static_root: String,
    pub backend_api_manifest_root: String,
    pub backend_api_route: String,
    pub cloudflare_zone: String,
    pub cloudflare_routes: Vec<CloudflareRouteV3>,
    pub operator_private_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudflareRouteV3 {
    pub pattern: String,
    pub script: Option<String>,
}

impl DeploymentExposureV3 {
    fn official() -> Self {
        Self {
            schema_version: DEPLOYMENT_EXPOSURE_SCHEMA_VERSION,
            public_origin: "https://robinhood.phiresky.xyz".into(),
            identity_signer_origin: "https://identity.robinhood.phiresky.xyz".into(),
            public_static_root: "cloudflare-public".into(),
            identity_signer_static_root: "cloudflare-identity-signer".into(),
            backend_api_manifest_root: "backend/manifests".into(),
            backend_api_route: "/api*".into(),
            cloudflare_zone: "phiresky.xyz".into(),
            cloudflare_routes: vec![
                CloudflareRouteV3 {
                    pattern: "robinhood.phiresky.xyz/api*".into(),
                    script: None,
                },
                CloudflareRouteV3 {
                    pattern: "robinhood.phiresky.xyz/.well-known/acme-challenge/*".into(),
                    script: None,
                },
                CloudflareRouteV3 {
                    pattern: "robinhood.phiresky.xyz/wasm/*".into(),
                    script: Some("robinhood-runtime-assets".into()),
                },
                CloudflareRouteV3 {
                    pattern: "robinhood.phiresky.xyz/datadirs/*".into(),
                    script: Some("robinhood-datadir-assets".into()),
                },
                CloudflareRouteV3 {
                    pattern: "robinhood.phiresky.xyz/*".into(),
                    script: Some("robinhood-public-site".into()),
                },
            ],
            operator_private_paths: vec![
                "backend/publication-v3.json".into(),
                "deployment".into(),
                "private".into(),
                "publication-lock-v3.json".into(),
                "publication-lock-v3.sha256".into(),
                "publication-manifest-v3.json".into(),
                "publication-manifest-v3.sha256".into(),
            ],
        }
    }

    fn validate_exact(&self) -> Result<()> {
        ensure!(
            self == &Self::official(),
            "deployment exposure differs from the reviewed V3 boundary"
        );
        Ok(())
    }
}

impl robin_run_protocol::Validate for DeploymentExposureV3 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self != &Self::official() {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "deployment_exposure_v3",
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for PublicationManifestV3 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != PUBLICATION_MANIFEST_SCHEMA_VERSION
            || self.projection_authority_matrix_sha256.is_zero()
            || self.official_content_digests_sha256.is_zero()
            || self.build_manifest_sha256.is_zero()
            || self.rules_config_sha256.is_empty()
            || self.policy_manifest_sha256.is_empty()
            || self.ruleset_manifest_sha256.is_empty()
            || self.published_rulesets.is_empty()
            || self.public_static_files.is_empty()
            || self.identity_signer_files.is_empty()
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "publication_manifest_v3",
            });
        }
        self.viewer_build_report.validate()?;
        if self.viewer_build_report.media_type != "application/json" {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "publication_manifest_v3.viewer_build_report.media_type",
            });
        }
        self.datadir_release_authority.validate()?;
        self.datadir_deployment_receipt.validate()?;
        if self.datadir_release_authority.media_type != "application/json"
            || self.datadir_deployment_receipt.media_type != "application/json"
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "publication_manifest_v3.datadir_metadata.media_type",
            });
        }
        self.verifier_operator_config.validate()?;
        for digests in [
            &self.rules_config_sha256,
            &self.policy_manifest_sha256,
            &self.ruleset_manifest_sha256,
        ] {
            if !strict_digests(digests) {
                return Err(robin_run_protocol::ValidationError::NotCanonicalOrder {
                    field: "publication_manifest_v3.digests",
                });
            }
        }
        if !self.competition_manifest_sha256.is_empty()
            && !strict_digests(&self.competition_manifest_sha256)
        {
            return Err(robin_run_protocol::ValidationError::NotCanonicalOrder {
                field: "publication_manifest_v3.competitions",
            });
        }
        if !self
            .published_rulesets
            .windows(2)
            .all(|pair| pair[0].ruleset_manifest_sha256 < pair[1].ruleset_manifest_sha256)
            || self.published_rulesets.iter().any(|published| {
                published.ruleset_manifest_sha256.is_zero()
                    || published.artifact.validate().is_err()
            })
            || !self
                .public_static_files
                .windows(2)
                .all(|pair| pair[0].published_path < pair[1].published_path)
            || self.public_static_files.iter().any(|file| {
                !valid_lock_path(&file.published_path) || file.artifact.validate().is_err()
            })
            || !self
                .identity_signer_files
                .windows(2)
                .all(|pair| pair[0].published_path < pair[1].published_path)
            || self.identity_signer_files.iter().any(|file| {
                !valid_lock_path(&file.published_path) || file.artifact.validate().is_err()
            })
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "publication_manifest_v3.artifacts",
            });
        }
        if !campaign_state_matrix_is_exact(&self.campaign_states, &self.rules_config_sha256)
            || self
                .campaign_states
                .iter()
                .any(|state| state.canonical_pin().validate().is_err())
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "publication_manifest_v3.campaign_states",
            });
        }
        Ok(())
    }
}

impl robin_run_protocol::Validate for PublicationLockV3 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != PUBLICATION_LOCK_SCHEMA_VERSION
            || self.publication_manifest_sha256.is_zero()
            || self.files.is_empty()
            || !self
                .files
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            || self.directories.first().map(|entry| entry.path.as_str()) != Some(".")
            || !self
                .directories
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            || self.directories.iter().any(|entry| {
                (entry.path != "." && !valid_lock_path(&entry.path))
                    || !valid_unix_mode(entry.unix_mode)
            })
            || self.file_modes.len() != self.files.len()
            || !self
                .file_modes
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
            || self
                .file_modes
                .iter()
                .any(|fact| !valid_lock_path(&fact.path) || !valid_unix_mode(fact.unix_mode))
            || !self
                .files
                .iter()
                .zip(&self.file_modes)
                .all(|(file, mode)| file.path == mode.path)
            || self
                .files
                .iter()
                .any(|file| !valid_lock_path(&file.path) || file.artifact.validate().is_err())
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "publication_lock_v3",
            });
        }
        Ok(())
    }
}

fn validate_cloudflare_materialized_tree_inventory_v1(
    inventory: &CloudflareMaterializedTreeInventoryV1,
) -> std::result::Result<(), robin_run_protocol::ValidationError> {
    if inventory.files.is_empty()
        || !inventory
            .files
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        || inventory.files.iter().any(|file| {
            !valid_lock_path(&file.path)
                || file.unix_mode != 0o444
                || file.artifact.validate().is_err()
        })
        || inventory
            .directories
            .first()
            .map(|entry| entry.path.as_str())
            != Some(".")
        || !inventory
            .directories
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        || inventory.directories.iter().any(|directory| {
            (directory.path != "." && !valid_lock_path(&directory.path))
                || directory.unix_mode != 0o555
        })
    {
        return Err(robin_run_protocol::ValidationError::ClaimMismatch {
            field: "cloudflare_materialized_tree_inventory_v1",
        });
    }
    let directories = inventory
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect::<BTreeSet<_>>();
    if inventory.files.iter().any(|file| {
        let parent = Path::new(&file.path)
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .and_then(|path| path.to_str())
            .unwrap_or(".");
        !directories.contains(parent)
    }) {
        return Err(robin_run_protocol::ValidationError::ClaimMismatch {
            field: "cloudflare_materialized_tree_inventory_v1.parent",
        });
    }
    Ok(())
}

impl robin_run_protocol::Validate for CloudflareMaterializedTreeInventoryV1 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        validate_cloudflare_materialized_tree_inventory_v1(self)
    }
}

impl robin_run_protocol::Validate for CloudflareMaterializedOriginInventoryV1 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
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
        if self.schema_version != CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION
            || self.root != self.origin.root()
            || self.files.iter().any(|file| {
                let folded = file.path.to_ascii_lowercase();
                PRIVATE_PATH_TERMS.iter().any(|term| folded.contains(term))
            })
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "cloudflare_materialized_origin_inventory_v1",
            });
        }
        validate_cloudflare_materialized_tree_inventory_v1(&CloudflareMaterializedTreeInventoryV1 {
            files: self.files.clone(),
            directories: self.directories.clone(),
        })
    }
}

impl CloudflareMaterializationOriginV1 {
    const fn root(self) -> &'static str {
        match self {
            Self::Public => "cloudflare-public",
            Self::IdentitySigner => "cloudflare-identity-signer",
            Self::DeploymentAuthority => "deployment",
        }
    }

    const fn inventory_path(self) -> &'static str {
        match self {
            Self::Public => "inventories/cloudflare-public-v1.json",
            Self::IdentitySigner => "inventories/cloudflare-identity-signer-v1.json",
            Self::DeploymentAuthority => "inventories/deployment-authority-v1.json",
        }
    }
}

impl robin_run_protocol::Validate for CloudflarePublicationMaterializationV1 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION
            || self.publication_schema_version != PUBLICATION_MANIFEST_SCHEMA_VERSION
            || !valid_lower_hex(&self.source_commit, 40)
            || !valid_lower_hex(&self.source_tree_sha1, 40)
            || self.cargo_lock_sha256.is_zero()
            || self.publication_manifest_sha256.is_zero()
            || self.publication_lock_sha256.is_zero()
            || self.origins.len() != 3
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "cloudflare_publication_materialization_v1",
            });
        }
        let expected = [
            CloudflareMaterializationOriginV1::Public,
            CloudflareMaterializationOriginV1::IdentitySigner,
            CloudflareMaterializationOriginV1::DeploymentAuthority,
        ];
        for (origin, expected) in self.origins.iter().zip(expected) {
            if origin.origin != expected
                || origin.root != expected.root()
                || origin.inventory_path != expected.inventory_path()
                || origin.inventory.validate().is_err()
                || origin.inventory.media_type != "application/json"
            {
                return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                    field: "cloudflare_publication_materialization_v1.origins",
                });
            }
        }
        validate_cloudflare_materialized_tree_inventory_v1(&self.output_inventory)
    }
}

fn valid_lock_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
}

fn valid_unix_mode(mode: u32) -> bool {
    mode != 0 && mode & !0o7777 == 0
}

impl robin_run_protocol::Validate for BackendPublicationV3 {
    fn validate(&self) -> std::result::Result<(), robin_run_protocol::ValidationError> {
        if self.schema_version != BACKEND_PUBLICATION_SCHEMA_VERSION
            || self.build_manifest_sha256.is_zero()
            || self.content_manifest_sha256.is_empty()
            || self.campaign_content_manifest_sha256.len() != 2
            || !strict_digests(&self.content_manifest_sha256)
            || !strict_digests(&self.campaign_content_manifest_sha256)
            || !strict_digests(&self.rules_config_sha256)
            || !strict_digests(&self.ruleset_manifest_sha256)
            || !strict_digests(&self.policy_manifest_sha256)
            || (!self.competition_manifest_sha256.is_empty()
                && !strict_digests(&self.competition_manifest_sha256))
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "backend_publication_v3",
            });
        }
        self.verifier_program.validate()?;
        self.verifier_operator_config.validate()?;
        if !campaign_state_matrix_is_exact(&self.campaign_states, &self.rules_config_sha256)
            || self
                .campaign_states
                .iter()
                .any(|state| state.canonical_pin().validate().is_err())
        {
            return Err(robin_run_protocol::ValidationError::ClaimMismatch {
                field: "backend_publication_v3.campaign_states",
            });
        }
        Ok(())
    }
}

fn strict_digests(values: &[Digest32]) -> bool {
    !values.is_empty()
        && values.iter().all(|digest| !digest.is_zero())
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn campaign_state_matrix_is_exact(
    states: &[CampaignStateArtifactV3],
    rules_config_sha256: &[Digest32],
) -> bool {
    let expected = rules_config_sha256
        .iter()
        .flat_map(|rules_config_sha256| {
            [
                (
                    *rules_config_sha256,
                    OfficialContentEditionV1::Demo,
                    CampaignStateKindV3::IndividualTemplate,
                ),
                (
                    *rules_config_sha256,
                    OfficialContentEditionV1::Full,
                    CampaignStateKindV3::FullCampaignGenesis,
                ),
            ]
        })
        .collect::<Vec<_>>();
    states
        .iter()
        .map(CampaignStateArtifactV3::matrix_key)
        .eq(expected)
}

impl DatadirDemoAuthorityV1 {
    fn validate_exact(&self) -> Result<()> {
        ensure!(
            self.content_manifest_url == DEMO_CONTENT_MANIFEST_URL,
            "datadir authority uses an unexpected Demo content-manifest URL"
        );
        ensure!(
            self.datadir_url == DEMO_DATADIR_URL,
            "datadir authority uses an unexpected Demo archive URL"
        );
        ensure!(
            !self.content_manifest_sha256.is_zero()
                && !self.datadir_sha256.is_zero()
                && !self.native_content_sha256.is_zero(),
            "datadir Demo identity contains a zero digest"
        );
        ensure!(
            self.datadir_byte_length > 0 && self.datadir_byte_length <= MAX_SAFE_JSON_INTEGER,
            "datadir Demo archive length is not a positive JSON-safe integer"
        );
        Ok(())
    }
}

impl DatadirReleaseAuthorityV1 {
    fn validate_exact(&self) -> Result<()> {
        ensure!(
            self.schema_version == DATADIR_RELEASE_SCHEMA_VERSION,
            "unsupported datadir release authority schema"
        );
        ensure!(
            valid_lower_hex(&self.source_commit, 40),
            "datadir authority source commit is not a full lowercase Git SHA-1"
        );
        ensure!(
            !self.cargo_lock_sha256.is_zero(),
            "datadir authority Cargo.lock digest is zero"
        );
        ensure!(
            !self.inventory_sha256.is_zero(),
            "datadir authority inventory digest is zero"
        );
        ensure!(
            self.worker_name == DATADIR_WORKER_NAME
                && self.route_pattern == DATADIR_ROUTE_PATTERN
                && self.public_root_url == DATADIR_PUBLIC_ROOT_URL,
            "datadir authority targets an unexpected Worker, route, or origin"
        );
        self.demo.validate_exact()
    }
}

impl DatadirDeploymentReceiptV1 {
    fn validate_exact(
        &self,
        authority: &DatadirReleaseAuthorityV1,
        authority_sha256: Digest32,
    ) -> Result<()> {
        ensure!(
            self.schema_version == DATADIR_RELEASE_SCHEMA_VERSION,
            "unsupported datadir deployment receipt schema"
        );
        ensure!(
            !self.authority_sha256.is_zero() && self.authority_sha256 == authority_sha256,
            "datadir deployment receipt does not bind the exact authority bytes"
        );
        ensure!(
            self.inventory_sha256 == authority.inventory_sha256
                && self.source_commit == authority.source_commit
                && self.worker_name == authority.worker_name
                && self.route_pattern == authority.route_pattern
                && self.public_root_url == authority.public_root_url
                && self.demo == authority.demo,
            "datadir deployment receipt differs from its release authority"
        );
        ensure!(
            valid_lowercase_uuid(&self.worker_version_id),
            "datadir Worker version is not a lowercase UUID"
        );
        Ok(())
    }
}

fn validate_datadir_binding(
    authority: &DatadirReleaseAuthorityV1,
    authority_sha256: Digest32,
    receipt: &DatadirDeploymentReceiptV1,
) -> Result<()> {
    authority.validate_exact()?;
    receipt.validate_exact(authority, authority_sha256)
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_lowercase_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

/// Assemble one exact deployable publication from a previously admitted
/// plan-v3 authority. The authority is validated but no receipt is regenerated.
pub fn assemble_publication_v3(plan_path: &Path, output: &Path) -> Result<Digest32> {
    ensure!(output.is_absolute(), "publication output must be absolute");
    let loaded = load_publication(plan_path)?;
    let staging = create_pinned_publication_staging_v3(output)?;
    let assembled = (|| {
        let official_authority = materialize_publication(staging.path(), &loaded)?;
        let manifest = publication_manifest(&loaded)?;
        write_canonical(
            &staging.path().join("publication-manifest-v3.json"),
            &manifest,
        )?;
        write_bytes(
            &staging.path().join("publication-manifest-v3.sha256"),
            manifest.canonical_digest()?.to_string().as_bytes(),
        )?;
        make_verifier_bundles_read_only(
            &staging
                .path()
                .join("private/official-content-authority/verifier-bundles"),
        )?;
        make_private_executables_and_states_read_only(staging.path(), &loaded)?;
        let mut topology =
            expected_publication_topology_from_loaded_v3(&loaded, &manifest, &official_authority)?;
        topology.seal_and_validate(staging.path(), &staging.root)?;
        let lock = publication_lock(&topology, manifest.canonical_digest()?)?;
        write_canonical(&staging.path().join("publication-lock-v3.json"), &lock)?;
        let lock_sha256 = lock.canonical_digest()?;
        write_bytes(
            &staging.path().join("publication-lock-v3.sha256"),
            lock_sha256.to_string().as_bytes(),
        )?;
        make_lock_files_read_only(staging.path())?;
        topology.register_canonical("publication-lock-v3.json".into(), &lock)?;
        topology.register_bytes(
            "publication-lock-v3.sha256".into(),
            lock_sha256.to_string().as_bytes(),
        )?;
        topology.seal_and_validate(staging.path(), &staging.root)?;
        let candidate = validate_pinned_publication_v3(staging.path(), &staging.root)?;
        ensure!(
            candidate.lock_sha256() == lock_sha256,
            "validated staging lock differs from the locally authored PublicationV3 lock"
        );
        validate_transition(&loaded.plan.transition, &candidate)?;
        Ok((lock_sha256, candidate))
    })();
    match assembled {
        Ok((lock_sha256, candidate)) => {
            candidate.ensure_live()?;
            match persist_publication_staging(&staging, &candidate, output) {
                Ok(PublicationPersistenceOutcome::Published) => {
                    Ok(lock_sha256)
                }
                Ok(PublicationPersistenceOutcome::PublishedButParentSyncFailed(sync_error)) => {
                    Err(installed_publication_durability_error(
                        output,
                        lock_sha256,
                        sync_error,
                    ))
                }
                Err(persist_error)
                    if persist_error
                        .downcast_ref::<PublicationPersistenceStateUncertain>()
                        .is_some() =>
                {
                    Err(persist_error)
                }
                Err(persist_error) => match discard_failed_publication_staging(staging) {
                    Ok(()) => Err(persist_error),
                    Err(cleanup_error) => Err(persist_error.context(format!(
                        "publication persistence also failed to securely remove staging: {cleanup_error:#}"
                    ))),
                },
            }
        }
        Err(assembly_error) => match discard_failed_publication_staging(staging) {
            Ok(()) => Err(assembly_error),
            Err(cleanup_error) => Err(assembly_error.context(format!(
                "publication assembly also failed to securely remove staging: {cleanup_error:#}"
            ))),
        },
    }
}
#[cfg(test)]
mod tests;

mod plan;

mod cloudflare;

mod closure;

mod staging;

mod inventory;

use plan::LoadedPublication;

use plan::load_publication;

use plan::validate_complete_ranked_rules_config;

#[cfg(test)]
use plan::validate_campaign_state_source;

#[cfg(test)]
use cloudflare::derive_cloudflare_origin_inventories_v1;

#[cfg(test)]
use cloudflare::CloudflareMaterializationProvenanceV1;

#[cfg(test)]
use cloudflare::resolve_cloudflare_materialization_git_authority_v1;

#[cfg(test)]
use cloudflare::expected_topology_from_materialized_inventory_v1;

#[cfg(test)]
use cloudflare::populate_cloudflare_materialization_staging_v1;

#[cfg(test)]
use cloudflare::persist_cloudflare_materialization_v1;

#[cfg(not(target_os = "linux"))]
#[cfg(target_os = "linux")]
pub use cloudflare::materialize_cloudflare_publication_v3;

#[cfg(not(target_os = "linux"))]
#[cfg(target_os = "linux")]
pub use cloudflare::validate_cloudflare_publication_materialization_v1;

use closure::validate_publication_closure;

#[cfg(test)]
use closure::validate_official_campaign_offer_fields;

use closure::expected_publication_topology_from_loaded_v3;

use closure::publication_lock;

#[cfg(test)]
use closure::publication_lock_from_actual_for_test;

#[cfg(test)]
use closure::validate_publication_inventory_against_lock_v3;

#[cfg(target_os = "linux")]
#[cfg(not(target_os = "linux"))]
pub use closure::validate_publication_v3;

#[cfg(target_os = "linux")]
pub(crate) use closure::validate_pinned_publication_v3;

#[cfg(target_os = "linux")]
pub(crate) use closure::validate_publication_v3_authority;

#[cfg(test)]
use closure::validate_deployment_exposure;

use closure::release_file_exposure;

#[cfg(test)]
use closure::validate_deployment_metadata_inventory;

#[cfg(test)]
use closure::scan_public_tree;

#[cfg(test)]
use closure::PublicJsonSchema;

#[cfg(test)]
use closure::public_json_schema;

#[cfg(test)]
use closure::reject_private_json_keys;

#[cfg(test)]
use closure::file_contains_bytes;

#[cfg(test)]
use closure::load_addressed_documents;

use closure::validate_transition;

#[cfg(test)]
use closure::validate_transition_with;

#[cfg(test)]
use closure::TransitionRule;

#[cfg(test)]
use closure::compare_transition;

use staging::materialize_publication;

use staging::backend_publication;

use staging::publication_manifest;

#[cfg(test)]
use staging::copy_directory_exact_preserving_modes;

#[cfg(test)]
use staging::copy_directory_exact_preserving_modes_with;

#[cfg(test)]
use staging::create_private_publication_root;

#[cfg(unix)]
use staging::make_private_executables_and_states_read_only;

#[cfg(unix)]
#[cfg(not(unix))]
use staging::make_lock_files_read_only;

pub(crate) use inventory::PublicationNodeIdentityV3;

use inventory::PublicationTreeInventoryV3;

pub(crate) use inventory::ValidatedPublicationV3;

pub(crate) use inventory::PublicationTreeSnapshotV3;

use inventory::PublicationTreeAuthorityV3;

use inventory::publication_inventory_matches_after_root_rename_v3;

#[cfg(target_os = "linux")]
use inventory::read_inventory_file_v3;

#[cfg(target_os = "linux")]
use inventory::load_inventory_canonical_document_v3;

#[cfg(target_os = "linux")]
use inventory::load_inventory_document_v3;

#[cfg(target_os = "linux")]
use inventory::inventory_artifact_v3;

use inventory::inventory_relative_files_v3;

use inventory::inventory_has_directory_v3;

#[cfg(target_os = "linux")]
use inventory::publication_node_identity_v3;

use inventory::publication_same_stable_node_v3;

#[cfg(target_os = "linux")]
use inventory::open_publication_root_v3;

#[cfg(target_os = "linux")]
use inventory::pin_publication_root_parent_v3;

#[cfg(target_os = "linux")]
use inventory::open_publication_child_v3;

#[cfg(target_os = "linux")]
use inventory::open_publication_child_identity_v3;

#[cfg(target_os = "linux")]
use inventory::open_optional_publication_child_identity_v3;

#[cfg(target_os = "linux")]
use inventory::publication_directory_entries_v3;

#[cfg(target_os = "linux")]
use inventory::validate_publication_node_v3;

#[cfg(target_os = "linux")]
use inventory::stable_publication_file_artifact_v3;

#[cfg(test)]
use inventory::stable_publication_file_artifact_v3_with;

#[cfg(target_os = "linux")]
use inventory::publication_tree_inventory_v3_from_fd;

#[cfg(test)]
use inventory::publication_tree_inventory_v3_from_fd_with;

#[cfg(target_os = "linux")]
#[cfg(not(target_os = "linux"))]
use inventory::publication_tree_inventory_v3;

#[cfg(test)]
use inventory::publication_directories;

#[cfg(test)]
use inventory::reject_publication_mounts_in_v3;

#[cfg(test)]
use inventory::publication_unix_mode;

use inventory::file_artifacts;

use inventory::immutable_transition_files;

use inventory::status_transition_files;
