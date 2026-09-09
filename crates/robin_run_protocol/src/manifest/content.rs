//! Official content provenance, projection receipts and canonical simulation-component closure.

use super::{ArtifactRefV1, RulesConfigIdentityV1};
use crate::CanonicalDocument as _;
use crate::{
    BuildManifestV2, CanonicalValue, Digest32, OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2,
    OfficialProjectionAuthorityManifestV2, OfficialProjectionExporterPlatformV2, Validate,
    ValidationError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Legacy semantic role for source-file audit inventories. Ranked manifests
/// bind canonical simulation projections instead; source packaging bytes are
/// not portable across loose and shipping datadirs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentFileRoleV1 {
    DatadirIndex,
    Mission {
        mission_id: String,
        component: String,
    },
    EngineOverlay,
    Script,
    Locale {
        locale: String,
    },
    Other {
        name: String,
    },
}

impl Validate for ContentFileRoleV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::DatadirIndex | Self::EngineOverlay | Self::Script => Ok(()),
            Self::Mission {
                mission_id,
                component,
            } => {
                crate::validation::text("content.mission_id", mission_id, 256)?;
                crate::validation::text("content.mission_component", component, 128)
            }
            Self::Locale { locale } => crate::validation::text("content.locale", locale, 64),
            Self::Other { name } => crate::validation::text("content.role_name", name, 128),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentFileV1 {
    pub path: String,
    pub role: ContentFileRoleV1,
    pub artifact: ArtifactRefV1,
}

/// Operator-attested source edition for official deterministic content.
/// This is typed and signed into content identities; consumers must never
/// infer it from filenames, directory names, or human-facing labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialContentEditionV1 {
    Demo,
    Full,
}

/// Stable gameplay subject of one logical simulation-input closure.
/// Headquarters deliberately has no visit ordinal: repeated HQ sessions use
/// the same content identity, while the campaign chain separately binds each
/// occurrence's ordinal and state transition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OfficialContentSubjectV1 {
    FieldMission { mission_id: String },
    Headquarters { mission_id: String },
}

impl Validate for OfficialContentSubjectV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::FieldMission { mission_id } | Self::Headquarters { mission_id } => {
                crate::validation::text("content.subject.mission_id", mission_id, 256)
            }
        }
    }
}

impl OfficialContentSubjectV1 {
    pub fn mission_id(&self) -> &str {
        match self {
            Self::FieldMission { mission_id } | Self::Headquarters { mission_id } => mission_id,
        }
    }
}

pub const OFFICIAL_DEMO_FIELD_MISSION_IDS_V1: [&str; 1] = ["Dem_Lei_MP"];

pub const OFFICIAL_FULL_FIELD_MISSION_IDS_V1: [&str; 38] = [
    "Emb01_FoA_EC",
    "Emb02_FoC_MK",
    "Emb03_FoC_MP",
    "Emb04_FoA_MP",
    "Emb05_FoB_MP",
    "Emb06_FoC_EC",
    "Emb07_FoB_JMS",
    "Emb08_FoA_JMS",
    "Emb09_FoB_JMS",
    "EmbTut_FoC_EC",
    "H01_Lin_VL",
    "H02_Not_EC",
    "H03_Der_MK",
    "H04_Lei_VL",
    "H05_Lin_EC",
    "H07_Not_MK",
    "H09_Not_VL",
    "H10_Yor_VL",
    "H12_Not_MP",
    "S01_Not_VL",
    "S02_Lei_MP",
    "S03_FoB_MP",
    "S04_Der_EC",
    "S05_Yrk_EC",
    "SherwoodOutro",
    "Str01_Lin_EC",
    "Str02_Der_MP",
    "Str03_Yor_MK",
    "Tac01_FoA_MP",
    "Tac02_FoB_EC",
    "Tac03_FoC_MP",
    "Tac04_FoA_EC",
    "Tac05_FoC_MP",
    "Tac06_FoB_EC",
    "Tac17_FoC_EC",
    "Tac18_FoA_EC",
    "Tac19_FoB_EC",
    "Tac21_FoB_EC",
];

pub const OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1: &str = "Sherwood";

/// First mission selected by an authentic freshly-reset FULL campaign. The
/// original campaign controller launches this field mission directly when it
/// is the sole accessible mission and the campaign is not passing through HQ.
pub const OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1: &str = "H01_Lin_VL";

/// Exact edition-specific subject matrix found in the official mounted
/// sources. DEMO intentionally has no headquarters subject; substituting FULL
/// Sherwood inputs would create an unauthentic hybrid edition.
pub fn official_content_subjects_v1(
    edition: OfficialContentEditionV1,
) -> Vec<OfficialContentSubjectV1> {
    let field_ids: &[&str] = match edition {
        OfficialContentEditionV1::Demo => &OFFICIAL_DEMO_FIELD_MISSION_IDS_V1,
        OfficialContentEditionV1::Full => &OFFICIAL_FULL_FIELD_MISSION_IDS_V1,
    };
    let mut subjects = field_ids
        .iter()
        .map(|mission_id| OfficialContentSubjectV1::FieldMission {
            mission_id: (*mission_id).into(),
        })
        .collect::<Vec<_>>();
    if edition == OfficialContentEditionV1::Full {
        subjects.push(OfficialContentSubjectV1::Headquarters {
            mission_id: OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.into(),
        });
    }
    subjects
}

