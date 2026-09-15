//! Signed replay submissions, username updates and input provenance.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::signed_request::{SignedRequestClaim, SignedRequestV2};
use crate::{ArtifactRefV1, BoardMetricV1, OpaqueId, PublicKey32, Validate, ValidationError};

pub const SUBMISSION_SIGNATURE_DOMAIN_V2: &[u8] = b"robinhood/leaderboards/2/submission\0";
pub const USERNAME_UPDATE_SIGNATURE_DOMAIN_V2: &[u8] =
    b"robinhood/leaderboards/2/username-update\0";

/// The one wire/storage format accepted for ranked replays. JSONL and older
/// Rust replay containers are local developer formats, not protocol lanes.
pub const RANKED_REPLAY_MEDIA_TYPE_V1: &str = "application/x-robin-rhrec+compact";

/// Mutable display name of one identity key. Usernames are not part of run
/// submissions and are not required to be unique. The server only accepts an
/// update newer than the last accepted update for the key, so replaying a
/// captured request cannot roll a later name back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsernameUpdateV2 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
    pub signed_at_unix_ms: u64,
    pub username: String,
}

impl Validate for UsernameUpdateV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "UsernameUpdateV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::nonzero("username_update.public_key", &self.public_key)?;
        crate::validation::nonzero("username_update.signed_at_unix_ms", &self.signed_at_unix_ms)?;
        crate::validation::text("username_update.username", &self.username, 48)
    }
}

impl SignedRequestClaim for UsernameUpdateV2 {
    const DOMAIN: &'static [u8] = USERNAME_UPDATE_SIGNATURE_DOMAIN_V2;

    fn signer_public_key(&self) -> PublicKey32 {
        self.public_key
    }

    fn signed_at_unix_ms(&self) -> u64 {
        self.signed_at_unix_ms
    }
}

pub type SignedUsernameUpdateV2 = SignedRequestV2<UsernameUpdateV2>;

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

/// One replay upload claimed by one identity key for one board and mission.
/// The signature covers the replay's SHA-256 and length; a replay already
/// pending or accepted cannot be submitted again by anyone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionV2 {
    pub schema_version: u32,
    pub uploader_public_key: PublicKey32,
    pub signed_at_unix_ms: u64,
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
        crate::validation::nonzero("submission.uploader_public_key", &self.uploader_public_key)?;
        crate::validation::nonzero("submission.signed_at_unix_ms", &self.signed_at_unix_ms)?;
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

impl SignedRequestClaim for SubmissionV2 {
    const DOMAIN: &'static [u8] = SUBMISSION_SIGNATURE_DOMAIN_V2;

    fn signer_public_key(&self) -> PublicKey32 {
        self.uploader_public_key
    }

    fn signed_at_unix_ms(&self) -> u64 {
        self.signed_at_unix_ms
    }
}

/// Multipart `submission` field of `POST /api/v1/submissions`.
pub type SignedSubmissionV2 = SignedRequestV2<SubmissionV2>;

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
    use crate::{Digest32, Signature64, SignatureAlgorithmV1};

    pub(crate) fn submission() -> SubmissionV2 {
        SubmissionV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            uploader_public_key: PublicKey32::from_bytes([4; 32]),
            signed_at_unix_ms: 1_800_000_000_000,
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
        let mut unsigned_time = submission();
        unsigned_time.signed_at_unix_ms = 0;
        assert!(unsigned_time.validate().is_err());
    }

    #[cfg(feature = "authentication")]
    #[test]
    fn uploader_signature_binds_the_exact_fresh_submission_and_operation() {
        use crate::signed_request::{SignedRequestError, SignedRequestWindowV1};
        use ed25519_dalek::{Signer as _, SigningKey};

        let key = SigningKey::from_bytes(&[9; 32]);
        let mut submission = submission();
        submission.uploader_public_key = PublicKey32::from_bytes(key.verifying_key().to_bytes());
        let now = submission.signed_at_unix_ms + 1_000;
        let signature = key.sign(&SignedSubmissionV2::signing_bytes(&submission).unwrap());
        let signed = SignedSubmissionV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            request: submission,
            algorithm: SignatureAlgorithmV1::Ed25519,
            signature: Signature64::from_bytes(signature.to_bytes()),
        };
        let window = SignedRequestWindowV1::default();
        assert!(signed.verify(window, now).is_ok());

        let mut changed = signed.clone();
        changed.request.board_id = OpaqueId::new("demo-original-normal").unwrap();
        assert!(matches!(
            changed.verify(window, now),
            Err(SignedRequestError::Signature(_))
        ));
        assert!(matches!(
            signed.verify(window, now + window.max_age_ms),
            Err(SignedRequestError::Freshness(_))
        ));

        // A signature made for another operation by the same key at the same
        // time never verifies as a submission.
        let update = UsernameUpdateV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            public_key: signed.request.uploader_public_key,
            signed_at_unix_ms: signed.request.signed_at_unix_ms,
            username: "Robin".into(),
        };
        let foreign = key.sign(&SignedUsernameUpdateV2::signing_bytes(&update).unwrap());
        let mut cross_operation = signed.clone();
        cross_operation.signature = Signature64::from_bytes(foreign.to_bytes());
        assert!(matches!(
            cross_operation.verify(window, now),
            Err(SignedRequestError::Signature(_))
        ));
    }
}
