//! Structural validation for the opaque campaign embedded in a replay.
//!
//! This is the boundary between bitcode's decoded object graph and code which
//! dereferences profile indices or constructs an `Engine`. It deliberately
//! does not repair, sort, deduplicate, or otherwise canonicalize submitted
//! state: the digest always describes the exact submitted campaign bytes.
//!
//! # Hostile bitcode containment
//!
//! `bitcode` 0.6 decodes a `Vec` by trusting its encoded capacity before a
//! post-decode validator can inspect the value. Consequently the byte-size
//! ceiling and this validator do **not** make `bitcode::decode` safe to run in
//! a long-lived public server process. Public replay verification must call
//! this API only in a disposable verifier child with hard memory and wall-time
//! limits. The supervisor must keep campaign bytes opaque. Tiny malicious
//! huge-length fixtures must likewise be exercised only as subprocess tests.
//!
//! # Content-dependent references
//!
//! Sight-obstacle, script-zone, and Sherwood beam-me upper bounds depend on
//! the approved mission data. Their exact bounded identities are surfaced in
//! [`DeferredReplayCampaignContentChecks`]. A verifier must validate those
//! identities against the already-approved loaded content in a second phase,
//! still before `Engine` construction or script dispatch. This structural
//! phase rejects invalid sentinels and every content-independent identity.