pub fn validate_official_content_subjects_v1(
    edition: OfficialContentEditionV1,
    subjects: &[OfficialContentSubjectV1],
) -> Result<(), ValidationError> {
    if subjects != official_content_subjects_v1(edition) {
        return Err(ValidationError::ClaimMismatch {
            field: "content.official_edition_subjects",
        });
    }
    Ok(())
}

pub fn official_content_manifest_name_v1(
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
) -> String {
    let edition_name = match edition {
        OfficialContentEditionV1::Demo => "DEMO",
        OfficialContentEditionV1::Full => "FULL",
    };
    match subject {
        OfficialContentSubjectV1::FieldMission { mission_id } => {
            format!("Official {edition_name} {mission_id}")
        }
        OfficialContentSubjectV1::Headquarters { mission_id } => {
            format!("Official {edition_name} Headquarters {mission_id}")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionSourceFormatV1 {
    LooseNativeV1,
    ShippingDatadirV10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionExporterIdentityV1 {
    pub exporter_version: u32,
    pub source_format: OfficialProjectionSourceFormatV1,
}

impl Validate for OfficialProjectionExporterIdentityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.exporter_version == 0 {
            return Err(ValidationError::Zero {
                field: "official_projection.exporter_version",
            });
        }
        Ok(())
    }
}

/// One exact physical file in a private approved source datadir. Zero-byte
/// files are legitimate source inventory entries, so this does not reuse the
/// nonempty downloadable-artifact contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialSourceFileV1 {
    pub path: String,
    pub sha256: Digest32,
    pub byte_length: u64,
}

impl Validate for OfficialSourceFileV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::canonical_relative_path("official_source_file.path", &self.path)?;
        if self.sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "official_source_file.sha256",
            });
        }
        Ok(())
    }
}

/// Private digest inventory for the exact raw mount from which an exporter
/// prepared simulation inputs. It must never become a public FULL download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialSourceTreeManifestV1 {
    pub schema_version: u32,
    pub edition: OfficialContentEditionV1,
    pub source_format: OfficialProjectionSourceFormatV1,
    pub files: Vec<OfficialSourceFileV1>,
}

impl Validate for OfficialSourceTreeManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("OfficialSourceTreeManifestV1", self.schema_version)?;
        if self.files.is_empty() || self.files.len() > 1_000_000 {
            return Err(ValidationError::CountOutOfRange {
                field: "official_source_tree.files",
            });
        }
        for file in &self.files {
            file.validate()?;
        }
        if !self
            .files
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "official_source_tree.files",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionSubjectReceiptV1 {
    pub content_manifest: ContentManifestV1,
}

/// Canonical proof that one supported exporter prepared the exact official
/// edition matrix from one inventoried raw mount. Native and shipping receipts
/// differ in source identity but must contain identical content manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialSimulationProjectionReceiptV1 {
    pub schema_version: u32,
    pub exporter: OfficialProjectionExporterIdentityV1,
    pub edition: OfficialContentEditionV1,
    pub source_tree_manifest_sha256: Digest32,
    pub source_file_count: u32,
    pub subjects: Vec<OfficialProjectionSubjectReceiptV1>,
}

impl Validate for OfficialSimulationProjectionReceiptV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("OfficialSimulationProjectionReceiptV1", self.schema_version)?;
        self.exporter.validate()?;
        if self.source_tree_manifest_sha256.is_zero() || self.source_file_count == 0 {
            return Err(ValidationError::Zero {
                field: "official_projection_receipt.source_tree",
            });
        }
        let subjects = self
            .subjects
            .iter()
            .map(|subject| subject.content_manifest.subject.clone())
            .collect::<Vec<_>>();
        validate_official_content_subjects_v1(self.edition, &subjects)?;
        for subject in &self.subjects {
            subject.content_manifest.validate()?;
            if subject.content_manifest.edition != self.edition
                || subject.content_manifest.projection_schema_version
                    != OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1
                || subject.content_manifest.name
                    != official_content_manifest_name_v1(
                        self.edition,
                        &subject.content_manifest.subject,
                    )
                || !matches!(
                    subject.content_manifest.speech_timing,
                    SimulationSpeechTimingSourceV1::BaseInstallation
                        | SimulationSpeechTimingSourceV1::CoreAudioDurationsV1
                )
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "official_projection_receipt.content_manifest",
                });
            }
        }
        Ok(())
    }
}

pub const OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2: u32 = 2;
pub const OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2: u32 = 2;
pub const OFFICIAL_PROJECTION_EXPORTER_VERSION_V2: u32 = 2;
pub const OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1: u32 = 2;

/// Exact physical-source selection policy used to author an official
/// simulation projection. The policy version belongs to each variant; adding
/// another loader closure must add another variant rather than broadening one
/// of these definitions after receipts have been signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialSourceClosureKindV2 {
    LooseNativeSimulationConsumedV1,
    ShippingDatadirV10ArchiveAndReferencedSplitsV1,
}

impl OfficialSourceClosureKindV2 {
    pub fn source_format(self) -> OfficialProjectionSourceFormatV1 {
        match self {
            Self::LooseNativeSimulationConsumedV1 => {
                OfficialProjectionSourceFormatV1::LooseNativeV1
            }
            Self::ShippingDatadirV10ArchiveAndReferencedSplitsV1 => {
                OfficialProjectionSourceFormatV1::ShippingDatadirV10
            }
        }
    }
}

