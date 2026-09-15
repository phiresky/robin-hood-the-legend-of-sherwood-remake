//! Upload challenges, signed replay submissions, usernames and input provenance.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    ArtifactRefV1, BoardMetricV1, ChallengeNonce32, OpaqueId, PublicKey32, Signature64, Validate,
    ValidationError,
};

pub const SUBMISSION_SIGNATURE_DOMAIN_V2: &[u8] = b"robinhood/leaderboards/2/submission\0";
pub const USERNAME_UPDATE_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/username-update\0";

/// The one wire/storage format accepted for ranked replays. JSONL and older
/// Rust replay containers are local developer formats, not protocol lanes.
pub const RANKED_REPLAY_MEDIA_TYPE_V1: &str = "application/x-robin-rhrec+compact";

/// Request for a one-use replay upload challenge bound to one identity key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadChallengeRequestV2 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
}

impl Validate for UploadChallengeRequestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "UploadChallengeRequestV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::nonzero("upload_challenge_request.public_key", &self.public_key)
    }
}

/// Server-authored one-use challenge embedded in exactly one signed submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadChallengeV1 {
    pub schema_version: u32,
    pub upload_challenge_id: OpaqueId,
    pub upload_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
}

impl Validate for UploadChallengeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UploadChallengeV1", self.schema_version)?;
        crate::validation::nonzero(
            "upload_challenge.upload_challenge_nonce",
            &self.upload_challenge_nonce,
        )?;
        crate::validation::nonzero(
            "upload_challenge.expires_at_unix_ms",
            &self.expires_at_unix_ms,
        )
    }
}

/// A one-use challenge dedicated to a mutable username update.
///
/// It is intentionally a different namespace from replay upload challenges,
/// preventing a challenge minted for one operation from authorizing the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameChallengeV1 {
    pub schema_version: u32,
    pub username_challenge_id: OpaqueId,
    pub username_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
}

impl Validate for UsernameChallengeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UsernameChallengeV1", self.schema_version)?;
        crate::validation::nonzero(
            "username_challenge.username_challenge_nonce",
            &self.username_challenge_nonce,
        )?;
        crate::validation::nonzero(
            "username_challenge.expires_at_unix_ms",
            &self.expires_at_unix_ms,
        )
    }
}

/// Request for a one-use username-update challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameChallengeRequestV1 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
}

impl Validate for UsernameChallengeRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UsernameChallengeRequestV1", self.schema_version)?;
        crate::validation::nonzero("username_challenge_request.public_key", &self.public_key)
    }
}

#[derive(Serialize)]
struct UsernameUpdateSignable<'a> {
    schema_version: u32,
    username_challenge_id: &'a OpaqueId,
    username_challenge_nonce: ChallengeNonce32,
    public_key: PublicKey32,
    username: &'a str,
}

/// Separately signed mutable username operation. Usernames are intentionally
/// not part of run submissions and are not required to be unique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameUpdateEnvelopeV1 {
    pub schema_version: u32,
    pub username_challenge_id: OpaqueId,
    pub username_challenge_nonce: ChallengeNonce32,
    pub public_key: PublicKey32,
    pub username: String,
    pub signature: Signature64,
}

impl UsernameUpdateEnvelopeV1 {
    /// Validate every signed claim while intentionally ignoring the signature
    /// field. This is the safe pre-signing entry point for identity bridges.
    pub fn validate_signing_claim(&self) -> Result<(), ValidationError> {
        crate::validation::schema("UsernameUpdateEnvelopeV1", self.schema_version)?;
        crate::validation::nonzero(
            "username_update.username_challenge_nonce",
            &self.username_challenge_nonce,
        )?;
        crate::validation::nonzero("username_update.public_key", &self.public_key)?;
        crate::validation::text("username_update.username", &self.username, 48)
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        crate::canonical::domain_separated_bytes(
            USERNAME_UPDATE_SIGNATURE_DOMAIN_V1,
            &UsernameUpdateSignable {
                schema_version: self.schema_version,
                username_challenge_id: &self.username_challenge_id,
                username_challenge_nonce: self.username_challenge_nonce,
                public_key: self.public_key,
                username: &self.username,
            },
        )
    }
}

impl Validate for UsernameUpdateEnvelopeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.validate_signing_claim()?;
        crate::validation::nonzero("username_update.signature", &self.signature)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayArtifactV1 {
    /// Replay bytes are transferred separately; this is their exact identity
    /// and length, not an inline payload.
    pub artifact: ArtifactRefV1,
    pub replay_schema_version: u32,
}