use robin_engine::campaign::{Campaign, CampaignPracticeReturnView};
use robin_engine::campaign_history::CAMPAIGN_HISTORY_SCHEMA_VERSION;
use robin_engine::profiles::{MissionLocation, ProfileManager};
use robin_engine::sector_production::Type as ProductionType;
use robin_replay_format::ReplayAdmissionLimits;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const CANONICAL_PRODUCTION_TYPES: [ProductionType; 13] = [
    ProductionType::MakeArrow,
    ProductionType::MakePurse,
    ProductionType::MakeStone,
    ProductionType::MakeApple,
    ProductionType::MakeAle,
    ProductionType::MakeLamblegg,
    ProductionType::MakePlant,
    ProductionType::MakeNet,
    ProductionType::MakeWaspNest,
    ProductionType::TrainBow,
    ProductionType::TrainHandToHand,
    ProductionType::Heal,
    ProductionType::Relic,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignLayer {
    Current,
    PreMissionSnapshot,
    PracticeReturnSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignCollection {
    Missions,
    Characters,
    AccessibleMissions,
    PendingAccessibleMissions,
    Gang,
    Reservists,
    MissionTeam,
    PeasantNames,
    CollectedRelics,
    ProductionSectors,
    ProductionPoints,
    ProductionOccupants,
    MissionAttempts,
    AttemptRecruitedCharacters,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignIndexField {
    LastMission,
    CurrentMission,
    NextMission,
    BlazonMission,
    HistoryReplayMission,
    AccessibleMission,
    PendingAccessibleMission,
    GangCharacter,
    ReservistCharacter,
    MissionTeamCharacter,
    ProductionOccupantCharacter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignCoordinateField {
    ProductionPointX,
    ProductionPointY,
    ProductionOccupantX,
    ProductionOccupantY,
}

/// Typed structural failure returned before any submitted index is
/// dereferenced by normal game code.
#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ReplayCampaignValidationError {
    #[error("campaign bytes exceed admission limit: observed {observed}, limit {limit}")]
    CampaignBytesLimit { observed: usize, limit: usize },
    #[error(
        "header mission identifier exceeds admission limit: observed {observed} UTF-8 bytes, limit {limit}"
    )]
    HeaderMissionIdBytesLimit { observed: usize, limit: usize },
    #[error("campaign bitcode decode failed: {message}")]
    Decode { message: String },
    #[error("campaign {layer:?} collection {collection:?} has {observed} entries, limit {limit}")]
    CollectionLimit {
        layer: CampaignLayer,
        collection: CampaignCollection,
        observed: usize,
        limit: usize,
    },
    #[error("campaign restart snapshot depth {observed} exceeds limit {limit}")]
    SnapshotDepthLimit { observed: usize, limit: usize },
    #[error("campaign {layer:?} mission {mission_index} has no profile index")]
    MissingMissionProfile {
        layer: CampaignLayer,
        mission_index: usize,
    },
    #[error(
        "campaign {layer:?} mission {mission_index} profile {profile_index} is outside {profile_count} profiles"
    )]
    MissionProfileOutOfRange {
        layer: CampaignLayer,
        mission_index: usize,
        profile_index: u32,
        profile_count: usize,
    },
    #[error(
        "campaign {layer:?} missions {first_mission} and {duplicate_mission} share profile {profile_index}"
    )]
    DuplicateMissionProfile {
        layer: CampaignLayer,
        profile_index: u32,
        first_mission: usize,
        duplicate_mission: usize,
    },
    #[error(
        "campaign {layer:?} has {observed} mission profiles; approved content requires exactly {approved}"
    )]
    MissionProfileSetSize {
        layer: CampaignLayer,
        observed: usize,
        approved: usize,
    },
    #[error(
        "campaign {layer:?} mission {mission_index} uses profile {observed}; canonical approved profile is {expected}"
    )]
    MissionProfileSetOrder {
        layer: CampaignLayer,
        mission_index: usize,
        observed: u32,
        expected: u32,
    },
    #[error("approved mission profile id {profile_id} occurs more than once")]
    ApprovedMissionProfileIdDuplicate { profile_id: u32 },
    #[error(
        "approved mission profile {profile_id} names absent prerequisite mission id {prerequisite_id}"
    )]
    ApprovedMissionPrerequisiteMissing {
        profile_id: u32,
        prerequisite_id: u32,
    },
    #[error("header mission `{mission_id}` is absent from approved mission profiles")]
    HeaderProfileMissing { mission_id: String },
    #[error("header mission `{mission_id}` matches multiple approved mission profiles")]
    HeaderProfileAmbiguous { mission_id: String },
    #[error("campaign {layer:?} does not contain header mission `{mission_id}`")]
    HeaderMissionMissing {
        layer: CampaignLayer,
        mission_id: String,
    },
    #[error(
        "campaign {layer:?} current mission {current:?} does not match header mission `{mission_id}` at index {expected}"
    )]
    CurrentMissionMismatch {
        layer: CampaignLayer,
        current: Option<usize>,
        expected: usize,
        mission_id: String,
    },
    #[error(
        "campaign {layer:?} field {field:?} index {index} is outside collection length {length}"
    )]
    IndexOutOfRange {
        layer: CampaignLayer,
        field: CampaignIndexField,
        index: usize,
        length: usize,
    },
    #[error("campaign {layer:?} collection {collection:?} repeats index {index}")]
    DuplicateIndex {
        layer: CampaignLayer,
        collection: CampaignCollection,
        index: usize,
    },
    #[error("campaign {layer:?} collections {left:?} and {right:?} both contain index {index}")]
    OverlappingIndex {
        layer: CampaignLayer,
        left: CampaignCollection,
        right: CampaignCollection,
        index: usize,
    },
    #[error("campaign {layer:?} mission-team character {index} is not in the active gang")]
    MissionTeamOutsideGang { layer: CampaignLayer, index: usize },
    #[error(
        "campaign {layer:?} character {character_index} profile {profile_index} is outside {profile_count} profiles"
    )]
    CharacterProfileOutOfRange {
        layer: CampaignLayer,
        character_index: usize,
        profile_index: u32,
        profile_count: usize,
    },
    #[error("campaign {layer:?} referenced character {character_index} has no character profile")]
    ReferencedCharacterHasNoProfile {
        layer: CampaignLayer,
        character_index: usize,
    },
    #[error(
        "campaign {layer:?} production occupant {character_index} is outside gang/reservist/team identity sets"
    )]
    ProductionOccupantOutsideRoster {
        layer: CampaignLayer,
        character_index: usize,
    },
    #[error(
        "campaign {layer:?} ARES state {ares} is outside the {available_states} states used by mission {mission_index}"
    )]
    AresOutOfRange {
        layer: CampaignLayer,
        ares: i8,
        mission_index: usize,
        available_states: usize,
    },
    #[error("campaign {layer:?} uses invalid ARES sentinel {ares}")]
    InvalidAresSentinel { layer: CampaignLayer, ares: i8 },
    #[error(
        "campaign {layer:?} mission {mission_index} uses invalid ARES override sentinel {ares}"
    )]
    InvalidMissionAresSentinel {
        layer: CampaignLayer,
        mission_index: usize,
        ares: i8,
    },
    #[error(
        "campaign {layer:?} mission {mission_index} ARES success override {ares} differs from approved authored state {authored}"
    )]
    MissionAresOverrideNotAuthored {
        layer: CampaignLayer,
        mission_index: usize,
        ares: i8,
        authored: i8,
    },
    #[error("campaign {layer:?} has {observed} production sectors; expected exactly {expected}")]
    ProductionSectorCount {
        layer: CampaignLayer,
        observed: usize,
        expected: usize,
    },
    #[error("campaign {layer:?} production slot {slot} has type {actual:?}; expected {expected:?}")]
    ProductionSectorType {
        layer: CampaignLayer,
        slot: usize,
        actual: ProductionType,
        expected: ProductionType,
    },
    #[error(
        "campaign {layer:?} production slot {slot} {field:?} coordinate is not finite (bits {bits:#010x})"
    )]
    NonFiniteProductionCoordinate {
        layer: CampaignLayer,
        slot: usize,
        field: CampaignCoordinateField,
        bits: u32,
    },
    #[error(
        "campaign {layer:?} character {character_index} has invalid Sherwood beam-me sentinel {value}"
    )]
    InvalidBeamMeSentinel {
        layer: CampaignLayer,
        character_index: usize,
        value: i16,
    },
    #[error("campaign {layer:?} collected-relic identity {value} is not a relic ordinal")]
    InvalidRelicIdentity { layer: CampaignLayer, value: u32 },
    #[error("campaign {layer:?} repeats collected-relic identity {value}")]
    DuplicateRelicIdentity { layer: CampaignLayer, value: u32 },
    #[error(
        "campaign {layer:?} mission {mission_index} history schema {observed} is unsupported; expected {expected}"
    )]
    HistorySchema {
        layer: CampaignLayer,
        mission_index: usize,
        observed: u16,
        expected: u16,
    },
    #[error(
        "campaign {layer:?} mission {mission_index} attempt sequence {sequence} does not follow {previous}"
    )]
    HistorySequenceOrder {
        layer: CampaignLayer,
        mission_index: usize,
        previous: u64,
        sequence: u64,
    },
    #[error("campaign {layer:?} repeats global attempt sequence {sequence}")]
    DuplicateHistorySequence { layer: CampaignLayer, sequence: u64 },
    #[error(
        "campaign {layer:?} attempt counter {observed} does not equal greatest stored sequence {expected}"
    )]
    HistorySequenceCounter {
        layer: CampaignLayer,
        observed: u64,
        expected: u64,
    },
    #[error(
        "campaign {layer:?} attempt counter {sequence} leaves no headroom for the next mission attempt"
    )]
    HistorySequenceExhausted { layer: CampaignLayer, sequence: u64 },
    #[error("campaign full-fidelity history is internally inconsistent: {message}")]
    HistoryInvariant { message: String },
    #[error(
        "campaign {layer:?} attempt {sequence} has an achievement attestation without raw results"
    )]
    AttestationWithoutResults { layer: CampaignLayer, sequence: u64 },
    #[error(
        "campaign {layer:?} attempt {sequence} achievement attestation decision does not match its policy, context, and raw results"
    )]
    AchievementAttestationDecisionMismatch { layer: CampaignLayer, sequence: u64 },
    #[error(
        "campaign restart checkpoint fields are inconsistent: snapshot={snapshot}, rng={rng}, config={config}, preselected={preselected}"
    )]
    CheckpointShape {
        snapshot: bool,
        rng: bool,
        config: bool,
        preselected: bool,
    },
    #[error("campaign restart snapshot checkpoint metadata differs from its outer campaign")]
    CheckpointMetadataMismatch,
    #[error("campaign restart snapshot carries a different practice-return checkpoint")]
    CheckpointPracticeReturnMismatch,
    #[error("campaign restart checkpoint differs from the exact ranked simulation authority")]
    CheckpointSimulationAuthorityMismatch,
    #[error(
        "campaign history replay mission {history_mission} differs from current mission {current:?}"
    )]
    HistoryReplayMissionMismatch {
        history_mission: usize,
        current: Option<usize>,
    },
    #[error("campaign history replay is missing a complete pre-mission restart checkpoint")]
    HistoryReplayWithoutCheckpoint,
    #[error("campaign {layer:?} {field} string has {observed} UTF-8 bytes, limit {limit}")]
    StringLimit {
        layer: CampaignLayer,
        field: String,
        observed: usize,
        limit: usize,
    },
    #[error("campaign {layer:?} repeats peasant name `{name}`")]
    DuplicatePeasantName { layer: CampaignLayer, name: String },
    #[error(
        "campaign requires approved-content validation before playback: obstacles={obstacles}, script_zones={script_zones}, point_topology={point_topology}, beam_mes={beam_mes}"
    )]
    DeferredContentValidationRequired {
        obstacles: usize,
        script_zones: usize,
        point_topology: usize,
        beam_mes: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionObstacleSource {
    Point,
    Occupant,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredSightObstacleReference {
    pub layer: CampaignLayer,
    pub production_slot: usize,
    pub source: ProductionObstacleSource,
    pub source_index: usize,
    pub obstacle_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredProductionScriptZoneReference {
    pub layer: CampaignLayer,
    pub production_slot: usize,
    pub script_zone_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredProductionPointTopologyReference {
    pub layer: CampaignLayer,
    pub production_slot: usize,
    pub point_index: usize,
    pub map_layer: u16,
    pub sector: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredSherwoodBeamMeReference {
    pub layer: CampaignLayer,
    pub character_index: usize,
    pub beam_me_index: u16,
}

/// Actual bounded content-dependent identities which the verifier must check
/// after loading approved assets and before constructing an `Engine`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredReplayCampaignContentChecks {
    pub sight_obstacle_references: Vec<DeferredSightObstacleReference>,
    pub production_script_zone_references: Vec<DeferredProductionScriptZoneReference>,
    pub production_point_topology_references: Vec<DeferredProductionPointTopologyReference>,
    pub sherwood_beam_me_references: Vec<DeferredSherwoodBeamMeReference>,
}

impl DeferredReplayCampaignContentChecks {
    fn append(&mut self, mut other: Self) {
        self.sight_obstacle_references
            .append(&mut other.sight_obstacle_references);
        self.production_script_zone_references
            .append(&mut other.production_script_zone_references);
        self.production_point_topology_references
            .append(&mut other.production_point_topology_references);
        self.sherwood_beam_me_references
            .append(&mut other.sherwood_beam_me_references);
    }

    pub fn is_empty(&self) -> bool {
        self.sight_obstacle_references.is_empty()
            && self.production_script_zone_references.is_empty()
            && self.production_point_topology_references.is_empty()
            && self.sherwood_beam_me_references.is_empty()
    }
}

/// Fully structurally validated campaign plus the digest of its exact encoded
/// bytes. No submitted campaign field is normalized or rewritten.
///
/// Every field is private and the value is neither cloneable nor
/// deserializable. This makes it an immutable, process-local type-state token:
/// submitted state can leave this module only through a consuming method
/// which either proves that no phase-two references exist or carries the
/// approved-content capability.
#[derive(Debug)]
pub struct ValidatedReplayCampaign {
    campaign: Campaign,
    mission_index: usize,
    mission_location: MissionLocation,
    submitted_campaign_sha256: [u8; 32],
    deferred_content_checks: DeferredReplayCampaignContentChecks,
    _seal: ValidatedReplayCampaignSeal,
}

#[derive(Debug)]
struct ValidatedReplayCampaignSeal;

/// Manifest identities asserted by the verifier's approved, read-only content
/// resolver. The worker must compare these with its signed request before
/// allowing the returned capability to reach engine construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayCampaignApprovedContentIdentity {
    pub build_manifest_sha256: [u8; 32],
    pub content_manifest_sha256: [u8; 32],
}

/// Opaque proof that every deferred identity was checked against approved,
/// mounted content. There is intentionally no public or crate-visible
/// unchecked constructor. The approved-content phase owns the sole private
/// construction site and must return this only after exhausting all carried
/// references.
///
/// This capability is deliberately not deserializable: an admission token is
/// process-local evidence, not data a submitter can provide.
#[derive(Debug)]
pub struct ApprovedReplayCampaignContent {
    validated: ValidatedReplayCampaign,
    approved_identity: ReplayCampaignApprovedContentIdentity,
    _seal: ApprovedReplayCampaignContentSeal,
}

#[derive(Debug)]
struct ApprovedReplayCampaignContentSeal;

mod engine_preparation;
pub use engine_preparation::{ApprovedRankedReplayPreparation, ApprovedReplayEngine};

impl ApprovedReplayCampaignContent {
    pub fn validated(&self) -> &ValidatedReplayCampaign {
        &self.validated
    }

    pub const fn approved_identity(&self) -> ReplayCampaignApprovedContentIdentity {
        self.approved_identity
    }

    /// Consume phase-one and phase-two proof together. This is the only path
    /// which exposes a campaign carrying deferred content-dependent
    /// references.
    fn into_playback_parts(
        self,
    ) -> (
        Campaign,
        usize,
        MissionLocation,
        [u8; 32],
        ReplayCampaignApprovedContentIdentity,
    ) {
        let Self {
            validated,
            approved_identity,
            _seal: _,
        } = self;
        let ValidatedReplayCampaign {
            campaign,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            deferred_content_checks: _,
            _seal: _,
        } = validated;
        (
            campaign,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            approved_identity,
        )
    }
}

mod approved_content;
#[cfg(test)]
use approved_content::validate_replay_campaign_approved_content_with_work_limit;
pub use approved_content::{
    ReplayCampaignApprovedBeamMe, ReplayCampaignApprovedContentMetadata,
    ReplayCampaignApprovedContentResolver, ReplayCampaignApprovedMapPoint,
    ReplayCampaignApprovedScriptZone, ReplayCampaignApprovedSector,
    ReplayCampaignApprovedStaticObstacle, ReplayCampaignContentValidationError,
    ReplayCampaignMetadataDerivationError, ReplayCampaignProductionPointTopology,
    derive_replay_campaign_approved_content_metadata, validate_replay_campaign_approved_content,
};

impl ValidatedReplayCampaign {
    /// Borrow the submitted campaign only inside the crate's approved raw
    /// content loader. This never transfers or clones ownership, so engine
    /// construction still requires consuming the sealed phase-two token.
    pub(super) const fn campaign_for_approved_loading(&self) -> &Campaign {
        &self.campaign
    }

    /// Bind the opaque submitted state to the simulation configuration
    /// authenticated by the replay/rules tuple. Structural validation alone
    /// only proves that outer and nested checkpoints agree with one another;
    /// both could otherwise carry the same unauthorized configuration.
    pub(super) fn validate_checkpoint_simulation_authority(
        &self,
        sim_config: robin_engine::engine::SimConfig,
    ) -> Result<(), ReplayCampaignValidationError> {
        if self.campaign.pre_mission_sim_config != Some(sim_config) {
            return Err(ReplayCampaignValidationError::CheckpointSimulationAuthorityMismatch);
        }
        Ok(())
    }

    pub const fn mission_index(&self) -> usize {
        self.mission_index
    }

    pub const fn mission_location(&self) -> MissionLocation {
        self.mission_location
    }

    pub const fn submitted_campaign_sha256(&self) -> [u8; 32] {
        self.submitted_campaign_sha256
    }

    pub const fn deferred_content_checks(&self) -> &DeferredReplayCampaignContentChecks {
        &self.deferred_content_checks
    }

    /// Fail closed when the caller has no approved-content phase available.
    /// A server verifier should instead validate every carried identity against
    /// the approved level data, then consume this value before engine creation.
    pub fn require_no_deferred_content_references(
        &self,
    ) -> Result<(), ReplayCampaignValidationError> {
        if self.deferred_content_checks.is_empty() {
            return Ok(());
        }
        Err(deferred_content_validation_required(
            &self.deferred_content_checks,
        ))
    }

    /// Consume a structurally validated campaign only when it carries no
    /// content-dependent references. Callers with such references must use
    /// [`validate_replay_campaign_approved_content`] and consume its returned
    /// capability instead.
    pub fn try_into_playback_parts(
        self,
    ) -> Result<(Campaign, usize, MissionLocation, [u8; 32]), ReplayCampaignValidationError> {
        if !self.deferred_content_checks.is_empty() {
            return Err(deferred_content_validation_required(
                &self.deferred_content_checks,
            ));
        }
        let Self {
            campaign,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
            deferred_content_checks: _,
            _seal: _,
        } = self;
        Ok((
            campaign,
            mission_index,
            mission_location,
            submitted_campaign_sha256,
        ))
    }
}

fn deferred_content_validation_required(
    checks: &DeferredReplayCampaignContentChecks,
) -> ReplayCampaignValidationError {
    ReplayCampaignValidationError::DeferredContentValidationRequired {
        obstacles: checks.sight_obstacle_references.len(),
        script_zones: checks.production_script_zone_references.len(),
        point_topology: checks.production_point_topology_references.len(),
        beam_mes: checks.sherwood_beam_me_references.len(),
    }
}

/// Hash, decode, and structurally validate a replay campaign.
///
/// The SHA-256 is calculated before decoding. See the module-level hostile
/// bitcode containment contract: public servers must invoke this in a
/// resource-limited verifier child, never in the API/supervisor process.
pub fn decode_and_validate_replay_campaign(
    submitted_bytes: &[u8],
    header_mission_id: &str,
    profiles: &ProfileManager,
    limits: &ReplayAdmissionLimits,
) -> Result<ValidatedReplayCampaign, ReplayCampaignValidationError> {
    if submitted_bytes.len() > limits.max_campaign_bytes {
        return Err(ReplayCampaignValidationError::CampaignBytesLimit {
            observed: submitted_bytes.len(),
            limit: limits.max_campaign_bytes,
        });
    }
    if header_mission_id.len() > limits.max_mission_id_bytes {
        return Err(ReplayCampaignValidationError::HeaderMissionIdBytesLimit {
            observed: header_mission_id.len(),
            limit: limits.max_mission_id_bytes,
        });
    }
    validate_approved_mission_profile_ids(profiles)?;
    let submitted_campaign_sha256 = Sha256::digest(submitted_bytes).into();

    // SECURITY: This allocation-capable decode is permitted only inside the
    // resource-limited verifier child described above. Structural limits are
    // necessarily enforced after the bitcode object graph exists.
    let campaign: Campaign = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bitcode::decode(submitted_bytes)
    }))
    .map_err(|_| ReplayCampaignValidationError::Decode {
        message: "bitcode decoder panicked on the submitted campaign".to_owned(),
    })?
    .map_err(|error| ReplayCampaignValidationError::Decode {
        message: error.to_string(),
    })?;

    validate_decoded_replay_campaign(
        campaign,
        submitted_campaign_sha256,
        header_mission_id,
        profiles,
        limits,
    )
}

