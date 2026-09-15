//! Plain run-identity data types shared by the deterministic engine and the
//! leaderboard wire protocol.
//!
//! `robin_engine` uses the board simulation policy and replay seat transcript
//! types, while `robin_run_protocol` builds the signed submission,
//! verification and query contracts on top of them and re-exports every item
//! unchanged. This crate deliberately has no signing, networking, service or
//! engine dependencies. Names, field order and serde attributes are wire
//! contracts: moving a type here must never change its encoding.

pub mod artifact;
pub mod canonical;
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
pub use content::OfficialContentEditionV1;
pub use digest::{
    Digest32, HexError, OpaqueId, PublicKey32, Signature64, SimulationSeed64, SimulationSeedError,
};
pub use ruleset::{
    BoardSimulationPolicyV1, RANKED_SIMULATION_POLICY_VERSION_V1, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, RankedSimulationPresetV1,
};
pub use session::{
    MAX_PARTICIPANT_INSTANCES_V1, MAX_REPLAY_SEATS_V1, ReplaySeatLifecycleEventV1,
    ReplaySeatLifecycleKindV1, ReplaySessionTranscriptV1,
};
pub use validation::{Validate, ValidationError};

/// Every explicitly named `V1` document carries this value on the wire.
pub const SCHEMA_VERSION_V1: u32 = 1;
/// Every explicitly named `V2` document carries this value on the wire.
pub const SCHEMA_VERSION_V2: u32 = 2;