impl ReplayArtifactV1 {
    /// Admission check for new uploads and verifier jobs. Stored runs keep
    /// their recorded schema and only need [`Validate`].
    pub fn validate_current_schema(&self) -> Result<(), ValidationError> {
        self.validate()?;
        if self.replay_schema_version != crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "replay.replay_schema_version",
            });
        }
        Ok(())
    }
}

impl Validate for ReplayArtifactV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.artifact.validate()?;
        if self.artifact.media_type != RANKED_REPLAY_MEDIA_TYPE_V1 {
            return Err(ValidationError::ClaimMismatch {
                field: "replay.artifact.media_type",
            });
        }
        if self.replay_schema_version == 0 {
            return Err(ValidationError::ClaimMismatch {
                field: "replay.replay_schema_version",
            });
        }
        Ok(())
    }
}

/// Controls only whether the public leaderboard shows the uploader's profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantPublicDisclosureV1 {
    NamedProfile,
    Anonymous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithmV1 {
    Ed25519,
}

/// One replay upload claimed by one identity key for one board and mission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionV2 {
    pub schema_version: u32,
    pub upload_challenge: UploadChallengeV1,
    pub uploader_public_key: PublicKey32,
    pub public_disclosure: ParticipantPublicDisclosureV1,
    pub board_id: OpaqueId,
    pub mission_id: String,
    pub replay: ReplayArtifactV1,
    pub requested_metrics: Vec<BoardMetricV1>,
}

impl Validate for SubmissionV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "SubmissionV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.upload_challenge.validate()?;
        crate::validation::nonzero("submission.uploader_public_key", &self.uploader_public_key)?;
        crate::validation::text("submission.mission_id", &self.mission_id, 256)?;
        self.replay.validate_current_schema()?;
        if self.requested_metrics.is_empty()
            || !crate::validation::strictly_sorted(&self.requested_metrics)
        {
            return Err(ValidationError::InvalidMetrics {
                field: "submission.requested_metrics",
            });
        }
        Ok(())
    }
}

/// A submission signed by its uploader. The server verifies the signature,
/// consumes the embedded challenge and queues the replay for verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedSubmissionV2 {
    pub schema_version: u32,
    pub submission: SubmissionV2,
    pub algorithm: SignatureAlgorithmV1,
    pub signature: Signature64,
}

impl SignedSubmissionV2 {
    /// Bytes the uploader signs: the domain-separated canonical submission.
    pub fn signing_bytes(
        submission: &SubmissionV2,
    ) -> Result<Vec<u8>, crate::canonical::CanonicalDocumentError> {
        submission.validate()?;
        Ok(crate::canonical::domain_separated_bytes(
            SUBMISSION_SIGNATURE_DOMAIN_V2,
            submission,
        )?)
    }

    /// Verify the uploader's Ed25519 signature over the exact submission.
    #[cfg(feature = "authentication")]
    pub fn verify_signature(&self) -> Result<(), crate::SignatureVerificationError> {
        let bytes = Self::signing_bytes(&self.submission)
            .map_err(|_| crate::SignatureVerificationError::InvalidSignature)?;
        crate::verify_ed25519_strict(
            self.submission.uploader_public_key.as_bytes(),
            self.signature.as_bytes(),
            &bytes,
        )
    }
}

impl Validate for SignedSubmissionV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "SignedSubmissionV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.submission.validate()?;
        crate::validation::nonzero("signed_submission.signature", &self.signature)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcomeV1 {
    Won,
    Lost,
    Interrupted,
}

/// Verifier-observed entry points which make a run ineligible for a public
/// verified board. The evidence is cumulative: once observed it cannot be
/// cleared later in the recording.
///
/// Replay resimulation proves a deterministic outcome under the board's rules
/// and our content. It does not prove that a human supplied the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputTaintKindV1 {
    HttpPlayerCommand,
    HttpSimulationStep,
    HttpStateMutation,
    ConsoleCommand,
    CheatCommand,
    HeadlessAutomation,
    ReplayPlayback,
    StateLoad,
    MissionRestart,
    DebugInputInjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputTaintV1 {
    pub kind: InputTaintKindV1,
    pub first_frame: u32,
}