/// Exact private exporter/build identity carried by a V2 receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionExporterIdentityV2 {
    pub exporter_version: u32,
    pub simulation_content_projection_schema_version: u32,
    pub platform: OfficialProjectionExporterPlatformV2,
    pub source_format: OfficialProjectionSourceFormatV1,
    /// Digest of the private OfficialProjectionAuthorityManifestV2. This
    /// receipt is operator-private and must not be copied into public run
    /// proofs or the public content catalog.
    pub projection_authority_manifest_sha256: Digest32,
    pub exporter_artifact: ArtifactRefV1,
}

impl Validate for OfficialProjectionExporterIdentityV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.exporter_version != OFFICIAL_PROJECTION_EXPORTER_VERSION_V2
            || self.simulation_content_projection_schema_version
                != OFFICIAL_SIMULATION_CONTENT_PROJECTION_SCHEMA_VERSION_V1
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_v2.exporter_schema_policy",
            });
        }
        if self.projection_authority_manifest_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "official_projection_v2.projection_authority_manifest_sha256",
            });
        }
        self.exporter_artifact.validate()?;
        if self.exporter_artifact.media_type != OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2 {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_v2.exporter_artifact.media_type",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialBuiltInOverlayKindV2 {
    CoreDatadirV1,
}

fn validate_official_source_files_v2(
    field: &'static str,
    files: &[OfficialSourceFileV1],
) -> Result<(), ValidationError> {
    if files.is_empty() || files.len() > 1_000_000 {
        return Err(ValidationError::CountOutOfRange { field });
    }
    for file in files {
        file.validate()?;
    }
    if !files.windows(2).all(|pair| pair[0].path < pair[1].path) {
        return Err(ValidationError::NotCanonicalOrder { field });
    }
    let mut case_folded_paths = BTreeSet::new();
    for file in files {
        let folded = file.path.to_lowercase();
        if official_source_path_is_forbidden_v2(&folded) {
            return Err(ValidationError::ClaimMismatch {
                field: "official_source_tree_v2.forbidden_mutable_path",
            });
        }
        if !case_folded_paths.insert(folded.clone()) {
            return Err(ValidationError::Duplicate {
                field,
                value: folded,
            });
        }
    }
    Ok(())
}

fn official_source_path_is_forbidden_v2(folded_path: &str) -> bool {
    folded_path.split('/').any(|component| {
        matches!(
            component,
            "savegame" | "cache" | ".codex-tmp" | "campaign.bck"
        ) || component.ends_with(".log")
            || component.ends_with(".jsonl")
            || component.ends_with(".bak")
            || component.ends_with(".backup")
    })
}

/// Canonical private inventory of exactly the physical files selected by one
/// official source-closure policy. It is intentionally not a whole-tree hash:
/// savegames, logs, debugging output and unrelated presentation assets are
/// outside the simulation-consumed closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialSourceTreeManifestV2 {
    pub schema_version: u32,
    pub edition: OfficialContentEditionV1,
    pub source_format: OfficialProjectionSourceFormatV1,
    pub closure_kind: OfficialSourceClosureKindV2,
    pub files: Vec<OfficialSourceFileV1>,
}

impl Validate for OfficialSourceTreeManifestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "OfficialSourceTreeManifestV2",
            OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        if self.closure_kind.source_format() != self.source_format {
            return Err(ValidationError::ClaimMismatch {
                field: "official_source_tree_v2.closure_source_format",
            });
        }
        validate_official_source_files_v2("official_source_tree_v2.files", &self.files)?;
        match self.closure_kind {
            OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1 => {
                let locale = match self.edition {
                    OfficialContentEditionV1::Demo => "1033/data/",
                    OfficialContentEditionV1::Full => "2047/data/",
                };
                if self.files.iter().any(|file| {
                    let folded = file.path.to_ascii_lowercase();
                    folded == "data/datadir.bin"
                        || (!folded.starts_with("data/") && !folded.starts_with(locale))
                }) {
                    return Err(ValidationError::ClaimMismatch {
                        field: "official_source_tree_v2.loose_namespace",
                    });
                }
            }
            OfficialSourceClosureKindV2::ShippingDatadirV10ArchiveAndReferencedSplitsV1 => {
                if !self
                    .files
                    .iter()
                    .any(|file| file.path.eq_ignore_ascii_case("Data/datadir.bin"))
                    || self
                        .files
                        .iter()
                        .any(|file| !file.path.to_ascii_lowercase().starts_with("data/"))
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "official_source_tree_v2.shipping_archive_closure",
                    });
                }
            }
        }
        Ok(())
    }
}

/// Canonical inventory for the one built-in overlay permitted in official
/// projection mode. User, mod and environment overlays have no representation
/// in this closed type and must be rejected before loader initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialBuiltInOverlaySourceManifestV2 {
    pub schema_version: u32,
    pub kind: OfficialBuiltInOverlayKindV2,
    pub files: Vec<OfficialSourceFileV1>,
}