fn validate_approved_mission_profile_ids(
    profiles: &ProfileManager,
) -> Result<(), ReplayCampaignValidationError> {
    let mut approved_ids = BTreeSet::new();
    for profile in &profiles.missions {
        if !approved_ids.insert(profile.id) {
            return Err(
                ReplayCampaignValidationError::ApprovedMissionProfileIdDuplicate {
                    profile_id: profile.id,
                },
            );
        }
    }
    // Official demo profile tables intentionally retain prerequisites for
    // retail missions which are absent from the demo. Those trusted authored
    // references mean "never unlock in this edition", not a malformed
    // submitted campaign. The exact content manifest authenticates the
    // profile table; only ambiguous duplicate ids are rejected here.
    Ok(())
}

fn validate_decoded_replay_campaign(
    campaign: Campaign,
    submitted_campaign_sha256: [u8; 32],
    header_mission_id: &str,
    profiles: &ProfileManager,
    limits: &ReplayAdmissionLimits,
) -> Result<ValidatedReplayCampaign, ReplayCampaignValidationError> {
    let header_profile_indices: Vec<usize> = profiles
        .missions
        .iter()
        .enumerate()
        .filter_map(|(index, profile)| {
            (profile.mission_filename == header_mission_id).then_some(index)
        })
        .collect();
    let header_profile_index = match header_profile_indices.as_slice() {
        [] => {
            return Err(ReplayCampaignValidationError::HeaderProfileMissing {
                mission_id: header_mission_id.to_owned(),
            });
        }
        [index] => *index,
        _ => {
            return Err(ReplayCampaignValidationError::HeaderProfileAmbiguous {
                mission_id: header_mission_id.to_owned(),
            });
        }
    };

    let (mission_index, mut deferred_content_checks) = validate_campaign_layer(
        &campaign.validation_view(),
        CampaignLayer::Current,
        Some(header_profile_index),
        header_mission_id,
        profiles,
        limits,
    )?;
    let mission_index =
        mission_index.ok_or_else(|| ReplayCampaignValidationError::HeaderMissionMissing {
            layer: CampaignLayer::Current,
            mission_id: header_mission_id.to_owned(),
        })?;

    let snapshot_present = campaign.pre_mission_snapshot.is_some();
    let rng_present = campaign.pre_mission_rng_seed.is_some();
    let config_present = campaign.pre_mission_sim_config.is_some();
    if snapshot_present != rng_present
        || snapshot_present != config_present
        || (!snapshot_present && campaign.pre_mission_was_preselected)
    {
        return Err(ReplayCampaignValidationError::CheckpointShape {
            snapshot: snapshot_present,
            rng: rng_present,
            config: config_present,
            preselected: campaign.pre_mission_was_preselected,
        });
    }

    if let Some(snapshot) = campaign.pre_mission_snapshot.as_ref() {
        if limits.max_campaign_snapshot_depth < 1 {
            return Err(ReplayCampaignValidationError::SnapshotDepthLimit {
                observed: 1,
                limit: limits.max_campaign_snapshot_depth,
            });
        }
        let (_, snapshot_deferred) = validate_campaign_layer(
            &snapshot.validation_view(),
            CampaignLayer::PreMissionSnapshot,
            campaign
                .pre_mission_was_preselected
                .then_some(header_profile_index),
            header_mission_id,
            profiles,
            limits,
        )?;
        deferred_content_checks.append(snapshot_deferred);

        if snapshot.pre_mission_rng_seed != campaign.pre_mission_rng_seed
            || snapshot.pre_mission_sim_config != campaign.pre_mission_sim_config
            || snapshot.pre_mission_was_preselected != campaign.pre_mission_was_preselected
        {
            return Err(ReplayCampaignValidationError::CheckpointMetadataMismatch);
        }
        let outer_practice = campaign
            .practice_return_snapshot
            .as_ref()
            .map(bitcode::encode);
        let snapshot_practice = snapshot
            .practice_return_snapshot
            .as_ref()
            .map(bitcode::encode);
        if snapshot_practice != outer_practice {
            return Err(ReplayCampaignValidationError::CheckpointPracticeReturnMismatch);
        }
    }

    if let Some(practice_return) = campaign.practice_return_snapshot.as_ref() {
        if limits.max_campaign_snapshot_depth < 1 {
            return Err(ReplayCampaignValidationError::SnapshotDepthLimit {
                observed: 1,
                limit: limits.max_campaign_snapshot_depth,
            });
        }
        let practice_view = practice_return.validation_view();
        let (_, practice_deferred) = validate_campaign_layer(
            &practice_view,
            CampaignLayer::PracticeReturnSnapshot,
            None,
            header_mission_id,
            profiles,
            limits,
        )?;
        deferred_content_checks.append(practice_deferred);
    }

    if let Some(history_mission) = campaign.history_replay_mission_idx {
        if campaign.current_mission_idx != Some(history_mission) {
            return Err(
                ReplayCampaignValidationError::HistoryReplayMissionMismatch {
                    history_mission,
                    current: campaign.current_mission_idx,
                },
            );
        }
        if !snapshot_present || !rng_present || !config_present {
            return Err(ReplayCampaignValidationError::HistoryReplayWithoutCheckpoint);
        }
    }

    // Repeat the proof through checked access rather than making the type-state
    // constructor itself depend on an earlier direct-index invariant.
    let mission = campaign.missions.get(mission_index).ok_or(
        ReplayCampaignValidationError::IndexOutOfRange {
            layer: CampaignLayer::Current,
            field: CampaignIndexField::CurrentMission,
            index: mission_index,
            length: campaign.missions.len(),
        },
    )?;
    let mission_profile_index =
        mission
            .profile_idx
            .ok_or(ReplayCampaignValidationError::MissingMissionProfile {
                layer: CampaignLayer::Current,
                mission_index,
            })? as usize;
    let mission_location = profiles
        .missions
        .get(mission_profile_index)
        .ok_or(ReplayCampaignValidationError::MissionProfileOutOfRange {
            layer: CampaignLayer::Current,
            mission_index,
            profile_index: mission_profile_index as u32,
            profile_count: profiles.missions.len(),
        })?
        .location;

    // The layer-specific verifier above emits bounded, typed failures for all
    // serialized history fields. Retain the engine's canonical invariant gate
    // as the final defense for cross-layer relationships such as the
    // history-replay/practice-return pairing.
    campaign
        .validate_history_schema()
        .map_err(|message| ReplayCampaignValidationError::HistoryInvariant { message })?;

    Ok(ValidatedReplayCampaign {
        campaign,
        mission_index,
        mission_location,
        submitted_campaign_sha256,
        deferred_content_checks,
        _seal: ValidatedReplayCampaignSeal,
    })
}