/// Stable, deliberately coarse reason safe to expose in public API results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputIneligibilityReasonV1 {
    HttpAutomation,
    ConsoleUsed,
    CheatUsed,
    HeadlessAutomation,
    ReplayPlayback,
    StateLoaded,
    MissionRestarted,
    DebugInputInjection,
}

impl InputTaintKindV1 {
    pub const fn public_reason(self) -> InputIneligibilityReasonV1 {
        match self {
            Self::HttpPlayerCommand | Self::HttpSimulationStep | Self::HttpStateMutation => {
                InputIneligibilityReasonV1::HttpAutomation
            }
            Self::ConsoleCommand => InputIneligibilityReasonV1::ConsoleUsed,
            Self::CheatCommand => InputIneligibilityReasonV1::CheatUsed,
            Self::HeadlessAutomation => InputIneligibilityReasonV1::HeadlessAutomation,
            Self::ReplayPlayback => InputIneligibilityReasonV1::ReplayPlayback,
            Self::StateLoad => InputIneligibilityReasonV1::StateLoaded,
            Self::MissionRestart => InputIneligibilityReasonV1::MissionRestarted,
            Self::DebugInputInjection => InputIneligibilityReasonV1::DebugInputInjection,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputProvenanceStatusV1 {
    Rankable,
    Tainted { taints: Vec<InputTaintV1> },
}

impl InputProvenanceStatusV1 {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Self::Tainted { taints } = self
            && (taints.is_empty() || !taints.windows(2).all(|pair| pair[0].kind < pair[1].kind))
        {
            return Err(ValidationError::InvalidInputTaints);
        }
        Ok(())
    }

    pub const fn is_rankable(&self) -> bool {
        matches!(self, Self::Rankable)
    }

    /// Coarse, stable reasons suitable for public UI. Multiple low-level HTTP
    /// taints intentionally collapse to one automation reason.
    pub fn public_reasons(&self) -> Vec<InputIneligibilityReasonV1> {
        match self {
            Self::Rankable => Vec::new(),
            Self::Tainted { taints } => taints
                .iter()
                .map(|taint| taint.kind.public_reason())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Digest32;

    pub(crate) fn submission() -> SubmissionV2 {
        SubmissionV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            upload_challenge: UploadChallengeV1 {
                schema_version: crate::SCHEMA_VERSION_V1,
                upload_challenge_id: OpaqueId::new("challenge-1").unwrap(),
                upload_challenge_nonce: ChallengeNonce32::from_bytes([3; 32]),
                expires_at_unix_ms: 10,
            },
            uploader_public_key: PublicKey32::from_bytes([4; 32]),
            public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
            board_id: OpaqueId::new("demo-standard-normal").unwrap(),
            mission_id: "Dem_Lei_MP".into(),
            replay: ReplayArtifactV1 {
                artifact: ArtifactRefV1 {
                    sha256: Digest32::from_bytes([5; 32]),
                    byte_length: 99,
                    media_type: RANKED_REPLAY_MEDIA_TYPE_V1.into(),
                },
                replay_schema_version: crate::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            },
            requested_metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
        }
    }

    #[test]
    fn submission_requires_current_replay_schema_and_canonical_metrics() {
        assert!(submission().validate().is_ok());
        let mut stale = submission();
        stale.replay.replay_schema_version -= 1;
        assert!(stale.validate().is_err());
        // Stored runs keep their recorded schema; only admission requires the current one.
        assert!(stale.replay.validate().is_ok());
        assert!(stale.replay.validate_current_schema().is_err());
        let mut unsorted = submission();
        unsorted.requested_metrics.reverse();
        assert!(unsorted.validate().is_err());
        let mut wrong_media = submission();
        wrong_media.replay.artifact.media_type = "application/jsonl".into();
        assert!(wrong_media.validate().is_err());
    }

    #[cfg(feature = "authentication")]
    #[test]
    fn uploader_signature_binds_the_exact_submission() {
        use ed25519_dalek::{Signer as _, SigningKey};
        let key = SigningKey::from_bytes(&[9; 32]);
        let mut submission = submission();
        submission.uploader_public_key = PublicKey32::from_bytes(key.verifying_key().to_bytes());
        let signature = key.sign(&SignedSubmissionV2::signing_bytes(&submission).unwrap());
        let mut signed = SignedSubmissionV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            submission,
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes(signature.to_bytes()),
        };
        assert!(signed.validate().is_ok());
        assert!(signed.verify_signature().is_ok());
        signed.submission.mission_id = "Other".into();
        assert!(signed.verify_signature().is_err());
    }
}