impl Validate for OfficialBuiltInOverlaySourceManifestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "OfficialBuiltInOverlaySourceManifestV2",
            OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        validate_official_source_files_v2("official_built_in_overlay_v2.files", &self.files)?;
        if self
            .files
            .iter()
            .any(|file| !file.path.to_ascii_lowercase().starts_with("data/"))
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_built_in_overlay_v2.namespace",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialBuiltInOverlayBindingV2 {
    pub kind: OfficialBuiltInOverlayKindV2,
    pub source_manifest_sha256: Digest32,
    pub source_file_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionDifficultyV1 {
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionCampaignPolicyV1 {
    FreshRepresentativeCampaignPerOfficialSubjectV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionLocalePolicyV1 {
    ExactReceiptLcidBeforeResourceInitializationV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionAudioDurationPolicyV1 {
    RebuildFromSourceClosureNoPersistentCacheV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionOverlayPolicyV1 {
    BuiltInCoreOnlyRejectUserModEnvironmentV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficialProjectionHostStatePolicyV1 {
    CanonicalInMemoryNoPersistedPreferencesIdentitySaveOrEnvironmentV1,
}

/// Canonical invocation authority for official projection authoring. This is
/// a typed body, not an opaque attestation: operator and exporter both inspect
/// the exact deterministic config and the closed policies which prevent local
/// preferences, locale order, caches or overlays from changing an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionExecutionPolicyV1 {
    pub schema_version: u32,
    pub policy_version: u32,
    pub difficulty: OfficialProjectionDifficultyV1,
    pub simulation_seed: crate::SimulationSeed64,
    pub rules_config: RulesConfigIdentityV1,
    pub campaign: OfficialProjectionCampaignPolicyV1,
    pub locale: OfficialProjectionLocalePolicyV1,
    pub audio_durations: OfficialProjectionAudioDurationPolicyV1,
    pub overlays: OfficialProjectionOverlayPolicyV1,
    pub host_state: OfficialProjectionHostStatePolicyV1,
}

impl Validate for OfficialProjectionExecutionPolicyV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("OfficialProjectionExecutionPolicyV1", self.schema_version)?;
        if self.policy_version != 1
            || self.rules_config.replay_schema_version
                != crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1
            || self.simulation_seed.get() != 0
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_execution_policy.version",
            });
        }
        self.rules_config.validate()?;
        if self.rules_config.sim_config.get("difficulty")
            != Some(&CanonicalValue::String("Medium".into()))
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_execution_policy.difficulty",
            });
        }
        Ok(())
    }
}

impl Validate for OfficialBuiltInOverlayBindingV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.source_manifest_sha256.is_zero() || self.source_file_count == 0 {
            return Err(ValidationError::Zero {
                field: "official_built_in_overlay_v2.source_manifest",
            });
        }
        Ok(())
    }
}

/// V2 official projection receipt. V1 whole-tree receipts remain decodeable
/// historical diagnostics but are not accepted by this authorization type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialSimulationProjectionReceiptV2 {
    pub schema_version: u32,
    pub exporter: OfficialProjectionExporterIdentityV2,
    pub edition: OfficialContentEditionV1,
    pub source_tree_manifest_sha256: Digest32,
    pub source_file_count: u32,
    pub built_in_overlay: OfficialBuiltInOverlayBindingV2,
    pub rules_config_sha256: Digest32,
    pub execution_policy_sha256: Digest32,
    pub execution_policy: OfficialProjectionExecutionPolicyV1,
    pub subjects: Vec<OfficialProjectionSubjectReceiptV1>,
}

impl Validate for OfficialSimulationProjectionReceiptV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "OfficialSimulationProjectionReceiptV2",
            OFFICIAL_PROJECTION_RECEIPT_SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.exporter.validate()?;
        self.built_in_overlay.validate()?;
        self.execution_policy.validate()?;
        if self.source_tree_manifest_sha256.is_zero()
            || self.source_file_count == 0
            || self.rules_config_sha256.is_zero()
            || self.execution_policy_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "official_projection_receipt_v2.authorities",
            });
        }
        if self
            .execution_policy
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.execution_policy_canonicalization",
            })?
            != self.execution_policy_sha256
            || self
                .execution_policy
                .rules_config
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "official_projection_receipt_v2.execution_rules_canonicalization",
                })?
                != self.rules_config_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.execution_policy",
            });
        }
        let subjects = self
            .subjects
            .iter()
            .map(|subject| subject.content_manifest.subject.clone())
            .collect::<Vec<_>>();
        validate_official_content_subjects_v1(self.edition, &subjects)?;
        let expected_resource_locale_root = match self.edition {
            OfficialContentEditionV1::Demo => "1033",
            OfficialContentEditionV1::Full => "2047",
        };
        for subject in &self.subjects {
            subject.content_manifest.validate()?;
            if subject.content_manifest.edition != self.edition
                || subject.content_manifest.projection_schema_version
                    != self.exporter.simulation_content_projection_schema_version
                || subject.content_manifest.name
                    != official_content_manifest_name_v1(
                        self.edition,
                        &subject.content_manifest.subject,
                    )
                || !matches!(
                    subject.content_manifest.speech_timing,
                    SimulationSpeechTimingSourceV1::BaseInstallation
                        | SimulationSpeechTimingSourceV1::CoreAudioDurationsV1
                )
                || subject.content_manifest.resource_locale_root.as_str()
                    != expected_resource_locale_root
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "official_projection_receipt_v2.content_manifest",
                });
            }
        }
        Ok(())
    }
}

