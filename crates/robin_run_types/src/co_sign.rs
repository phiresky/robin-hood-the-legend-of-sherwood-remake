//! Fixed-size leaderboard co-sign request relayed by multiplayer peers.

use serde::{Deserialize, Serialize};

use crate::{Digest32, Validate, ValidationError};

/// Domain for the one fixed-size payload accepted by the leaderboard identity
/// key for participant and campaign-controller co-signatures.
pub const LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/co-sign-payload\0";
/// The payload consists of this domain, a one-byte purpose tag, the replay
/// session digest, the authoritative submission-offer digest, and the exact
/// purpose-specific document digest.
pub const LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1: usize =
    LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1.len() + 1 + Digest32::LENGTH * 3;

/// Closed purpose set for the only leaderboard co-signing payload accepted by
/// the durable game identity. The numeric tag is part of the fixed signature
/// contract; new purposes require a new contract version rather than reusing a
/// tag.
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
pub enum LeaderboardCoSignPurposeV1 {
    CampaignContinuation,
    Submission,
}

impl LeaderboardCoSignPurposeV1 {
    const fn signing_tag(self) -> u8 {
        match self {
            Self::CampaignContinuation => 1,
            Self::Submission => 2,
        }
    }
}

/// Deterministic, server-reconstructible identity for one co-sign operation.
///
/// The replay session prevents cross-session use while the digest of the exact
/// server-issued offer binds its one-use upload challenge and nonce. Callers
/// must derive this value from an authoritative validated offer; it is never a
/// client-selected sequence number. `robin_run_protocol` owns that derivation
/// because the offer document is a service contract.
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
#[serde(deny_unknown_fields)]
pub struct LeaderboardCoSignInstanceV1 {
    pub purpose: LeaderboardCoSignPurposeV1,
    pub replay_session_id: Digest32,
    pub submission_offer_sha256: Digest32,
}

impl Validate for LeaderboardCoSignInstanceV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.replay_session_id.is_zero() || self.submission_offer_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "leaderboard_co_sign.instance",
            });
        }
        Ok(())
    }
}

/// The exact request signed by a local player or a remote multiplayer
/// participant. `signing_bytes` is deliberately fixed-size and is the sole
/// co-signature payload; neither native nor browser identities expose a raw
/// signing operation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bitcode::Encode, bitcode::Decode,
)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardCoSignRequestV1 {
    pub instance: LeaderboardCoSignInstanceV1,
    pub run_digest: Digest32,
}

impl LeaderboardCoSignRequestV1 {
    pub fn signing_bytes(
        &self,
    ) -> Result<[u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1], ValidationError> {
        self.validate()?;
        let mut bytes = [0_u8; LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1];
        let mut offset = 0;
        let mut append = |part: &[u8]| {
            let end = offset + part.len();
            bytes[offset..end].copy_from_slice(part);
            offset = end;
        };
        append(LEADERBOARD_CO_SIGN_PAYLOAD_DOMAIN_V1);
        append(&[self.instance.purpose.signing_tag()]);
        append(self.instance.replay_session_id.as_bytes());
        append(self.instance.submission_offer_sha256.as_bytes());
        append(self.run_digest.as_bytes());
        debug_assert_eq!(offset, LEADERBOARD_CO_SIGN_PAYLOAD_LENGTH_V1);
        Ok(bytes)
    }
}

impl Validate for LeaderboardCoSignRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.instance.validate()?;
        crate::validation::nonzero("leaderboard_co_sign.run_digest", &self.run_digest)?;
        Ok(())
    }
}