fn validate_campaign_layer(
    campaign: &CampaignPracticeReturnView<'_>,
    layer: CampaignLayer,
    required_header_profile_index: Option<usize>,
    header_mission_id: &str,
    profiles: &ProfileManager,
    limits: &ReplayAdmissionLimits,
) -> Result<(Option<usize>, DeferredReplayCampaignContentChecks), ReplayCampaignValidationError> {
    let missions = campaign.missions;
    let characters = campaign.characters;
    check_collection_limit(
        layer,
        CampaignCollection::Missions,
        missions.len(),
        limits.max_campaign_missions,
    )?;
    check_collection_limit(
        layer,
        CampaignCollection::Characters,
        characters.len(),
        limits.max_campaign_characters,
    )?;

    let mut mission_by_profile = BTreeMap::new();
    for (mission_index, mission) in missions.iter().enumerate() {
        let profile_index =
            mission
                .profile_idx
                .ok_or(ReplayCampaignValidationError::MissingMissionProfile {
                    layer,
                    mission_index,
                })?;
        if profile_index as usize >= profiles.missions.len() {
            return Err(ReplayCampaignValidationError::MissionProfileOutOfRange {
                layer,
                mission_index,
                profile_index,
                profile_count: profiles.missions.len(),
            });
        }
        if let Some(first_mission) = mission_by_profile.insert(profile_index, mission_index) {
            return Err(ReplayCampaignValidationError::DuplicateMissionProfile {
                layer,
                profile_index,
                first_mission,
                duplicate_mission: mission_index,
            });
        }
    }
    if missions.len() != profiles.missions.len() {
        return Err(ReplayCampaignValidationError::MissionProfileSetSize {
            layer,
            observed: missions.len(),
            approved: profiles.missions.len(),
        });
    }
    for (mission_index, mission) in missions.iter().enumerate() {
        let observed =
            mission
                .profile_idx
                .ok_or(ReplayCampaignValidationError::MissingMissionProfile {
                    layer,
                    mission_index,
                })?;
        let expected = u32::try_from(mission_index).map_err(|_| {
            ReplayCampaignValidationError::MissionProfileSetSize {
                layer,
                observed: missions.len(),
                approved: profiles.missions.len(),
            }
        })?;
        if observed != expected {
            return Err(ReplayCampaignValidationError::MissionProfileSetOrder {
                layer,
                mission_index,
                observed,
                expected,
            });
        }
    }
    let required_mission_index = required_header_profile_index
        .map(|header_profile_index| {
            mission_by_profile
                .get(&(header_profile_index as u32))
                .copied()
                .ok_or_else(|| ReplayCampaignValidationError::HeaderMissionMissing {
                    layer,
                    mission_id: header_mission_id.to_owned(),
                })
        })
        .transpose()?;

    validate_optional_mission_index(
        campaign.last_mission_idx,
        CampaignIndexField::LastMission,
        missions.len(),
        layer,
    )?;
    validate_optional_mission_index(
        campaign.current_mission_idx,
        CampaignIndexField::CurrentMission,
        missions.len(),
        layer,
    )?;
    validate_optional_mission_index(
        campaign.next_mission_idx,
        CampaignIndexField::NextMission,
        missions.len(),
        layer,
    )?;
    validate_optional_mission_index(
        campaign.blazon_mission_idx,
        CampaignIndexField::BlazonMission,
        missions.len(),
        layer,
    )?;
    validate_optional_mission_index(
        campaign.history_replay_mission_idx,
        CampaignIndexField::HistoryReplayMission,
        missions.len(),
        layer,
    )?;
    if let Some(mission_index) = required_mission_index
        && campaign.current_mission_idx != Some(mission_index)
    {
        return Err(ReplayCampaignValidationError::CurrentMissionMismatch {
            layer,
            current: campaign.current_mission_idx,
            expected: mission_index,
            mission_id: header_mission_id.to_owned(),
        });
    }

    let accessible = validate_index_collection(
        campaign.accessible_mission_indices,
        CampaignCollection::AccessibleMissions,
        CampaignIndexField::AccessibleMission,
        missions.len(),
        layer,
        limits,
    )?;
    let pending = validate_index_collection(
        campaign.pending_accessible_mission_indices,
        CampaignCollection::PendingAccessibleMissions,
        CampaignIndexField::PendingAccessibleMission,
        missions.len(),
        layer,
        limits,
    )?;
    validate_disjoint(
        &accessible,
        &pending,
        CampaignCollection::AccessibleMissions,
        CampaignCollection::PendingAccessibleMissions,
        layer,
    )?;
    for (character_index, character) in characters.iter().enumerate() {
        check_string_limit(
            layer,
            format!("characters[{character_index}].status.name"),
            &character.status.name,
            limits.max_campaign_string_bytes,
        )?;
        if let Some(profile_index) = character.character_profile_idx
            && usize::from(profile_index) >= profiles.characters.len()
        {
            return Err(ReplayCampaignValidationError::CharacterProfileOutOfRange {
                layer,
                character_index,
                profile_index: profile_index.0,
                profile_count: profiles.characters.len(),
            });
        }
        if character.status.beam_me_index_in_sherwood < -1 {
            return Err(ReplayCampaignValidationError::InvalidBeamMeSentinel {
                layer,
                character_index,
                value: character.status.beam_me_index_in_sherwood,
            });
        }
    }

    let gang = validate_index_collection(
        campaign.gang_indices,
        CampaignCollection::Gang,
        CampaignIndexField::GangCharacter,
        characters.len(),
        layer,
        limits,
    )?;
    let reservists = validate_index_collection(
        campaign.reservist_indices,
        CampaignCollection::Reservists,
        CampaignIndexField::ReservistCharacter,
        characters.len(),
        layer,
        limits,
    )?;
    let team = validate_index_collection(
        campaign.mission_team_indices,
        CampaignCollection::MissionTeam,
        CampaignIndexField::MissionTeamCharacter,
        characters.len(),
        layer,
        limits,
    )?;
    validate_disjoint(
        &gang,
        &reservists,
        CampaignCollection::Gang,
        CampaignCollection::Reservists,
        layer,
    )?;
    for &character_index in &team {
        if !gang.contains(&character_index) {
            return Err(ReplayCampaignValidationError::MissionTeamOutsideGang {
                layer,
                index: character_index,
            });
        }
    }
    for character_index in gang
        .iter()
        .chain(reservists.iter())
        .chain(team.iter())
        .copied()
    {
        if characters[character_index].character_profile_idx.is_none() {
            return Err(
                ReplayCampaignValidationError::ReferencedCharacterHasNoProfile {
                    layer,
                    character_index,
                },
            );
        }
    }

    check_collection_limit(
        layer,
        CampaignCollection::PeasantNames,
        campaign.peasant_names.len(),
        limits.max_campaign_collection_entries,
    )?;
    let mut peasant_names = BTreeSet::new();
    for (name_index, name) in campaign.peasant_names.iter().enumerate() {
        check_string_limit(
            layer,
            format!("peasant_names[{name_index}]"),
            name,
            limits.max_campaign_string_bytes,
        )?;
        if !peasant_names.insert(name.as_str()) {
            return Err(ReplayCampaignValidationError::DuplicatePeasantName {
                layer,
                name: name.clone(),
            });
        }
    }
    check_collection_limit(
        layer,
        CampaignCollection::CollectedRelics,
        campaign.collected_relics.len(),
        limits.max_campaign_collection_entries,
    )?;
    let mut relics = BTreeSet::new();
    for &relic in campaign.collected_relics {
        if !(12..=18).contains(&relic) {
            return Err(ReplayCampaignValidationError::InvalidRelicIdentity {
                layer,
                value: relic,
            });
        }
        if !relics.insert(relic) {
            return Err(ReplayCampaignValidationError::DuplicateRelicIdentity {
                layer,
                value: relic,
            });
        }
    }

    if campaign.ares < -1 {
        return Err(ReplayCampaignValidationError::InvalidAresSentinel {
            layer,
            ares: campaign.ares,
        });
    }
    if campaign.ares != -1 {
        for (mission_index, mission) in missions.iter().enumerate() {
            let profile_index =
                mission
                    .profile_idx
                    .ok_or(ReplayCampaignValidationError::MissingMissionProfile {
                        layer,
                        mission_index,
                    })? as usize;
            let profile = profiles.missions.get(profile_index).ok_or(
                ReplayCampaignValidationError::MissionProfileOutOfRange {
                    layer,
                    mission_index,
                    profile_index: profile_index as u32,
                    profile_count: profiles.missions.len(),
                },
            )?;
            if profile.ares_sensible
                && usize::try_from(campaign.ares)
                    .ok()
                    .filter(|&index| index < profile.available_in_ares_state.len())
                    .is_none()
            {
                return Err(ReplayCampaignValidationError::AresOutOfRange {
                    layer,
                    ares: campaign.ares,
                    mission_index,
                    available_states: profile.available_in_ares_state.len(),
                });
            }
        }
    }
    for (mission_index, mission) in missions.iter().enumerate() {
        if let Some(ares) = mission.ares_state_override {
            if ares < -1 {
                return Err(ReplayCampaignValidationError::InvalidMissionAresSentinel {
                    layer,
                    mission_index,
                    ares,
                });
            }
            let profile_index =
                mission
                    .profile_idx
                    .ok_or(ReplayCampaignValidationError::MissingMissionProfile {
                        layer,
                        mission_index,
                    })? as usize;
            let profile = profiles.missions.get(profile_index).ok_or(
                ReplayCampaignValidationError::MissionProfileOutOfRange {
                    layer,
                    mission_index,
                    profile_index: profile_index as u32,
                    profile_count: profiles.missions.len(),
                },
            )?;
            if ares != profile.ares_state_succeeded {
                return Err(
                    ReplayCampaignValidationError::MissionAresOverrideNotAuthored {
                        layer,
                        mission_index,
                        ares,
                        authored: profile.ares_state_succeeded,
                    },
                );
            }
        }
    }

    let deferred_content_checks =
        validate_production(campaign, layer, limits, &gang, &reservists, &team)?;
    validate_history(campaign, layer, limits)?;

    Ok((required_mission_index, deferred_content_checks))
}