impl OfficialSimulationProjectionReceiptV2 {
    /// Validate every canonical authority referenced by the receipt. Operator
    /// tooling and the exporter both use this method; merely copying digests
    /// into a receipt is never authorization.
    pub fn validate_against(
        &self,
        public_build: &BuildManifestV2,
        projection_authority: &OfficialProjectionAuthorityManifestV2,
        rules_config: &RulesConfigIdentityV1,
        source_tree: &OfficialSourceTreeManifestV2,
        built_in_overlay: &OfficialBuiltInOverlaySourceManifestV2,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        projection_authority.validate_against(public_build)?;
        rules_config.validate()?;
        source_tree.validate()?;
        built_in_overlay.validate()?;
        if projection_authority
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.projection_authority_canonicalization",
            })?
            != self.exporter.projection_authority_manifest_sha256
            || projection_authority.projection_exporter.exporter_version
                != self.exporter.exporter_version
            || projection_authority
                .projection_exporter
                .simulation_content_projection_schema_version
                != self.exporter.simulation_content_projection_schema_version
            || projection_authority.projection_exporter.platform != self.exporter.platform
            || projection_authority.projection_exporter.artifact != self.exporter.exporter_artifact
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.projection_authority",
            });
        }
        if rules_config
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.rules_config_canonicalization",
            })?
            != self.rules_config_sha256
            || rules_config != &self.execution_policy.rules_config
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.rules_config",
            });
        }
        if source_tree.edition != self.edition
            || source_tree.source_format != self.exporter.source_format
            || source_tree.files.len() != self.source_file_count as usize
            || source_tree
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "official_projection_receipt_v2.source_tree_canonicalization",
                })?
                != self.source_tree_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.source_tree",
            });
        }
        if built_in_overlay.kind != self.built_in_overlay.kind
            || built_in_overlay.files.len() != self.built_in_overlay.source_file_count as usize
            || built_in_overlay
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "official_projection_receipt_v2.overlay_canonicalization",
                })?
                != self.built_in_overlay.source_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.built_in_overlay",
            });
        }
        Ok(())
    }
}

/// Canonical stdout emitted by the private schema-2 projection exporter.
///
/// This report is deliberately protocol-owned and fully typed. Operator
/// tooling must parse it as this document, validate it against the six
/// supplied authorities, and additionally require the two output roots to be
/// the exact sandbox paths it provided. No opaque JSON claims are accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialProjectionExportReportV2 {
    pub schema_version: u32,
    pub edition: OfficialContentEditionV1,
    pub source_format: OfficialProjectionSourceFormatV1,
    pub catalog_root: String,
    pub receipt_root: String,
    pub source_tree_manifest_sha256: Digest32,
    pub projection_receipt_sha256: Digest32,
    pub public_build_manifest_sha256: Digest32,
    pub projection_authority_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub execution_policy_sha256: Digest32,
    pub core_overlay_manifest_sha256: Digest32,
    pub exporter_artifact: ArtifactRefV1,
    pub source_file_count: u32,
    pub core_overlay_file_count: u32,
    pub resource_locale_root: ResourceLocaleRootV1,
    pub subjects: Vec<OfficialContentSubjectV1>,
    pub component_file_count: u32,
}

pub(super) const OFFICIAL_SIMULATION_COMPONENT_FILE_COUNT_PER_SUBJECT_V1: usize = 8;

fn validate_report_absolute_path(field: &'static str, path: &str) -> Result<(), ValidationError> {
    crate::validation::text(field, path, 4096)?;
    if !path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path[1..]
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ValidationError::ClaimMismatch { field });
    }
    Ok(())
}

impl Validate for OfficialProjectionExportReportV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "OfficialProjectionExportReportV2",
            OFFICIAL_PROJECTION_EXPORT_REPORT_SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        validate_report_absolute_path(
            "official_projection_report.catalog_root",
            &self.catalog_root,
        )?;
        validate_report_absolute_path(
            "official_projection_report.receipt_root",
            &self.receipt_root,
        )?;
        self.exporter_artifact.validate()?;
        if self.exporter_artifact.media_type != OFFICIAL_PROJECTION_EXPORTER_MEDIA_TYPE_V2 {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_report.exporter_artifact.media_type",
            });
        }
        if self.source_tree_manifest_sha256.is_zero()
            || self.projection_receipt_sha256.is_zero()
            || self.public_build_manifest_sha256.is_zero()
            || self.projection_authority_manifest_sha256.is_zero()
            || self.rules_config_sha256.is_zero()
            || self.execution_policy_sha256.is_zero()
            || self.core_overlay_manifest_sha256.is_zero()
            || self.source_file_count == 0
            || self.core_overlay_file_count == 0
        {
            return Err(ValidationError::Zero {
                field: "official_projection_report.authorities",
            });
        }
        validate_official_content_subjects_v1(self.edition, &self.subjects)?;
        let expected_locale = match self.edition {
            OfficialContentEditionV1::Demo => "1033",
            OfficialContentEditionV1::Full => "2047",
        };
        if self.resource_locale_root.as_str() != expected_locale
            || usize::try_from(self.component_file_count).ok()
                != self
                    .subjects
                    .len()
                    .checked_mul(OFFICIAL_SIMULATION_COMPONENT_FILE_COUNT_PER_SUBJECT_V1)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_report.content_inventory",
            });
        }
        Ok(())
    }
}

