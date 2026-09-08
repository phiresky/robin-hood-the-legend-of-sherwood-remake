//! Atomic PublicationV3 deployment assembled from an admitted Plan-V3 authority.
//!
//! This layer never regenerates a projection receipt. It validates the exact
//! immutable authority in place, closes rules/policies/competitions over it,
//! copies only Demo-safe assets into the Cloudflare public-static origin, and
//! publishes one complete backend, private-verifier, public-static, and
//! isolated-signer tree with a no-replace rename.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufReader, Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};

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

fn load_pinned_json_source<T>(source: &PinnedArtifactSourceV3) -> Result<T>
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
struct LoadedPublication {
    plan: OperatorPublicationPlanV3,
    authority: ValidatedOfficialContentV3,
    build_draft: BuildDraftV2,
    build_sha256: Digest32,
    viewer_build_report_artifact: ArtifactRefV1,
    rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1>,
    policies: BTreeMap<Digest32, ImmutablePolicyManifestV1>,
    published: BTreeMap<Digest32, PublishedRulesetV1>,
    competitions: BTreeMap<Digest32, CompetitionManifestV1>,
    campaign_states: Vec<CampaignStateArtifactV3>,
}

impl OperatorPublicationPlanV3 {
    fn load(path: &Path) -> Result<Self> {
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

#[derive(Debug)]
struct PinnedPublicationStagingV3 {
    path: PathBuf,
    name: std::ffi::OsString,
    root: fs::File,
    parent_path: PathBuf,
    parent: fs::File,
    parent_identity: PublicationNodeIdentityV3,
}

impl PinnedPublicationStagingV3 {
    fn path(&self) -> &Path {
        &self.path
    }

    fn ensure_live(&self) -> Result<()> {
        let rebound_parent = open_publication_root_v3(&self.parent_path)?;
        ensure!(
            publication_same_stable_node_v3(
                &publication_node_identity_v3(&self.parent.metadata()?),
                &self.parent_identity,
            ) && publication_same_stable_node_v3(
                &publication_node_identity_v3(&rebound_parent.metadata()?),
                &self.parent_identity,
            ),
            "PublicationV3 staging parent was substituted"
        );
        let rebound = open_publication_child_v3(&rebound_parent, Path::new(&self.name))?;
        ensure!(
            publication_same_stable_node_v3(
                &publication_node_identity_v3(&rebound.metadata()?),
                &publication_node_identity_v3(&self.root.metadata()?),
            ),
            "PublicationV3 staging basename was substituted"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn create_pinned_publication_staging_v3(output: &Path) -> Result<PinnedPublicationStagingV3> {
    use rustix::fs::{Mode, mkdirat};
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::MetadataExt as _;

    let parent_path = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("PublicationV3 output has no parent")?
        .to_path_buf();
    let parent = open_publication_root_v3(&parent_path)?;
    let parent_metadata = parent.metadata()?;
    ensure!(
        parent_metadata.is_dir() && parent_metadata.uid() == rustix::process::geteuid().as_raw(),
        "PublicationV3 output parent is not an owned directory"
    );
    let mut created_name = None;
    for _ in 0..128 {
        let name = std::ffi::OsString::from(format!(
            ".robin-manifestctl-{}-{:016x}.partial",
            std::process::id(),
            fastrand::u64(..)
        ));
        match mkdirat(
            parent.as_fd(),
            Path::new(&name),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        ) {
            Ok(()) => {
                created_name = Some(name);
                break;
            }
            Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let name = created_name.context("exhausted PublicationV3 staging name attempts")?;
    let root = open_publication_child_v3(&parent, Path::new(&name))?;
    let root_metadata = root.metadata()?;
    ensure!(
        root_metadata.is_dir()
            && root_metadata.uid() == rustix::process::geteuid().as_raw()
            && root_metadata.dev() == parent_metadata.dev(),
        "PublicationV3 staging root is not an owned same-device directory"
    );
    let parent_identity = publication_node_identity_v3(&parent.metadata()?);
    let path = parent_path.join(&name);
    let staging = PinnedPublicationStagingV3 {
        path,
        name,
        root,
        parent_path,
        parent,
        parent_identity,
    };
    staging.ensure_live()?;
    Ok(staging)
}

#[cfg(not(target_os = "linux"))]
fn create_pinned_publication_staging_v3(_output: &Path) -> Result<PinnedPublicationStagingV3> {
    anyhow::bail!("PublicationV3 pinned staging requires Linux openat2")
}

#[cfg(target_os = "linux")]
struct CloudflareMaterializationOutputBuilderV1 {
    directories: BTreeMap<String, fs::File>,
    files: BTreeMap<String, fs::File>,
}

#[cfg(target_os = "linux")]
impl CloudflareMaterializationOutputBuilderV1 {
    fn new(staging: &PinnedPublicationStagingV3) -> Result<Self> {
        Ok(Self {
            directories: BTreeMap::from([(".".to_owned(), staging.root.try_clone()?)]),
            files: BTreeMap::new(),
        })
    }

    fn create_directory(&mut self, path: &str) -> Result<()> {
        use rustix::fs::{Mode, mkdirat};
        use std::os::fd::AsFd as _;

        ensure!(
            path != "." && valid_publication_relative_path_v3(path),
            "invalid Cloudflare materialization directory {path}"
        );
        ensure!(
            !self.directories.contains_key(path),
            "duplicate Cloudflare materialization directory {path}"
        );
        let relative = Path::new(path);
        let parent = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(path_to_manifest)
            .transpose()?
            .unwrap_or_else(|| ".".to_owned());
        let parent = self
            .directories
            .get(&parent)
            .with_context(|| format!("Cloudflare materialization omits parent of {path}"))?;
        let name = relative
            .file_name()
            .context("Cloudflare materialization directory has no basename")?;
        mkdirat(parent.as_fd(), name, Mode::RUSR | Mode::WUSR | Mode::XUSR)?;
        let directory = open_publication_child_v3(parent, Path::new(name))?;
        ensure!(
            directory.metadata()?.is_dir(),
            "Cloudflare materialization directory is not a directory"
        );
        self.directories.insert(path.to_owned(), directory);
        Ok(())
    }

    fn create_file(&mut self, path: &str) -> Result<fs::File> {
        use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
        use std::os::fd::AsFd as _;

        ensure!(
            valid_publication_relative_path_v3(path) && !self.files.contains_key(path),
            "invalid or duplicate Cloudflare materialization file {path}"
        );
        let relative = Path::new(path);
        let parent_key = relative
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(path_to_manifest)
            .transpose()?
            .unwrap_or_else(|| ".".to_owned());
        let parent = self
            .directories
            .get(&parent_key)
            .with_context(|| format!("Cloudflare materialization omits parent of {path}"))?;
        let name = relative
            .file_name()
            .context("Cloudflare materialization file has no basename")?;
        let descriptor = openat2(
            parent.as_fd(),
            Path::new(name),
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )?;
        let file = fs::File::from(descriptor);
        self.files.insert(path.to_owned(), file.try_clone()?);
        Ok(file)
    }

    fn write_bytes(&mut self, path: &str, bytes: &[u8]) -> Result<ArtifactRefV1> {
        let mut file = self.create_file(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len())?,
            media_type: "application/json".into(),
        })
    }

    fn ensure_exact_retained_nodes(&self, inventory: &PublicationTreeInventoryV3) -> Result<()> {
        ensure!(
            self.files.len() == inventory.files.len()
                && self.directories.len() == inventory.directories.len(),
            "Cloudflare materialization retained output topology is incomplete"
        );
        for file in &inventory.files {
            let retained = self.files.get(&file.path).with_context(|| {
                format!("Cloudflare materialization did not retain {}", file.path)
            })?;
            ensure!(
                publication_node_identity_v3(&retained.metadata()?) == file.identity,
                "Cloudflare materialization file inode was substituted at {}",
                file.path
            );
        }
        for ((path, retained), (identity_path, identity)) in
            self.directories.iter().zip(&inventory.directory_identities)
        {
            ensure!(
                path == identity_path
                    && publication_node_identity_v3(&retained.metadata()?) == *identity,
                "Cloudflare materialization directory inode was substituted at {path}"
            );
        }
        Ok(())
    }
}

fn load_publication(plan_path: &Path) -> Result<LoadedPublication> {
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

fn validate_complete_ranked_rules_config(config: &RulesConfigIdentityV1) -> Result<()> {
    validate_complete_ranked_rules_config_v1(config)
}

fn load_documents<T>(paths: &[PathBuf]) -> Result<BTreeMap<Digest32, T>>
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

fn load_published(paths: &[PathBuf]) -> Result<BTreeMap<Digest32, PublishedRulesetV1>> {
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

fn validate_publication_closure(loaded: &LoadedPublication) -> Result<()> {
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

fn validate_document_closure(
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

fn validate_official_campaign_board_policy(
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

fn validate_official_campaign_offer_fields(
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

fn validate_authentic_content_catalogs(
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

fn materialize_publication(
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

fn backend_publication(loaded: &LoadedPublication) -> Result<BackendPublicationV3> {
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

fn publication_manifest(loaded: &LoadedPublication) -> Result<PublicationManifestV3> {
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

fn expected_publication_topology_from_loaded_v3(
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedPublicationFileV3 {
    artifact: ArtifactRefV1,
    unix_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedPublicationTopologyV3 {
    files: BTreeMap<String, ExpectedPublicationFileV3>,
    directories: BTreeMap<String, u32>,
}

impl ExpectedPublicationTopologyV3 {
    fn new() -> Self {
        Self::new_with_root_mode(0o700)
    }

    fn new_with_root_mode(root_mode: u32) -> Self {
        Self {
            files: BTreeMap::new(),
            directories: BTreeMap::from([(".".to_owned(), root_mode)]),
        }
    }

    fn register_directory(&mut self, path: &str) -> Result<()> {
        ensure!(
            path == "." || valid_publication_relative_path_v3(path),
            "invalid expected PublicationV3 directory {path}"
        );
        if path != "." {
            let mut parent = Path::new(path).parent().map(Path::to_path_buf);
            while let Some(directory_path) = parent {
                let directory = directory_path.as_path();
                if directory.as_os_str().is_empty() {
                    break;
                }
                let directory = path_to_manifest(directory)?;
                match self.directories.insert(directory.clone(), 0o555) {
                    Some(mode) => ensure!(
                        mode == 0o555,
                        "conflicting expected PublicationV3 directory mode at {directory}"
                    ),
                    None => {}
                }
                parent = Path::new(&directory).parent().map(Path::to_path_buf);
            }
        }
        let mode = if path == "." {
            *self
                .directories
                .get(".")
                .context("expected PublicationV3 topology omits its root")?
        } else {
            0o555
        };
        match self.directories.insert(path.to_owned(), mode) {
            Some(previous) => ensure!(
                previous == mode,
                "conflicting expected PublicationV3 directory mode at {path}"
            ),
            None => {}
        }
        Ok(())
    }

    fn register_file(
        &mut self,
        path: String,
        artifact: &ArtifactRefV1,
        executable: bool,
    ) -> Result<()> {
        ensure!(
            valid_publication_relative_path_v3(&path),
            "invalid expected PublicationV3 file {path}"
        );
        let parent = Path::new(&path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(path_to_manifest)
            .transpose()?
            .unwrap_or_else(|| ".".to_owned());
        self.register_directory(&parent)?;
        let mut artifact = artifact.clone();
        artifact.media_type = "application/octet-stream".into();
        ensure!(
            self.files
                .insert(
                    path.clone(),
                    ExpectedPublicationFileV3 {
                        artifact,
                        unix_mode: if executable { 0o555 } else { 0o444 },
                    },
                )
                .is_none(),
            "duplicate expected PublicationV3 file registration at {path}"
        );
        Ok(())
    }

    fn register_canonical<T>(&mut self, path: String, document: &T) -> Result<()>
    where
        T: Serialize,
    {
        let bytes = canonical_json_bytes(document)?;
        self.register_file(
            path,
            &ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&bytes),
                byte_length: u64::try_from(bytes.len())?,
                media_type: "application/json".into(),
            },
            false,
        )
    }

    fn register_bytes(&mut self, path: String, bytes: &[u8]) -> Result<()> {
        self.register_file(
            path,
            &ArtifactRefV1 {
                sha256: Digest32::digest_bytes(bytes),
                byte_length: u64::try_from(bytes.len())?,
                media_type: "application/octet-stream".into(),
            },
            false,
        )
    }

    fn register_inventory_file(
        &mut self,
        inventory: &PublicationTreeInventoryV3,
        path: String,
        executable: bool,
    ) -> Result<()> {
        let artifact = inventory
            .files
            .iter()
            .find(|file| file.path == path)
            .with_context(|| format!("validated PublicationV3 omits expected file {path}"))?
            .artifact
            .clone();
        self.register_file(path, &artifact, executable)
    }

    fn validate_inventory(&self, inventory: &PublicationTreeInventoryV3) -> Result<()> {
        let actual_files = inventory
            .files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    ExpectedPublicationFileV3 {
                        artifact: file.artifact.clone(),
                        unix_mode: file.unix_mode,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let actual_directories = inventory
            .directories
            .iter()
            .map(|directory| (directory.path.clone(), directory.unix_mode))
            .collect::<BTreeMap<_, _>>();
        ensure!(
            actual_files == self.files && actual_directories == self.directories,
            "PublicationV3 file/directory topology differs from its independently derived typed closure"
        );
        Ok(())
    }

    fn validate_inventory_content(&self, inventory: &PublicationTreeInventoryV3) -> Result<()> {
        let actual_files = inventory
            .files
            .iter()
            .map(|file| (file.path.clone(), file.artifact.clone()))
            .collect::<BTreeMap<_, _>>();
        let expected_files = self
            .files
            .iter()
            .map(|(path, file)| (path.clone(), file.artifact.clone()))
            .collect::<BTreeMap<_, _>>();
        let actual_directories = inventory
            .directories
            .iter()
            .map(|directory| directory.path.clone())
            .collect::<BTreeSet<_>>();
        let expected_directories = self.directories.keys().cloned().collect::<BTreeSet<_>>();
        ensure!(
            actual_files == expected_files && actual_directories == expected_directories,
            "PublicationV3 content topology differs from its independently derived typed closure"
        );
        Ok(())
    }

    fn materialized_inventory(&self) -> CloudflareMaterializedTreeInventoryV1 {
        CloudflareMaterializedTreeInventoryV1 {
            files: self
                .files
                .iter()
                .map(|(path, file)| CloudflareMaterializedFileV1 {
                    path: path.clone(),
                    artifact: file.artifact.clone(),
                    unix_mode: file.unix_mode,
                })
                .collect(),
            directories: self
                .directories
                .iter()
                .map(|(path, unix_mode)| PublicationDirectoryV3 {
                    path: path.clone(),
                    unix_mode: *unix_mode,
                })
                .collect(),
        }
    }

    #[cfg(target_os = "linux")]
    fn seal_and_validate(&self, root_path: &Path, root: &fs::File) -> Result<()> {
        use rustix::fs::{Mode, fchmod};
        use std::os::fd::AsFd as _;

        let inventory = publication_tree_inventory_v3_from_fd(root_path, root)?;
        self.validate_inventory_content(&inventory)?;
        for file in &inventory.files {
            let expected = self
                .files
                .get(&file.path)
                .context("expected PublicationV3 file disappeared while sealing")?;
            fchmod(file.file.as_fd(), Mode::from_raw_mode(expected.unix_mode))?;
            file.file.sync_all()?;
        }
        let mut directories = inventory.directory_files.iter().collect::<Vec<_>>();
        directories
            .sort_by_key(|(path, _)| std::cmp::Reverse(Path::new(path).components().count()));
        for (path, directory) in directories {
            let mode = self
                .directories
                .get(path)
                .context("expected PublicationV3 directory disappeared while sealing")?;
            fchmod(directory.as_fd(), Mode::from_raw_mode(*mode))?;
            directory.sync_all()?;
        }
        let sealed = publication_tree_inventory_v3_from_fd(root_path, root)?;
        self.validate_inventory(&sealed)
    }

    fn lock(&self, manifest_sha256: Digest32) -> Result<PublicationLockV3> {
        ensure!(
            !self.files.contains_key("publication-lock-v3.json")
                && !self.files.contains_key("publication-lock-v3.sha256"),
            "PublicationV3 lock files were registered before lock authoring"
        );
        let files = self
            .files
            .iter()
            .map(|(path, file)| ReleaseFileV1 {
                exposure: release_file_exposure(path),
                path: path.clone(),
                artifact: file.artifact.clone(),
            })
            .collect();
        let file_modes = self
            .files
            .iter()
            .map(|(path, file)| PublicationFileModeV3 {
                path: path.clone(),
                unix_mode: file.unix_mode,
            })
            .collect();
        let directories = self
            .directories
            .iter()
            .map(|(path, unix_mode)| PublicationDirectoryV3 {
                path: path.clone(),
                unix_mode: *unix_mode,
            })
            .collect();
        let lock = PublicationLockV3 {
            schema_version: PUBLICATION_LOCK_SCHEMA_VERSION,
            publication_manifest_sha256: manifest_sha256,
            files,
            directories,
            file_modes,
        };
        lock.validate()?;
        Ok(lock)
    }
}

fn valid_publication_relative_path_v3(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && path.split('/').all(|component| {
            !component.is_empty() && !matches!(component, "." | "..") && !component.contains('\\')
        })
}

fn publication_lock(
    topology: &ExpectedPublicationTopologyV3,
    manifest_sha256: Digest32,
) -> Result<PublicationLockV3> {
    topology.lock(manifest_sha256)
}

#[cfg(all(test, target_os = "linux"))]
fn publication_lock_from_actual_for_test(
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

fn validate_publication_inventory_against_lock_v3(
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
        return Ok(validate_publication_v3_authority(root)?.lock_sha256);
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("PublicationV3 validation requires Linux openat2 filesystem authority")
}

fn cloudflare_origin_inventory_v1(
    origin: CloudflareMaterializationOriginV1,
    files: Vec<(String, ArtifactRefV1)>,
) -> Result<CloudflareMaterializedOriginInventoryV1> {
    let mut admitted = BTreeMap::new();
    for (path, artifact) in files {
        ensure!(
            valid_publication_relative_path_v3(&path),
            "invalid Cloudflare materialization path {path}"
        );
        artifact.validate()?;
        ensure!(
            admitted.insert(path.clone(), artifact).is_none(),
            "Cloudflare materialization repeats typed path {path}"
        );
    }
    ensure!(
        !admitted.is_empty(),
        "Cloudflare materialization origin is empty"
    );
    let mut directories = BTreeSet::from([".".to_owned()]);
    for path in admitted.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            directories.insert(path_to_manifest(directory)?);
            parent = directory.parent();
        }
    }
    let inventory = CloudflareMaterializedOriginInventoryV1 {
        schema_version: CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION,
        origin,
        root: origin.root().into(),
        files: admitted
            .into_iter()
            .map(|(path, artifact)| CloudflareMaterializedFileV1 {
                path,
                artifact,
                unix_mode: 0o444,
            })
            .collect(),
        directories: directories
            .into_iter()
            .map(|path| PublicationDirectoryV3 {
                path,
                unix_mode: 0o555,
            })
            .collect(),
    };
    inventory.validate()?;
    Ok(inventory)
}

fn derive_cloudflare_origin_inventories_v1(
    publication: &ValidatedPublicationV3,
    manifest: &PublicationManifestV3,
    build: &BuildManifestV2,
) -> Result<Vec<CloudflareMaterializedOriginInventoryV1>> {
    let build_path = format!("manifests/builds/{}.json", manifest.build_manifest_sha256);
    let mut public = vec![(
        build_path,
        ArtifactRefV1 {
            sha256: manifest.build_manifest_sha256,
            byte_length: u64::try_from(canonical_json_bytes(build)?.len())?,
            media_type: "application/json".into(),
        },
    )];
    public.extend(
        build
            .viewer
            .engine
            .artifacts
            .iter()
            .map(|named| {
                Ok((
                    build_artifact_object_path_v1(manifest.build_manifest_sha256, named)?,
                    named.artifact.clone(),
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    public.extend(
        manifest
            .public_static_files
            .iter()
            .map(|file| (file.published_path.clone(), file.artifact.clone())),
    );
    let signer = manifest
        .identity_signer_files
        .iter()
        .map(|file| (file.published_path.clone(), file.artifact.clone()))
        .collect();
    let deployment = [
        "exposure-v3.json",
        "datadir-authority.json",
        "datadir-deployment.json",
    ]
    .into_iter()
    .map(|relative| {
        Ok((
            relative.to_owned(),
            publication.artifact(&format!("deployment/{relative}"), "application/json")?,
        ))
    })
    .collect::<Result<Vec<_>>>()?;
    let inventories = vec![
        cloudflare_origin_inventory_v1(CloudflareMaterializationOriginV1::Public, public)?,
        cloudflare_origin_inventory_v1(CloudflareMaterializationOriginV1::IdentitySigner, signer)?,
        cloudflare_origin_inventory_v1(
            CloudflareMaterializationOriginV1::DeploymentAuthority,
            deployment,
        )?,
    ];
    for inventory in &inventories {
        let actual_files = publication
            .relative_files(&inventory.root)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let expected_files = inventory
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            actual_files == expected_files,
            "PublicationV3 {} origin differs from its typed materialization file closure",
            inventory.root
        );
        let actual_directories = publication
            .relative_directories(&inventory.root)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let expected_directories = inventory
            .directories
            .iter()
            .map(|directory| directory.path.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            actual_directories == expected_directories,
            "PublicationV3 {} origin differs from its typed materialization directory closure",
            inventory.root
        );
        for file in &inventory.files {
            ensure!(
                publication.artifact(
                    &format!("{}/{}", inventory.root, file.path),
                    &file.artifact.media_type,
                )? == file.artifact,
                "PublicationV3 typed materialization artifact differs at {}/{}",
                inventory.root,
                file.path
            );
        }
    }
    Ok(inventories)
}

fn register_cloudflare_origin_v1(
    expected: &mut ExpectedPublicationTopologyV3,
    inventory: &CloudflareMaterializedOriginInventoryV1,
) -> Result<()> {
    for directory in &inventory.directories {
        let path = if directory.path == "." {
            inventory.root.clone()
        } else {
            format!("{}/{}", inventory.root, directory.path)
        };
        expected.register_directory(&path)?;
    }
    for file in &inventory.files {
        expected.register_file(
            format!("{}/{}", inventory.root, file.path),
            &file.artifact,
            false,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct CloudflareMaterializationProvenanceV1 {
    source_commit: String,
    source_tree_sha1: String,
    cargo_lock_sha256: Digest32,
    publication_manifest_sha256: Digest32,
    publication_lock_sha256: Digest32,
}

#[cfg(target_os = "linux")]
fn resolve_cloudflare_materialization_git_authority_v1(
    repository: &fs::File,
) -> Result<(String, String)> {
    use std::os::fd::AsRawFd as _;

    let repository_fd = format!("/proc/self/fd/{}", repository.as_raw_fd());
    let resolve = |revision: &str| -> Result<String> {
        let git = std::process::Command::new("/usr/bin/git")
            .args([
                "--no-replace-objects",
                "rev-parse",
                "--verify",
                "--end-of-options",
                revision,
            ])
            .current_dir(&repository_fd)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_NAMESPACE")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .with_context(|| {
                format!("resolve exact Cloudflare materialization Git revision {revision}")
            })?;
        ensure!(
            git.status.success(),
            "resolve Cloudflare materialization Git revision {revision} failed: {}",
            String::from_utf8_lossy(&git.stderr)
        );
        let resolved = std::str::from_utf8(&git.stdout)
            .context("Git authority output is not UTF-8")?
            .strip_suffix('\n')
            .context("Git authority output omits its one terminal newline")?;
        ensure!(
            valid_lower_hex(resolved, 40),
            "Cloudflare materialization Git revision {revision} is not one exact SHA-1"
        );
        Ok(resolved.to_owned())
    };
    let source_commit = resolve("HEAD^{commit}")?;
    let source_tree_sha1 = resolve(&format!("{source_commit}^{{tree}}"))?;
    Ok((source_commit, source_tree_sha1))
}

#[cfg(target_os = "linux")]
fn load_cloudflare_materialization_provenance_v1(
    publication: &mut ValidatedPublicationV3,
    expected_publication_lock_sha256: Digest32,
    repo_root: &Path,
) -> Result<(
    PublicationManifestV3,
    BuildManifestV2,
    CloudflareMaterializationProvenanceV1,
)> {
    use std::os::unix::fs::MetadataExt as _;

    ensure!(
        publication.lock_sha256() == expected_publication_lock_sha256,
        "PublicationV3 lock differs from the independently approved digest"
    );
    let manifest: PublicationManifestV3 =
        publication.load_document("publication-manifest-v3.json")?;
    let publication_manifest_sha256 = manifest.canonical_digest()?;
    let build_path = format!(
        "backend/manifests/builds/{}.json",
        manifest.build_manifest_sha256
    );
    let build: BuildManifestV2 = publication.load_document(&build_path)?;
    ensure!(
        build.canonical_digest()? == manifest.build_manifest_sha256
            && valid_lower_hex(&build.source_commit, 40),
        "PublicationV3 BuildManifestV2 has invalid source authority"
    );

    ensure!(
        repo_root.is_absolute(),
        "Cloudflare materialization repository root must be absolute"
    );
    let repository = open_publication_root_v3(repo_root)?;
    let repository_identity = publication_node_identity_v3(&repository.metadata()?);
    ensure!(
        repository.metadata()?.is_dir()
            && repository.metadata()?.uid() == rustix::process::geteuid().as_raw(),
        "Cloudflare materialization repository is not an owned directory"
    );
    let mut cargo_lock = open_publication_child_v3(&repository, Path::new("Cargo.lock"))?;
    let cargo_identity = publication_node_identity_v3(&cargo_lock.metadata()?);
    ensure!(
        cargo_lock.metadata()?.is_file()
            && cargo_lock.metadata()?.uid() == rustix::process::geteuid().as_raw()
            && cargo_lock.metadata()?.dev() == repository.metadata()?.dev()
            && cargo_lock.metadata()?.nlink() == 1,
        "Cloudflare materialization Cargo.lock is not an owned same-device singleton"
    );
    let cargo_artifact =
        stable_publication_file_artifact_v3(&mut cargo_lock, &cargo_identity, "Cargo.lock")?;
    ensure!(
        cargo_artifact.sha256 == build.cargo_lock_sha256,
        "Cloudflare materialization Cargo.lock differs from BuildManifestV2"
    );

    let (source_commit, source_tree_sha1) =
        resolve_cloudflare_materialization_git_authority_v1(&repository)?;
    ensure!(
        source_commit == build.source_commit,
        "Cloudflare materialization checkout differs from BuildManifestV2"
    );

    let rebound_repository = open_publication_root_v3(repo_root)?;
    let rebound_cargo = open_publication_child_v3(&repository, Path::new("Cargo.lock"))?;
    ensure!(
        publication_same_stable_node_v3(
            &publication_node_identity_v3(&rebound_repository.metadata()?),
            &repository_identity,
        ) && publication_node_identity_v3(&rebound_cargo.metadata()?) == cargo_identity,
        "Cloudflare materialization repository authority changed during admission"
    );
    publication.ensure_live()?;
    Ok((
        manifest,
        build,
        CloudflareMaterializationProvenanceV1 {
            source_commit,
            source_tree_sha1,
            cargo_lock_sha256: cargo_artifact.sha256,
            publication_manifest_sha256,
            publication_lock_sha256: expected_publication_lock_sha256,
        },
    ))
}

fn same_artifact_bytes_v1(left: &ArtifactRefV1, right: &ArtifactRefV1) -> bool {
    left.sha256 == right.sha256 && left.byte_length == right.byte_length
}

fn expected_topology_from_materialized_inventory_v1(
    inventory: &CloudflareMaterializedTreeInventoryV1,
) -> Result<ExpectedPublicationTopologyV3> {
    inventory.validate()?;
    let mut expected = ExpectedPublicationTopologyV3::new_with_root_mode(0o555);
    for directory in &inventory.directories {
        if directory.path != "." {
            expected.register_directory(&directory.path)?;
        }
    }
    for file in &inventory.files {
        expected.register_file(file.path.clone(), &file.artifact, false)?;
    }
    ensure!(
        expected.directories.keys().eq(inventory
            .directories
            .iter()
            .map(|directory| &directory.path)),
        "Cloudflare materialization inventory has an untyped empty directory"
    );
    Ok(expected)
}

fn register_prefixed_materialized_origin_v1(
    expected: &mut ExpectedPublicationTopologyV3,
    inventory: &CloudflareMaterializedOriginInventoryV1,
) -> Result<()> {
    inventory.validate()?;
    for directory in &inventory.directories {
        let path = if directory.path == "." {
            inventory.root.clone()
        } else {
            format!("{}/{}", inventory.root, directory.path)
        };
        expected.register_directory(&path)?;
    }
    for file in &inventory.files {
        expected.register_file(
            format!("{}/{}", inventory.root, file.path),
            &file.artifact,
            false,
        )?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn load_cloudflare_materialization_canonical_document_v1<T>(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = read_inventory_file_v3(inventory, path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse Cloudflare materialization document {path}"))?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "Cloudflare materialization document {path} is not canonical JSON"
    );
    Ok(document)
}

#[cfg(target_os = "linux")]
fn derive_cloudflare_materialized_origin_authority_v1(
    inventory: &mut PublicationTreeInventoryV3,
    receipt: &CloudflarePublicationMaterializationV1,
    admitted: &BTreeMap<CloudflareMaterializationOriginV1, CloudflareMaterializedOriginInventoryV1>,
) -> Result<Vec<CloudflareMaterializedOriginInventoryV1>> {
    let public = admitted
        .get(&CloudflareMaterializationOriginV1::Public)
        .context("Cloudflare materialization omits its public origin inventory")?;
    let build_paths = public
        .files
        .iter()
        .filter_map(|file| {
            let name = file.path.strip_prefix("manifests/builds/")?;
            let digest = name.strip_suffix(".json")?;
            (valid_lower_hex(digest, 64) && file.artifact.media_type == "application/json")
                .then_some((file.path.clone(), digest.to_owned(), file.artifact.clone()))
        })
        .collect::<Vec<_>>();
    ensure!(
        build_paths.len() == 1,
        "Cloudflare materialization public origin must contain one canonical BuildManifestV2"
    );
    let (build_path, build_digest, claimed_build_artifact) = &build_paths[0];
    let full_build_path = format!("cloudflare-public/{build_path}");
    let build: BuildManifestV2 = load_inventory_document_v3(inventory, &full_build_path)?;
    validate_current_official_ranked_build_v2(&build)?;
    let build_bytes = canonical_json_bytes(&build)?;
    let build_sha256 = Digest32::digest_bytes(&build_bytes);
    ensure!(
        build_sha256.to_string() == *build_digest
            && receipt.source_commit == build.source_commit
            && receipt.cargo_lock_sha256 == build.cargo_lock_sha256
            && *claimed_build_artifact
                == ArtifactRefV1 {
                    sha256: build_sha256,
                    byte_length: u64::try_from(build_bytes.len())?,
                    media_type: "application/json".into(),
                },
        "Cloudflare materialization receipt/build authority is inconsistent"
    );

    let mut public_files = vec![(build_path.clone(), claimed_build_artifact.clone())];
    public_files.extend(
        build
            .viewer
            .engine
            .artifacts
            .iter()
            .map(|named| {
                Ok((
                    build_artifact_object_path_v1(build_sha256, named)?,
                    named.artifact.clone(),
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    public_files.extend(
        build
            .viewer
            .pages_shell
            .public_origin_artifacts
            .iter()
            .map(|file| (file.path.clone(), file.artifact.clone())),
    );
    let signer_files = build
        .viewer
        .identity_signer
        .identity_signer_origin_artifacts
        .iter()
        .map(|file| (file.path.clone(), file.artifact.clone()))
        .collect();

    let exposure_path = "deployment/exposure-v3.json";
    let datadir_authority_path = "deployment/datadir-authority.json";
    let datadir_receipt_path = "deployment/datadir-deployment.json";
    let _: DeploymentExposureV3 = load_inventory_document_v3(inventory, exposure_path)?;
    let datadir_authority: DatadirReleaseAuthorityV1 =
        load_cloudflare_materialization_canonical_document_v1(inventory, datadir_authority_path)?;
    let datadir_receipt: DatadirDeploymentReceiptV1 =
        load_cloudflare_materialization_canonical_document_v1(inventory, datadir_receipt_path)?;
    let datadir_authority_artifact =
        inventory_artifact_v3(inventory, datadir_authority_path, "application/json")?;
    validate_datadir_binding(
        &datadir_authority,
        datadir_authority_artifact.sha256,
        &datadir_receipt,
    )?;
    let deployment_files = [
        (
            "exposure-v3.json".to_owned(),
            inventory_artifact_v3(inventory, exposure_path, "application/json")?,
        ),
        (
            "datadir-authority.json".to_owned(),
            datadir_authority_artifact,
        ),
        (
            "datadir-deployment.json".to_owned(),
            inventory_artifact_v3(inventory, datadir_receipt_path, "application/json")?,
        ),
    ];
    let expected = vec![
        cloudflare_origin_inventory_v1(CloudflareMaterializationOriginV1::Public, public_files)?,
        cloudflare_origin_inventory_v1(
            CloudflareMaterializationOriginV1::IdentitySigner,
            signer_files,
        )?,
        cloudflare_origin_inventory_v1(
            CloudflareMaterializationOriginV1::DeploymentAuthority,
            deployment_files.into_iter().collect(),
        )?,
    ];
    ensure!(
        expected
            .iter()
            .all(|origin| admitted.get(&origin.origin) == Some(origin)),
        "Cloudflare materialization origin inventories differ from their embedded typed authority"
    );
    Ok(expected)
}

#[cfg(target_os = "linux")]
fn validate_cloudflare_materialization_inventory_v1(
    root_path: &Path,
    root: &fs::File,
    expected_receipt_sha256: Digest32,
) -> Result<(
    CloudflarePublicationMaterializationV1,
    PublicationTreeInventoryV3,
)> {
    let mut inventory = publication_tree_inventory_v3_from_fd(root_path, root)?;
    let initial_snapshot = inventory.snapshot();
    let receipt: CloudflarePublicationMaterializationV1 =
        load_inventory_document_v3(&mut inventory, CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH)?;
    let receipt_sha256 = receipt.canonical_digest()?;
    ensure!(
        receipt_sha256 == expected_receipt_sha256
            && read_inventory_file_v3(
                &mut inventory,
                CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH,
                64,
            )? == receipt_sha256.to_string().as_bytes(),
        "Cloudflare materialization receipt differs from its approved digest or sidecar"
    );
    let mut admitted_origins = BTreeMap::new();
    for binding in &receipt.origins {
        let document: CloudflareMaterializedOriginInventoryV1 =
            load_inventory_document_v3(&mut inventory, &binding.inventory_path)?;
        ensure!(
            document.origin == binding.origin
                && document.root == binding.root
                && inventory_artifact_v3(&inventory, &binding.inventory_path, "application/json")?
                    == binding.inventory,
            "Cloudflare materialization origin inventory is substituted"
        );
        ensure!(
            admitted_origins.insert(document.origin, document).is_none(),
            "Cloudflare materialization repeats one origin inventory"
        );
    }
    let expected_origins = derive_cloudflare_materialized_origin_authority_v1(
        &mut inventory,
        &receipt,
        &admitted_origins,
    )?;
    let mut expected_pre_receipt = ExpectedPublicationTopologyV3::new_with_root_mode(0o555);
    for document in &expected_origins {
        register_prefixed_materialized_origin_v1(&mut expected_pre_receipt, document)?;
    }
    for binding in &receipt.origins {
        expected_pre_receipt.register_inventory_file(
            &inventory,
            binding.inventory_path.clone(),
            false,
        )?;
    }
    ensure!(
        expected_pre_receipt.materialized_inventory() == receipt.output_inventory,
        "Cloudflare materialization receipt output inventory differs from its typed origin authorities"
    );
    let mut expected_final =
        expected_topology_from_materialized_inventory_v1(&receipt.output_inventory)?;
    expected_final.register_canonical(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH.into(), &receipt)?;
    expected_final.register_bytes(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH.into(),
        receipt_sha256.to_string().as_bytes(),
    )?;
    expected_final.validate_inventory(&inventory)?;
    ensure!(
        publication_tree_inventory_v3_from_fd(root_path, root)?.snapshot() == initial_snapshot,
        "Cloudflare materialization changed during receipt validation"
    );
    Ok((receipt, inventory))
}

#[cfg(target_os = "linux")]
fn populate_cloudflare_materialization_staging_v1<F>(
    staging: &PinnedPublicationStagingV3,
    publication: &mut ValidatedPublicationV3,
    provenance: &CloudflareMaterializationProvenanceV1,
    origins: &[CloudflareMaterializedOriginInventoryV1],
    before_source_acceptance: F,
) -> Result<(Digest32, ValidatedPublicationV3)>
where
    F: FnOnce(),
{
    let mut expected = ExpectedPublicationTopologyV3::new_with_root_mode(0o555);
    for origin in origins {
        register_cloudflare_origin_v1(&mut expected, origin)?;
    }
    expected.register_directory("inventories")?;
    let mut builder = CloudflareMaterializationOutputBuilderV1::new(staging)?;
    for directory in expected
        .directories
        .keys()
        .filter(|path| path.as_str() != ".")
    {
        builder.create_directory(directory)?;
    }
    for origin in origins {
        for file in &origin.files {
            let source_path = format!("{}/{}", origin.root, file.path);
            let mut output = builder.create_file(&source_path)?;
            let copied = publication.copy_file_to(&source_path, &mut output)?;
            ensure!(
                same_artifact_bytes_v1(&copied, &file.artifact),
                "retained PublicationV3 extraction differs at {source_path}"
            );
        }
    }
    let mut bindings = Vec::with_capacity(origins.len());
    for origin in origins {
        let bytes = canonical_json_bytes(origin)?;
        let artifact = builder.write_bytes(origin.origin.inventory_path(), &bytes)?;
        expected.register_file(origin.origin.inventory_path().into(), &artifact, false)?;
        bindings.push(CloudflareMaterializedOriginAuthorityV1 {
            origin: origin.origin,
            root: origin.root.clone(),
            inventory_path: origin.origin.inventory_path().into(),
            inventory: artifact,
        });
    }
    let receipt = CloudflarePublicationMaterializationV1 {
        schema_version: CLOUDFLARE_MATERIALIZATION_SCHEMA_VERSION,
        publication_schema_version: PUBLICATION_MANIFEST_SCHEMA_VERSION,
        source_commit: provenance.source_commit.clone(),
        source_tree_sha1: provenance.source_tree_sha1.clone(),
        cargo_lock_sha256: provenance.cargo_lock_sha256,
        publication_manifest_sha256: provenance.publication_manifest_sha256,
        publication_lock_sha256: provenance.publication_lock_sha256,
        origins: bindings,
        output_inventory: expected.materialized_inventory(),
    };
    receipt.validate()?;
    let receipt_bytes = canonical_json_bytes(&receipt)?;
    let receipt_sha256 = Digest32::digest_bytes(&receipt_bytes);
    let receipt_artifact =
        builder.write_bytes(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH, &receipt_bytes)?;
    expected.register_file(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH.into(),
        &receipt_artifact,
        false,
    )?;
    let sidecar_artifact = builder.write_bytes(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH,
        receipt_sha256.to_string().as_bytes(),
    )?;
    expected.register_file(
        CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH.into(),
        &sidecar_artifact,
        false,
    )?;
    expected.seal_and_validate(staging.path(), &staging.root)?;
    let (_, inventory) = validate_cloudflare_materialization_inventory_v1(
        staging.path(),
        &staging.root,
        receipt_sha256,
    )?;
    builder.ensure_exact_retained_nodes(&inventory)?;
    before_source_acceptance();
    publication.ensure_live()?;
    let candidate = ValidatedPublicationV3 {
        root_path: staging.path.clone(),
        root: staging.root.try_clone()?,
        root_parent_path: staging.parent_path.clone(),
        root_parent: staging.parent.try_clone()?,
        root_parent_identity: staging.parent_identity.clone(),
        root_name: staging.name.clone(),
        inventory,
        lock_sha256: receipt_sha256,
    };
    candidate.ensure_live()?;
    Ok((receipt_sha256, candidate))
}

#[cfg(target_os = "linux")]
fn persist_cloudflare_materialization_v1(
    staging: PinnedPublicationStagingV3,
    publication: &ValidatedPublicationV3,
    candidate: &ValidatedPublicationV3,
    output: &Path,
    materialization_sha256: Digest32,
) -> Result<Digest32> {
    let persistence = publication
        .ensure_live()
        .and_then(|()| candidate.ensure_live())
        .and_then(|()| persist_publication_staging(&staging, candidate, output));
    match persistence {
        Ok(PublicationPersistenceOutcome::Published) => Ok(materialization_sha256),
        Ok(PublicationPersistenceOutcome::PublishedButParentSyncFailed(source)) => {
            Err(CloudflareMaterializationInstalledButParentSyncFailed {
                output: output.to_path_buf(),
                materialization_sha256,
                source,
            }
            .into())
        }
        Err(error) => match error.downcast::<PublicationPersistenceStateUncertain>() {
            Ok(state) => Err(CloudflareMaterializationPersistenceStateUncertain {
                last_staging_path: state.staging_path,
                candidate_device: state.candidate_device,
                candidate_inode: state.candidate_inode,
                last_parent_path: state.parent_path,
                parent_device: state.parent_device,
                parent_inode: state.parent_inode,
                intended_output: state.intended_output,
            }
            .into()),
            Err(persist_error) => match discard_failed_publication_staging(staging) {
                Ok(()) => Err(persist_error),
                Err(cleanup_error) => Err(persist_error.context(format!(
                    "Cloudflare materialization persistence also failed to securely remove staging: {cleanup_error:#}"
                ))),
            },
        },
    }
}

/// Copy the exact public, isolated-signer, and deployment-authority closure
/// from one retained PublicationV3 into an independently reviewable immutable
/// Cloudflare materialization. The Publication path is never reopened after
/// validation; every copied byte comes from the retained validated file FD.
pub fn materialize_cloudflare_publication_v3(
    publication_root: &Path,
    output: &Path,
    repo_root: &Path,
    expected_publication_lock_sha256: Digest32,
) -> Result<Digest32> {
    ensure!(
        publication_root.is_absolute() && output.is_absolute() && repo_root.is_absolute(),
        "Cloudflare materialization paths must be absolute"
    );
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            publication_root,
            output,
            repo_root,
            expected_publication_lock_sha256,
        );
        anyhow::bail!("Cloudflare PublicationV3 materialization requires Linux openat2");
    }
    #[cfg(target_os = "linux")]
    {
        validate_mount_root(publication_root)?;
        let mut publication = validate_publication_v3_authority(publication_root)?;
        let (manifest, build, provenance) = load_cloudflare_materialization_provenance_v1(
            &mut publication,
            expected_publication_lock_sha256,
            repo_root,
        )?;
        let origins = derive_cloudflare_origin_inventories_v1(&publication, &manifest, &build)?;
        let staging = create_pinned_publication_staging_v3(output)?;
        let assembled = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        );
        match assembled {
            Ok((materialization_sha256, candidate)) => persist_cloudflare_materialization_v1(
                staging,
                &publication,
                &candidate,
                output,
                materialization_sha256,
            ),
            Err(assembly_error) => match discard_failed_publication_staging(staging) {
                Ok(()) => Err(assembly_error),
                Err(cleanup_error) => Err(assembly_error.context(format!(
                    "Cloudflare materialization assembly also failed to securely remove staging: {cleanup_error:#}"
                ))),
            },
        }
    }
}

/// Revalidate a materialized Cloudflare V1 output against an independently
/// recorded receipt digest. This library boundary is used by tests and
/// embedding tools; the operator CLI deliberately exposes one atomic
/// materialization command rather than a validate-then-copy workflow.
pub fn validate_cloudflare_publication_materialization_v1(
    root_path: &Path,
    expected_materialization_sha256: Digest32,
) -> Result<CloudflarePublicationMaterializationV1> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root_path, expected_materialization_sha256);
        anyhow::bail!("Cloudflare materialization validation requires Linux openat2");
    }
    #[cfg(target_os = "linux")]
    {
        validate_mount_root(root_path)?;
        let root = open_publication_root_v3(root_path)?;
        let (receipt, _) = validate_cloudflare_materialization_inventory_v1(
            root_path,
            &root,
            expected_materialization_sha256,
        )?;
        Ok(receipt)
    }
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
fn validate_publication_v3_contents(
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

fn validate_deployment_exposure(inventory: &mut PublicationTreeInventoryV3) -> Result<()> {
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

fn release_file_exposure(path: &str) -> ReleaseFileExposureV1 {
    if path.starts_with("cloudflare-public/") || path.starts_with("cloudflare-identity-signer/") {
        ReleaseFileExposureV1::PublicStatic
    } else if path.starts_with("backend/manifests/") {
        ReleaseFileExposureV1::BackendManifest
    } else {
        ReleaseFileExposureV1::OperatorPrivate
    }
}

fn validate_pinned_projection_exporter_v3(
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

fn authority_relative_v3(relative: &str) -> String {
    format!("private/official-content-authority/{relative}")
}

fn load_authority_document_v3<T>(
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
struct ExpectedOfficialAuthorityTopologyV3 {
    files: BTreeSet<String>,
    directories: BTreeSet<String>,
}

fn expected_official_authority_topology_v3(
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

fn validate_embedded_official_content_v3(
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

fn load_admitted_profile_managers_from_inventory_v3(
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
            let document: SimulationContentComponentDocumentV1 = strict_json_from_slice(&bytes)
                .context("decode authenticated Profiles component")?;
            document.validate()?;
            ensure!(
                document.kind == SimulationContentComponentKindV1::Profiles
                    && document.canonical_bytes()? == bytes,
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

fn validate_materialized_document_closure(
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

fn expected_publication_topology_from_validated_v3(
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

fn validate_public_tree_inventory(
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

fn validate_materialized_datadir_binding(
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

fn validate_deployment_metadata_inventory(inventory: &PublicationTreeInventoryV3) -> Result<()> {
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

fn validate_public_privacy(
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

fn scan_public_inventory_v3(
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
fn scan_public_tree(root: &Path, forbidden_values: &[Vec<u8>], label: &str) -> Result<()> {
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
enum PublicJsonSchema {
    Other,
    RulesetManifest,
    PublishedRuleset,
    CompetitionManifest,
}

fn public_json_schema(
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

fn reject_private_json_keys(
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
fn file_contains_bytes(path: &Path, needle: &[u8]) -> Result<bool> {
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

fn load_one_inventory_addressed_document_v3<T>(
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

fn load_inventory_addressed_documents_v3<T>(
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

fn load_inventory_published_rulesets_v3(
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
fn load_addressed_documents<T>(
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

fn validate_inventory_artifact_v3(
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

fn ensure_required_backend_layout(inventory: &PublicationTreeInventoryV3) -> Result<()> {
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

fn ensure_no_full_public_leak(inventory: &mut PublicationTreeInventoryV3) -> Result<()> {
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

fn validate_transition(
    transition: &PublicationTransitionV3,
    candidate: &ValidatedPublicationV3,
) -> Result<()> {
    validate_transition_with(transition, candidate, || {})
}

fn validate_transition_with<F>(
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
enum TransitionRule {
    Update,
    StatusOnly,
    Exact,
}

fn compare_transition(previous: &Path, candidate: &Path, rule: TransitionRule) -> Result<()> {
    let previous_root = open_publication_root_v3(previous)?;
    let candidate_root = open_publication_root_v3(candidate)?;
    let previous = publication_tree_inventory_v3_from_fd(previous, &previous_root)?;
    let candidate = publication_tree_inventory_v3_from_fd(candidate, &candidate_root)?;
    compare_transition_authorities(&previous.authority(), &candidate.authority(), rule)
}

fn compare_transition_authorities(
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PublicationNodeIdentityV3 {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) group: u32,
    pub(crate) links: u64,
    pub(crate) mode: u32,
    pub(crate) length: u64,
    pub(crate) modified_seconds: i64,
    pub(crate) modified_nanoseconds: i64,
    pub(crate) changed_seconds: i64,
    pub(crate) changed_nanoseconds: i64,
}

#[derive(Debug)]
struct PublicationFileInventoryV3 {
    path: String,
    file: fs::File,
    artifact: ArtifactRefV1,
    unix_mode: u32,
    identity: PublicationNodeIdentityV3,
}

#[derive(Debug)]
struct PublicationTreeInventoryV3 {
    files: Vec<PublicationFileInventoryV3>,
    directories: Vec<PublicationDirectoryV3>,
    directory_identities: Vec<(String, PublicationNodeIdentityV3)>,
    directory_files: Vec<(String, fs::File)>,
}

/// One fully validated PublicationV3 authority that retains the exact root,
/// file, and directory descriptors used for semantic validation. Consumers
/// must keep this value alive through extraction and call `ensure_live` at
/// their acceptance boundary.
#[derive(Debug)]
pub(crate) struct ValidatedPublicationV3 {
    root_path: PathBuf,
    root: fs::File,
    root_parent_path: PathBuf,
    root_parent: fs::File,
    root_parent_identity: PublicationNodeIdentityV3,
    root_name: std::ffi::OsString,
    inventory: PublicationTreeInventoryV3,
    lock_sha256: Digest32,
}

impl ValidatedPublicationV3 {
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn synthetic_for_consumer_test(root_path: &Path) -> Result<Self> {
        let root = open_publication_root_v3(root_path)?;
        let (root_parent_path, root_parent, root_name) = pin_publication_root_parent_v3(root_path)?;
        Ok(Self {
            root_path: root_path.to_path_buf(),
            root: root.try_clone()?,
            root_parent_path,
            root_parent_identity: publication_node_identity_v3(&root_parent.metadata()?),
            root_parent,
            root_name,
            inventory: publication_tree_inventory_v3_from_fd(root_path, &root)?,
            lock_sha256: Digest32::digest_bytes(b"synthetic PublicationV3 consumer lock"),
        })
    }

    pub(crate) const fn lock_sha256(&self) -> Digest32 {
        self.lock_sha256
    }

    pub(crate) fn snapshot(&self) -> PublicationTreeSnapshotV3 {
        self.inventory.snapshot()
    }

    pub(crate) fn read_file(&mut self, path: &str, maximum: u64) -> Result<Vec<u8>> {
        read_inventory_file_v3(&mut self.inventory, path, maximum)
    }

    pub(crate) fn load_document<T>(&mut self, path: &str) -> Result<T>
    where
        T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
    {
        load_inventory_document_v3(&mut self.inventory, path)
    }

    pub(crate) fn artifact(&self, path: &str, media_type: &str) -> Result<ArtifactRefV1> {
        inventory_artifact_v3(&self.inventory, path, media_type)
    }

    pub(crate) fn relative_files(&self, prefix: &str) -> Vec<String> {
        inventory_relative_files_v3(&self.inventory, prefix)
            .into_iter()
            .collect()
    }

    pub(crate) fn relative_directories(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.trim_end_matches('/');
        let nested_prefix = format!("{prefix}/");
        self.inventory
            .directories
            .iter()
            .filter_map(|directory| {
                if directory.path == prefix {
                    Some(".".to_owned())
                } else {
                    directory
                        .path
                        .strip_prefix(&nested_prefix)
                        .map(str::to_owned)
                }
            })
            .collect()
    }

    pub(crate) fn copy_file_to(
        &mut self,
        path: &str,
        output: &mut fs::File,
    ) -> Result<ArtifactRefV1> {
        use std::os::unix::fs::MetadataExt as _;

        let source = self
            .inventory
            .files
            .iter_mut()
            .find(|file| file.path == path)
            .with_context(|| format!("validated PublicationV3 omits {path}"))?;
        let output_before = output.metadata()?;
        ensure!(
            output_before.is_file()
                && output_before.nlink() == 1
                && output_before.uid() == rustix::process::geteuid().as_raw()
                && output_before.len() == 0,
            "PublicationV3 extraction output is not an empty owned singleton file"
        );
        source.file.seek(std::io::SeekFrom::Start(0))?;
        output.seek(std::io::SeekFrom::Start(0))?;
        let copied = std::io::copy(&mut source.file, output)?;
        ensure!(
            copied == source.artifact.byte_length
                && publication_node_identity_v3(&source.file.metadata()?) == source.identity,
            "validated PublicationV3 source changed while extracting {path}"
        );
        output.sync_all()?;
        let output_identity = publication_node_identity_v3(&output.metadata()?);
        ensure!(
            stable_publication_file_artifact_v3(output, &output_identity, path)? == source.artifact,
            "PublicationV3 extraction changed bytes at {path}"
        );
        Ok(source.artifact.clone())
    }

    pub(crate) fn ensure_live(&self) -> Result<()> {
        let rebound_parent = open_publication_root_v3(&self.root_parent_path)?;
        ensure!(
            publication_same_stable_node_v3(
                &publication_node_identity_v3(&self.root_parent.metadata()?),
                &self.root_parent_identity,
            ) && publication_same_stable_node_v3(
                &publication_node_identity_v3(&rebound_parent.metadata()?),
                &self.root_parent_identity,
            ),
            "validated PublicationV3 root parent was substituted"
        );
        let rebound_root = open_publication_child_v3(&rebound_parent, Path::new(&self.root_name))?;
        ensure!(
            publication_node_identity_v3(&rebound_root.metadata()?)
                == publication_node_identity_v3(&self.root.metadata()?),
            "validated PublicationV3 root path was substituted"
        );
        ensure!(
            publication_tree_inventory_v3_from_fd(&self.root_path, &self.root)?.snapshot()
                == self.inventory.snapshot(),
            "validated PublicationV3 changed before consumer acceptance"
        );
        Ok(())
    }

    fn sync_exact_tree(&self) -> Result<()> {
        for file in &self.inventory.files {
            file.file.sync_all()?;
        }
        let mut directories = self.inventory.directory_files.iter().collect::<Vec<_>>();
        directories
            .sort_by_key(|(path, _)| std::cmp::Reverse(Path::new(path).components().count()));
        for (_, directory) in directories {
            directory.sync_all()?;
        }
        self.ensure_live()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicationTreeSnapshotV3 {
    pub(crate) files: Vec<(String, ArtifactRefV1, u32, PublicationNodeIdentityV3)>,
    pub(crate) directories: Vec<(PublicationDirectoryV3, PublicationNodeIdentityV3)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationTreeAuthorityV3 {
    files: Vec<(String, ArtifactRefV1, u32)>,
    directories: Vec<PublicationDirectoryV3>,
}

impl PublicationTreeInventoryV3 {
    fn snapshot(&self) -> PublicationTreeSnapshotV3 {
        PublicationTreeSnapshotV3 {
            files: self
                .files
                .iter()
                .map(|file| {
                    (
                        file.path.clone(),
                        file.artifact.clone(),
                        file.unix_mode,
                        file.identity.clone(),
                    )
                })
                .collect(),
            directories: self
                .directories
                .iter()
                .cloned()
                .zip(
                    self.directory_identities
                        .iter()
                        .map(|(_, identity)| identity.clone()),
                )
                .collect(),
        }
    }

    fn authority(&self) -> PublicationTreeAuthorityV3 {
        PublicationTreeAuthorityV3 {
            files: self
                .files
                .iter()
                .map(|file| (file.path.clone(), file.artifact.clone(), file.unix_mode))
                .collect(),
            directories: self.directories.clone(),
        }
    }
}

fn publication_inventory_matches_after_root_rename_v3(
    before: &PublicationTreeInventoryV3,
    after: &PublicationTreeInventoryV3,
) -> bool {
    before.authority() == after.authority()
        && before
            .files
            .iter()
            .zip(&after.files)
            .all(|(left, right)| left.path == right.path && left.identity == right.identity)
        && before
            .directory_identities
            .iter()
            .zip(&after.directory_identities)
            .all(|((left_path, left), (right_path, right))| {
                left_path == right_path
                    && if left_path == "." {
                        publication_same_stable_node_v3(left, right)
                    } else {
                        left == right
                    }
            })
}

#[cfg(target_os = "linux")]
fn read_inventory_file_v3(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
    maximum: u64,
) -> Result<Vec<u8>> {
    let file = inventory
        .files
        .iter_mut()
        .find(|file| file.path == path)
        .with_context(|| format!("PublicationV3 inventory omits {path}"))?;
    ensure!(
        file.artifact.byte_length <= maximum,
        "PublicationV3 file {path} exceeds its read bound"
    );
    let capacity = usize::try_from(file.artifact.byte_length)
        .context("PublicationV3 file length does not fit usize")?;
    file.file.seek(std::io::SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.file.read_to_end(&mut bytes)?;
    ensure!(
        u64::try_from(bytes.len()).ok() == Some(file.artifact.byte_length)
            && Digest32::digest_bytes(&bytes) == file.artifact.sha256
            && publication_node_identity_v3(&file.file.metadata()?) == file.identity,
        "PublicationV3 file changed while read at {path}"
    );
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn load_inventory_canonical_document_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    let bytes = read_inventory_file_v3(inventory, path, MAX_DOCUMENT_BYTES)?;
    let document: T = strict_json_from_slice(&bytes)
        .with_context(|| format!("parse canonical PublicationV3 document {path}"))?;
    ensure!(
        canonical_json_bytes(&document)? == bytes,
        "PublicationV3 document is not byte-for-byte canonical at {path}"
    );
    Ok(document)
}

#[cfg(target_os = "linux")]
fn load_inventory_document_v3<T>(
    inventory: &mut PublicationTreeInventoryV3,
    path: &str,
) -> Result<T>
where
    T: DeserializeOwned + Serialize + robin_run_protocol::Validate,
{
    let document: T = load_inventory_canonical_document_v3(inventory, path)?;
    document.validate()?;
    Ok(document)
}

#[cfg(target_os = "linux")]
fn inventory_artifact_v3(
    inventory: &PublicationTreeInventoryV3,
    path: &str,
    media_type: &str,
) -> Result<ArtifactRefV1> {
    let file = inventory
        .files
        .iter()
        .find(|file| file.path == path)
        .with_context(|| format!("PublicationV3 inventory omits {path}"))?;
    let mut artifact = file.artifact.clone();
    artifact.media_type = media_type.into();
    Ok(artifact)
}

fn inventory_relative_files_v3(
    inventory: &PublicationTreeInventoryV3,
    prefix: &str,
) -> BTreeSet<String> {
    let prefix = format!("{}/", prefix.trim_end_matches('/'));
    inventory
        .files
        .iter()
        .filter_map(|file| file.path.strip_prefix(&prefix).map(str::to_owned))
        .collect()
}

fn inventory_has_directory_v3(inventory: &PublicationTreeInventoryV3, path: &str) -> bool {
    inventory
        .directories
        .binary_search_by(|directory| directory.path.as_str().cmp(path))
        .is_ok()
}

#[cfg(target_os = "linux")]
fn publication_node_identity_v3(metadata: &fs::Metadata) -> PublicationNodeIdentityV3 {
    use std::os::unix::fs::MetadataExt as _;

    PublicationNodeIdentityV3 {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        group: metadata.gid(),
        links: metadata.nlink(),
        mode: metadata.mode(),
        length: metadata.size(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

fn publication_same_stable_node_v3(
    left: &PublicationNodeIdentityV3,
    right: &PublicationNodeIdentityV3,
) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.owner == right.owner
        && left.group == right.group
        && left.mode == right.mode
}

#[cfg(target_os = "linux")]
fn open_publication_root_v3(root: &Path) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};

    let descriptor = openat2(
        rustix::fs::CWD,
        root,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .with_context(|| format!("pin PublicationV3 root {}", root.display()))?;
    Ok(fs::File::from(descriptor))
}

#[cfg(target_os = "linux")]
fn pin_publication_root_parent_v3(root: &Path) -> Result<(PathBuf, fs::File, std::ffi::OsString)> {
    let parent_path = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let name = root
        .file_name()
        .context("PublicationV3 root has no basename")?
        .to_owned();
    let parent = open_publication_root_v3(&parent_path)?;
    Ok((parent_path, parent, name))
}

#[cfg(target_os = "linux")]
fn open_publication_child_v3(parent: &fs::File, name: &Path) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let descriptor = openat2(
        parent.as_fd(),
        name,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    Ok(fs::File::from(descriptor))
}

#[cfg(target_os = "linux")]
fn open_publication_child_identity_v3(parent: &fs::File, name: &Path) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    let descriptor = openat2(
        parent.as_fd(),
        name,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    )?;
    Ok(fs::File::from(descriptor))
}

#[cfg(target_os = "linux")]
fn open_optional_publication_child_identity_v3(
    parent: &fs::File,
    name: &Path,
) -> Result<Option<fs::File>> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::AsFd as _;

    match openat2(
        parent.as_fd(),
        name,
        OFlags::PATH | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH
            | ResolveFlags::NO_SYMLINKS
            | ResolveFlags::NO_MAGICLINKS
            | ResolveFlags::NO_XDEV,
    ) {
        Ok(descriptor) => Ok(Some(fs::File::from(descriptor))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
fn publication_directory_entries_v3(
    directory: &fs::File,
) -> Result<Vec<(std::ffi::OsString, u64, rustix::fs::FileType)>> {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let mut reader = rustix::fs::Dir::read_from(directory)?;
    let mut entries = Vec::new();
    for entry in &mut reader {
        let entry = entry?;
        let bytes = entry.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        ensure!(
            !bytes.is_empty() && !bytes.contains(&b'/'),
            "PublicationV3 directory contains an invalid entry name"
        );
        entries.push((
            std::ffi::OsString::from_vec(bytes.to_vec()),
            entry.ino(),
            entry.file_type(),
        ));
    }
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(entries)
}

#[cfg(target_os = "linux")]
fn validate_publication_node_v3(
    identity: &PublicationNodeIdentityV3,
    expected_uid: u32,
    expected_device: u64,
    path: &str,
) -> Result<()> {
    ensure!(
        identity.owner == expected_uid && identity.device == expected_device,
        "PublicationV3 contains mixed ownership or devices at {path}"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn stable_publication_file_artifact_v3(
    file: &mut fs::File,
    expected_identity: &PublicationNodeIdentityV3,
    path: &str,
) -> Result<ArtifactRefV1> {
    stable_publication_file_artifact_v3_with(file, expected_identity, path, || {})
}

#[cfg(target_os = "linux")]
fn stable_publication_file_artifact_v3_with<F>(
    file: &mut fs::File,
    expected_identity: &PublicationNodeIdentityV3,
    path: &str,
    between_hashes: F,
) -> Result<ArtifactRefV1>
where
    F: FnOnce(),
{
    file.seek(std::io::SeekFrom::Start(0))?;
    let first = Digest32::digest_reader(BufReader::new(&mut *file))?;
    let after_first = publication_node_identity_v3(&file.metadata()?);
    between_hashes();
    file.seek(std::io::SeekFrom::Start(0))?;
    let second = Digest32::digest_reader(BufReader::new(&mut *file))?;
    let after_second = publication_node_identity_v3(&file.metadata()?);
    ensure!(
        expected_identity == &after_first && expected_identity == &after_second && first == second,
        "PublicationV3 file changed while pinned and hashed at {path}"
    );
    Ok(ArtifactRefV1 {
        sha256: first,
        byte_length: expected_identity.length,
        media_type: "application/octet-stream".into(),
    })
}

/// Enumerate and hash the complete PublicationV3 tree through pinned dirfds.
#[cfg(target_os = "linux")]
fn publication_tree_inventory_v3_from_fd(
    root_path: &Path,
    root: &fs::File,
) -> Result<PublicationTreeInventoryV3> {
    publication_tree_inventory_v3_from_fd_with(root_path, root, || {})
}

#[cfg(target_os = "linux")]
fn publication_tree_inventory_v3_from_fd_with<F>(
    root_path: &Path,
    root: &fs::File,
    after_walk: F,
) -> Result<PublicationTreeInventoryV3>
where
    F: FnOnce(),
{
    let root_metadata = root.metadata()?;
    ensure!(
        root_metadata.is_dir(),
        "PublicationV3 root is not a directory"
    );
    let root_identity = publication_node_identity_v3(&root_metadata);
    let (root_parent_path, root_parent, root_name) = pin_publication_root_parent_v3(root_path)?;
    let root_parent_identity = publication_node_identity_v3(&root_parent.metadata()?);
    let rooted = open_publication_child_v3(&root_parent, Path::new(&root_name))?;
    ensure!(
        publication_node_identity_v3(&rooted.metadata()?) == root_identity,
        "PublicationV3 root path differs from its pinned descriptor"
    );
    let expected_uid = rustix::process::geteuid().as_raw();
    let expected_device = root_identity.device;
    validate_publication_node_v3(&root_identity, expected_uid, expected_device, ".")?;
    ensure!(expected_device != 0, "PublicationV3 root device is zero");

    struct InventoryBuilder {
        files: Vec<PublicationFileInventoryV3>,
        directories: Vec<PublicationDirectoryV3>,
        directory_identities: Vec<(String, PublicationNodeIdentityV3)>,
        directory_files: Vec<(String, fs::File)>,
        seen: usize,
        expected_uid: u32,
        expected_device: u64,
    }

    fn walk(
        directory: &fs::File,
        relative: &Path,
        depth: usize,
        builder: &mut InventoryBuilder,
    ) -> Result<()> {
        use rustix::fs::FileType;

        ensure!(
            depth <= MAX_PUBLICATION_TREE_DEPTH,
            "PublicationV3 tree exceeds its depth bound"
        );
        let before = publication_node_identity_v3(&directory.metadata()?);
        let path = if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            path_to_manifest(relative)?
        };
        validate_publication_node_v3(
            &before,
            builder.expected_uid,
            builder.expected_device,
            &path,
        )?;
        ensure!(
            directory.metadata()?.is_dir(),
            "PublicationV3 node is not a directory"
        );
        builder.directories.push(PublicationDirectoryV3 {
            path: path.clone(),
            unix_mode: before.mode & 0o7777,
        });
        builder
            .directory_identities
            .push((path.clone(), before.clone()));
        builder.directory_files.push((path, directory.try_clone()?));

        let entries = publication_directory_entries_v3(directory)?;
        for (name, observed_inode, observed_type) in &entries {
            builder.seen = builder
                .seen
                .checked_add(1)
                .context("PublicationV3 entry count overflow")?;
            ensure!(
                builder.seen <= MAX_PUBLICATION_TREE_ENTRIES,
                "PublicationV3 tree exceeds its entry bound"
            );
            let child_relative = relative.join(name);
            let child_path = path_to_manifest(&child_relative)?;
            let observed_child = open_publication_child_identity_v3(directory, Path::new(name))
                .with_context(|| format!("pin PublicationV3 child {child_path}"))?;
            let child_metadata = observed_child.metadata()?;
            let identity = publication_node_identity_v3(&child_metadata);
            validate_publication_node_v3(
                &identity,
                builder.expected_uid,
                builder.expected_device,
                &child_path,
            )?;
            ensure!(
                *observed_inode == 0 || *observed_inode == identity.inode,
                "PublicationV3 directory entry inode changed at {child_path}"
            );
            let opened_type = if child_metadata.is_dir() {
                FileType::Directory
            } else if child_metadata.is_file() {
                FileType::RegularFile
            } else {
                FileType::Unknown
            };
            ensure!(
                *observed_type == FileType::Unknown || *observed_type == opened_type,
                "PublicationV3 directory entry type changed at {child_path}"
            );
            match opened_type {
                FileType::Directory => {
                    let child = open_publication_child_v3(directory, Path::new(name))?;
                    ensure!(
                        publication_node_identity_v3(&child.metadata()?) == identity,
                        "PublicationV3 directory was substituted while opened at {child_path}"
                    );
                    walk(&child, &child_relative, depth + 1, builder)?;
                }
                FileType::RegularFile => {
                    ensure!(
                        identity.links == 1,
                        "PublicationV3 contains hard-linked file {child_path}"
                    );
                    let mut child = open_publication_child_v3(directory, Path::new(name))?;
                    ensure!(
                        publication_node_identity_v3(&child.metadata()?) == identity,
                        "PublicationV3 file was substituted while opened at {child_path}"
                    );
                    let artifact =
                        stable_publication_file_artifact_v3(&mut child, &identity, &child_path)?;
                    let unix_mode = identity.mode & 0o7777;
                    builder.files.push(PublicationFileInventoryV3 {
                        path: child_path.clone(),
                        file: child,
                        artifact,
                        unix_mode,
                        identity: identity.clone(),
                    });
                }
                _ => anyhow::bail!("PublicationV3 contains special node {child_path}"),
            }
            let rebound = open_publication_child_v3(directory, Path::new(name))?;
            ensure!(
                publication_node_identity_v3(&rebound.metadata()?) == identity,
                "PublicationV3 child was substituted after use at {child_path}"
            );
        }
        let rebound_entries = publication_directory_entries_v3(directory)?;
        ensure!(
            rebound_entries == entries,
            "PublicationV3 directory entries changed while traversing {relative:?}"
        );
        ensure!(
            publication_node_identity_v3(&directory.metadata()?) == before,
            "PublicationV3 directory changed while traversing {relative:?}"
        );
        Ok(())
    }

    let mut builder = InventoryBuilder {
        files: Vec::new(),
        directories: Vec::new(),
        directory_identities: Vec::new(),
        directory_files: Vec::new(),
        seen: 1,
        expected_uid,
        expected_device,
    };
    walk(root, Path::new(""), 0, &mut builder)?;
    after_walk();
    builder
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    builder
        .directories
        .sort_by(|left, right| left.path.cmp(&right.path));
    builder
        .directory_identities
        .sort_by(|left, right| left.0.cmp(&right.0));
    builder
        .directory_files
        .sort_by(|left, right| left.0.cmp(&right.0));
    ensure!(
        builder
            .directories
            .first()
            .is_some_and(|entry| entry.path == ".")
            && builder
                .directories
                .iter()
                .map(|entry| entry.path.as_str())
                .eq(builder
                    .directory_identities
                    .iter()
                    .map(|(path, _)| path.as_str()))
            && builder
                .directories
                .iter()
                .map(|entry| entry.path.as_str())
                .eq(builder
                    .directory_files
                    .iter()
                    .map(|(path, _)| path.as_str())),
        "PublicationV3 directory inventory is incomplete or misbound"
    );
    let rebound_parent = open_publication_root_v3(&root_parent_path)?;
    ensure!(
        publication_same_stable_node_v3(
            &publication_node_identity_v3(&rebound_parent.metadata()?),
            &root_parent_identity,
        ),
        "PublicationV3 root parent path was substituted during traversal"
    );
    let rebound_root = open_publication_child_v3(&rebound_parent, Path::new(&root_name))?;
    ensure!(
        publication_node_identity_v3(&rebound_root.metadata()?) == root_identity,
        "PublicationV3 root path was substituted during traversal"
    );
    for (directory, (_, expected_identity)) in builder
        .directories
        .iter()
        .zip(&builder.directory_identities)
    {
        if directory.path == "." {
            continue;
        }
        let rebound = open_publication_child_v3(root, Path::new(&directory.path))?;
        ensure!(
            rebound.metadata()?.is_dir()
                && publication_node_identity_v3(&rebound.metadata()?) == *expected_identity,
            "PublicationV3 directory failed final root-relative rebind at {}",
            directory.path
        );
    }
    for expected in &builder.files {
        let rebound = open_publication_child_v3(root, Path::new(&expected.path))?;
        ensure!(
            rebound.metadata()?.is_file()
                && publication_node_identity_v3(&rebound.metadata()?) == expected.identity,
            "PublicationV3 file failed final root-relative rebind at {}",
            expected.path
        );
    }
    Ok(PublicationTreeInventoryV3 {
        files: builder.files,
        directories: builder.directories,
        directory_identities: builder.directory_identities,
        directory_files: builder.directory_files,
    })
}

fn publication_tree_inventory_v3(root: &Path) -> Result<PublicationTreeInventoryV3> {
    #[cfg(target_os = "linux")]
    {
        let descriptor = open_publication_root_v3(root)?;
        return publication_tree_inventory_v3_from_fd(root, &descriptor);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        anyhow::bail!("PublicationV3 requires Linux openat2 filesystem authority")
    }
}

fn publication_directories(root: &Path) -> Result<Vec<PublicationDirectoryV3>> {
    Ok(publication_tree_inventory_v3(root)?.directories)
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
fn reject_publication_mounts_in_v3(canonical_root: &Path, mountinfo: &[u8]) -> Result<()> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    for line in mountinfo
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let encoded = line
            .split(|byte| *byte == b' ')
            .nth(4)
            .context("malformed /proc/self/mountinfo line")?;
        let mut decoded = Vec::with_capacity(encoded.len());
        let mut index = 0;
        while index < encoded.len() {
            if encoded[index] == b'\\'
                && index + 3 < encoded.len()
                && encoded[index + 1..index + 4]
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'7'))
            {
                decoded.push(
                    (encoded[index + 1] - b'0') * 64
                        + (encoded[index + 2] - b'0') * 8
                        + (encoded[index + 3] - b'0'),
                );
                index += 4;
            } else {
                decoded.push(encoded[index]);
                index += 1;
            }
        }
        let mount = PathBuf::from(OsString::from_vec(decoded));
        ensure!(
            mount != canonical_root && !mount.starts_with(canonical_root),
            "PublicationV3 contains mount point {}",
            mount.display()
        );
    }
    Ok(())
}

#[cfg(all(unix, test))]
fn publication_unix_mode(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
}

#[cfg(all(not(unix), test))]
fn publication_unix_mode(_path: &Path) -> Result<u32> {
    anyhow::bail!("operator publications require Unix permission semantics")
}

fn file_artifacts(root: &Path) -> Result<BTreeMap<String, ArtifactRefV1>> {
    publication_tree_inventory_v3(root)?
        .files
        .into_iter()
        .map(|file| Ok((file.path, file.artifact)))
        .collect()
}

fn mutable_transition_path(path: &str) -> bool {
    path.starts_with("backend/manifests/published-rulesets/")
        || matches!(
            path,
            "backend/publication-v3.json"
                | "publication-manifest-v3.json"
                | "publication-manifest-v3.sha256"
                | "publication-lock-v3.json"
                | "publication-lock-v3.sha256"
        )
}

fn immutable_transition_files(
    files: &BTreeMap<String, ArtifactRefV1>,
) -> BTreeMap<&str, &ArtifactRefV1> {
    files
        .iter()
        .filter(|(path, _)| !mutable_transition_path(path))
        .map(|(path, artifact)| (path.as_str(), artifact))
        .collect()
}

fn status_transition_files(
    files: &BTreeMap<String, ArtifactRefV1>,
) -> BTreeMap<&str, &ArtifactRefV1> {
    files
        .iter()
        .filter(|(path, _)| path.starts_with("backend/manifests/published-rulesets/"))
        .map(|(path, artifact)| (path.as_str(), artifact))
        .collect()
}

fn validate_pinned_source(source: &PinnedArtifactSourceV3) -> Result<()> {
    validate_regular_file(&source.source)?;
    source.artifact.validate()?;
    ensure!(
        artifact_from_file(&source.source, &source.artifact.media_type)? == source.artifact,
        "pinned source differs from its artifact identity"
    );
    Ok(())
}

fn validate_campaign_state_source(
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

fn copy_directory_exact(source: &Path, destination: &Path) -> Result<()> {
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

fn copy_directory_exact_preserving_modes(
    source: &Path,
    destination: &Path,
) -> Result<PublicationTreeAuthorityV3> {
    copy_directory_exact_preserving_modes_with(source, destination, || {})
}

fn copy_directory_exact_preserving_modes_with<F>(
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

fn create_private_publication_root(root: &Path) -> Result<PathBuf> {
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

const MAX_FAILED_PUBLICATION_STAGING_ENTRIES: usize = 262_144;
const MAX_FAILED_PUBLICATION_STAGING_DEPTH: usize = 128;

#[derive(Debug)]
pub struct PublicationInstalledButParentSyncFailed {
    pub output: PathBuf,
    pub publication_lock_sha256: Digest32,
    pub source: anyhow::Error,
}

#[derive(Debug)]
pub struct CloudflareMaterializationInstalledButParentSyncFailed {
    pub output: PathBuf,
    pub materialization_sha256: Digest32,
    pub source: anyhow::Error,
}

impl std::fmt::Display for CloudflareMaterializationInstalledButParentSyncFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Cloudflare publication materialization {} was atomically installed with receipt {} but parent-directory durability sync failed; the exact immutable output exists and must be treated as installed",
            self.output.display(),
            self.materialization_sha256,
        )
    }
}

impl std::error::Error for CloudflareMaterializationInstalledButParentSyncFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
pub struct CloudflareMaterializationPersistenceStateUncertain {
    pub last_staging_path: PathBuf,
    pub candidate_device: u64,
    pub candidate_inode: u64,
    pub last_parent_path: PathBuf,
    pub parent_device: u64,
    pub parent_inode: u64,
    pub intended_output: PathBuf,
}

impl std::fmt::Display for CloudflareMaterializationPersistenceStateUncertain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Cloudflare publication materialization persistence is uncertain; preserve candidate dev={} ino={} last staging name {}, pinned parent dev={} ino={} last path {}, and intended output {} for operator reconciliation",
            self.candidate_device,
            self.candidate_inode,
            self.last_staging_path.display(),
            self.parent_device,
            self.parent_inode,
            self.last_parent_path.display(),
            self.intended_output.display(),
        )
    }
}

impl std::error::Error for CloudflareMaterializationPersistenceStateUncertain {}

#[derive(Debug)]
struct PublicationPersistenceStateUncertain {
    staging_path: PathBuf,
    candidate_device: u64,
    candidate_inode: u64,
    parent_path: PathBuf,
    parent_device: u64,
    parent_inode: u64,
    intended_output: PathBuf,
}

impl std::fmt::Display for PublicationPersistenceStateUncertain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "PublicationV3 persistence state is uncertain; preserve candidate dev={} ino={} last staging name {}, pinned parent dev={} ino={} last path {}, and intended output {} for operator reconciliation",
            self.candidate_device,
            self.candidate_inode,
            self.staging_path.display(),
            self.parent_device,
            self.parent_inode,
            self.parent_path.display(),
            self.intended_output.display(),
        )
    }
}

impl std::error::Error for PublicationPersistenceStateUncertain {}

fn publication_persistence_state_uncertain(
    staging: &PinnedPublicationStagingV3,
    candidate: &PublicationNodeIdentityV3,
    output: &Path,
) -> anyhow::Error {
    PublicationPersistenceStateUncertain {
        staging_path: staging.path.clone(),
        candidate_device: candidate.device,
        candidate_inode: candidate.inode,
        parent_path: staging.parent_path.clone(),
        parent_device: staging.parent_identity.device,
        parent_inode: staging.parent_identity.inode,
        intended_output: output.to_path_buf(),
    }
    .into()
}

impl std::fmt::Display for PublicationInstalledButParentSyncFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "publication {} was atomically installed with lock {} but parent-directory durability sync failed; the immutable final exists and must be validated and treated as published",
            self.output.display(),
            self.publication_lock_sha256,
        )
    }
}

impl std::error::Error for PublicationInstalledButParentSyncFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

fn installed_publication_durability_error(
    output: &Path,
    publication_lock_sha256: Digest32,
    source: anyhow::Error,
) -> anyhow::Error {
    PublicationInstalledButParentSyncFailed {
        output: output.to_path_buf(),
        publication_lock_sha256,
        source,
    }
    .into()
}

#[derive(Debug)]
enum PublicationPersistenceOutcome {
    Published,
    PublishedButParentSyncFailed(anyhow::Error),
}

fn persist_publication_staging(
    staging: &PinnedPublicationStagingV3,
    candidate: &ValidatedPublicationV3,
    output: &Path,
) -> Result<PublicationPersistenceOutcome> {
    persist_publication_staging_with(
        staging,
        candidate,
        output,
        || {},
        |parent, source, destination| {
            use std::os::fd::AsFd as _;
            rustix::fs::renameat_with(
                parent.as_fd(),
                source,
                parent.as_fd(),
                destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
            Ok(())
        },
        |parent| {
            parent.sync_all()?;
            Ok(())
        },
    )
}

fn persist_publication_staging_with<B, R, S>(
    staging: &PinnedPublicationStagingV3,
    candidate: &ValidatedPublicationV3,
    output: &Path,
    before_rename: B,
    rename_stage: R,
    sync_parent: S,
) -> Result<PublicationPersistenceOutcome>
where
    B: FnOnce(),
    R: FnOnce(&fs::File, &Path, &Path) -> Result<()>,
    S: FnOnce(&fs::File) -> Result<()>,
{
    staging.ensure_live()?;
    candidate.ensure_live()?;
    ensure!(
        publication_same_stable_node_v3(
            &publication_node_identity_v3(&staging.root.metadata()?),
            &publication_node_identity_v3(&candidate.root.metadata()?),
        ),
        "validated PublicationV3 candidate is not the pinned staging root"
    );
    candidate.sync_exact_tree()?;
    let output_parent_path = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("PublicationV3 output has no parent")?;
    let output_name = output
        .file_name()
        .context("PublicationV3 output has no basename")?;
    ensure!(
        output_parent_path == staging.parent_path
            && open_optional_publication_child_identity_v3(
                &staging.parent,
                Path::new(output_name)
            )?
            .is_none(),
        "PublicationV3 final output already exists or changed parent"
    );
    before_rename();
    staging.ensure_live()?;
    candidate.ensure_live()?;
    let root_identity = publication_node_identity_v3(&candidate.root.metadata()?);

    let rename_result = rename_stage(
        &staging.parent,
        Path::new(&staging.name),
        Path::new(output_name),
    );
    let source_after = match open_optional_publication_child_identity_v3(
        &staging.parent,
        Path::new(&staging.name),
    ) {
        Ok(source) => source,
        Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    };
    let output_after = match open_optional_publication_child_identity_v3(
        &staging.parent,
        Path::new(output_name),
    ) {
        Ok(output) => output,
        Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    };
    let source_is_candidate = source_after
        .as_ref()
        .map(|source| {
            Ok::<bool, anyhow::Error>(publication_same_stable_node_v3(
                &publication_node_identity_v3(&source.metadata()?),
                &root_identity,
            ))
        })
        .transpose()?
        .unwrap_or(false);
    let output_is_candidate = output_after
        .as_ref()
        .map(|installed| {
            Ok::<bool, anyhow::Error>(publication_same_stable_node_v3(
                &publication_node_identity_v3(&installed.metadata()?),
                &root_identity,
            ))
        })
        .transpose()?
        .unwrap_or(false);
    let mut installed_uncertainty = None;
    match (rename_result, source_is_candidate, output_is_candidate) {
        (Ok(()), false, true) => {}
        (Err(error), true, false) => return Err(anyhow::anyhow!("{error:#}")),
        (Err(error), false, true) => {
            installed_uncertainty =
                Some(error.context("rename reported failure after PublicationV3 became installed"));
        }
        _ => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    }

    match publication_tree_inventory_v3_from_fd(output, &candidate.root) {
        Ok(installed_inventory)
            if publication_inventory_matches_after_root_rename_v3(
                &candidate.inventory,
                &installed_inventory,
            ) => {}
        Ok(_) | Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    }
    if let Err(error) = sync_parent(&staging.parent) {
        installed_uncertainty = Some(match installed_uncertainty {
            Some(rename_error) => rename_error.context(format!(
                "installed PublicationV3 parent sync also failed: {error:#}"
            )),
            None => error.context("sync installed PublicationV3 parent"),
        });
    }
    if open_publication_root_v3(&staging.parent_path)
        .and_then(|rebound_parent| {
            ensure!(
                publication_same_stable_node_v3(
                    &publication_node_identity_v3(&rebound_parent.metadata()?),
                    &publication_node_identity_v3(&staging.parent.metadata()?),
                ),
                "PublicationV3 output parent changed after persistence"
            );
            let rebound_output =
                open_publication_child_v3(&rebound_parent, Path::new(output_name))?;
            ensure!(
                publication_same_stable_node_v3(
                    &publication_node_identity_v3(&rebound_output.metadata()?),
                    &root_identity,
                ),
                "PublicationV3 output basename changed after persistence"
            );
            Ok(())
        })
        .is_err()
    {
        return Err(publication_persistence_state_uncertain(
            staging,
            &root_identity,
            output,
        ));
    }
    match publication_tree_inventory_v3_from_fd(output, &candidate.root) {
        Ok(terminal_inventory)
            if publication_inventory_matches_after_root_rename_v3(
                &candidate.inventory,
                &terminal_inventory,
            ) => {}
        Ok(_) | Err(_) => {
            return Err(publication_persistence_state_uncertain(
                staging,
                &root_identity,
                output,
            ));
        }
    }
    Ok(match installed_uncertainty {
        None => PublicationPersistenceOutcome::Published,
        Some(error) => PublicationPersistenceOutcome::PublishedButParentSyncFailed(error),
    })
}

#[cfg(target_os = "linux")]
fn discard_failed_publication_staging(staging: PinnedPublicationStagingV3) -> Result<()> {
    discard_failed_publication_staging_with(&staging, |_| {})
}

#[cfg(target_os = "linux")]
fn discard_failed_publication_staging_with<F>(
    staging: &PinnedPublicationStagingV3,
    mut before_operation: F,
) -> Result<()>
where
    F: FnMut(usize),
{
    use rustix::fs::{AtFlags, FileType, Mode, fchmod, statat, unlinkat};
    use std::os::fd::AsFd as _;

    #[derive(Debug)]
    struct CleanupDirectoryV3 {
        path: String,
        descriptor: fs::File,
        parent_index: Option<usize>,
        name: Option<std::ffi::OsString>,
        identity: PublicationNodeIdentityV3,
    }

    #[derive(Debug)]
    struct CleanupLeafV3 {
        parent_index: usize,
        name: std::ffi::OsString,
        identity: PublicationNodeIdentityV3,
        symlink: bool,
    }

    fn symlink_identity(stat: &rustix::fs::Stat) -> PublicationNodeIdentityV3 {
        PublicationNodeIdentityV3 {
            device: stat.st_dev,
            inode: stat.st_ino,
            owner: stat.st_uid,
            group: stat.st_gid,
            links: stat.st_nlink,
            mode: stat.st_mode,
            length: stat.st_size as u64,
            modified_seconds: stat.st_mtime,
            modified_nanoseconds: stat.st_mtime_nsec as i64,
            changed_seconds: stat.st_ctime,
            changed_nanoseconds: stat.st_ctime_nsec as i64,
        }
    }

    staging.ensure_live()?;
    let expected_uid = rustix::process::geteuid().as_raw();
    let expected_device = publication_node_identity_v3(&staging.root.metadata()?).device;
    let mut directories = vec![CleanupDirectoryV3 {
        path: ".".into(),
        descriptor: staging.root.try_clone()?,
        parent_index: None,
        name: None,
        identity: publication_node_identity_v3(&staging.root.metadata()?),
    }];
    let mut leaves = Vec::new();
    let mut cursor = 0;
    let mut seen = 1_usize;
    while cursor < directories.len() {
        let depth = Path::new(&directories[cursor].path).components().count();
        ensure!(
            depth <= MAX_FAILED_PUBLICATION_STAGING_DEPTH,
            "failed PublicationV3 staging exceeds cleanup depth bound"
        );
        fchmod(
            directories[cursor].descriptor.as_fd(),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )?;
        directories[cursor].identity =
            publication_node_identity_v3(&directories[cursor].descriptor.metadata()?);
        let entries = publication_directory_entries_v3(&directories[cursor].descriptor)?;
        for (name, observed_inode, observed_type) in entries {
            seen = seen
                .checked_add(1)
                .context("failed PublicationV3 staging cleanup entry overflow")?;
            ensure!(
                seen <= MAX_FAILED_PUBLICATION_STAGING_ENTRIES,
                "failed PublicationV3 staging exceeds cleanup entry bound"
            );
            let child_path = if directories[cursor].path == "." {
                path_to_manifest(Path::new(&name))?
            } else {
                format!(
                    "{}/{}",
                    directories[cursor].path,
                    path_to_manifest(Path::new(&name))?
                )
            };
            match observed_type {
                FileType::Directory => {
                    let child = open_publication_child_v3(
                        &directories[cursor].descriptor,
                        Path::new(&name),
                    )?;
                    let identity = publication_node_identity_v3(&child.metadata()?);
                    validate_publication_node_v3(
                        &identity,
                        expected_uid,
                        expected_device,
                        &child_path,
                    )?;
                    ensure!(
                        observed_inode == 0 || observed_inode == identity.inode,
                        "failed PublicationV3 staging directory changed during cleanup scan"
                    );
                    directories.push(CleanupDirectoryV3 {
                        path: child_path,
                        descriptor: child,
                        parent_index: Some(cursor),
                        name: Some(name),
                        identity,
                    });
                }
                FileType::RegularFile => {
                    let child = open_publication_child_identity_v3(
                        &directories[cursor].descriptor,
                        Path::new(&name),
                    )?;
                    let identity = publication_node_identity_v3(&child.metadata()?);
                    validate_publication_node_v3(
                        &identity,
                        expected_uid,
                        expected_device,
                        &child_path,
                    )?;
                    ensure!(
                        identity.links == 1
                            && (observed_inode == 0 || observed_inode == identity.inode),
                        "failed PublicationV3 staging contains a hard-linked or substituted file"
                    );
                    leaves.push(CleanupLeafV3 {
                        parent_index: cursor,
                        name,
                        identity,
                        symlink: false,
                    });
                }
                FileType::Symlink => {
                    let stat = statat(
                        directories[cursor].descriptor.as_fd(),
                        Path::new(&name),
                        AtFlags::SYMLINK_NOFOLLOW,
                    )?;
                    let identity = symlink_identity(&stat);
                    validate_publication_node_v3(
                        &identity,
                        expected_uid,
                        expected_device,
                        &child_path,
                    )?;
                    ensure!(
                        observed_inode == 0 || observed_inode == identity.inode,
                        "failed PublicationV3 staging symlink changed during cleanup scan"
                    );
                    leaves.push(CleanupLeafV3 {
                        parent_index: cursor,
                        name,
                        identity,
                        symlink: true,
                    });
                }
                _ => anyhow::bail!(
                    "failed PublicationV3 staging contains a special node at {child_path}; preserve pinned stage"
                ),
            }
        }
        cursor += 1;
    }

    let mut operation = 0_usize;
    for leaf in leaves.into_iter().rev() {
        before_operation(operation);
        operation += 1;
        staging.ensure_live()?;
        let parent = &directories[leaf.parent_index].descriptor;
        let rebound = if leaf.symlink {
            symlink_identity(&statat(
                parent.as_fd(),
                Path::new(&leaf.name),
                AtFlags::SYMLINK_NOFOLLOW,
            )?)
        } else {
            publication_node_identity_v3(
                &open_publication_child_identity_v3(parent, Path::new(&leaf.name))?.metadata()?,
            )
        };
        ensure!(
            rebound == leaf.identity,
            "failed PublicationV3 staging leaf was substituted; preserve pinned stage"
        );
        unlinkat(parent.as_fd(), Path::new(&leaf.name), AtFlags::empty())?;
    }
    for index in (1..directories.len()).rev() {
        before_operation(operation);
        operation += 1;
        staging.ensure_live()?;
        let directory = &directories[index];
        let parent = &directories[directory
            .parent_index
            .context("cleanup directory has no parent")?];
        let name = directory
            .name
            .as_ref()
            .context("cleanup directory has no name")?;
        let rebound = open_publication_child_v3(&parent.descriptor, Path::new(name))?;
        let rebound_identity = publication_node_identity_v3(&rebound.metadata()?);
        ensure!(
            publication_same_stable_node_v3(&rebound_identity, &directory.identity)
                && publication_directory_entries_v3(&rebound)?.is_empty(),
            "failed PublicationV3 staging directory was substituted; preserve pinned stage"
        );
        unlinkat(
            parent.descriptor.as_fd(),
            Path::new(name),
            AtFlags::REMOVEDIR,
        )?;
    }
    before_operation(operation);
    staging.ensure_live()?;
    ensure!(
        publication_directory_entries_v3(&staging.root)?.is_empty(),
        "failed PublicationV3 staging root changed before final cleanup"
    );
    unlinkat(
        staging.parent.as_fd(),
        Path::new(&staging.name),
        AtFlags::REMOVEDIR,
    )?;
    ensure!(
        open_optional_publication_child_identity_v3(&staging.parent, Path::new(&staging.name),)?
            .is_none(),
        "failed PublicationV3 staging basename remains after cleanup"
    );
    staging.parent.sync_all()?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn discard_failed_publication_staging(_staging: PinnedPublicationStagingV3) -> Result<()> {
    anyhow::bail!("PublicationV3 guarded cleanup requires Linux dirfds")
}

fn copy_file_exact(source: &Path, destination: &Path) -> Result<()> {
    let artifact = artifact_from_file(source, "application/octet-stream")?;
    copy_artifact_exact(source, destination, &artifact)
}

fn make_private_executables_and_states_read_only(
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

fn make_lock_files_read_only(root: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    fn synthetic_validated_staging_v3(
        staging: &PinnedPublicationStagingV3,
    ) -> Result<ValidatedPublicationV3> {
        Ok(ValidatedPublicationV3 {
            root_path: staging.path.clone(),
            root: staging.root.try_clone()?,
            root_parent_path: staging.parent_path.clone(),
            root_parent: staging.parent.try_clone()?,
            root_parent_identity: publication_node_identity_v3(&staging.parent.metadata()?),
            root_name: staging.name.clone(),
            inventory: publication_tree_inventory_v3_from_fd(staging.path(), &staging.root)?,
            lock_sha256: Digest32::digest_bytes(b"synthetic PublicationV3 lock"),
        })
    }

    #[cfg(target_os = "linux")]
    fn synthetic_cloudflare_materialization_authority_v1(
        source: &Path,
    ) -> Result<(
        ValidatedPublicationV3,
        CloudflareMaterializationProvenanceV1,
        Vec<CloudflareMaterializedOriginInventoryV1>,
    )> {
        use robin_run_protocol::{
            BrowserIdentitySignerBuildIdentityV2, BrowserIdentitySignerBuildRecipeV2,
            BrowserIdentitySignerDeploymentPolicyV2, BrowserPagesArtifactV2,
            BrowserPagesShellBuildIdentityV2, BrowserPagesShellBuildRecipeV2,
            BrowserViewerBuildIdentityV2, BrowserViewerEngineBuildIdentityV2,
            BrowserViewerEngineBuildRecipeV2, BuildToolAuthorityV1, NamedArtifactV1,
            NativeBuildPlatformV2, NativeLinkageV2, RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2,
            RustToolchainAuthorityV1, VerifierBuildIdentityV2, ViewerArtifactRoleV1,
            WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1, WASM_BINDGEN_CLI_VERSION_V1,
        };

        let artifact = |bytes: &[u8], media_type: &str| ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: media_type.into(),
        };
        let tool = |byte: u8, version: &str| BuildToolAuthorityV1 {
            version: version.into(),
            authority_sha256: if version == WASM_BINDGEN_CLI_VERSION_V1 {
                WASM_BINDGEN_CLI_AUTHORITY_SHA256_V1
            } else {
                Digest32::from_bytes([byte; 32])
            },
        };
        let binaryen_authority: BuildToolAuthorityDocumentV1 =
            serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.github/tool-authorities/binaryen-wasm-opt-v132.json"
            )))?;
        let wabt_authority: BuildToolAuthorityDocumentV1 =
            serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.github/tool-authorities/wabt-wasm-strip-v1.0.41.json"
            )))?;
        let rust_toolchain = RustToolchainAuthorityV1 {
            schema_version: 1,
            channel: "nightly-2026-08-25".into(),
            components: vec!["rust-src".into(), "rustc-codegen-cranelift-preview".into()],
            targets: vec!["wasm32-unknown-unknown".into()],
        };
        let engine_js = b"synthetic engine JavaScript";
        let engine_wasm = b"synthetic engine WebAssembly";
        let public_headers = b"synthetic public headers";
        let public_index = b"synthetic public index";
        let signer_js = b"synthetic signer JavaScript";
        let signer_wasm = b"synthetic signer WebAssembly";
        let signer_index = b"synthetic signer index";
        let build = BuildManifestV2 {
            schema_version: 2,
            source_commit: "a".repeat(40),
            cargo_lock_sha256: Digest32::digest_bytes(b"Cargo.lock"),
            replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            save_schema_version: robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
            network_protocol_version:
                robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
            verifier: VerifierBuildIdentityV2 {
                platform: NativeBuildPlatformV2::X86_64UnknownLinuxMusl,
                target_triple: "x86_64-unknown-linux-musl".into(),
                cargo_profile: "release".into(),
                cargo_features: vec![],
                cargo_package: "robin_replay_verifier".into(),
                cargo_binary: "robin-replay-verifier".into(),
                linkage: NativeLinkageV2::FullyStaticNoInterpreterOrNeededLibraries,
                artifact: artifact(b"synthetic verifier", RANKED_REPLAY_VERIFIER_MEDIA_TYPE_V2),
            },
            viewer: BrowserViewerBuildIdentityV2 {
                engine: BrowserViewerEngineBuildIdentityV2 {
                    target_triple: "wasm32-unknown-unknown".into(),
                    cargo_profile: "wasm-release".into(),
                    cargo_features: vec!["audio".into()],
                    cargo_package: "robin_rs".into(),
                    cargo_binary: "robin".into(),
                    recipe: BrowserViewerEngineBuildRecipeV2::WasmBindgenWebBinaryenOzStripDebugDwarfWabtStripV1,
                    rust_toolchain_sha256: rust_toolchain.canonical_digest()?,
                    rust_toolchain: rust_toolchain.clone(),
                    wasm_bindgen_cli: tool(16, "0.2.127"),
                    binaryen_wasm_opt: BuildToolAuthorityV1 {
                        version: binaryen_authority.version.clone(),
                        authority_sha256: binaryen_authority.canonical_digest()?,
                    },
                    wabt_wasm_strip: BuildToolAuthorityV1 {
                        version: wabt_authority.version.clone(),
                        authority_sha256: wabt_authority.canonical_digest()?,
                    },
                    artifacts: vec![
                        NamedArtifactV1 {
                            path: "viewer/robin.js".into(),
                            role: ViewerArtifactRoleV1::EntryJavaScript,
                            artifact: artifact(engine_js, "text/javascript"),
                        },
                        NamedArtifactV1 {
                            path: "viewer/robin_bg.wasm".into(),
                            role: ViewerArtifactRoleV1::WebAssembly,
                            artifact: artifact(engine_wasm, "application/wasm"),
                        },
                    ],
                },
                pages_shell: BrowserPagesShellBuildIdentityV2 {
                    recipe: BrowserPagesShellBuildRecipeV2::PnpmFrozenLockfileViteStaticShellV1,
                    node: tool(22, "24.19.0"),
                    pnpm: tool(19, "9.15.0"),
                    package_json_sha256: Digest32::digest_bytes(b"package.json"),
                    pnpm_lock_sha256: Digest32::digest_bytes(b"pnpm-lock.yaml"),
                    public_origin_artifacts: vec![
                        BrowserPagesArtifactV2 {
                            path: "_headers".into(),
                            artifact: artifact(public_headers, "text/plain"),
                        },
                        BrowserPagesArtifactV2 {
                            path: "index.html".into(),
                            artifact: artifact(public_index, "text/html"),
                        },
                    ],
                },
                identity_signer: BrowserIdentitySignerBuildIdentityV2 {
                    target_triple: "wasm32-unknown-unknown".into(),
                    cargo_profile: "wasm-release".into(),
                    cargo_features: vec!["identity-signer-bridge".into()],
                    cargo_package: "robin_rs".into(),
                    cargo_binary: "leaderboard_identity_bridge".into(),
                    recipe: BrowserIdentitySignerBuildRecipeV2::WasmBindgenWebSeparateOriginBridgeV1,
                    deployment_policy: BrowserIdentitySignerDeploymentPolicyV2::SeparateAllowlistedOriginCspFrameAncestorsAndBridgeShaV1,
                    rust_toolchain_sha256: rust_toolchain.canonical_digest()?,
                    rust_toolchain,
                    wasm_bindgen_cli: tool(16, "0.2.127"),
                    identity_signer_origin_artifacts: vec![
                        BrowserPagesArtifactV2 {
                            path: "identity-signer/bridge/leaderboard_identity_bridge.js".into(),
                            artifact: artifact(signer_js, "text/javascript"),
                        },
                        BrowserPagesArtifactV2 {
                            path: "identity-signer/bridge/leaderboard_identity_bridge_bg.wasm".into(),
                            artifact: artifact(signer_wasm, "application/wasm"),
                        },
                        BrowserPagesArtifactV2 {
                            path: "identity-signer/index.html".into(),
                            artifact: artifact(signer_index, "text/html"),
                        },
                    ],
                },
            },
        };
        build.validate()?;
        let build_sha256 = build.canonical_digest()?;
        let build_path = format!("manifests/builds/{build_sha256}.json");
        let engine_files = [
            ("viewer/robin.js", engine_js.as_slice()),
            ("viewer/robin_bg.wasm", engine_wasm.as_slice()),
        ];
        let mut source_files = vec![
            (
                format!("cloudflare-public/{build_path}"),
                canonical_json_bytes(&build)?,
            ),
            (
                "cloudflare-public/_headers".into(),
                public_headers.to_vec(),
            ),
            (
                "cloudflare-public/index.html".into(),
                public_index.to_vec(),
            ),
            (
                "cloudflare-identity-signer/identity-signer/bridge/leaderboard_identity_bridge.js"
                    .into(),
                signer_js.to_vec(),
            ),
            (
                "cloudflare-identity-signer/identity-signer/bridge/leaderboard_identity_bridge_bg.wasm"
                    .into(),
                signer_wasm.to_vec(),
            ),
            (
                "cloudflare-identity-signer/identity-signer/index.html".into(),
                signer_index.to_vec(),
            ),
        ];
        for named in &build.viewer.engine.artifacts {
            let bytes = engine_files
                .iter()
                .find_map(|(path, bytes)| (*path == named.path).then_some(*bytes))
                .context("synthetic engine artifact bytes are absent")?;
            source_files.push((
                format!(
                    "cloudflare-public/{}",
                    build_artifact_object_path_v1(build_sha256, named)?
                ),
                bytes.to_vec(),
            ));
        }
        let (datadir_authority, _, datadir_receipt) = datadir_binding()?;
        source_files.extend([
            (
                "deployment/exposure-v3.json".into(),
                canonical_json_bytes(&DeploymentExposureV3::official())?,
            ),
            (
                "deployment/datadir-authority.json".into(),
                canonical_json_bytes(&datadir_authority)?,
            ),
            (
                "deployment/datadir-deployment.json".into(),
                canonical_json_bytes(&datadir_receipt)?,
            ),
        ]);
        for (path, bytes) in source_files {
            let path = source.join(path);
            fs::create_dir_all(path.parent().context("synthetic CF file has no parent")?)?;
            fs::write(path, bytes)?;
        }
        let publication = ValidatedPublicationV3::synthetic_for_consumer_test(source)?;
        let manifest = PublicationManifestV3 {
            schema_version: 3,
            projection_authority_matrix_sha256: Digest32::digest_bytes(b"matrix"),
            official_content_digests_sha256: Digest32::digest_bytes(b"content"),
            build_manifest_sha256: build_sha256,
            viewer_build_report: fact(b"viewer report"),
            datadir_release_authority: fact(b"datadir authority"),
            datadir_deployment_receipt: fact(b"datadir receipt"),
            verifier_operator_config: fact(b"operator config"),
            campaign_states: vec![],
            rules_config_sha256: vec![],
            policy_manifest_sha256: vec![],
            ruleset_manifest_sha256: vec![],
            published_rulesets: vec![],
            competition_manifest_sha256: vec![],
            public_static_files: build
                .viewer
                .pages_shell
                .public_origin_artifacts
                .iter()
                .map(|file| PublicStaticFileArtifactV3 {
                    published_path: file.path.clone(),
                    artifact: file.artifact.clone(),
                })
                .collect(),
            identity_signer_files: build
                .viewer
                .identity_signer
                .identity_signer_origin_artifacts
                .iter()
                .map(|file| PublicStaticFileArtifactV3 {
                    published_path: file.path.clone(),
                    artifact: file.artifact.clone(),
                })
                .collect(),
        };
        let origins = derive_cloudflare_origin_inventories_v1(&publication, &manifest, &build)?;
        Ok((
            publication,
            CloudflareMaterializationProvenanceV1 {
                source_commit: "a".repeat(40),
                source_tree_sha1: "b".repeat(40),
                cargo_lock_sha256: Digest32::digest_bytes(b"Cargo.lock"),
                publication_manifest_sha256: Digest32::digest_bytes(b"PublicationManifestV3"),
                publication_lock_sha256: Digest32::digest_bytes(b"PublicationLockV3"),
            },
            origins,
        ))
    }

    #[cfg(target_os = "linux")]
    fn make_test_tree_writable(root: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let inventory = publication_tree_inventory_v3(root)?;
        for file in inventory.files {
            fs::set_permissions(root.join(file.path), fs::Permissions::from_mode(0o600))?;
        }
        let mut directories = inventory.directories;
        directories.sort_by_key(|directory| {
            std::cmp::Reverse(Path::new(&directory.path).components().count())
        });
        for directory in directories {
            let path = if directory.path == "." {
                root.to_path_buf()
            } else {
                root.join(directory.path)
            };
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    fn fact(label: &[u8]) -> ArtifactRefV1 {
        ArtifactRefV1 {
            sha256: Digest32::digest_bytes(label),
            byte_length: label.len() as u64,
            media_type: "application/octet-stream".into(),
        }
    }

    fn pinned_json(label: &[u8]) -> PinnedArtifactSourceV3 {
        let mut artifact = fact(label);
        artifact.media_type = "application/json".into();
        PinnedArtifactSourceV3 {
            source: PathBuf::from("metadata.json"),
            artifact,
        }
    }

    fn datadir_binding() -> Result<(
        DatadirReleaseAuthorityV1,
        Digest32,
        DatadirDeploymentReceiptV1,
    )> {
        let demo = DatadirDemoAuthorityV1 {
            content_manifest_url: DEMO_CONTENT_MANIFEST_URL.into(),
            content_manifest_sha256: Digest32::digest_bytes(b"content manifest"),
            datadir_url: DEMO_DATADIR_URL.into(),
            datadir_sha256: Digest32::digest_bytes(b"datadir"),
            datadir_byte_length: 123,
            native_content_sha256: Digest32::digest_bytes(b"native content"),
        };
        let authority = DatadirReleaseAuthorityV1 {
            schema_version: DATADIR_RELEASE_SCHEMA_VERSION,
            source_commit: "a".repeat(40),
            cargo_lock_sha256: Digest32::digest_bytes(b"Cargo.lock"),
            inventory_sha256: Digest32::digest_bytes(b"inventory"),
            worker_name: DATADIR_WORKER_NAME.into(),
            route_pattern: DATADIR_ROUTE_PATTERN.into(),
            public_root_url: DATADIR_PUBLIC_ROOT_URL.into(),
            demo,
        };
        let authority_sha256 = Digest32::digest_bytes(&canonical_json_bytes(&authority)?);
        let receipt = DatadirDeploymentReceiptV1 {
            schema_version: DATADIR_RELEASE_SCHEMA_VERSION,
            authority_sha256,
            inventory_sha256: authority.inventory_sha256,
            source_commit: authority.source_commit.clone(),
            worker_name: authority.worker_name.clone(),
            worker_version_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
            route_pattern: authority.route_pattern.clone(),
            public_root_url: authority.public_root_url.clone(),
            demo: authority.demo.clone(),
        };
        Ok((authority, authority_sha256, receipt))
    }

    fn campaign_artifact(label: &[u8]) -> ArtifactRefV1 {
        let mut artifact = fact(label);
        artifact.media_type = RANKED_CAMPAIGN_MEDIA_TYPE_V1.into();
        artifact
    }

    fn campaign_matrix(rules: &[Digest32]) -> Vec<CampaignStateArtifactV3> {
        let shared = campaign_artifact(b"shared canonical campaign");
        rules
            .iter()
            .flat_map(|rules_config_sha256| {
                [
                    CampaignStateArtifactV3 {
                        edition: OfficialContentEditionV1::Demo,
                        kind: CampaignStateKindV3::IndividualTemplate,
                        rules_config_sha256: *rules_config_sha256,
                        artifact: shared.clone(),
                    },
                    CampaignStateArtifactV3 {
                        edition: OfficialContentEditionV1::Full,
                        kind: CampaignStateKindV3::FullCampaignGenesis,
                        rules_config_sha256: *rules_config_sha256,
                        artifact: shared.clone(),
                    },
                ]
            })
            .collect()
    }

    fn ranked_rules_config() -> Result<RulesConfigIdentityV1> {
        use robin_engine::engine::SimConfig;
        use robin_engine::player_profile::DifficultyLevel;
        use robin_run_protocol::{CanonicalValue, RankedSimulationPolicyV1};

        let CanonicalValue::Object(sim_config) = serde_json::from_value(serde_json::to_value(
            SimConfig::standard_ranked(DifficultyLevel::Medium),
        )?)?
        else {
            anyhow::bail!("SimConfig must canonicalize as an object")
        };
        Ok(RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: RankedSimulationPolicyV1::standard(
                robin_run_protocol::RankedSimulationDifficultyV1::Medium,
            ),
            sim_config,
            rules: BTreeMap::from([("ranked".into(), CanonicalValue::Bool(true))]),
        })
    }

    fn privacy_ruleset_manifest() -> RulesetManifestV1 {
        use robin_run_protocol::*;

        fn policy(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
            ImmutablePolicyIdentityV1 {
                kind,
                version: 1,
                manifest_sha256: Digest32::from_bytes([byte; 32]),
            }
        }

        let rules_config_sha256 = Digest32::from_bytes([2; 32]);
        RulesetManifestV1 {
            schema_version: 1,
            display_name: "Full / Standard / Normal".into(),
            preset_id: OpaqueId::new("standard").unwrap(),
            preset_name: "Standard".into(),
            difficulty_id: OpaqueId::new("normal").unwrap(),
            difficulty_name: "Normal".into(),
            rules_config_sha256,
            rules_config_constraint: RulesConfigConstraintV1::ExactCanonicalDigestOnly,
            allowed_build_manifest_sha256: vec![Digest32::from_bytes([8; 32])],
            allowed_content_manifest_sha256: vec![Digest32::from_bytes([9; 32])],
            allowed_campaign_content_manifest_sha256: vec![Digest32::from_bytes([10; 32])],
            board_scopes: vec![
                RulesetBoardScopeV1::CampaignMission,
                RulesetBoardScopeV1::FullCampaign,
            ],
            campaign_completion_policy: CampaignCompletionPolicyRequirementV1::Required(
                official_full_campaign_completion_policy_v1(),
            ),
            metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
            metric_ranking: vec![
                MetricRankingPolicyV1::OriginalScoreDescending,
                MetricRankingPolicyV1::FastestSuccessAscending,
            ],
            achievement_policies: official_achievement_policies_v1(),
            canonical_start_policy:
                CanonicalStartPolicyV1::RulesConfigBoundOperatorStateAndVerifiedPredecessor,
            canonical_campaign_state: CanonicalCampaignStateRequirementV1 {
                edition: OfficialContentEditionV1::Full,
                kind: CanonicalCampaignStateKindV1::FullCampaignGenesis,
                rules_config_sha256,
            },
            run_preflight_grant_public_key: PublicKey32::from_bytes([44; 32]),
            full_campaign_chain_policy:
                FullCampaignChainPolicyV1::CanonicalGenesisEveryFieldAndHeadquartersSessionIndependentCompletion,
            campaign_roster_continuity:
                CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets,
            campaign_aggregation_consent_policy:
                CampaignAggregationConsentPolicyV1::EveryAuthenticatedKeyFinalCosignsEachSession,
            participant_eligibility: ParticipantEligibilityV1 {
                allow_single_player: true,
                allow_multiplayer: true,
                named_policy:
                    NamedParticipantPolicyV1::HostGenesisGuestTransportJoinAttestationAndFinalCosign,
                anonymous_policy:
                    AnonymousParticipantPolicyV1::AllowedAuthenticatedButPubliclyRedacted,
                minimum_max_concurrent_players: 1,
                maximum_max_concurrent_players: MAX_REPLAY_SEATS_V1,
                maximum_participant_instances: MAX_PARTICIPANT_INSTANCES_V1,
            },
            replay_schema_versions: vec![CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1],
            network_protocol_versions: vec![CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1],
            input_provenance_policy: policy(ImmutablePolicyKindV1::InputProvenance, 10),
            command_admission_policy: policy(ImmutablePolicyKindV1::CommandAdmission, 11),
            submission_admission_policy: policy(ImmutablePolicyKindV1::SubmissionAdmission, 12),
            verifier_policy: policy(ImmutablePolicyKindV1::Verification, 13),
            input_provenance_eligibility:
                InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
            terminal_result_policy: TerminalResultPolicyV1::IndependentlyReachedWonOnly,
            score_algorithm:
                ScoreAlgorithmV1::OriginalMissionAttemptWrappingSubtotalCampaignDeltaV1,
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
        }
    }

    fn privacy_competition_manifest() -> CompetitionManifestV1 {
        use robin_run_protocol::*;

        CompetitionManifestV1 {
            schema_version: 1,
            competition_id: OpaqueId::new("daily-mission-1").unwrap(),
            competition_version: 1,
            display_name: "Daily Mission 1".into(),
            description: "A pinned-seed daily board.".into(),
            subject: LeaderboardSubjectV1::Mission {
                mission_id: "mission_1".into(),
                category: BoardCategoryV1::IndividualLevel,
            },
            metric: BoardMetricV1::OriginalScore,
            rules_config_sha256: Digest32::from_bytes([2; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([3; 32]),
            canonical_campaign_state: CanonicalCampaignStateRequirementV1 {
                edition: OfficialContentEditionV1::Demo,
                kind: CanonicalCampaignStateKindV1::IndividualTemplate,
                rules_config_sha256: Digest32::from_bytes([2; 32]),
            },
            content: RunContentIdentityV1::Mission {
                content_manifest_sha256: Digest32::from_bytes([1; 32]),
            },
            seed_policy: CompetitionSeedPolicyV1::Pinned {
                simulation_seed: SimulationSeed64::new(42),
            },
            participant_composition: CompetitionParticipantCompositionV1::SinglePlayer,
            competition_run_grant_public_key: PublicKey32::from_bytes([7; 32]),
            starts_at_unix_ms: 1_800_000_000_000,
            ends_at_unix_ms: 1_800_086_400_000,
        }
    }

    #[test]
    fn campaign_state_matrix_is_config_bound_complete_and_allows_shared_bytes() -> Result<()> {
        let mut rules = vec![
            Digest32::digest_bytes(b"standard medium"),
            Digest32::digest_bytes(b"original parity medium"),
        ];
        rules.sort();
        let states = campaign_matrix(&rules);
        ensure!(campaign_state_matrix_is_exact(&states, &rules));
        ensure!(
            states
                .iter()
                .all(|state| state.artifact == states[0].artifact
                    && state.canonical_pin().validate().is_ok()),
            "test matrix does not exercise shared physical campaign bytes"
        );

        let mut legacy = serde_json::to_value(&states[0])?;
        legacy
            .as_object_mut()
            .context("campaign pin is not an object")?
            .remove("rules_config_sha256");
        ensure!(
            serde_json::from_value::<CampaignStateArtifactV3>(legacy).is_err(),
            "campaign pin accepted the former unbound schema"
        );

        let mut missing = states.clone();
        missing.pop();
        ensure!(!campaign_state_matrix_is_exact(&missing, &rules));
        let mut substituted = states;
        substituted[0].rules_config_sha256 = Digest32::digest_bytes(b"substituted rules");
        ensure!(!campaign_state_matrix_is_exact(&substituted, &rules));
        Ok(())
    }

    #[test]
    fn campaign_state_source_must_be_canonical_fresh_campaign_bitcode() -> Result<()> {
        use robin_engine::campaign::Campaign;
        use robin_engine::player_profile::DifficultyLevel;
        use robin_engine::profiles::{CharacterProfile, MissionProfile, ProfileManager};

        let rules = ranked_rules_config()?;
        let rules_config_sha256 = rules.canonical_digest()?;
        let mut profiles = ProfileManager::new();
        for name in ["Robin des villes", "Robin des bois", "Petit Jean"] {
            profiles.characters.push(CharacterProfile {
                profile_name: name.into(),
                ..Default::default()
            });
        }
        profiles.missions.push(MissionProfile::default());
        let campaign = Campaign::from_profiles(&profiles, DifficultyLevel::Medium);
        let admitted_profiles = AdmittedProfileManagersV1 {
            demo: profiles.clone(),
            full: profiles.clone(),
        };
        let root = tempfile::tempdir()?;
        let path = root.path().join("campaign.bitcode");
        let bytes = bitcode::encode(&campaign);
        fs::write(&path, &bytes)?;
        let mut source = CampaignStateSourceV3 {
            edition: OfficialContentEditionV1::Demo,
            kind: CampaignStateKindV3::IndividualTemplate,
            rules_config_sha256,
            source: path.clone(),
            artifact: ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&bytes),
                byte_length: bytes.len() as u64,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
            },
        };
        validate_campaign_state_source(&source, &rules, &admitted_profiles)?;

        // A canonical, fresh campaign with the same edition/difficulty/shape
        // is still untrusted when it was authored from a different profile
        // catalog. Exact rederivation must reject it.
        let mut substituted_profiles = profiles;
        substituted_profiles.missions[0].blazon_price += 1;
        let substituted = Campaign::from_profiles(&substituted_profiles, DifficultyLevel::Medium);
        let bytes = bitcode::encode(&substituted);
        fs::write(&path, &bytes)?;
        source.artifact.sha256 = Digest32::digest_bytes(&bytes);
        source.artifact.byte_length = bytes.len() as u64;
        ensure!(
            validate_campaign_state_source(&source, &rules, &admitted_profiles).is_err(),
            "publication accepted a fresh campaign authored from a substituted ProfileManager"
        );

        let mut progressed = campaign;
        progressed.current_mission_idx = Some(0);
        let bytes = bitcode::encode(&progressed);
        fs::write(&path, &bytes)?;
        source.artifact.sha256 = Digest32::digest_bytes(&bytes);
        source.artifact.byte_length = bytes.len() as u64;
        ensure!(
            validate_campaign_state_source(&source, &rules, &admitted_profiles).is_err(),
            "publication accepted a selected-mission campaign as canonical genesis"
        );
        Ok(())
    }

    #[test]
    fn publication_plan_rejects_omitted_datadir_binding() -> Result<()> {
        let pin = serde_json::to_value(pinned_json(b"operator config"))?;
        let plan = serde_json::json!({
            "schema_version": PUBLICATION_PLAN_SCHEMA_VERSION,
            "official_content_authority": "authority",
            "build_draft_v2": "build.json",
            "viewer_build_report": "report.json",
            "verifier_operator_config": pin,
            "campaign_states": [],
            "policies": [],
            "published_rulesets": [],
            "transition": {"kind": "fresh"}
        });
        ensure!(
            serde_json::from_value::<OperatorPublicationPlanV3>(plan).is_err(),
            "publication plan accepted an omitted datadir authority and receipt"
        );
        Ok(())
    }

    #[test]
    fn datadir_receipt_rejects_substitution() -> Result<()> {
        let (authority, authority_sha256, mut receipt) = datadir_binding()?;
        receipt.demo.datadir_sha256 = Digest32::digest_bytes(b"substituted datadir");
        ensure!(
            validate_datadir_binding(&authority, authority_sha256, &receipt).is_err(),
            "deployment receipt accepted a substituted Demo archive"
        );

        let (_, _, receipt) = datadir_binding()?;
        let mut substituted_authority = authority.clone();
        substituted_authority.demo.native_content_sha256 =
            Digest32::digest_bytes(b"substituted native content");
        ensure!(
            validate_datadir_binding(&substituted_authority, authority_sha256, &receipt).is_err(),
            "deployment receipt accepted a substituted release authority"
        );
        Ok(())
    }

    #[test]
    fn prior_immutable_datadir_authority_is_independent_of_later_build() -> Result<()> {
        let (authority, authority_sha256, receipt) = datadir_binding()?;
        let later_build_source_commit = "b".repeat(40);
        let later_build_cargo_lock = Digest32::digest_bytes(b"later Cargo.lock");
        ensure!(
            authority.source_commit != later_build_source_commit
                && authority.cargo_lock_sha256 != later_build_cargo_lock,
            "test does not model a datadir produced by an earlier build"
        );
        validate_datadir_binding(&authority, authority_sha256, &receipt)?;
        Ok(())
    }

    #[test]
    fn datadir_producer_provenance_must_remain_well_formed_and_receipt_bound() -> Result<()> {
        let (authority, authority_sha256, receipt) = datadir_binding()?;
        let mut invalid_source = authority.clone();
        invalid_source.source_commit = "A".repeat(40);
        ensure!(
            validate_datadir_binding(&invalid_source, authority_sha256, &receipt).is_err(),
            "publication accepted malformed datadir producer provenance"
        );

        let mut zero_lock = authority.clone();
        zero_lock.cargo_lock_sha256 = Digest32::from_bytes([0; 32]);
        ensure!(
            validate_datadir_binding(&zero_lock, authority_sha256, &receipt).is_err(),
            "publication accepted zero datadir Cargo.lock provenance"
        );

        let mut substituted_receipt = receipt;
        substituted_receipt.source_commit = "b".repeat(40);
        ensure!(
            validate_datadir_binding(&authority, authority_sha256, &substituted_receipt).is_err(),
            "publication accepted a receipt from another datadir producer"
        );
        Ok(())
    }

    #[test]
    fn datadir_authority_schema_has_no_full_payload_lane() -> Result<()> {
        let (authority, _, _) = datadir_binding()?;
        let mut value = serde_json::to_value(authority)?;
        value
            .as_object_mut()
            .context("authority is not an object")?
            .insert(
                "full".into(),
                serde_json::json!({"datadir_url": "forbidden"}),
            );
        ensure!(
            serde_json::from_value::<DatadirReleaseAuthorityV1>(value).is_err(),
            "datadir authority accepted a Full retail payload field"
        );
        Ok(())
    }

    #[test]
    fn publication_deployment_inventory_rejects_datadir_payload_bytes() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("deployment"))?;
        for name in [
            "datadir-authority.json",
            "datadir-deployment.json",
            "exposure-v3.json",
        ] {
            fs::write(root.path().join("deployment").join(name), b"{}")?;
        }
        let inventory = publication_tree_inventory_v3(root.path())?;
        validate_deployment_metadata_inventory(&inventory)?;
        fs::write(
            root.path().join("deployment/v8-web-opus-q80.rhdata.zst"),
            b"forbidden datadir payload",
        )?;
        ensure!(
            validate_deployment_metadata_inventory(&publication_tree_inventory_v3(root.path())?)
                .is_err(),
            "normal publication accepted datadir payload bytes"
        );
        Ok(())
    }

    #[test]
    fn status_transition_rejects_immutable_change_and_accepts_status_only() {
        let old = BTreeMap::from([
            ("backend/manifests/builds/a.json".into(), fact(b"build")),
            (
                "backend/manifests/published-rulesets/r.json".into(),
                fact(b"active"),
            ),
        ]);
        let mut new = old.clone();
        new.insert(
            "backend/manifests/published-rulesets/r.json".into(),
            fact(b"quarantined"),
        );
        assert_eq!(
            immutable_transition_files(&old),
            immutable_transition_files(&new)
        );
        assert_ne!(status_transition_files(&old), status_transition_files(&new));
        new.insert("backend/manifests/builds/a.json".into(), fact(b"changed"));
        assert_ne!(
            immutable_transition_files(&old),
            immutable_transition_files(&new)
        );
    }

    #[test]
    fn public_static_paths_reject_reserved_and_ambiguous_forms() {
        for invalid in ["/index.html", "a/../b", "a\\b", "a%2fb", "", "a/"] {
            let artifact = robin_run_protocol::BrowserPagesArtifactV2 {
                path: invalid.into(),
                artifact: fact(b"page"),
            };
            assert!(artifact.validate().is_err(), "accepted {invalid:?}");
        }
        let valid = robin_run_protocol::BrowserPagesArtifactV2 {
            path: "assets/app.js".into(),
            artifact: fact(b"page"),
        };
        assert!(valid.validate().is_ok());
        assert!(matches!(
            "private/secret".split('/').next(),
            Some("builds" | "content" | "manifests" | "private")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn nested_private_authority_materialization_creates_only_the_exact_parent() -> Result<()> {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

        let sandbox = tempfile::tempdir()?;
        let staging = sandbox.path().join("staging");
        fs::create_dir(&staging)?;
        let authority = sandbox.path().join("authority");
        fs::create_dir(&authority)?;
        fs::create_dir(authority.join("manifests"))?;
        fs::create_dir(authority.join("empty"))?;
        fs::write(
            authority.join("official-content-digests.json"),
            b"authority root",
        )?;
        fs::write(authority.join("manifests/build.json"), b"nested manifest")?;
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o555))?;
        fs::set_permissions(
            authority.join("manifests"),
            fs::Permissions::from_mode(0o555),
        )?;
        fs::set_permissions(authority.join("empty"), fs::Permissions::from_mode(0o711))?;
        fs::set_permissions(
            authority.join("manifests/build.json"),
            fs::Permissions::from_mode(0o444),
        )?;

        let private_root = create_private_publication_root(&staging)?;
        assert_eq!(private_root, staging.join("private"));
        assert_eq!(
            fs::metadata(&private_root)?.permissions().mode() & 0o777,
            0o755
        );
        let copied = private_root.join("official-content-authority");
        copy_directory_exact_preserving_modes(&authority, &copied)?;
        assert_eq!(
            fs::read(copied.join("official-content-digests.json"))?,
            b"authority root"
        );
        assert_eq!(
            fs::read(copied.join("manifests/build.json"))?,
            b"nested manifest"
        );
        assert_eq!(fs::metadata(&copied)?.permissions().mode() & 0o777, 0o555);
        assert_eq!(
            fs::metadata(copied.join("empty"))?.permissions().mode() & 0o777,
            0o711
        );
        let source_file_metadata = fs::metadata(authority.join("manifests/build.json"))?;
        let copied_file_metadata = fs::metadata(copied.join("manifests/build.json"))?;
        assert_eq!(copied_file_metadata.permissions().mode() & 0o777, 0o444);
        assert_ne!(source_file_metadata.ino(), copied_file_metadata.ino());
        assert_eq!(copied_file_metadata.nlink(), 1);
        assert_eq!(
            publication_directories(&staging)?
                .into_iter()
                .map(|directory| directory.path)
                .collect::<Vec<_>>(),
            vec![
                ".".to_owned(),
                "private".to_owned(),
                "private/official-content-authority".to_owned(),
                "private/official-content-authority/empty".to_owned(),
                "private/official-content-authority/manifests".to_owned(),
            ]
        );

        let attacked_staging = sandbox.path().join("attacked-staging");
        let outside = sandbox.path().join("outside");
        fs::create_dir(&attacked_staging)?;
        fs::create_dir(&outside)?;
        symlink(&outside, attacked_staging.join("private"))?;
        ensure!(
            create_private_publication_root(&attacked_staging).is_err(),
            "private publication root followed a symlink"
        );
        ensure!(
            fs::read_dir(&outside)?.next().is_none(),
            "symlink rejection modified the external target"
        );

        // Restore write permission so TempDir can clean up the deliberately
        // read-only source and copied authority trees.
        for directory in [
            &authority,
            &authority.join("empty"),
            &authority.join("manifests"),
            &copied,
            &copied.join("empty"),
            &copied.join("manifests"),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mode_preserving_copy_rejects_late_source_and_destination_substitution() -> Result<()> {
        let sandbox = tempfile::tempdir()?;

        let source = sandbox.path().join("source");
        fs::create_dir(&source)?;
        fs::write(source.join("payload"), b"reviewed")?;
        let destination = sandbox.path().join("destination");
        assert!(
            copy_directory_exact_preserving_modes_with(&source, &destination, || {
                fs::rename(
                    destination.join("payload"),
                    destination.join("copied-payload"),
                )
                .unwrap();
                fs::write(destination.join("payload"), b"reviewed").unwrap();
            })
            .is_err()
        );

        let source_two = sandbox.path().join("source-two");
        fs::create_dir(&source_two)?;
        fs::write(source_two.join("payload"), b"reviewed")?;
        let destination_two = sandbox.path().join("destination-two");
        assert!(
            copy_directory_exact_preserving_modes_with(&source_two, &destination_two, || {
                fs::write(source_two.join("payload"), b"substituted").unwrap()
            },)
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn publication_failure_never_creates_requested_output() {
        let root = tempfile::tempdir().unwrap();
        let plan = root.path().join("invalid.json");
        fs::write(&plan, b"{}").unwrap();
        let output = root.path().join("publication");
        assert!(assemble_publication_v3(&plan, &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn publication_lock_binds_the_wasm_bindgen_authority_document() -> Result<()> {
        let root = tempfile::tempdir()?;
        let authority: robin_run_protocol::BuildToolAuthorityDocumentV1 = serde_json::from_slice(
            include_bytes!("../../../.github/tool-authorities/wasm-bindgen-cli-v0.2.127.json"),
        )?;
        let digest = authority.canonical_digest()?;
        let relative = format!(
            "private/official-content-authority/manifests/build-tool-authorities/{digest}.json"
        );
        let path = root.path().join(&relative);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, canonical_json_bytes(&authority)?)?;

        let lock = publication_lock_from_actual_for_test(
            root.path(),
            Digest32::digest_bytes(b"publication"),
        )?;
        let entry = lock
            .files
            .iter()
            .find(|entry| entry.path == relative)
            .context("wasm-bindgen authority is absent from publication lock")?;
        assert_eq!(
            entry.artifact.sha256,
            artifact_from_file(&path, "application/octet-stream")?.sha256
        );
        assert_eq!(entry.exposure, ReleaseFileExposureV1::OperatorPrivate);
        Ok(())
    }

    #[test]
    fn privacy_scanner_finds_cross_chunk_sentinel_and_private_json_key() -> Result<()> {
        let root = tempfile::tempdir()?;
        let binary = root.path().join("viewer.wasm");
        let needle = b"private-authority-digest";
        let mut bytes = vec![b'x'; 64 * 1024 - 7];
        bytes.extend_from_slice(needle);
        bytes.extend_from_slice(b"tail");
        fs::write(&binary, bytes)?;
        ensure!(
            file_contains_bytes(&binary, needle)?,
            "privacy scanner missed a chunk-boundary sentinel"
        );

        let public = serde_json::json!({
            "safe": [{"projection_authority_manifest_sha256": "secret"}]
        });
        ensure!(
            reject_private_json_keys(
                &public,
                "test public JSON",
                PublicJsonSchema::Other,
                &mut Vec::new(),
            )
            .is_err(),
            "privacy scanner accepted a nested private authority field"
        );
        reject_private_json_keys(
            &serde_json::json!({"build_manifest_sha256": "public"}),
            "test public JSON",
            PublicJsonSchema::Other,
            &mut Vec::new(),
        )?;
        Ok(())
    }

    #[test]
    fn privacy_scanner_allows_only_typed_addressed_campaign_requirements() -> Result<()> {
        let root = tempfile::tempdir()?;
        let ruleset = privacy_ruleset_manifest();
        ruleset.validate()?;
        let ruleset_digest = ruleset.canonical_digest()?;
        let ruleset_value = serde_json::to_value(&ruleset)?;
        let ruleset_directory = root.path().join("ruleset-manifests");
        fs::create_dir(&ruleset_directory)?;
        fs::write(
            ruleset_directory.join(format!("{ruleset_digest}.json")),
            ruleset.canonical_bytes()?,
        )?;

        let published = PublishedRulesetV1 {
            schema_version: 1,
            ruleset_manifest_sha256: ruleset_digest,
            manifest: ruleset.clone(),
            operational_status: robin_run_protocol::RulesetOperationalStatusV1::Active,
        };
        published.validate()?;
        let published_directory = root.path().join("published-rulesets");
        fs::create_dir(&published_directory)?;
        fs::write(
            published_directory.join(format!("{ruleset_digest}.json")),
            published.canonical_bytes()?,
        )?;

        let competition = privacy_competition_manifest();
        competition.validate()?;
        let competition_digest = competition.canonical_digest()?;
        let competition_directory = root.path().join("competitions");
        fs::create_dir(&competition_directory)?;
        fs::write(
            competition_directory.join(format!("{competition_digest}.json")),
            competition.canonical_bytes()?,
        )?;
        scan_public_tree(root.path(), &[], "synthetic backend manifests")?;

        ensure!(
            public_json_schema(
                &format!("ruleset-manifests/{}.json", "A".repeat(64)),
                &ruleset_value,
                "uppercase addressed ruleset",
            )
            .is_err(),
            "privacy exception accepted an uppercase or mismatched address"
        );
        let requirement = serde_json::to_value(ruleset.canonical_campaign_state)?;
        let nested = serde_json::json!({"nested": {"canonical_campaign_state": requirement}});
        ensure!(
            reject_private_json_keys(
                &nested,
                "nested ruleset field",
                PublicJsonSchema::RulesetManifest,
                &mut Vec::new(),
            )
            .is_err(),
            "privacy exception widened to a nested field"
        );
        ensure!(
            reject_private_json_keys(
                &serde_json::json!({"canonical_campaign_state": ruleset.canonical_campaign_state}),
                "wrong document",
                PublicJsonSchema::Other,
                &mut Vec::new(),
            )
            .is_err(),
            "privacy exception widened to another document schema"
        );
        let mut requirement_with_private_payload =
            serde_json::to_value(ruleset.canonical_campaign_state)?;
        requirement_with_private_payload
            .as_object_mut()
            .context("requirement is not an object")?
            .insert("artifact".into(), serde_json::json!({"bytes": "private"}));
        ensure!(
            reject_private_json_keys(
                &serde_json::json!({
                    "canonical_campaign_state": requirement_with_private_payload,
                }),
                "ruleset with private campaign payload",
                PublicJsonSchema::RulesetManifest,
                &mut Vec::new(),
            )
            .is_err(),
            "typed public requirement accepted an unknown private payload"
        );
        for forbidden_key in ["canonical_campaign_state_path", "raw_campaign_state"] {
            ensure!(
                reject_private_json_keys(
                    &serde_json::json!({forbidden_key: "private"}),
                    "suffixed campaign state field",
                    PublicJsonSchema::Other,
                    &mut Vec::new(),
                )
                .is_err(),
                "privacy scanner accepted {forbidden_key}"
            );
        }
        Ok(())
    }

    #[test]
    fn public_tree_scanner_rejects_private_paths_and_concrete_values() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("assets"))?;
        fs::write(root.path().join("assets/app.js"), b"const x='secret-pin'")?;
        ensure!(
            scan_public_tree(root.path(), &[b"secret-pin".to_vec()], "test origin").is_err(),
            "public scanner accepted a concrete private value"
        );
        let campaign_bytes = b"\0canonical-private-campaign\xff".to_vec();
        fs::write(root.path().join("assets/app.js"), &campaign_bytes)?;
        ensure!(
            scan_public_tree(root.path(), &[campaign_bytes], "test origin").is_err(),
            "public scanner accepted concrete private campaign bytes"
        );
        let private_path = b"/release/private/campaign-states/secret".to_vec();
        fs::write(root.path().join("assets/app.js"), &private_path)?;
        ensure!(
            scan_public_tree(root.path(), &[private_path], "test origin").is_err(),
            "public scanner accepted an absolute private campaign path"
        );
        let operator_config = b"operator-config-secret-value".to_vec();
        fs::write(root.path().join("assets/app.js"), &operator_config)?;
        ensure!(
            scan_public_tree(root.path(), &[operator_config], "test origin").is_err(),
            "public scanner accepted concrete operator-config bytes"
        );
        fs::remove_file(root.path().join("assets/app.js"))?;
        fs::write(root.path().join("projection-receipt.json"), b"{}")?;
        ensure!(
            scan_public_tree(root.path(), &[], "test origin").is_err(),
            "public scanner accepted a private namespace path"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failed_read_only_staging_cleanup_is_bounded_and_never_follows_symlinks() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let staging_path = staging.path().to_path_buf();
        let sealed = staging
            .path()
            .join("private/official-content-authority/manifests");
        fs::create_dir_all(&sealed)?;
        fs::write(sealed.join("authority.json"), b"sealed authority")?;
        fs::set_permissions(
            sealed.join("authority.json"),
            fs::Permissions::from_mode(0o440),
        )?;
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o550))?;
        fs::set_permissions(
            sealed.parent().context("sealed authority has no parent")?,
            fs::Permissions::from_mode(0o550),
        )?;

        let outside = sandbox.path().join("outside");
        fs::create_dir(&outside)?;
        fs::write(outside.join("sentinel"), b"outside remains unchanged")?;
        symlink(&outside, staging.path().join("outside-link"))?;

        discard_failed_publication_staging(staging)?;
        ensure!(!staging_path.exists(), "failed staging path remains");
        ensure!(
            fs::read(outside.join("sentinel"))? == b"outside remains unchanged",
            "cleanup followed a symlink outside staging"
        );
        ensure!(
            fs::metadata(&outside)?.permissions().mode() & 0o777 != 0o700,
            "cleanup changed external directory permissions"
        );
        ensure!(
            fs::read_dir(sandbox.path())?
                .collect::<std::io::Result<Vec<_>>>()?
                .iter()
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".robin-manifestctl-")),
            "failed cleanup left a same-filesystem staging directory"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_staging_cleanup_preserves_authentic_tree_on_root_substitution() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        fs::write(staging.path().join("authentic"), b"preserve me")?;
        let authentic = sandbox.path().join("authentic-stage");
        let substitute_path = staging.path().to_path_buf();
        fs::rename(staging.path(), &authentic)?;
        fs::create_dir(staging.path())?;
        fs::write(staging.path().join("substitute"), b"do not delete")?;

        ensure!(
            discard_failed_publication_staging(staging).is_err(),
            "cleanup accepted a substituted staging basename"
        );
        ensure!(
            fs::read(authentic.join("authentic"))? == b"preserve me"
                && fs::read(substitute_path.join("substitute"))? == b"do not delete",
            "uncertain cleanup removed the authentic or substitute tree"
        );
        fs::remove_dir_all(&authentic)?;
        fs::remove_dir_all(substitute_path)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_staging_cleanup_rejects_mid_operation_swap_and_hardlinks() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        fs::write(staging.path().join("authentic"), b"preserve me")?;
        let authentic = sandbox.path().join("authentic-stage");
        let substitute_path = staging.path().to_path_buf();
        let cleanup = discard_failed_publication_staging_with(&staging, |operation| {
            if operation == 0 {
                fs::rename(&substitute_path, &authentic).unwrap();
                fs::create_dir(&substitute_path).unwrap();
                fs::write(substitute_path.join("substitute"), b"do not delete").unwrap();
            }
        });
        ensure!(
            cleanup.is_err()
                && authentic.join("authentic").is_file()
                && substitute_path.join("substitute").is_file(),
            "cleanup deleted a tree after a mid-operation root substitution"
        );
        fs::remove_dir_all(&authentic)?;
        fs::remove_dir_all(&substitute_path)?;
        drop(staging);

        let hardlink_output = sandbox.path().join("hardlink-publication");
        let hardlinked = create_pinned_publication_staging_v3(&hardlink_output)?;
        fs::write(hardlinked.path().join("one"), b"shared")?;
        fs::hard_link(hardlinked.path().join("one"), hardlinked.path().join("two"))?;
        let hardlinked_path = hardlinked.path().to_path_buf();
        ensure!(
            discard_failed_publication_staging(hardlinked).is_err()
                && hardlinked_path.join("one").is_file()
                && hardlinked_path.join("two").is_file(),
            "guarded cleanup deleted a hard-linked or uncertain staging tree"
        );
        fs::remove_dir_all(hardlinked_path)?;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn publication_persistence_noreplace_race_cleans_staging_without_overwrite() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let staging_path = staging.path().to_path_buf();
        fs::create_dir(staging.path().join("sealed"))?;
        fs::write(staging.path().join("sealed/data"), b"candidate")?;
        fs::set_permissions(
            staging.path().join("sealed"),
            fs::Permissions::from_mode(0o550),
        )?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        fs::create_dir(&output)?;
        fs::write(output.join("winner"), b"racing publisher")?;

        let persist_error = persist_publication_staging(&staging, &candidate, &output)
            .expect_err("NOREPLACE persistence overwrote a racing publisher");
        ensure!(
            persist_error
                .downcast_ref::<PublicationInstalledButParentSyncFailed>()
                .is_none(),
            "pre-rename failure was misclassified as an installed publication"
        );
        discard_failed_publication_staging(staging)?;
        ensure!(!staging_path.exists(), "raced staging path remains");
        ensure!(
            fs::read(output.join("winner"))? == b"racing publisher",
            "NOREPLACE persistence modified the racing output"
        );
        ensure!(
            !output.join("sealed/data").exists(),
            "NOREPLACE persistence partially merged the candidate"
        );
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn post_rename_sync_failure_reports_published_outcome_without_staging() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let staging_path = staging.path().to_path_buf();
        fs::write(staging.path().join("complete"), b"complete")?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        let outcome = persist_publication_staging_with(
            &staging,
            &candidate,
            &output,
            || {},
            |parent, source, destination| {
                use std::os::fd::AsFd as _;
                rustix::fs::renameat_with(
                    parent.as_fd(),
                    source,
                    parent.as_fd(),
                    destination,
                    rustix::fs::RenameFlags::NOREPLACE,
                )?;
                Ok(())
            },
            |_| anyhow::bail!("injected parent sync failure"),
        )?;
        let PublicationPersistenceOutcome::PublishedButParentSyncFailed(sync_error) = outcome
        else {
            anyhow::bail!("post-rename failure was not reported as an installed publication")
        };
        ensure!(!staging_path.exists(), "renamed staging path remains");
        ensure!(
            fs::read(output.join("complete"))? == b"complete",
            "published output is incomplete after post-rename sync failure"
        );
        let expected_lock = Digest32::digest_bytes(b"publication lock");
        let classified = installed_publication_durability_error(&output, expected_lock, sync_error);
        let installed = classified
            .downcast_ref::<PublicationInstalledButParentSyncFailed>()
            .context("installed-but-unsynced error is not downcastable")?;
        ensure!(
            installed.output == output && installed.publication_lock_sha256 == expected_lock,
            "installed-but-unsynced error lost its exact output identity"
        );
        ensure!(
            format!("{:#}", installed.source).contains("injected parent sync failure"),
            "installed-but-unsynced error lost its source"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_persistence_rejects_identical_stage_and_parent_substitution() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        fs::write(staging.path().join("payload"), b"reviewed")?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        let authentic = sandbox.path().join("authentic-stage");
        let stage_path = staging.path().to_path_buf();
        let result = persist_publication_staging_with(
            &staging,
            &candidate,
            &output,
            || {
                fs::rename(&stage_path, &authentic).unwrap();
                fs::create_dir(&stage_path).unwrap();
                fs::write(stage_path.join("payload"), b"reviewed").unwrap();
            },
            |_, _, _| anyhow::bail!("rename must not run after stage substitution"),
            |_| anyhow::bail!("sync must not run after stage substitution"),
        );
        ensure!(
            result.is_err() && !output.exists(),
            "persistence accepted an identical-byte stage inode substitution"
        );
        fs::remove_dir_all(authentic)?;
        fs::remove_dir_all(stage_path)?;
        drop(candidate);
        drop(staging);

        let outer = tempfile::tempdir()?;
        let parent = outer.path().join("release-parent");
        let moved_parent = outer.path().join("authentic-parent");
        fs::create_dir(&parent)?;
        let parent_output = parent.join("publication");
        let parent_staging = create_pinned_publication_staging_v3(&parent_output)?;
        fs::write(parent_staging.path().join("payload"), b"reviewed")?;
        let parent_candidate = synthetic_validated_staging_v3(&parent_staging)?;
        let result = persist_publication_staging_with(
            &parent_staging,
            &parent_candidate,
            &parent_output,
            || {
                fs::rename(&parent, &moved_parent).unwrap();
                fs::create_dir(&parent).unwrap();
            },
            |_, _, _| anyhow::bail!("rename must not run after parent substitution"),
            |_| anyhow::bail!("sync must not run after parent substitution"),
        );
        ensure!(
            result.is_err()
                && moved_parent
                    .join(
                        parent_staging
                            .path()
                            .file_name()
                            .context("stage basename is absent")?
                    )
                    .join("payload")
                    .is_file()
                && !parent_output.exists(),
            "persistence accepted or removed a substituted output parent"
        );
        fs::remove_dir_all(&parent)?;
        fs::remove_dir_all(&moved_parent)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_persistence_reconciles_rename_side_effect_then_error() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let staging_path = staging.path().to_path_buf();
        fs::write(staging.path().join("payload"), b"reviewed")?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        let outcome = persist_publication_staging_with(
            &staging,
            &candidate,
            &output,
            || {},
            |parent, source, destination| {
                use std::os::fd::AsFd as _;
                rustix::fs::renameat_with(
                    parent.as_fd(),
                    source,
                    parent.as_fd(),
                    destination,
                    rustix::fs::RenameFlags::NOREPLACE,
                )?;
                anyhow::bail!("injected error after successful rename")
            },
            |parent| {
                parent.sync_all()?;
                Ok(())
            },
        )?;
        let PublicationPersistenceOutcome::PublishedButParentSyncFailed(uncertainty) = outcome
        else {
            anyhow::bail!("rename side-effect error was not classified as installed uncertainty")
        };
        let uncertainty_text = format!("{uncertainty:#}");
        ensure!(
            uncertainty_text.contains("injected error after successful rename")
                && !staging_path.exists()
                && fs::read(output.join("payload"))? == b"reviewed",
            "persistence did not reconcile the exact installed candidate after rename error: {uncertainty_text}"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_persistence_classifies_parent_swap_after_install() -> Result<()> {
        let outer = tempfile::tempdir()?;
        let parent = outer.path().join("release-parent");
        let moved_parent = outer.path().join("authentic-parent");
        fs::create_dir(&parent)?;
        let output = parent.join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        fs::write(staging.path().join("payload"), b"reviewed")?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        let uncertainty = persist_publication_staging_with(
            &staging,
            &candidate,
            &output,
            || {},
            |parent_fd, source, destination| {
                use std::os::fd::AsFd as _;
                rustix::fs::renameat_with(
                    parent_fd.as_fd(),
                    source,
                    parent_fd.as_fd(),
                    destination,
                    rustix::fs::RenameFlags::NOREPLACE,
                )?;
                fs::rename(&parent, &moved_parent)?;
                fs::create_dir(&parent)?;
                Ok(())
            },
            |parent_fd| {
                parent_fd.sync_all()?;
                Ok(())
            },
        )
        .expect_err("post-install parent substitution was reported as a canonical installation");
        ensure!(
            uncertainty
                .downcast_ref::<PublicationPersistenceStateUncertain>()
                .is_some()
                && !output.exists()
                && fs::read(moved_parent.join("publication/payload"))? == b"reviewed",
            "post-install parent substitution was not classified as persistence uncertainty"
        );
        fs::remove_dir_all(parent)?;
        fs::remove_dir_all(moved_parent)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_persistence_rejects_output_loss_or_substitution_after_rename() -> Result<()> {
        for substitute in [false, true] {
            let sandbox = tempfile::tempdir()?;
            let output = sandbox.path().join("publication");
            let orphan = sandbox.path().join("orphaned-candidate");
            let staging = create_pinned_publication_staging_v3(&output)?;
            fs::write(staging.path().join("payload"), b"reviewed")?;
            let candidate = synthetic_validated_staging_v3(&staging)?;
            let uncertainty = persist_publication_staging_with(
                &staging,
                &candidate,
                &output,
                || {},
                |parent_fd, source, destination| {
                    use std::os::fd::AsFd as _;
                    rustix::fs::renameat_with(
                        parent_fd.as_fd(),
                        source,
                        parent_fd.as_fd(),
                        destination,
                        rustix::fs::RenameFlags::NOREPLACE,
                    )?;
                    Ok(())
                },
                |parent_fd| {
                    fs::rename(&output, &orphan)?;
                    if substitute {
                        fs::create_dir(&output)?;
                        fs::write(output.join("payload"), b"reviewed")?;
                    }
                    parent_fd.sync_all()?;
                    Ok(())
                },
            )
            .expect_err("lost canonical output was reported as installed");
            let state = uncertainty
                .downcast_ref::<PublicationPersistenceStateUncertain>()
                .context("canonical output loss was not typed persistence uncertainty")?;
            ensure!(
                state.intended_output == output
                    && state.parent_path == sandbox.path()
                    && fs::read(orphan.join("payload"))? == b"reviewed"
                    && (substitute == output.join("payload").is_file()),
                "persistence uncertainty lost orphan/retry evidence"
            );
            fs::remove_dir_all(orphan)?;
            if substitute {
                fs::remove_dir_all(output)?;
            }
        }
        Ok(())
    }

    #[test]
    fn deployment_exposure_serves_only_reviewed_roots() -> Result<()> {
        let root = tempfile::tempdir()?;
        for directory in [
            "cloudflare-public",
            "cloudflare-identity-signer",
            "backend/manifests",
        ] {
            fs::create_dir_all(root.path().join(directory))?;
        }
        write_canonical(
            &root.path().join("deployment/exposure-v3.json"),
            &DeploymentExposureV3::official(),
        )?;
        validate_deployment_exposure(&mut publication_tree_inventory_v3(root.path())?)?;

        let mut hostile = DeploymentExposureV3::official();
        hostile.cloudflare_routes[0].script = Some("robinhood-public-site".into());
        fs::write(
            root.path().join("deployment/exposure-v3.json"),
            canonical_json_bytes(&hostile)?,
        )?;
        ensure!(
            validate_deployment_exposure(&mut publication_tree_inventory_v3(root.path())?).is_err(),
            "deployment accepted the public Worker on the VPS API route"
        );
        Ok(())
    }

    #[test]
    fn release_exposure_classifies_only_manifest_registry_as_backend_visible() {
        assert_eq!(
            release_file_exposure("backend/manifests/builds/a.json"),
            ReleaseFileExposureV1::BackendManifest
        );
        for path in [
            "backend/publication-v3.json",
            "private/verifier/operator-config/secret",
            "deployment/exposure-v3.json",
            "publication-lock-v3.json",
        ] {
            assert_eq!(
                release_file_exposure(path),
                ReleaseFileExposureV1::OperatorPrivate
            );
        }
        assert_eq!(
            release_file_exposure("cloudflare-identity-signer/index.html"),
            ReleaseFileExposureV1::PublicStatic
        );
        assert_eq!(
            release_file_exposure("cloudflare-public/leaderboards/index.html"),
            ReleaseFileExposureV1::PublicStatic
        );
    }

    #[test]
    fn transition_rejects_mode_and_empty_directory_substitution() -> Result<()> {
        let reviewed = tempfile::tempdir()?;
        let candidate = tempfile::tempdir()?;
        for root in [reviewed.path(), candidate.path()] {
            fs::create_dir_all(root.join("backend/manifests/builds"))?;
            fs::write(root.join("backend/manifests/builds/a.json"), b"same")?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(
                candidate.path().join("backend/manifests/builds/a.json"),
                fs::Permissions::from_mode(0o600),
            )?;
            ensure!(
                compare_transition(reviewed.path(), candidate.path(), TransitionRule::Exact)
                    .is_err(),
                "rollback accepted a mode substitution"
            );
            fs::set_permissions(
                candidate.path().join("backend/manifests/builds/a.json"),
                fs::Permissions::from_mode(publication_unix_mode(
                    &reviewed.path().join("backend/manifests/builds/a.json"),
                )?),
            )?;
        }

        fs::create_dir(candidate.path().join("unexpected-empty"))?;
        ensure!(
            compare_transition(reviewed.path(), candidate.path(), TransitionRule::Exact).is_err(),
            "rollback accepted an extra empty directory"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn publication_inventory_rejects_symlink_and_special_node() -> Result<()> {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("real"))?;
        symlink("real", root.path().join("alias"))?;
        assert!(publication_directories(root.path()).is_err());

        let special_root = tempfile::tempdir()?;
        let _socket = std::os::unix::net::UnixListener::bind(special_root.path().join("socket"))?;
        assert!(publication_tree_inventory_v3(special_root.path()).is_err());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_lock_binds_complete_directory_set_and_modes() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("empty"))?;
        fs::create_dir_all(root.path().join("nested/leaf"))?;
        fs::write(root.path().join("nested/payload"), b"locked")?;
        fs::set_permissions(
            root.path().join("nested/payload"),
            fs::Permissions::from_mode(0o644),
        )?;
        fs::set_permissions(root.path().join("empty"), fs::Permissions::from_mode(0o711))?;
        fs::set_permissions(
            root.path().join("nested/leaf"),
            fs::Permissions::from_mode(0o750),
        )?;

        let lock = publication_lock_from_actual_for_test(
            root.path(),
            Digest32::digest_bytes(b"manifest"),
        )?;
        assert_eq!(
            lock.directories
                .iter()
                .map(|directory| directory.path.as_str())
                .collect::<Vec<_>>(),
            vec![".", "empty", "nested", "nested/leaf"]
        );
        assert_eq!(
            lock.directories
                .iter()
                .find(|directory| directory.path == "empty")
                .context("empty directory is absent")?
                .unix_mode,
            0o711
        );
        assert_eq!(
            lock.directories
                .iter()
                .find(|directory| directory.path == "nested/leaf")
                .context("leaf directory is absent")?
                .unix_mode,
            0o750
        );
        validate_publication_inventory_against_lock_v3(
            &publication_tree_inventory_v3(root.path())?,
            &lock,
        )?;

        fs::create_dir(root.path().join("extra-empty"))?;
        assert!(
            validate_publication_inventory_against_lock_v3(
                &publication_tree_inventory_v3(root.path())?,
                &lock,
            )
            .is_err()
        );
        fs::remove_dir(root.path().join("extra-empty"))?;

        fs::remove_dir(root.path().join("empty"))?;
        assert!(
            validate_publication_inventory_against_lock_v3(
                &publication_tree_inventory_v3(root.path())?,
                &lock,
            )
            .is_err()
        );
        fs::create_dir(root.path().join("empty"))?;
        fs::set_permissions(root.path().join("empty"), fs::Permissions::from_mode(0o711))?;

        fs::set_permissions(
            root.path().join("nested/payload"),
            fs::Permissions::from_mode(0o600),
        )?;
        assert!(
            validate_publication_inventory_against_lock_v3(
                &publication_tree_inventory_v3(root.path())?,
                &lock,
            )
            .is_err()
        );
        fs::set_permissions(
            root.path().join("nested/payload"),
            fs::Permissions::from_mode(0o644),
        )?;

        fs::set_permissions(
            root.path().join("nested/leaf"),
            fs::Permissions::from_mode(0o700),
        )?;
        assert!(
            validate_publication_inventory_against_lock_v3(
                &publication_tree_inventory_v3(root.path())?,
                &lock,
            )
            .is_err()
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn expected_publication_topology_rejects_prelock_extras_everywhere() -> Result<()> {
        fn fixture() -> Result<(tempfile::TempDir, ExpectedPublicationTopologyV3)> {
            let root = tempfile::tempdir()?;
            let mut expected = ExpectedPublicationTopologyV3::new();
            for (path, bytes) in [
                ("backend/manifests/builds/a.json", b"build".as_slice()),
                (
                    "private/verifier/operator-config/config",
                    b"operator config".as_slice(),
                ),
                ("cloudflare-public/index.html", b"public".as_slice()),
                (
                    "cloudflare-identity-signer/index.html",
                    b"signer".as_slice(),
                ),
            ] {
                let path = path.to_owned();
                expected.register_bytes(path.clone(), bytes)?;
                let absolute = root.path().join(path);
                fs::create_dir_all(absolute.parent().context("fixture file has no parent")?)?;
                fs::write(absolute, bytes)?;
            }
            expected.register_directory("backend/manifests/competitions")?;
            fs::create_dir_all(root.path().join("backend/manifests/competitions"))?;
            expected.validate_inventory_content(&publication_tree_inventory_v3(root.path())?)?;
            Ok((root, expected))
        }

        for attack in [
            "root-extra",
            "backend/manifests/untyped/extra.json",
            "private/verifier/operator-config/extra",
            "cloudflare-public/extra.js",
            "private/untyped/extra",
        ] {
            let (root, expected) = fixture()?;
            let attack = root.path().join(attack);
            fs::create_dir_all(attack.parent().context("attack file has no parent")?)?;
            fs::write(attack, b"attacker injected")?;
            ensure!(
                expected
                    .validate_inventory_content(&publication_tree_inventory_v3(root.path())?)
                    .is_err(),
                "typed PublicationV3 topology accepted extra file"
            );
        }
        for attack in [
            "unexpected-empty",
            "backend/manifests/untyped-empty",
            "private/verifier/operator-config/unexpected-empty",
            "cloudflare-public/unexpected-empty",
            "private/unexpected-empty",
        ] {
            let (root, expected) = fixture()?;
            fs::create_dir_all(root.path().join(attack))?;
            ensure!(
                expected
                    .validate_inventory_content(&publication_tree_inventory_v3(root.path())?)
                    .is_err(),
                "typed PublicationV3 topology accepted extra empty directory"
            );
        }

        let (root, mut expected) = fixture()?;
        ensure!(
            expected
                .register_bytes("cloudflare-public/index.html".into(), b"public")
                .is_err(),
            "typed PublicationV3 topology accepted a duplicate file registration"
        );
        expected.seal_and_validate(root.path(), &open_publication_root_v3(root.path())?)?;
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            root.path().join("cloudflare-public/index.html"),
            fs::Permissions::from_mode(0o644),
        )?;
        ensure!(
            expected
                .validate_inventory(&publication_tree_inventory_v3(root.path())?)
                .is_err(),
            "typed PublicationV3 topology accepted a wrong file mode"
        );
        for file in publication_tree_inventory_v3(root.path())?.files {
            fs::set_permissions(
                root.path().join(file.path),
                fs::Permissions::from_mode(0o600),
            )?;
        }
        for directory in publication_tree_inventory_v3(root.path())?.directories {
            let path = if directory.path == "." {
                root.path().to_path_buf()
            } else {
                root.path().join(directory.path)
            };
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_inventory_rejects_hardlinks_and_nested_mount_inventory() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("one"), b"shared inode")?;
        fs::hard_link(root.path().join("one"), root.path().join("two"))?;
        assert!(publication_tree_inventory_v3(root.path()).is_err());

        let canonical = Path::new("/srv/publication-v3");
        let mountinfo = b"41 24 0:38 / /srv/publication-v3/nested rw - tmpfs tmpfs rw\n";
        assert!(reject_publication_mounts_in_v3(canonical, mountinfo).is_err());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_inventory_rejects_late_file_and_directory_substitution() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root_sandbox = tempfile::tempdir()?;
        let root_path = root_sandbox.path().join("publication");
        let moved_root = root_sandbox.path().join("reviewed-publication");
        fs::create_dir(&root_path)?;
        fs::write(root_path.join("payload"), b"reviewed")?;
        let root_fd = open_publication_root_v3(&root_path)?;
        assert!(
            publication_tree_inventory_v3_from_fd_with(&root_path, &root_fd, || {
                fs::rename(&root_path, &moved_root).unwrap();
                symlink(&moved_root, &root_path).unwrap();
            })
            .is_err(),
            "PublicationV3 accepted a root basename symlink to the exact reviewed inode"
        );
        fs::remove_file(&root_path)?;
        fs::rename(&moved_root, &root_path)?;

        let file_root = tempfile::tempdir()?;
        fs::write(file_root.path().join("payload"), b"reviewed")?;
        let file_root_fd = open_publication_root_v3(file_root.path())?;
        assert!(
            publication_tree_inventory_v3_from_fd_with(file_root.path(), &file_root_fd, || {
                fs::rename(
                    file_root.path().join("payload"),
                    file_root.path().join("reviewed-payload"),
                )
                .unwrap();
                symlink("reviewed-payload", file_root.path().join("payload")).unwrap();
            },)
            .is_err()
        );

        let directory_root = tempfile::tempdir()?;
        fs::create_dir(directory_root.path().join("catalog"))?;
        fs::write(directory_root.path().join("catalog/item"), b"reviewed")?;
        let directory_root_fd = open_publication_root_v3(directory_root.path())?;
        assert!(
            publication_tree_inventory_v3_from_fd_with(
                directory_root.path(),
                &directory_root_fd,
                || {
                    fs::rename(
                        directory_root.path().join("catalog"),
                        directory_root.path().join("reviewed-catalog"),
                    )
                    .unwrap();
                    fs::create_dir(directory_root.path().join("catalog")).unwrap();
                    fs::write(directory_root.path().join("catalog/item"), b"reviewed").unwrap();
                },
            )
            .is_err()
        );

        let inode_root = tempfile::tempdir()?;
        fs::write(inode_root.path().join("payload"), b"reviewed")?;
        let inode_root_fd = open_publication_root_v3(inode_root.path())?;
        assert!(
            publication_tree_inventory_v3_from_fd_with(inode_root.path(), &inode_root_fd, || {
                fs::rename(
                    inode_root.path().join("payload"),
                    inode_root.path().join("reviewed-payload"),
                )
                .unwrap();
                fs::write(inode_root.path().join("payload"), b"reviewed").unwrap();
            },)
            .is_err()
        );

        let mode_root = tempfile::tempdir()?;
        fs::create_dir(mode_root.path().join("catalog"))?;
        fs::write(mode_root.path().join("catalog/item"), b"reviewed")?;
        let mode_root_fd = open_publication_root_v3(mode_root.path())?;
        assert!(
            publication_tree_inventory_v3_from_fd_with(mode_root.path(), &mode_root_fd, || {
                fs::set_permissions(
                    mode_root.path().join("catalog"),
                    fs::Permissions::from_mode(0o700),
                )
                .unwrap();
            })
            .is_err()
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn retained_authority_rejects_root_substitution_after_validation() -> Result<()> {
        use std::os::unix::fs::symlink;

        let sandbox = tempfile::tempdir()?;
        let output = sandbox.path().join("publication");
        let staging = create_pinned_publication_staging_v3(&output)?;
        fs::write(staging.path().join("payload"), b"reviewed")?;
        let candidate = synthetic_validated_staging_v3(&staging)?;
        let authentic = sandbox.path().join("authentic-stage");
        fs::rename(staging.path(), &authentic)?;
        symlink(&authentic, staging.path())?;

        ensure!(
            validate_transition_with(&PublicationTransitionV3::Fresh, &candidate, || {}).is_err(),
            "transition accepted a retained candidate whose named root became a symlink"
        );
        fs::remove_file(staging.path())?;
        fs::rename(&authentic, staging.path())?;
        discard_failed_publication_staging(staging)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_file_hash_rejects_content_mutation_between_passes() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir()?;
        let path = root.path().join("payload");
        fs::write(&path, b"reviewed")?;
        let mut file = fs::File::open(&path)?;
        let identity = publication_node_identity_v3(&file.metadata()?);
        assert!(
            stable_publication_file_artifact_v3_with(&mut file, &identity, "payload", || {
                fs::write(&path, b"substituted").unwrap()
            },)
            .is_err()
        );

        let mode_path = root.path().join("mode-payload");
        fs::write(&mode_path, b"reviewed")?;
        fs::set_permissions(&mode_path, fs::Permissions::from_mode(0o644))?;
        let mut mode_file = fs::File::open(&mode_path)?;
        let mode_identity = publication_node_identity_v3(&mode_file.metadata()?);
        assert!(
            stable_publication_file_artifact_v3_with(
                &mut mode_file,
                &mode_identity,
                "mode-payload",
                || {
                    fs::set_permissions(&mode_path, fs::Permissions::from_mode(0o600)).unwrap();
                },
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn publication_lock_v3_rejects_v2_schema() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("payload"), b"locked")?;
        let mut lock = publication_lock_from_actual_for_test(
            root.path(),
            Digest32::digest_bytes(b"manifest"),
        )?;
        lock.schema_version = 2;
        assert!(lock.validate().is_err());
        Ok(())
    }

    #[test]
    fn official_campaign_offer_requires_exact_full_completion_and_no_other_policy() -> Result<()> {
        let full_catalog = Digest32::digest_bytes(b"full-campaign-catalog");
        let mission_only = [RulesetBoardScopeV1::IndividualLevel];
        let full_board = [
            RulesetBoardScopeV1::IndividualLevel,
            RulesetBoardScopeV1::FullCampaign,
        ];
        let not_offered = CampaignCompletionPolicyRequirementV1::NotOffered;
        let exact = CampaignCompletionPolicyRequirementV1::Required(
            official_full_campaign_completion_policy_v1(),
        );

        for edition in [
            OfficialContentEditionV1::Demo,
            OfficialContentEditionV1::Full,
        ] {
            validate_official_campaign_offer_fields(
                edition,
                &mission_only,
                &[],
                &not_offered,
                full_catalog,
            )?;
            ensure!(
                validate_official_campaign_offer_fields(
                    edition,
                    &mission_only,
                    &[],
                    &exact,
                    full_catalog,
                )
                .is_err(),
                "a non-FullCampaign board accepted a completion policy"
            );
        }

        validate_official_campaign_offer_fields(
            OfficialContentEditionV1::Full,
            &full_board,
            &[full_catalog],
            &exact,
            full_catalog,
        )?;
        ensure!(
            validate_official_campaign_offer_fields(
                OfficialContentEditionV1::Demo,
                &full_board,
                &[full_catalog],
                &exact,
                full_catalog,
            )
            .is_err(),
            "Demo advertised a FullCampaign board"
        );
        ensure!(
            validate_official_campaign_offer_fields(
                OfficialContentEditionV1::Full,
                &full_board,
                &[full_catalog],
                &not_offered,
                full_catalog,
            )
            .is_err(),
            "FullCampaign omitted its completion policy"
        );
        let wrong_percent = CampaignCompletionPolicyRequirementV1::Required(
            robin_run_protocol::CampaignCompletionPolicyV1 {
                required_progression_percent: 99,
                ..official_full_campaign_completion_policy_v1()
            },
        );
        ensure!(
            validate_official_campaign_offer_fields(
                OfficialContentEditionV1::Full,
                &full_board,
                &[full_catalog],
                &wrong_percent,
                full_catalog,
            )
            .is_err(),
            "FullCampaign accepted less than 100% progression"
        );
        let wrong_terminal = CampaignCompletionPolicyRequirementV1::Required(
            robin_run_protocol::CampaignCompletionPolicyV1 {
                terminal_subject: robin_run_protocol::OfficialContentSubjectV1::FieldMission {
                    mission_id: "H10_Yor_VL".into(),
                },
                required_progression_percent: 100,
            },
        );
        ensure!(
            validate_official_campaign_offer_fields(
                OfficialContentEditionV1::Full,
                &full_board,
                &[full_catalog],
                &wrong_terminal,
                full_catalog,
            )
            .is_err(),
            "FullCampaign accepted a terminal subject other than H12_Not_MP"
        );
        ensure!(
            validate_official_campaign_offer_fields(
                OfficialContentEditionV1::Full,
                &full_board,
                &[Digest32::digest_bytes(b"substituted-catalog")],
                &exact,
                full_catalog,
            )
            .is_err(),
            "FullCampaign accepted a substituted campaign catalog"
        );
        Ok(())
    }

    #[test]
    fn addressed_document_loader_rejects_extra_and_substitution() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ranked_simulation_policy: robin_run_protocol::RankedSimulationPolicyV1::standard(
                robin_run_protocol::RankedSimulationDifficultyV1::Medium,
            ),
            sim_config: serde_json::from_value(serde_json::to_value(
                robin_engine::engine::SimConfig::default(),
            )?)?,
            rules: BTreeMap::from([(
                "ranked".into(),
                robin_run_protocol::CanonicalValue::Bool(true),
            )]),
        };
        config.validate()?;
        let digest = config.canonical_digest()?;
        fs::write(
            root.path().join(format!("{digest}.json")),
            canonical_json_bytes(&config)?,
        )?;
        let loaded: BTreeMap<Digest32, RulesConfigIdentityV1> =
            load_addressed_documents(root.path(), &[digest])?;
        assert_eq!(loaded.get(&digest), Some(&config));

        fs::write(root.path().join("extra.json"), b"{}")?;
        assert!(load_addressed_documents::<RulesConfigIdentityV1>(root.path(), &[digest]).is_err());
        fs::remove_file(root.path().join("extra.json"))?;
        fs::write(root.path().join(format!("{digest}.json")), b"{}")?;
        assert!(load_addressed_documents::<RulesConfigIdentityV1>(root.path(), &[digest]).is_err());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_copies_one_retained_authority_and_validates_exactly() -> Result<()>
    {
        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        )?;
        assert!(matches!(
            persist_publication_staging(&staging, &candidate, &output)?,
            PublicationPersistenceOutcome::Published
        ));
        let receipt = validate_cloudflare_publication_materialization_v1(&output, receipt_sha256)?;
        ensure!(
            receipt.schema_version == 1
                && receipt.publication_schema_version == 3
                && receipt.publication_lock_sha256 == provenance.publication_lock_sha256
                && receipt.origins.len() == 3
                && receipt.output_inventory.files.iter().any(|file| {
                    file.path == "cloudflare-public/_headers" && file.unix_mode == 0o444
                }),
            "materialization receipt omitted its exact source/output authority"
        );
        use std::os::unix::fs::PermissionsExt as _;
        ensure!(
            fs::metadata(&output)?.permissions().mode() & 0o777 == 0o555
                && fs::metadata(output.join("cloudflare-public/index.html"))?
                    .permissions()
                    .mode()
                    & 0o777
                    == 0o444,
            "materialization output modes are not canonical"
        );
        make_test_tree_writable(&output)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_resolves_one_exact_git_commit_and_tree_from_pinned_root()
    -> Result<()> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .context("manifest tool is not below the repository root")?;
        let repository = open_publication_root_v3(repository)?;
        let (commit, tree) = resolve_cloudflare_materialization_git_authority_v1(&repository)?;
        ensure!(
            valid_lower_hex(&commit, 40) && valid_lower_hex(&tree, 40) && commit != tree,
            "pinned Git authority did not produce one commit/tree pair"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_git_tree_ignores_replacement_objects() -> Result<()> {
        let sandbox = tempfile::tempdir()?;
        let repository = sandbox.path().join("repository");
        fs::create_dir(&repository)?;
        let git = |arguments: &[&str]| -> Result<String> {
            let output = std::process::Command::new("/usr/bin/git")
                .args(arguments)
                .current_dir(&repository)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()?;
            ensure!(
                output.status.success(),
                "synthetic Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(String::from_utf8(output.stdout)?.trim().to_owned())
        };
        git(&["init", "--quiet"])?;
        git(&["config", "user.name", "Publication V3 test"])?;
        git(&["config", "user.email", "publication-v3@example.invalid"])?;
        fs::write(repository.join("tracked"), b"reviewed tree")?;
        git(&["add", "tracked"])?;
        git(&["commit", "--quiet", "-m", "reviewed"])?;
        let reviewed_commit = git(&["rev-parse", "HEAD^{commit}"])?;
        let reviewed_tree = git(&[
            "--no-replace-objects",
            "rev-parse",
            &format!("{reviewed_commit}^{{tree}}"),
        ])?;
        fs::write(repository.join("tracked"), b"attacker tree")?;
        git(&["add", "tracked"])?;
        git(&["commit", "--quiet", "-m", "attacker"])?;
        let attacker_commit = git(&["rev-parse", "HEAD^{commit}"])?;
        let attacker_tree = git(&[
            "--no-replace-objects",
            "rev-parse",
            &format!("{attacker_commit}^{{tree}}"),
        ])?;
        git(&[
            "--no-replace-objects",
            "checkout",
            "--quiet",
            "--detach",
            &reviewed_commit,
        ])?;
        git(&["replace", &reviewed_commit, &attacker_commit])?;

        let repository_fd = open_publication_root_v3(&repository)?;
        let (resolved_commit, resolved_tree) =
            resolve_cloudflare_materialization_git_authority_v1(&repository_fd)?;
        ensure!(
            resolved_commit == reviewed_commit
                && resolved_tree == reviewed_tree
                && resolved_tree != attacker_tree,
            "Git replacement object substituted Cloudflare source-tree provenance"
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_rejects_late_source_file_directory_and_root_swaps() -> Result<()>
    {
        for attack in ["file", "directory", "root"] {
            let sandbox = tempfile::tempdir()?;
            let source = sandbox.path().join("publication");
            fs::create_dir(&source)?;
            let (mut publication, provenance, origins) =
                synthetic_cloudflare_materialization_authority_v1(&source)?;
            let output = sandbox.path().join("materialized");
            let staging = create_pinned_publication_staging_v3(&output)?;
            let original = sandbox.path().join(format!("original-{attack}"));
            let result = populate_cloudflare_materialization_staging_v1(
                &staging,
                &mut publication,
                &provenance,
                &origins,
                || match attack {
                    "file" => {
                        let path = source.join("cloudflare-public/index.html");
                        fs::rename(&path, &original).unwrap();
                        fs::write(path, b"public index").unwrap();
                    }
                    "directory" => {
                        let path = source.join("cloudflare-public");
                        fs::rename(&path, &original).unwrap();
                        fs::create_dir(&path).unwrap();
                        fs::write(path.join("_headers"), b"public headers").unwrap();
                        fs::write(path.join("index.html"), b"public index").unwrap();
                    }
                    "root" => {
                        fs::rename(&source, &original).unwrap();
                        std::os::unix::fs::symlink(&original, &source).unwrap();
                    }
                    _ => unreachable!(),
                },
            );
            ensure!(
                result.is_err(),
                "Cloudflare materialization accepted late {attack} substitution"
            );
            discard_failed_publication_staging(staging)?;
            if attack == "root" {
                fs::remove_file(&source)?;
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_rejects_receipt_mismatch_extras_missing_and_private_leakage()
    -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        for attack in ["extra", "empty", "missing", "private", "mode", "inventory"] {
            let sandbox = tempfile::tempdir()?;
            let source = sandbox.path().join("publication");
            fs::create_dir(&source)?;
            let (mut publication, provenance, origins) =
                synthetic_cloudflare_materialization_authority_v1(&source)?;
            let output = sandbox.path().join("materialized");
            let staging = create_pinned_publication_staging_v3(&output)?;
            let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
                &staging,
                &mut publication,
                &provenance,
                &origins,
                || {},
            )?;
            assert!(matches!(
                persist_publication_staging(&staging, &candidate, &output)?,
                PublicationPersistenceOutcome::Published
            ));
            ensure!(
                validate_cloudflare_publication_materialization_v1(
                    &output,
                    Digest32::digest_bytes(b"wrong receipt")
                )
                .is_err(),
                "Cloudflare materialization accepted wrong receipt digest"
            );
            fs::set_permissions(&output, fs::Permissions::from_mode(0o755))?;
            match attack {
                "extra" => fs::write(output.join("extra"), b"injected")?,
                "empty" => fs::create_dir(output.join("unexpected-empty"))?,
                "missing" => {
                    fs::set_permissions(
                        output.join("cloudflare-public"),
                        fs::Permissions::from_mode(0o755),
                    )?;
                    fs::remove_file(output.join("cloudflare-public/index.html"))?;
                }
                "private" => {
                    fs::create_dir(output.join("private"))?;
                    fs::write(output.join("private/secret"), b"not deployable")?;
                }
                "mode" => fs::set_permissions(
                    output.join("cloudflare-public/index.html"),
                    fs::Permissions::from_mode(0o644),
                )?,
                "inventory" => {
                    fs::set_permissions(
                        output.join("inventories"),
                        fs::Permissions::from_mode(0o755),
                    )?;
                    let public = output.join("inventories/cloudflare-public-v1.json");
                    let signer = output.join("inventories/cloudflare-identity-signer-v1.json");
                    let temporary = output.join("inventories/swapped.json");
                    fs::rename(&public, &temporary)?;
                    fs::rename(&signer, &public)?;
                    fs::rename(&temporary, &signer)?;
                }
                _ => unreachable!(),
            }
            ensure!(
                validate_cloudflare_publication_materialization_v1(&output, receipt_sha256)
                    .is_err(),
                "Cloudflare materialization accepted {attack} topology attack"
            );
            make_test_tree_writable(&output)?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_rejects_self_consistent_untyped_origin_authority() -> Result<()> {
        for attack in ["private", "empty", "protocol"] {
            let sandbox = tempfile::tempdir()?;
            let source = sandbox.path().join("publication");
            fs::create_dir(&source)?;
            let (mut publication, provenance, origins) =
                synthetic_cloudflare_materialization_authority_v1(&source)?;
            let output = sandbox.path().join("materialized");
            let staging = create_pinned_publication_staging_v3(&output)?;
            let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
                &staging,
                &mut publication,
                &provenance,
                &origins,
                || {},
            )?;
            assert!(matches!(
                persist_publication_staging(&staging, &candidate, &output)?,
                PublicationPersistenceOutcome::Published
            ));
            let mut receipt: CloudflarePublicationMaterializationV1 = strict_json_from_slice(
                &fs::read(output.join(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH))?,
            )?;
            let public_binding = receipt
                .origins
                .iter_mut()
                .find(|binding| binding.origin == CloudflareMaterializationOriginV1::Public)
                .context("synthetic materialization omits public binding")?;
            let public_inventory_path = output.join(&public_binding.inventory_path);
            let mut public: CloudflareMaterializedOriginInventoryV1 =
                strict_json_from_slice(&fs::read(&public_inventory_path)?)?;
            make_test_tree_writable(&output)?;

            match attack {
                "private" => {
                    let bytes = b"self-authorized private leak";
                    fs::create_dir(output.join("cloudflare-public/private"))?;
                    fs::write(output.join("cloudflare-public/private/secret"), bytes)?;
                    let artifact = ArtifactRefV1 {
                        sha256: Digest32::digest_bytes(bytes),
                        byte_length: u64::try_from(bytes.len())?,
                        media_type: "application/octet-stream".into(),
                    };
                    public.files.push(CloudflareMaterializedFileV1 {
                        path: "private/secret".into(),
                        artifact: artifact.clone(),
                        unix_mode: 0o444,
                    });
                    public.directories.push(PublicationDirectoryV3 {
                        path: "private".into(),
                        unix_mode: 0o555,
                    });
                    receipt
                        .output_inventory
                        .files
                        .push(CloudflareMaterializedFileV1 {
                            path: "cloudflare-public/private/secret".into(),
                            artifact,
                            unix_mode: 0o444,
                        });
                    receipt
                        .output_inventory
                        .directories
                        .push(PublicationDirectoryV3 {
                            path: "cloudflare-public/private".into(),
                            unix_mode: 0o555,
                        });
                }
                "empty" => {
                    fs::create_dir(output.join("cloudflare-public/unexpected-empty"))?;
                    public.directories.push(PublicationDirectoryV3 {
                        path: "unexpected-empty".into(),
                        unix_mode: 0o555,
                    });
                    receipt
                        .output_inventory
                        .directories
                        .push(PublicationDirectoryV3 {
                            path: "cloudflare-public/unexpected-empty".into(),
                            unix_mode: 0o555,
                        });
                }
                "protocol" => {
                    let build_file = public
                        .files
                        .iter()
                        .find(|file| {
                            file.path.starts_with("manifests/builds/")
                                && file.path.ends_with(".json")
                        })
                        .context("synthetic public inventory omits BuildManifestV2")?
                        .clone();
                    let old_digest = build_file
                        .path
                        .strip_prefix("manifests/builds/")
                        .and_then(|name| name.strip_suffix(".json"))
                        .context("synthetic BuildManifestV2 path is malformed")?
                        .to_owned();
                    let old_build_path = output.join("cloudflare-public").join(&build_file.path);
                    let mut build: BuildManifestV2 =
                        strict_json_from_slice(&fs::read(&old_build_path)?)?;
                    build.network_protocol_version = build
                        .network_protocol_version
                        .checked_add(1)
                        .context("synthetic network protocol overflow")?;
                    build.validate()?;
                    let build_bytes = canonical_json_bytes(&build)?;
                    let new_digest = Digest32::digest_bytes(&build_bytes).to_string();
                    let new_build_relative = format!("manifests/builds/{new_digest}.json");
                    let new_build_path = output.join("cloudflare-public").join(&new_build_relative);
                    fs::rename(&old_build_path, &new_build_path)?;
                    fs::write(&new_build_path, &build_bytes)?;
                    fs::rename(
                        output.join(format!("cloudflare-public/builds/{old_digest}")),
                        output.join(format!("cloudflare-public/builds/{new_digest}")),
                    )?;
                    let rewrite = |path: &mut String| {
                        if *path == build_file.path {
                            *path = new_build_relative.clone();
                        } else if *path == format!("builds/{old_digest}") {
                            *path = format!("builds/{new_digest}");
                        } else if path.starts_with(&format!("builds/{old_digest}/")) {
                            *path = path.replacen(
                                &format!("builds/{old_digest}/"),
                                &format!("builds/{new_digest}/"),
                                1,
                            );
                        } else if *path == format!("cloudflare-public/builds/{old_digest}") {
                            *path = format!("cloudflare-public/builds/{new_digest}");
                        } else if path
                            .starts_with(&format!("cloudflare-public/builds/{old_digest}/"))
                        {
                            *path = path.replacen(
                                &format!("cloudflare-public/builds/{old_digest}/"),
                                &format!("cloudflare-public/builds/{new_digest}/"),
                                1,
                            );
                        } else if *path
                            == format!("cloudflare-public/manifests/builds/{old_digest}.json")
                        {
                            *path = format!("cloudflare-public/manifests/builds/{new_digest}.json");
                        }
                    };
                    for file in &mut public.files {
                        rewrite(&mut file.path);
                        if file.path == new_build_relative {
                            file.artifact.sha256 = Digest32::digest_bytes(&build_bytes);
                            file.artifact.byte_length = u64::try_from(build_bytes.len())?;
                        }
                    }
                    for directory in &mut public.directories {
                        rewrite(&mut directory.path);
                    }
                    for file in &mut receipt.output_inventory.files {
                        rewrite(&mut file.path);
                        if file.path
                            == format!("cloudflare-public/manifests/builds/{new_digest}.json")
                        {
                            file.artifact.sha256 = Digest32::digest_bytes(&build_bytes);
                            file.artifact.byte_length = u64::try_from(build_bytes.len())?;
                        }
                    }
                    for directory in &mut receipt.output_inventory.directories {
                        rewrite(&mut directory.path);
                    }
                }
                _ => unreachable!(),
            }
            public
                .files
                .sort_by(|left, right| left.path.cmp(&right.path));
            public
                .directories
                .sort_by(|left, right| left.path.cmp(&right.path));
            receipt
                .output_inventory
                .files
                .sort_by(|left, right| left.path.cmp(&right.path));
            receipt
                .output_inventory
                .directories
                .sort_by(|left, right| left.path.cmp(&right.path));
            let public_bytes = canonical_json_bytes(&public)?;
            fs::write(&public_inventory_path, &public_bytes)?;
            public_binding.inventory = ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&public_bytes),
                byte_length: u64::try_from(public_bytes.len())?,
                media_type: "application/json".into(),
            };
            let output_public_inventory = receipt
                .output_inventory
                .files
                .iter_mut()
                .find(|file| file.path == public_binding.inventory_path)
                .context("receipt output inventory omits public inventory")?;
            output_public_inventory.artifact.sha256 = public_binding.inventory.sha256;
            output_public_inventory.artifact.byte_length = public_binding.inventory.byte_length;
            receipt.validate()?;
            let receipt_bytes = canonical_json_bytes(&receipt)?;
            let malicious_receipt_sha256 = Digest32::digest_bytes(&receipt_bytes);
            fs::write(
                output.join(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH),
                &receipt_bytes,
            )?;
            fs::write(
                output.join(CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH),
                malicious_receipt_sha256.to_string(),
            )?;
            let output_root = open_publication_root_v3(&output)?;
            let mut expected =
                expected_topology_from_materialized_inventory_v1(&receipt.output_inventory)?;
            expected
                .register_canonical(CLOUDFLARE_MATERIALIZATION_RECEIPT_PATH.into(), &receipt)?;
            expected.register_bytes(
                CLOUDFLARE_MATERIALIZATION_RECEIPT_SIDECAR_PATH.into(),
                malicious_receipt_sha256.to_string().as_bytes(),
            )?;
            expected.seal_and_validate(&output, &output_root)?;
            ensure!(
                receipt_sha256 != malicious_receipt_sha256
                    && validate_cloudflare_publication_materialization_v1(
                        &output,
                        malicious_receipt_sha256,
                    )
                    .is_err(),
                "standalone validation accepted self-consistent {attack} origin authority"
            );
            make_test_tree_writable(&output)?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_rejects_late_identical_output_inode_swaps() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        for attack in ["file", "directory", "root"] {
            let sandbox = tempfile::tempdir()?;
            let source = sandbox.path().join("publication");
            fs::create_dir(&source)?;
            let (mut publication, provenance, origins) =
                synthetic_cloudflare_materialization_authority_v1(&source)?;
            let output = sandbox.path().join("materialized");
            let staging = create_pinned_publication_staging_v3(&output)?;
            let orphan = sandbox.path().join(format!("orphan-{attack}"));
            let staging_path = staging.path().to_path_buf();
            let result = populate_cloudflare_materialization_staging_v1(
                &staging,
                &mut publication,
                &provenance,
                &origins,
                || match attack {
                    "file" => {
                        let parent = staging_path.join("cloudflare-public");
                        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
                        let path = parent.join("index.html");
                        fs::rename(&path, &orphan).unwrap();
                        fs::write(&path, b"public index").unwrap();
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
                        fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
                    }
                    "directory" => {
                        fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o755))
                            .unwrap();
                        let path = staging_path.join("cloudflare-public");
                        let displaced = staging_path.join("displaced-public");
                        fs::rename(&path, &displaced).unwrap();
                        copy_directory_exact_preserving_modes(&displaced, &path).unwrap();
                        make_test_tree_writable(&displaced).unwrap();
                        fs::remove_dir_all(&displaced).unwrap();
                        fs::set_permissions(&staging_path, fs::Permissions::from_mode(0o555))
                            .unwrap();
                    }
                    "root" => {
                        fs::rename(&staging_path, &orphan).unwrap();
                        std::os::unix::fs::symlink(&orphan, &staging_path).unwrap();
                    }
                    _ => unreachable!(),
                },
            );
            ensure!(
                result.is_err(),
                "Cloudflare materialization accepted late identical {attack} output inode substitution"
            );
            if attack == "root" {
                fs::remove_file(&staging_path)?;
                fs::rename(&orphan, &staging_path)?;
            }
            discard_failed_publication_staging(staging)?;
            if orphan.exists() {
                make_test_tree_writable(&orphan).ok();
                if orphan.is_dir() {
                    fs::remove_dir_all(orphan)?;
                } else {
                    fs::remove_file(orphan)?;
                }
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_pre_persist_failures_cleanup_or_preserve_exact_evidence()
    -> Result<()> {
        for attack in ["source", "candidate"] {
            let sandbox = tempfile::tempdir()?;
            let source = sandbox.path().join("publication");
            fs::create_dir(&source)?;
            let (mut publication, provenance, origins) =
                synthetic_cloudflare_materialization_authority_v1(&source)?;
            let output = sandbox.path().join("materialized");
            let staging = create_pinned_publication_staging_v3(&output)?;
            let staging_path = staging.path().to_path_buf();
            let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
                &staging,
                &mut publication,
                &provenance,
                &origins,
                || {},
            )?;
            let displaced = sandbox.path().join(format!("displaced-{attack}"));
            match attack {
                "source" => {
                    let path = source.join("cloudflare-public/index.html");
                    fs::rename(&path, &displaced)?;
                    fs::write(path, b"public index")?;
                }
                "candidate" => {
                    fs::rename(&staging_path, &displaced)?;
                    std::os::unix::fs::symlink(&displaced, &staging_path)?;
                }
                _ => unreachable!(),
            }
            ensure!(
                persist_cloudflare_materialization_v1(
                    staging,
                    &publication,
                    &candidate,
                    &output,
                    receipt_sha256,
                )
                .is_err(),
                "Cloudflare materialization accepted a late pre-persist {attack} substitution"
            );
            ensure!(
                !output.exists(),
                "Cloudflare materialization published after a late pre-persist {attack} substitution"
            );
            if attack == "source" {
                ensure!(
                    !staging_path.exists(),
                    "ordinary pre-persist source failure left staging behind"
                );
            } else {
                ensure!(
                    staging_path.is_symlink() && displaced.is_dir(),
                    "uncertain pre-persist candidate identity did not preserve exact evidence"
                );
                fs::remove_file(&staging_path)?;
                make_test_tree_writable(&displaced)?;
                fs::remove_dir_all(&displaced)?;
            }
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cloudflare_materialization_noreplace_and_sync_outcomes_reuse_exact_candidate() -> Result<()>
    {
        for sync_failure in [false, true] {
            let sandbox = tempfile::tempdir()?;
            let source = sandbox.path().join("publication");
            fs::create_dir(&source)?;
            let (mut publication, provenance, origins) =
                synthetic_cloudflare_materialization_authority_v1(&source)?;
            let output = sandbox.path().join("materialized");
            let staging = create_pinned_publication_staging_v3(&output)?;
            let (receipt_sha256, candidate) = populate_cloudflare_materialization_staging_v1(
                &staging,
                &mut publication,
                &provenance,
                &origins,
                || {},
            )?;
            let outcome = persist_publication_staging_with(
                &staging,
                &candidate,
                &output,
                || {},
                |parent, source, destination| {
                    use std::os::fd::AsFd as _;
                    rustix::fs::renameat_with(
                        parent.as_fd(),
                        source,
                        parent.as_fd(),
                        destination,
                        rustix::fs::RenameFlags::NOREPLACE,
                    )?;
                    Ok(())
                },
                |parent| {
                    if sync_failure {
                        anyhow::bail!("injected Cloudflare materialization fsync failure")
                    }
                    parent.sync_all()?;
                    Ok(())
                },
            )?;
            ensure!(
                matches!(
                    outcome,
                    PublicationPersistenceOutcome::PublishedButParentSyncFailed(_)
                ) == sync_failure,
                "Cloudflare materialization parent-sync outcome was misclassified"
            );
            validate_cloudflare_publication_materialization_v1(&output, receipt_sha256)?;
            make_test_tree_writable(&output)?;
        }

        let sandbox = tempfile::tempdir()?;
        let source = sandbox.path().join("publication");
        fs::create_dir(&source)?;
        let (mut publication, provenance, origins) =
            synthetic_cloudflare_materialization_authority_v1(&source)?;
        let output = sandbox.path().join("materialized");
        let staging = create_pinned_publication_staging_v3(&output)?;
        let (_, candidate) = populate_cloudflare_materialization_staging_v1(
            &staging,
            &mut publication,
            &provenance,
            &origins,
            || {},
        )?;
        fs::create_dir(&output)?;
        ensure!(
            persist_publication_staging(&staging, &candidate, &output).is_err() && output.is_dir(),
            "Cloudflare materialization overwrote a pre-existing output"
        );
        discard_failed_publication_staging(staging)?;
        Ok(())
    }
}