fn validate_optional_mission_index(
    index: Option<usize>,
    field: CampaignIndexField,
    length: usize,
    layer: CampaignLayer,
) -> Result<(), ReplayCampaignValidationError> {
    if let Some(index) = index
        && index >= length
    {
        return Err(ReplayCampaignValidationError::IndexOutOfRange {
            layer,
            field,
            index,
            length,
        });
    }
    Ok(())
}

fn validate_index_collection(
    indices: &[usize],
    collection: CampaignCollection,
    field: CampaignIndexField,
    target_length: usize,
    layer: CampaignLayer,
    limits: &ReplayAdmissionLimits,
) -> Result<BTreeSet<usize>, ReplayCampaignValidationError> {
    check_collection_limit(
        layer,
        collection,
        indices.len(),
        limits.max_campaign_collection_entries,
    )?;
    let mut unique = BTreeSet::new();
    for &index in indices {
        if index >= target_length {
            return Err(ReplayCampaignValidationError::IndexOutOfRange {
                layer,
                field,
                index,
                length: target_length,
            });
        }
        if !unique.insert(index) {
            return Err(ReplayCampaignValidationError::DuplicateIndex {
                layer,
                collection,
                index,
            });
        }
    }
    Ok(unique)
}

fn validate_disjoint(
    left: &BTreeSet<usize>,
    right: &BTreeSet<usize>,
    left_kind: CampaignCollection,
    right_kind: CampaignCollection,
    layer: CampaignLayer,
) -> Result<(), ReplayCampaignValidationError> {
    if let Some(&index) = left.intersection(right).next() {
        return Err(ReplayCampaignValidationError::OverlappingIndex {
            layer,
            left: left_kind,
            right: right_kind,
            index,
        });
    }
    Ok(())
}

fn validate_production(
    campaign: &CampaignPracticeReturnView<'_>,
    layer: CampaignLayer,
    limits: &ReplayAdmissionLimits,
    gang: &BTreeSet<usize>,
    reservists: &BTreeSet<usize>,
    team: &BTreeSet<usize>,
) -> Result<DeferredReplayCampaignContentChecks, ReplayCampaignValidationError> {
    let production_sectors = campaign.production_sectors;
    let characters = campaign.characters;
    if production_sectors.len() != CANONICAL_PRODUCTION_TYPES.len() {
        return Err(ReplayCampaignValidationError::ProductionSectorCount {
            layer,
            observed: production_sectors.len(),
            expected: CANONICAL_PRODUCTION_TYPES.len(),
        });
    }

    let mut total_points = 0usize;
    let mut total_occupants = 0usize;
    let mut deferred = DeferredReplayCampaignContentChecks::default();
    let mut occupant_identities = BTreeSet::new();
    for (slot, (sector, expected_type)) in production_sectors
        .iter()
        .zip(CANONICAL_PRODUCTION_TYPES)
        .enumerate()
    {
        if sector.prod_type != expected_type {
            return Err(ReplayCampaignValidationError::ProductionSectorType {
                layer,
                slot,
                actual: sector.prod_type,
                expected: expected_type,
            });
        }
        total_points = total_points.saturating_add(sector.production_points.len());
        check_collection_limit(
            layer,
            CampaignCollection::ProductionPoints,
            total_points,
            limits.max_campaign_production_points,
        )?;
        total_occupants = total_occupants.saturating_add(sector.occupants.len());
        check_collection_limit(
            layer,
            CampaignCollection::ProductionOccupants,
            total_occupants,
            limits.max_campaign_production_occupants,
        )?;

        if let Some(script_zone_index) = sector.script_zone {
            deferred.production_script_zone_references.push(
                DeferredProductionScriptZoneReference {
                    layer,
                    production_slot: slot,
                    script_zone_index,
                },
            );
        }
        for (point_index, point) in sector.production_points.iter().enumerate() {
            deferred.production_point_topology_references.push(
                DeferredProductionPointTopologyReference {
                    layer,
                    production_slot: slot,
                    point_index,
                    map_layer: point.layer,
                    sector: point.sector,
                },
            );
            validate_finite_coordinate(
                layer,
                slot,
                CampaignCoordinateField::ProductionPointX,
                point.x,
            )?;
            validate_finite_coordinate(
                layer,
                slot,
                CampaignCoordinateField::ProductionPointY,
                point.y,
            )?;
            if let Some(obstacle) = point.obstacle {
                deferred
                    .sight_obstacle_references
                    .push(DeferredSightObstacleReference {
                        layer,
                        production_slot: slot,
                        source: ProductionObstacleSource::Point,
                        source_index: point_index,
                        obstacle_index: obstacle.get(),
                    });
            }
        }
        for (occupant_index, occupant) in sector.occupants.iter().enumerate() {
            if occupant.pc_description_idx >= characters.len() {
                return Err(ReplayCampaignValidationError::IndexOutOfRange {
                    layer,
                    field: CampaignIndexField::ProductionOccupantCharacter,
                    index: occupant.pc_description_idx,
                    length: characters.len(),
                });
            }
            if !occupant_identities.insert(occupant.pc_description_idx) {
                return Err(ReplayCampaignValidationError::DuplicateIndex {
                    layer,
                    collection: CampaignCollection::ProductionOccupants,
                    index: occupant.pc_description_idx,
                });
            }
            if characters[occupant.pc_description_idx]
                .character_profile_idx
                .is_none()
            {
                return Err(
                    ReplayCampaignValidationError::ReferencedCharacterHasNoProfile {
                        layer,
                        character_index: occupant.pc_description_idx,
                    },
                );
            }
            // An occupant is captured outside the active gang/reservist/team
            // membership sets only if submitted state has conflicting PC
            // identities. Keep this explicit rather than silently adopting it.
            if !gang.contains(&occupant.pc_description_idx)
                && !reservists.contains(&occupant.pc_description_idx)
                && !team.contains(&occupant.pc_description_idx)
            {
                return Err(
                    ReplayCampaignValidationError::ProductionOccupantOutsideRoster {
                        layer,
                        character_index: occupant.pc_description_idx,
                    },
                );
            }
            validate_finite_coordinate(
                layer,
                slot,
                CampaignCoordinateField::ProductionOccupantX,
                occupant.x,
            )?;
            validate_finite_coordinate(
                layer,
                slot,
                CampaignCoordinateField::ProductionOccupantY,
                occupant.y,
            )?;
            if let Some(obstacle) = occupant.obstacle {
                deferred
                    .sight_obstacle_references
                    .push(DeferredSightObstacleReference {
                        layer,
                        production_slot: slot,
                        source: ProductionObstacleSource::Occupant,
                        source_index: occupant_index,
                        obstacle_index: obstacle.get(),
                    });
            }
        }
    }
    for (character_index, character) in characters.iter().enumerate() {
        if character.status.beam_me_index_in_sherwood >= 0 {
            deferred
                .sherwood_beam_me_references
                .push(DeferredSherwoodBeamMeReference {
                    layer,
                    character_index,
                    beam_me_index: character.status.beam_me_index_in_sherwood as u16,
                });
        }
    }
    Ok(deferred)
}

fn validate_finite_coordinate(
    layer: CampaignLayer,
    slot: usize,
    field: CampaignCoordinateField,
    value: f32,
) -> Result<(), ReplayCampaignValidationError> {
    if !value.is_finite() {
        return Err(
            ReplayCampaignValidationError::NonFiniteProductionCoordinate {
                layer,
                slot,
                field,
                bits: value.to_bits(),
            },
        );
    }
    Ok(())
}

fn validate_history(
    campaign: &CampaignPracticeReturnView<'_>,
    layer: CampaignLayer,
    limits: &ReplayAdmissionLimits,
) -> Result<(), ReplayCampaignValidationError> {
    if campaign.mission_attempt_sequence.checked_add(1).is_none() {
        return Err(ReplayCampaignValidationError::HistorySequenceExhausted {
            layer,
            sequence: campaign.mission_attempt_sequence,
        });
    }
    let mut total_attempts = 0usize;
    let mut total_recruited_characters = 0usize;
    let mut sequences = BTreeSet::new();
    let mut greatest_sequence = 0u64;
    for (mission_index, mission) in campaign.missions.iter().enumerate() {
        let history = mission.attempt_history();
        if history.schema_version() != CAMPAIGN_HISTORY_SCHEMA_VERSION {
            return Err(ReplayCampaignValidationError::HistorySchema {
                layer,
                mission_index,
                observed: history.schema_version(),
                expected: CAMPAIGN_HISTORY_SCHEMA_VERSION,
            });
        }
        total_attempts = total_attempts.saturating_add(history.attempts().len());
        check_collection_limit(
            layer,
            CampaignCollection::MissionAttempts,
            total_attempts,
            limits.max_campaign_history_attempts,
        )?;
        let mut previous = None;
        for attempt in history.attempts() {
            let sequence = attempt.sequence();
            if let Some(previous_sequence) = previous
                && sequence <= previous_sequence
            {
                return Err(ReplayCampaignValidationError::HistorySequenceOrder {
                    layer,
                    mission_index,
                    previous: previous_sequence,
                    sequence,
                });
            }
            if !sequences.insert(sequence) {
                return Err(ReplayCampaignValidationError::DuplicateHistorySequence {
                    layer,
                    sequence,
                });
            }
            if let Some(attestation) = attempt.achievement_attestation() {
                let results = attempt.achievements().ok_or(
                    ReplayCampaignValidationError::AttestationWithoutResults { layer, sequence },
                )?;
                if attestation
                    .policy()
                    .evaluate(attestation.context(), results)
                    != attestation.decision()
                {
                    return Err(
                        ReplayCampaignValidationError::AchievementAttestationDecisionMismatch {
                            layer,
                            sequence,
                        },
                    );
                }
            }
            if let Some(recruited_characters) = attempt.stats().recruited_characters.as_ref() {
                total_recruited_characters =
                    total_recruited_characters.saturating_add(recruited_characters.len());
                check_collection_limit(
                    layer,
                    CampaignCollection::AttemptRecruitedCharacters,
                    total_recruited_characters,
                    limits.max_campaign_collection_entries,
                )?;
                for (recruited_index, recruited) in recruited_characters.iter().enumerate() {
                    check_string_limit(
                        layer,
                        format!(
                            "missions[{mission_index}].attempts[{sequence}].recruited_characters[{recruited_index}]"
                        ),
                        &recruited.fallback,
                        limits.max_campaign_string_bytes,
                    )?;
                }
            }
            previous = Some(sequence);
            greatest_sequence = greatest_sequence.max(sequence);
        }
    }
    if campaign.mission_attempt_sequence != greatest_sequence {
        return Err(ReplayCampaignValidationError::HistorySequenceCounter {
            layer,
            observed: campaign.mission_attempt_sequence,
            expected: greatest_sequence,
        });
    }
    Ok(())
}