impl OfficialProjectionExportReportV2 {
    /// Cross-bind the stdout report to the exact canonical receipt and every
    /// authority which authorized that receipt. This intentionally repeats
    /// the receipt's own cross-validation before comparing report claims.
    pub fn validate_against(
        &self,
        receipt: &OfficialSimulationProjectionReceiptV2,
        public_build: &BuildManifestV2,
        projection_authority: &OfficialProjectionAuthorityManifestV2,
        rules_config: &RulesConfigIdentityV1,
        source_tree: &OfficialSourceTreeManifestV2,
        built_in_overlay: &OfficialBuiltInOverlaySourceManifestV2,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        receipt.validate_against(
            public_build,
            projection_authority,
            rules_config,
            source_tree,
            built_in_overlay,
        )?;

        let receipt_subjects = receipt
            .subjects
            .iter()
            .map(|subject| subject.content_manifest.subject.clone())
            .collect::<Vec<_>>();
        let receipt_locale = &receipt
            .subjects
            .first()
            .ok_or(ValidationError::CountOutOfRange {
                field: "official_projection_report.receipt_subjects",
            })?
            .content_manifest
            .resource_locale_root;
        if self.edition != receipt.edition
            || self.source_format != receipt.exporter.source_format
            || self.catalog_root == self.receipt_root
            || self.source_tree_manifest_sha256 != receipt.source_tree_manifest_sha256
            || self.projection_receipt_sha256
                != receipt
                    .canonical_digest()
                    .map_err(|_| ValidationError::ClaimMismatch {
                        field: "official_projection_report.receipt_canonicalization",
                    })?
            || self.public_build_manifest_sha256
                != projection_authority.public_build_manifest_sha256
            || self.projection_authority_manifest_sha256
                != receipt.exporter.projection_authority_manifest_sha256
            || self.rules_config_sha256 != receipt.rules_config_sha256
            || self.execution_policy_sha256 != receipt.execution_policy_sha256
            || self.core_overlay_manifest_sha256 != receipt.built_in_overlay.source_manifest_sha256
            || self.exporter_artifact != receipt.exporter.exporter_artifact
            || self.source_file_count != receipt.source_file_count
            || self.core_overlay_file_count != receipt.built_in_overlay.source_file_count
            || &self.resource_locale_root != receipt_locale
            || self.subjects != receipt_subjects
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_report.receipt_bindings",
            });
        }
        Ok(())
    }
}

/// Require the authentic Demo/Full × loose/shipping four-lane authority set.
/// Every lane must use the same build, exporter executable, rules config and
/// built-in overlay, and the two physical representations of each edition
/// must publish byte-identical semantic content manifests.
pub fn validate_official_projection_receipt_matrix_v2(
    receipts: &[OfficialSimulationProjectionReceiptV2],
) -> Result<(), ValidationError> {
    if receipts.len() != 4 {
        return Err(ValidationError::CountOutOfRange {
            field: "official_projection_receipt_v2.matrix",
        });
    }
    for receipt in receipts {
        receipt.validate()?;
    }
    let authority = &receipts[0];
    for receipt in &receipts[1..] {
        if receipt.exporter.exporter_version != authority.exporter.exporter_version
            || receipt
                .exporter
                .simulation_content_projection_schema_version
                != authority
                    .exporter
                    .simulation_content_projection_schema_version
            || receipt.exporter.projection_authority_manifest_sha256
                != authority.exporter.projection_authority_manifest_sha256
            || receipt.exporter.platform != authority.exporter.platform
            || receipt.exporter.exporter_artifact != authority.exporter.exporter_artifact
            || receipt.built_in_overlay != authority.built_in_overlay
            || receipt.rules_config_sha256 != authority.rules_config_sha256
            || receipt.execution_policy_sha256 != authority.execution_policy_sha256
            || receipt.execution_policy != authority.execution_policy
        {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.matrix_authorities",
            });
        }
    }
    for edition in [
        OfficialContentEditionV1::Demo,
        OfficialContentEditionV1::Full,
    ] {
        let mut loose = None;
        let mut shipping = None;
        for receipt in receipts.iter().filter(|receipt| receipt.edition == edition) {
            let slot = match receipt.exporter.source_format {
                OfficialProjectionSourceFormatV1::LooseNativeV1 => &mut loose,
                OfficialProjectionSourceFormatV1::ShippingDatadirV10 => &mut shipping,
            };
            if slot.replace(receipt).is_some() {
                return Err(ValidationError::Duplicate {
                    field: "official_projection_receipt_v2.matrix_lane",
                    value: format!("{edition:?}/{:?}", receipt.exporter.source_format),
                });
            }
        }
        let (Some(loose), Some(shipping)) = (loose, shipping) else {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.matrix_lane",
            });
        };
        if loose.subjects != shipping.subjects {
            return Err(ValidationError::ClaimMismatch {
                field: "official_projection_receipt_v2.matrix_content",
            });
        }
    }
    Ok(())
}

/// Canonical safe projection path shared by exporters, verifier bundles and
/// operator tooling. Field mission IDs are lowercase hexadecimal UTF-8 so no
/// engine identifier can introduce a separator or platform-specific path.
/// Each edition has exactly one headquarters projection path; its exact engine
/// mission ID remains bound by the typed subject and content manifest.
pub fn simulation_content_component_relative_path_v1(
    subject: &OfficialContentSubjectV1,
    kind: SimulationContentComponentKindV1,
) -> Result<String, ValidationError> {
    subject.validate()?;
    let directory = match subject {
        OfficialContentSubjectV1::FieldMission { mission_id } => {
            let mut encoded = String::with_capacity(mission_id.len() * 2);
            for byte in mission_id.as_bytes() {
                use std::fmt::Write as _;
                write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
            }
            format!("field-missions/{encoded}")
        }
        OfficialContentSubjectV1::Headquarters { .. } => "headquarters".into(),
    };
    Ok(format!(
        "{directory}/{}",
        simulation_component_filename_v1(kind)
    ))
}

