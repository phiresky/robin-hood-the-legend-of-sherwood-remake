//! Official content subjects and the canonical simulation-component closure.

use serde::{Deserialize, Serialize};

use crate::{ArtifactRefV1, CanonicalValue, Validate, ValidationError};

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

/// First mission selected by an authentic freshly-reset FULL campaign. The
/// original campaign controller launches this field mission directly when it
/// is the sole accessible mission and the campaign is not passing through HQ.
pub const OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1: &str = "H01_Lin_VL";

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