fn check_collection_limit(
    layer: CampaignLayer,
    collection: CampaignCollection,
    observed: usize,
    limit: usize,
) -> Result<(), ReplayCampaignValidationError> {
    if observed > limit {
        return Err(ReplayCampaignValidationError::CollectionLimit {
            layer,
            collection,
            observed,
            limit,
        });
    }
    Ok(())
}

fn check_string_limit(
    layer: CampaignLayer,
    field: String,
    value: &str,
    limit: usize,
) -> Result<(), ReplayCampaignValidationError> {
    if value.len() > limit {
        return Err(ReplayCampaignValidationError::StringLimit {
            layer,
            field,
            observed: value.len(),
            limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::achievement::{
        AchievementEvaluation, AchievementId, AchievementRunContext, AchievementUnlockPolicy,
        MissionAchievementState,
    };
    use robin_engine::campaign::PcDescription;
    use robin_engine::campaign_history::{MissionAttemptKey, MissionAttemptOutcome};
    use robin_engine::mission::Mission;
    use robin_engine::profiles::{CharacterProfile, CharacterProfileIdx, MissionProfile};
    use robin_engine::sector_production::{Occupant, Point};
    use std::path::Path;

    fn operator_datadir(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../datadirs")
            .join(name)
    }

    fn fixture() -> (ProfileManager, Campaign) {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 17,
            mission_filename: "MissionA".into(),
            location: MissionLocation::Nottingham,
            ..Default::default()
        });
        profiles.characters.push(CharacterProfile::default());

        let mut campaign = Campaign::default();
        campaign.missions.push(Mission {
            profile_idx: Some(0),
            ..Default::default()
        });
        campaign.characters.push(PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            ..Default::default()
        });
        campaign.gang_indices.push(0);
        campaign.mission_team_indices.push(0);
        campaign.current_mission_idx = Some(0);
        campaign.snapshot_with_simulation(0x1010, robin_engine::engine::SimConfig::default());
        (profiles, campaign)
    }

    fn validate(
        profiles: &ProfileManager,
        campaign: &Campaign,
    ) -> Result<ValidatedReplayCampaign, ReplayCampaignValidationError> {
        decode_and_validate_replay_campaign(
            &bitcode::encode(campaign),
            "MissionA",
            profiles,
            &ReplayAdmissionLimits::default(),
        )
    }

    fn load_legacy_profiles(datadir: &Path) -> ProfileManager {
        let title_case = datadir.join("Data");
        let upper_case = datadir.join("DATA");
        let data_root = match (title_case.is_dir(), upper_case.is_dir()) {
            (true, false) => title_case,
            (false, true) => upper_case,
            (true, true) => panic!(
                "real data fixture {} ambiguously contains both Data and DATA",
                datadir.display()
            ),
            (false, false) => panic!(
                "real data fixture {} contains neither Data nor DATA",
                datadir.display()
            ),
        };
        let path = data_root.join("Configuration/profile.cpf");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("read real profile fixture {}: {error}", path.display())
        });
        let mut file = robin_engine::sbfile::SbFile::from_owned_bytes(
            bytes,
            path.to_string_lossy().into_owned(),
        );
        let mut profiles = ProfileManager::new();
        profiles
            .load_all_legacy_cpf(&mut file)
            .unwrap_or_else(|error| panic!("decode real profile fixture: {error}"));
        profiles
    }

    fn campaign_for_real_mission(profiles: &ProfileManager, mission_id: &str) -> (Campaign, usize) {
        let mission_index = profiles
            .missions
            .iter()
            .position(|profile| profile.mission_filename.eq_ignore_ascii_case(mission_id))
            .unwrap_or_else(|| panic!("real profile fixture has no mission `{mission_id}`"));
        let mut campaign = Campaign::from_profiles(
            profiles,
            robin_engine::player_profile::DifficultyLevel::Medium,
        );
        campaign.current_mission_idx = Some(mission_index);
        campaign.mission_team_indices = campaign.gang_indices.clone();
        campaign.snapshot_with_simulation(0x5eed, robin_engine::engine::SimConfig::default());
        (campaign, mission_index)
    }

    #[test]
    #[ignore = "requires operator-mounted demo Leicester data"]
    fn real_demo_campaign_fixture_passes_structural_admission() {
        let datadir = operator_datadir("demo_leicester_ecoste");
        let profiles = load_legacy_profiles(&datadir);
        let (campaign, mission_index) = campaign_for_real_mission(&profiles, "Dem_Lei_MP");
        let bytes = bitcode::encode(&campaign);
        let admitted = decode_and_validate_replay_campaign(
            &bytes,
            "Dem_Lei_MP",
            &profiles,
            &ReplayAdmissionLimits::default(),
        )
        .expect("real demo campaign must pass phase one");
        assert_eq!(admitted.mission_index(), mission_index);
        assert_eq!(
            admitted.submitted_campaign_sha256(),
            <[u8; 32]>::from(Sha256::digest(&bytes))
        );
    }

    #[test]
    #[ignore = "requires operator-mounted full retail data"]
    fn real_full_hq_campaign_phase_two_accepts_exact_topology_and_rejects_forgery() {
        use robin_engine::level_data::{
            ChunkReader, LevelFormat, LoadedLevel, load_mission, load_proto_level,
        };
        use robin_engine::profiles::CivilianType;

        let datadir = operator_datadir("fullgame_linux");
        let profiles = load_legacy_profiles(&datadir);
        let (mut campaign, _) = campaign_for_real_mission(&profiles, "Sherwood");
        let proto_path = datadir.join("Data/Levels/sherwood.rhp");
        let proto_bytes = std::fs::read(&proto_path).expect("read real Sherwood proto");
        let mut proto_reader = ChunkReader::new(robin_engine::sbfile::SbFile::from_owned_bytes(
            proto_bytes,
            proto_path.to_string_lossy().into_owned(),
        ));
        let format = LevelFormat::detect(&proto_reader.peek_next_chunk().unwrap()).unwrap();
        let proto =
            load_proto_level(&mut proto_reader, format).expect("decode real Sherwood proto");

        let beggars = profiles
            .civilians
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                (profile.civilian_type == CivilianType::Beggar).then_some(index as u32)
            })
            .collect::<BTreeSet<_>>();
        let mission_path = datadir.join("Data/Levels/Sherwood.rhm");
        let mission_bytes = std::fs::read(&mission_path).expect("read real Sherwood mission");
        let mut mission_reader = ChunkReader::new(robin_engine::sbfile::SbFile::from_owned_bytes(
            mission_bytes,
            mission_path.to_string_lossy().into_owned(),
        ));
        let mission = load_mission(&mut mission_reader, format, &|index| {
            beggars.contains(&index)
        })
        .expect("decode real Sherwood mission");
        let loaded = LoadedLevel {
            proto,
            mission,
            diplomacy: None,
        };
        let metadata = derive_replay_campaign_approved_content_metadata(&loaded, &campaign)
            .expect("derive real approved Sherwood topology");
        let sector = metadata
            .production_sectors
            .first()
            .expect("real Sherwood has motion topology");
        let point = sector.polygon[0];
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: point.x,
                y: point.y,
                layer: sector.topology.map_layer,
                sector: sector.topology.sector,
                obstacle: None,
            });
        campaign.snapshot_with_simulation(0x5eed, robin_engine::engine::SimConfig::default());

        struct Resolver(ReplayCampaignApprovedContentMetadata);
        impl ReplayCampaignApprovedContentResolver for Resolver {
            fn approved_content_identity(&self) -> ReplayCampaignApprovedContentIdentity {
                ReplayCampaignApprovedContentIdentity {
                    build_manifest_sha256: [1; 32],
                    content_manifest_sha256: [2; 32],
                }
            }

            fn sherwood_campaign_metadata(&self) -> Option<&ReplayCampaignApprovedContentMetadata> {
                Some(&self.0)
            }
        }
        let resolver = Resolver(metadata);
        let validated = decode_and_validate_replay_campaign(
            &bitcode::encode(&campaign),
            "Sherwood",
            &profiles,
            &ReplayAdmissionLimits::default(),
        )
        .expect("real full HQ campaign phase one");
        validate_replay_campaign_approved_content(validated, &resolver)
            .expect("real full HQ campaign exact phase two");

        campaign.production_sectors[0].production_points[0].sector = u16::MAX;
        campaign.snapshot_with_simulation(0x5eed, robin_engine::engine::SimConfig::default());
        let forged = decode_and_validate_replay_campaign(
            &bitcode::encode(&campaign),
            "Sherwood",
            &profiles,
            &ReplayAdmissionLimits::default(),
        )
        .expect("forged topology remains a phase-two identity");
        assert!(matches!(
            validate_replay_campaign_approved_content(forged, &resolver),
            Err(ReplayCampaignContentValidationError::ProductionPointTopologyMissing { .. })
        ));
    }

    #[test]
    fn valid_campaign_hashes_exact_submitted_bytes() {
        let (profiles, campaign) = fixture();
        let bytes = bitcode::encode(&campaign);
        let validated = decode_and_validate_replay_campaign(
            &bytes,
            "MissionA",
            &profiles,
            &ReplayAdmissionLimits::default(),
        )
        .unwrap();
        assert_eq!(
            validated.submitted_campaign_sha256(),
            <[u8; 32]>::from(Sha256::digest(&bytes))
        );
        assert_eq!(validated.mission_index(), 0);
        assert_eq!(validated.mission_location(), MissionLocation::Nottingham);
    }

    #[test]
    fn rejects_campaign_and_mission_id_limits_before_decode() {
        let (profiles, campaign) = fixture();
        let bytes = bitcode::encode(&campaign);
        let limits = ReplayAdmissionLimits {
            max_campaign_bytes: bytes.len() - 1,
            ..Default::default()
        };
        assert!(matches!(
            decode_and_validate_replay_campaign(&bytes, "MissionA", &profiles, &limits),
            Err(ReplayCampaignValidationError::CampaignBytesLimit { .. })
        ));

        let limits = ReplayAdmissionLimits {
            max_mission_id_bytes: 0,
            ..Default::default()
        };
        assert!(matches!(
            decode_and_validate_replay_campaign(b"not bitcode", "MissionA", &profiles, &limits),
            Err(ReplayCampaignValidationError::HeaderMissionIdBytesLimit { .. })
        ));
    }

    #[test]
    fn no_deferred_content_consuming_path_is_checked() {
        let (profiles, campaign) = fixture();
        let validated = validate(&profiles, &campaign).unwrap();
        let (_campaign, mission_index, location, digest) =
            validated.try_into_playback_parts().unwrap();
        assert_eq!(mission_index, 0);
        assert_eq!(location, MissionLocation::Nottingham);
        assert_ne!(digest, [0; 32]);

        let (_, mut campaign) = fixture();
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: 1.0,
                y: 2.0,
                layer: 3,
                sector: 4,
                obstacle: None,
            });
        assert!(matches!(
            validate(&profiles, &campaign)
                .unwrap()
                .try_into_playback_parts(),
            Err(
                ReplayCampaignValidationError::DeferredContentValidationRequired {
                    point_topology: 1,
                    ..
                }
            )
        ));
    }

    #[test]
    fn rejects_missing_and_out_of_range_mission_profiles_before_deref() {
        let (profiles, mut campaign) = fixture();
        campaign.missions[0].profile_idx = None;
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::MissingMissionProfile { .. })
        ));

        campaign.missions[0].profile_idx = Some(99);
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::MissionProfileOutOfRange { .. })
        ));
    }

    #[test]
    fn rejects_header_current_and_snapshot_mismatch() {
        let (profiles, mut campaign) = fixture();
        campaign.current_mission_idx = None;
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::CurrentMissionMismatch {
                layer: CampaignLayer::Current,
                ..
            })
        ));

        let (_, mut campaign) = fixture();
        campaign.pre_mission_was_preselected = true;
        campaign
            .pre_mission_snapshot
            .as_mut()
            .unwrap()
            .pre_mission_was_preselected = true;
        campaign
            .pre_mission_snapshot
            .as_mut()
            .unwrap()
            .current_mission_idx = None;
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::CurrentMissionMismatch {
                layer: CampaignLayer::PreMissionSnapshot,
                ..
            })
        ));
    }

    #[test]
    fn rejects_duplicate_disjoint_and_out_of_range_character_identities() {
        let (profiles, mut campaign) = fixture();
        campaign.gang_indices.push(0);
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::DuplicateIndex {
                collection: CampaignCollection::Gang,
                ..
            })
        ));

        let (_, mut campaign) = fixture();
        campaign.reservist_indices.push(0);
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::OverlappingIndex { .. })
        ));

        let (_, mut campaign) = fixture();
        campaign.mission_team_indices[0] = 7;
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::IndexOutOfRange {
                field: CampaignIndexField::MissionTeamCharacter,
                ..
            })
        ));
    }

    #[test]
    fn rejects_production_shape_occupants_and_nonfinite_coordinates() {
        let (profiles, mut campaign) = fixture();
        campaign.production_sectors.pop();
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::ProductionSectorCount { .. })
                | Err(ReplayCampaignValidationError::CollectionLimit { .. })
        ));

        let (_, mut campaign) = fixture();
        campaign.production_sectors[0].occupants.push(Occupant {
            pc_description_idx: 99,
            ..Default::default()
        });
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::IndexOutOfRange {
                field: CampaignIndexField::ProductionOccupantCharacter,
                ..
            })
        ));

        let (_, mut campaign) = fixture();
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: f32::NAN,
                ..Default::default()
            });
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::NonFiniteProductionCoordinate { .. })
        ));
    }

    #[test]
    fn rejects_relic_ares_history_and_collection_limits() {
        let (mut profiles, mut campaign) = fixture();
        campaign.collected_relics = vec![12, 12];
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::DuplicateRelicIdentity { .. })
        ));

        let (_, mut campaign) = fixture();
        profiles.missions[0].ares_sensible = true;
        campaign.ares = 10;
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::AresOutOfRange { .. })
        ));

        let (_, campaign) = fixture();
        let mut wire = serde_json::to_value(campaign).unwrap();
        wire["missions"][0]["attempt_history"]["schema_version"] = serde_json::json!(999);
        let mut campaign: Campaign = serde_json::from_value(wire).unwrap();
        campaign.snapshot_with_simulation(0x1010, robin_engine::engine::SimConfig::default());
        let error = validate(&profiles, &campaign).unwrap_err();
        assert!(
            matches!(
                &error,
                ReplayCampaignValidationError::HistorySchema {
                    layer: CampaignLayer::Current,
                    ..
                }
            ),
            "unexpected history schema rejection: {error:?}"
        );

        let (_, campaign) = fixture();
        let limits = ReplayAdmissionLimits {
            max_campaign_characters: 0,
            ..Default::default()
        };
        assert!(matches!(
            decode_and_validate_replay_campaign(
                &bitcode::encode(&campaign),
                "MissionA",
                &profiles,
                &limits,
            ),
            Err(ReplayCampaignValidationError::CollectionLimit {
                collection: CampaignCollection::Characters,
                ..
            })
        ));
    }

    #[test]
    fn rejects_unapproved_mission_ares_override_and_exhausted_history_sequence() {
        let (mut profiles, mut campaign) = fixture();
        profiles.missions[0].ares_state_succeeded = 2;
        campaign.missions[0].ares_state_override = Some(3);
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(
                ReplayCampaignValidationError::MissionAresOverrideNotAuthored {
                    layer: CampaignLayer::Current,
                    mission_index: 0,
                    ..
                }
            )
        ));

        let (_, mut campaign) = fixture();
        campaign.mission_attempt_sequence = u64::MAX;
        campaign.snapshot_with_simulation(0x1010, robin_engine::engine::SimConfig::default());
        let error = validate(&profiles, &campaign).unwrap_err();
        assert!(
            matches!(
                &error,
                ReplayCampaignValidationError::HistorySequenceExhausted {
                    layer: CampaignLayer::Current,
                    sequence: u64::MAX,
                }
            ),
            "unexpected exhausted sequence rejection: {error:?}"
        );
    }

    #[test]
    fn requires_the_complete_canonical_approved_mission_profile_identity() {
        let (mut profiles, campaign) = fixture();
        profiles.missions.push(MissionProfile {
            id: 18,
            mission_filename: "MissionB".into(),
            ..MissionProfile::default()
        });
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::MissionProfileSetSize {
                observed: 1,
                approved: 2,
                ..
            })
        ));

        let (mut profiles, campaign) = fixture();
        profiles.missions[0].missions_required_to_be_done = vec![999];
        assert!(
            validate(&profiles, &campaign).is_ok(),
            "trusted edition profiles may retain unavailable cross-edition prerequisites"
        );

        let (mut profiles, campaign) = fixture();
        profiles.missions.push(MissionProfile {
            id: 17,
            mission_filename: "MissionB".into(),
            ..MissionProfile::default()
        });
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(
                ReplayCampaignValidationError::ApprovedMissionProfileIdDuplicate { profile_id: 17 }
            )
        ));
    }

    #[test]
    fn recomputes_serialized_achievement_attestation_decisions() {
        let (profiles, mut campaign) = fixture();
        let mut achievements = MissionAchievementState::from_mission_start();
        achievements
            .record_evaluation(AchievementId::CleanHands, AchievementEvaluation::Earned)
            .unwrap();
        let results = *achievements.finalize_success();
        campaign.record_mission_attempt(
            0,
            MissionAttemptOutcome::Won,
            None,
            Some(42),
            10,
            robin_engine::engine::SimConfig::default(),
            &robin_engine::mission_stat::MissionStat::default(),
            Some(results),
        );
        campaign
            .attest_mission_achievement_attempt(
                MissionAttemptKey {
                    campaign_run_id: 42,
                    sequence: 1,
                },
                AchievementUnlockPolicy::default(),
                AchievementRunContext::default(),
                &profiles,
            )
            .unwrap();
        validate(&profiles, &campaign).unwrap();

        let mut forged = serde_json::to_value(&campaign).unwrap();
        forged["missions"][0]["attempt_history"]["attempts"][0]["achievement_attestation"]["decision"]
            ["eligible_earned"] = serde_json::json!(0);
        let forged: Campaign = serde_json::from_value(forged).unwrap();
        assert!(matches!(
            validate(&profiles, &forged),
            Err(
                ReplayCampaignValidationError::AchievementAttestationDecisionMismatch {
                    layer: CampaignLayer::Current,
                    sequence: 1,
                }
            )
        ));
    }

    #[test]
    fn rejects_inconsistent_restart_checkpoint_and_depth_limit() {
        let (profiles, mut campaign) = fixture();
        campaign.pre_mission_rng_seed = None;
        assert!(matches!(
            validate(&profiles, &campaign),
            Err(ReplayCampaignValidationError::CheckpointShape { .. })
        ));

        let (_, campaign) = fixture();
        let limits = ReplayAdmissionLimits {
            max_campaign_snapshot_depth: 0,
            ..Default::default()
        };
        assert!(matches!(
            decode_and_validate_replay_campaign(
                &bitcode::encode(&campaign),
                "MissionA",
                &profiles,
                &limits,
            ),
            Err(ReplayCampaignValidationError::SnapshotDepthLimit { .. })
        ));
    }

    #[test]
    fn rejects_oversized_campaign_owned_strings() {
        let (profiles, mut campaign) = fixture();
        campaign.characters[0].status.name = "six!!!".into();
        let limits = ReplayAdmissionLimits {
            max_campaign_string_bytes: 5,
            ..Default::default()
        };
        assert!(matches!(
            decode_and_validate_replay_campaign(
                &bitcode::encode(&campaign),
                "MissionA",
                &profiles,
                &limits,
            ),
            Err(ReplayCampaignValidationError::StringLimit { .. })
        ));
    }

    #[test]
    fn unresolved_content_references_fail_closed_and_retain_identity() {
        let (profiles, mut campaign) = fixture();
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: 1.0,
                y: 2.0,
                layer: 3,
                sector: 4,
                obstacle: None,
            });
        let validated = validate(&profiles, &campaign).unwrap();
        assert_eq!(
            validated
                .deferred_content_checks()
                .production_point_topology_references,
            vec![DeferredProductionPointTopologyReference {
                layer: CampaignLayer::Current,
                production_slot: 0,
                point_index: 0,
                map_layer: 3,
                sector: 4,
            }]
        );
        assert!(matches!(
            validated.require_no_deferred_content_references(),
            Err(
                ReplayCampaignValidationError::DeferredContentValidationRequired {
                    point_topology: 1,
                    ..
                }
            )
        ));
    }

    struct ApprovedFixture(ReplayCampaignApprovedContentMetadata);

    impl ReplayCampaignApprovedContentResolver for ApprovedFixture {
        fn approved_content_identity(&self) -> ReplayCampaignApprovedContentIdentity {
            approved_identity()
        }

        fn sherwood_campaign_metadata(&self) -> Option<&ReplayCampaignApprovedContentMetadata> {
            Some(&self.0)
        }
    }

    fn approved_identity() -> ReplayCampaignApprovedContentIdentity {
        ReplayCampaignApprovedContentIdentity {
            build_manifest_sha256: [0x42; 32],
            content_manifest_sha256: [0x24; 32],
        }
    }

    struct ApprovedIdentityWithoutMetadata;

    impl ReplayCampaignApprovedContentResolver for ApprovedIdentityWithoutMetadata {
        fn approved_content_identity(&self) -> ReplayCampaignApprovedContentIdentity {
            approved_identity()
        }

        fn sherwood_campaign_metadata(&self) -> Option<&ReplayCampaignApprovedContentMetadata> {
            None
        }
    }

    #[test]
    fn approved_content_phase_needs_no_sherwood_metadata_without_deferred_references() {
        let (profiles, campaign) = fixture();
        let validated = validate(&profiles, &campaign).unwrap();
        assert!(validated.deferred_content_checks().is_empty());

        let approved =
            validate_replay_campaign_approved_content(validated, &ApprovedIdentityWithoutMetadata)
                .unwrap();
        assert_eq!(approved.approved_identity(), approved_identity());
        assert_eq!(approved.validated().mission_index(), 0);
    }

    struct UnboundApprovedFixture(ReplayCampaignApprovedContentMetadata);

    impl ReplayCampaignApprovedContentResolver for UnboundApprovedFixture {
        fn approved_content_identity(&self) -> ReplayCampaignApprovedContentIdentity {
            ReplayCampaignApprovedContentIdentity {
                build_manifest_sha256: [0; 32],
                content_manifest_sha256: [0x24; 32],
            }
        }

        fn sherwood_campaign_metadata(&self) -> Option<&ReplayCampaignApprovedContentMetadata> {
            Some(&self.0)
        }
    }

    fn square() -> Vec<ReplayCampaignApprovedMapPoint> {
        vec![
            ReplayCampaignApprovedMapPoint { x: 0.0, y: 0.0 },
            ReplayCampaignApprovedMapPoint { x: 8.0, y: 0.0 },
            ReplayCampaignApprovedMapPoint { x: 8.0, y: 8.0 },
            ReplayCampaignApprovedMapPoint { x: 0.0, y: 8.0 },
        ]
    }

    #[test]
    fn approved_content_phase_checks_ordered_static_and_topology_identities() {
        let (profiles, mut campaign) = fixture();
        let topology = ReplayCampaignProductionPointTopology {
            map_layer: 3,
            sector: 4,
        };
        campaign.production_sectors[0].script_zone = Some(0);
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: 1.0,
                y: 2.0,
                layer: topology.map_layer,
                sector: topology.sector,
                obstacle: robin_engine::sight_obstacle::SightObstacleIndex::new(0),
            });
        campaign.characters[0].status.beam_me_index_in_sherwood = 0;
        let validated = validate(&profiles, &campaign).unwrap();
        let approved = ApprovedFixture(ReplayCampaignApprovedContentMetadata {
            static_sight_obstacles: vec![ReplayCampaignApprovedStaticObstacle {
                projection_topology: Some(topology),
                projected_polygon: square(),
            }],
            script_zones: vec![ReplayCampaignApprovedScriptZone {
                production_slot: Some(0),
            }],
            beam_mes: vec![ReplayCampaignApprovedBeamMe {
                map_layer: 0,
                sector: 0,
            }],
            production_sectors: vec![ReplayCampaignApprovedSector {
                topology,
                polygon: square(),
            }],
        });
        let token = validate_replay_campaign_approved_content(validated, &approved).unwrap();
        assert_eq!(token.validated().mission_index(), 0);
        assert_eq!(token.approved_identity(), approved_identity());
        let (_campaign, mission_index, location, _digest, identity) = token.into_playback_parts();
        assert_eq!(mission_index, 0);
        assert_eq!(location, MissionLocation::Nottingham);
        assert_eq!(identity, approved_identity());

        let validated = validate(&profiles, &campaign).unwrap();
        let mut wrong = approved.0.clone();
        wrong.static_sight_obstacles[0].projection_topology =
            Some(ReplayCampaignProductionPointTopology {
                map_layer: 9,
                sector: 4,
            });
        assert!(matches!(
            validate_replay_campaign_approved_content(validated, &ApprovedFixture(wrong)),
            Err(
                ReplayCampaignContentValidationError::ProductionPointObstacleTopologyMismatch { .. }
            )
        ));
    }

    #[test]
    fn approved_content_phase_checks_occupant_obstacle_projection_polygon() {
        let (profiles, mut campaign) = fixture();
        campaign.production_sectors[0].occupants.push(Occupant {
            pc_description_idx: 0,
            x: 99.0,
            y: 99.0,
            obstacle: robin_engine::sight_obstacle::SightObstacleIndex::new(0),
        });
        let validated = validate(&profiles, &campaign).unwrap();
        let approved = ApprovedFixture(ReplayCampaignApprovedContentMetadata {
            static_sight_obstacles: vec![ReplayCampaignApprovedStaticObstacle {
                projection_topology: Some(ReplayCampaignProductionPointTopology {
                    map_layer: 3,
                    sector: 4,
                }),
                projected_polygon: square(),
            }],
            script_zones: Vec::new(),
            beam_mes: Vec::new(),
            production_sectors: Vec::new(),
        });
        assert!(matches!(
            validate_replay_campaign_approved_content(validated, &approved),
            Err(ReplayCampaignContentValidationError::ProductionOccupantOutsideObstacle { .. })
        ));
    }

    #[test]
    fn approved_content_phase_rejects_unbound_identity_before_metadata_use() {
        let (profiles, campaign) = fixture();
        let validated = validate(&profiles, &campaign).unwrap();
        let resolver = UnboundApprovedFixture(ReplayCampaignApprovedContentMetadata {
            static_sight_obstacles: Vec::new(),
            script_zones: Vec::new(),
            beam_mes: Vec::new(),
            production_sectors: Vec::new(),
        });
        assert!(matches!(
            validate_replay_campaign_approved_content(validated, &resolver),
            Err(ReplayCampaignContentValidationError::InvalidApprovedContentIdentity { .. })
        ));
    }

    #[test]
    fn approved_content_phase_bounds_aggregate_containment_work() {
        let (profiles, mut campaign) = fixture();
        let topology = ReplayCampaignProductionPointTopology {
            map_layer: 3,
            sector: 4,
        };
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: 1.0,
                y: 2.0,
                layer: topology.map_layer,
                sector: topology.sector,
                obstacle: None,
            });
        let validated = validate(&profiles, &campaign).unwrap();
        let approved = ApprovedFixture(ReplayCampaignApprovedContentMetadata {
            static_sight_obstacles: Vec::new(),
            script_zones: Vec::new(),
            beam_mes: Vec::new(),
            production_sectors: vec![ReplayCampaignApprovedSector {
                topology,
                polygon: square(),
            }],
        });
        assert!(matches!(
            validate_replay_campaign_approved_content_with_work_limit(validated, &approved, 3),
            Err(
                ReplayCampaignContentValidationError::ApprovedContainmentWorkLimit {
                    attempted: 4,
                    limit: 3,
                }
            )
        ));
    }

    #[test]
    fn approved_content_phase_reports_corrupt_deferred_indices_without_panicking() {
        let (profiles, mut campaign) = fixture();
        let topology = ReplayCampaignProductionPointTopology {
            map_layer: 3,
            sector: 4,
        };
        campaign.production_sectors[0]
            .production_points
            .push(Point {
                x: 1.0,
                y: 2.0,
                layer: topology.map_layer,
                sector: topology.sector,
                obstacle: None,
            });
        let mut validated = validate(&profiles, &campaign).unwrap();
        validated
            .deferred_content_checks
            .production_point_topology_references[0]
            .point_index = usize::MAX;
        let approved = ApprovedFixture(ReplayCampaignApprovedContentMetadata {
            static_sight_obstacles: Vec::new(),
            script_zones: Vec::new(),
            beam_mes: Vec::new(),
            production_sectors: vec![ReplayCampaignApprovedSector {
                topology,
                polygon: square(),
            }],
        });
        assert!(matches!(
            validate_replay_campaign_approved_content(validated, &approved),
            Err(
                ReplayCampaignContentValidationError::DeferredTopologyPointOutOfRange {
                    point_count: 1,
                    ..
                }
            )
        ));
    }

    // Intentionally no in-process `usize::MAX`/huge-Vec-length fixture here.
    // bitcode can allocate before returning to this validator; that adversary
    // belongs in the verifier-child subprocess suite under a memory ceiling.
}