pub const fn simulation_component_filename_v1(
    kind: SimulationContentComponentKindV1,
) -> &'static str {
    match kind {
        SimulationContentComponentKindV1::Profiles => "profiles.bitcode",
        SimulationContentComponentKindV1::LoadedLevel => "loaded_level.bitcode",
        SimulationContentComponentKindV1::MissionScripts => "mission_scripts.bitcode",
        SimulationContentComponentKindV1::SpriteSimulationMetadata => {
            "sprite_simulation_metadata.bitcode"
        }
        SimulationContentComponentKindV1::MapGeometryMetadata => "map_geometry_metadata.bitcode",
        SimulationContentComponentKindV1::LocalizedDeterministicText => {
            "localized_deterministic_text.bitcode"
        }
        SimulationContentComponentKindV1::SoundDurationTables => "sound_duration_tables.bitcode",
        SimulationContentComponentKindV1::InterfaceSimulationMetadata => {
            "interface_simulation_metadata.bitcode"
        }
    }
}

/// The manifest names only canonical static projections capable of affecting
/// deterministic simulation. Presentation packages and transcoded sprite,
/// texture, music, or audio payload bytes are outside this identity when their
/// simulation-relevant derived metadata is bound by a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentClosureKindV1 {
    /// Static official-content portion of the prepared mission inputs. The
    /// ranked genesis separately binds campaign state, seed, rules and the
    /// speech authority; consumers seal the complete run-specific inputs at
    /// the engine boundary.
    StaticPreparedMissionContentProjection,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum SimulationContentComponentKindV1 {
    Profiles,
    LoadedLevel,
    MissionScripts,
    SpriteSimulationMetadata,
    MapGeometryMetadata,
    LocalizedDeterministicText,
    SoundDurationTables,
    InterfaceSimulationMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimulationSpeechTimingSourceV1 {
    CoreAudioDurationsV1,
    BaseInstallation,
    LanguagePack { canonical_locale: String },
}

/// Exact single-component locale directory selected below an approved raw
/// official-content root (for example `1033` or `2047`). This authority is
/// independent from speech timing: localized deterministic text resources
/// and speech-duration tables need not come from the same package policy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceLocaleRootV1(String);

impl ResourceLocaleRootV1 {
    pub fn new(component: impl Into<String>) -> Result<Self, ValidationError> {
        let value = Self(component.into());
        value.validate()?;
        Ok(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Validate for ResourceLocaleRootV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        let component = self.as_str();
        if component.is_empty()
            || component.len() > 8
            || component.starts_with('0')
            || !component.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "content.resource_locale_root",
            });
        }
        Ok(())
    }
}

impl Validate for SimulationSpeechTimingSourceV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Self::LanguagePack { canonical_locale } = self {
            crate::validation::text(
                "simulation_content.speech_timing.canonical_locale",
                canonical_locale,
                64,
            )?;
            if canonical_locale.starts_with('-')
                || canonical_locale.ends_with('-')
                || canonical_locale
                    .split('-')
                    .any(|part| part.is_empty() || part.len() > 8)
                || !canonical_locale
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(ValidationError::ClaimMismatch {
                    field: "simulation_content.speech_timing.canonical_locale",
                });
            }
        }
        Ok(())
    }
}

pub const SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1: &str =
    "application/vnd.robinhood.simulation-content-component-v2+bitcode";

/// One canonical component of the exact `PreparedMissionInputs` projection.
/// The referenced object is a canonical `SimulationContentComponentDocumentV1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationContentComponentV1 {
    pub kind: SimulationContentComponentKindV1,
    pub component_schema_version: u32,
    pub artifact: ArtifactRefV1,
}

impl Validate for SimulationContentComponentV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.component_schema_version == 0 {
            return Err(ValidationError::Zero {
                field: "simulation_content.component_schema_version",
            });
        }
        self.artifact.validate()?;
        if self.artifact.media_type != SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "simulation_content.component.media_type",
            });
        }
        Ok(())
    }
}

/// Canonical object produced from one typed field group of
/// `PreparedMissionInputs`. Projection code must encode every `f32`/`f64` as
/// its unsigned IEEE-754 bit pattern in this integer-only payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationContentComponentDocumentV1 {
    pub schema_version: u32,
    pub kind: SimulationContentComponentKindV1,
    pub component_schema_version: u32,
    pub payload: CanonicalValue,
}

// The semantic document stays inspectable through serde. Published bytes use
// native bitcode exclusively; the format marker rejects former JSON artifacts.
impl SimulationContentComponentDocumentV1 {
    pub fn bitcode_bytes(&self) -> Result<Vec<u8>, crate::bitcode_value::ProjectionBitcodeError> {
        self.validate()?;
        Ok(bitcode::encode(&(
            *b"RHSC0002",
            self.schema_version,
            self.kind,
            self.component_schema_version,
            crate::bitcode_value::BitcodeValue::from_value(&self.payload)?,
        )))
    }

