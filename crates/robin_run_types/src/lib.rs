//! Plain run-identity data types shared by the deterministic engine and the
//! leaderboard wire protocol.
//!
//! `robin_engine` binds ranked simulation inputs, replay seat transcripts and
//! multiplayer co-sign requests to these documents, while
//! `robin_run_protocol` builds the signed submission, verification and query
//! contracts on top of them and re-exports every item unchanged. This crate
//! deliberately has no signing, networking, service or engine dependencies.
//! Names, field order and serde/bitcode attributes are wire contracts: moving
//! a type here must never change its encoding, signing bytes or digest.

pub mod artifact;
pub mod bitcode_value;
pub mod canonical;
pub mod co_sign;
pub mod content;
pub mod digest;
pub mod ruleset;
pub mod session;
pub mod validation;

pub use artifact::ArtifactRefV1;
pub use canonical::{
    CanonicalDocument, CanonicalDocumentError, CanonicalError, CanonicalValue, DomainSignedClaim,
    canonical_json_bytes,
};
pub use co_sign::{
    LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1, LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1,
    LeaderboardCoSignInstanceV1, LeaderboardCoSignPurposeV1, LeaderboardCoSignRequestV1,
};
pub use content::{
    ContentClosureKindV1, ContentManifestV1, OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1,
    OfficialContentEditionV1, OfficialContentSubjectV1, ResourceLocaleRootV1,
    SIMULATION_CONTENT_COMPONENT_MEDIA_TYPE_V1, SimulationContentComponentDocumentV1,
    SimulationContentComponentKindV1, SimulationContentComponentV1, SimulationSpeechTimingSourceV1,
};
pub use digest::{
    ChallengeNonce32, Digest32, HexError, OpaqueId, PublicKey32, Signature64, SimulationSeed64,
    SimulationSeedError,
};
pub use ruleset::{
    RANKED_SIMULATION_POLICY_VERSION_V1, RankedSimulationDifficultyV1, RankedSimulationPolicyV1,
    RankedSimulationPresetV1, RulesConfigIdentityV1,
};
pub use session::{
    MAX_PARTICIPANT_INSTANCES_V1, MAX_REPLAY_SEATS_V1, PreparedMissionInputsSealV1,
    RANKED_CAMPAIGN_MEDIA_TYPE_V1, ReplaySeatLifecycleEventV1, ReplaySeatLifecycleKindV1,
    ReplaySessionTranscriptV1, SpeechTimingAuthorityV1,
};
pub use validation::{Validate, ValidationError};

/// Every explicitly named `V1` document carries this value on the wire.
pub const SCHEMA_VERSION_V1: u32 = 1;