    pub fn from_bitcode(
        bytes: &[u8],
    ) -> Result<Self, crate::bitcode_value::ProjectionBitcodeError> {
        use crate::bitcode_value::{BitcodeValue, ProjectionBitcodeError};
        let (magic, schema_version, kind, component_schema_version, payload): (
            [u8; 8],
            u32,
            SimulationContentComponentKindV1,
            u32,
            BitcodeValue,
        ) = bitcode::decode(bytes)?;
        if magic != *b"RHSC0002" {
            return Err(ProjectionBitcodeError::Invalid(
                "unsupported component format",
            ));
        }
        let document = Self {
            schema_version,
            kind,
            component_schema_version,
            payload: payload.into_value()?,
        };
        document.validate()?;
        if document.bitcode_bytes()? != bytes {
            return Err(ProjectionBitcodeError::Invalid(
                "noncanonical bitcode encoding",
            ));
        }
        Ok(document)
    }
}

impl Validate for SimulationContentComponentDocumentV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SimulationContentComponentDocumentV1", self.schema_version)?;
        if self.component_schema_version == 0 {
            return Err(ValidationError::Zero {
                field: "simulation_content.component_document_schema_version",
            });
        }
        self.payload.validate_depth(128)
    }
}

impl Validate for ContentFileV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::canonical_relative_path("content.files.path", &self.path)?;
        self.role.validate()?;
        self.artifact.validate()
    }
}

/// Complete, fail-closed content fingerprint.
///
/// A listed path is required. The consumer must reject a missing file or a
/// length/digest mismatch; this type intentionally has no fallback source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentManifestV1 {
    pub schema_version: u32,
    pub name: String,
    pub edition: OfficialContentEditionV1,
    pub subject: OfficialContentSubjectV1,
    pub closure: ContentClosureKindV1,
    pub projection_schema_version: u32,
    pub resource_locale_root: ResourceLocaleRootV1,
    pub speech_timing: SimulationSpeechTimingSourceV1,
    pub components: Vec<SimulationContentComponentV1>,
}

impl Validate for ContentManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("ContentManifestV1", self.schema_version)?;
        crate::validation::text("content.name", &self.name, 256)?;
        self.subject.validate()?;
        if self.projection_schema_version == 0 {
            return Err(ValidationError::Zero {
                field: "content.projection_schema_version",
            });
        }
        self.resource_locale_root.validate()?;
        self.speech_timing.validate()?;
        for component in &self.components {
            component.validate()?;
        }
        let required = [
            SimulationContentComponentKindV1::Profiles,
            SimulationContentComponentKindV1::LoadedLevel,
            SimulationContentComponentKindV1::MissionScripts,
            SimulationContentComponentKindV1::SpriteSimulationMetadata,
            SimulationContentComponentKindV1::MapGeometryMetadata,
            SimulationContentComponentKindV1::LocalizedDeterministicText,
            SimulationContentComponentKindV1::SoundDurationTables,
            SimulationContentComponentKindV1::InterfaceSimulationMetadata,
        ];
        if self.components.len() != required.len()
            || self
                .components
                .iter()
                .map(|component| component.kind)
                .ne(required)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "content.components",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContentEntryV1 {
    pub subject: OfficialContentSubjectV1,
    pub content_manifest_sha256: Digest32,
}

impl Validate for CampaignContentEntryV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.subject.validate()?;
        if self.content_manifest_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "campaign_content.content_manifest_sha256",
            });
        }
        Ok(())
    }
}

/// Stable edition catalog used by full-campaign boards.
///
/// Entries are keyed by gameplay subject rather than play order, so branches
/// and repeated headquarters visits do not change the board identity. Each
/// session in a verified chain must resolve to exactly one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignContentManifestV1 {
    pub schema_version: u32,
    pub edition: OfficialContentEditionV1,
    pub entries: Vec<CampaignContentEntryV1>,
}

impl Validate for CampaignContentManifestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("CampaignContentManifestV1", self.schema_version)?;
        if self.entries.is_empty() || self.entries.len() > 4_096 {
            return Err(ValidationError::CountOutOfRange {
                field: "campaign_content.entries",
            });
        }
        for entry in &self.entries {
            entry.validate()?;
        }
        if !self
            .entries
            .windows(2)
            .all(|pair| pair[0].subject < pair[1].subject)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "campaign_content.entries",
            });
        }
        Ok(())
    }
}

impl CampaignContentManifestV1 {
    pub fn content_for(&self, subject: &OfficialContentSubjectV1) -> Option<Digest32> {
        self.entries
            .binary_search_by(|entry| entry.subject.cmp(subject))
            .ok()
            .map(|index| self.entries[index].content_manifest_sha256)
    }
}

/// Official-demo canonical simulation-component object path. Packaging and
/// native source paths never participate in this URL.
pub fn demo_content_object_path_v1(
    content_manifest_sha256: Digest32,
    component: &SimulationContentComponentV1,
) -> Result<String, ValidationError> {
    if content_manifest_sha256.is_zero() {
        return Err(ValidationError::Zero {
            field: "content_object_path.content_manifest_sha256",
        });
    }
    component.validate()?;
    Ok(format!(
        "content/{content_manifest_sha256}/objects/{}",
        component.artifact.sha256
    ))
}
