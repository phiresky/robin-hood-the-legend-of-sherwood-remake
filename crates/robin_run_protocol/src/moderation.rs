use serde::{Deserialize, Serialize};

use crate::{ChallengeNonce32, OpaqueId, PublicKey32, Signature64, Validate, ValidationError};

pub const DELETION_REQUEST_SIGNATURE_DOMAIN_V1: &[u8] =
    b"robinhood/leaderboards/1/deletion-request\0";

/// An owner may tombstone either an upload still in its submission lifecycle
/// or a published verified run. A run target also covers its published replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeletionTargetV1 {
    Submission { submission_id: OpaqueId },
    Run { run_id: OpaqueId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionChallengeRequestV1 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
    pub target: DeletionTargetV1,
}

impl Validate for DeletionChallengeRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("DeletionChallengeRequestV1", self.schema_version)?;
        if self.public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "deletion_challenge_request.public_key",
            });
        }
        Ok(())
    }
}

/// Server-authored, one-use challenge binding the owner key and exact target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionChallengeV1 {
    pub schema_version: u32,
    pub deletion_challenge_id: OpaqueId,
    pub deletion_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
    pub public_key: PublicKey32,
    pub target: DeletionTargetV1,
}

impl Validate for DeletionChallengeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("DeletionChallengeV1", self.schema_version)?;
        if self.deletion_challenge_nonce.is_zero() {
            return Err(ValidationError::Zero {
                field: "deletion_challenge.deletion_challenge_nonce",
            });
        }
        if self.expires_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "deletion_challenge.expires_at_unix_ms",
            });
        }
        if self.public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "deletion_challenge.public_key",
            });
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct DeletionRequestSignable<'a> {
    schema_version: u32,
    challenge: &'a DeletionChallengeV1,
}

/// Key-signed owner request. The complete server challenge is nested so the
/// target, owner, expiry, nonce, and challenge namespace are all covered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionRequestEnvelopeV1 {
    pub schema_version: u32,
    pub challenge: DeletionChallengeV1,
    pub signature: Signature64,
}

impl DeletionRequestEnvelopeV1 {
    /// Validate the claim before asking a native or WASM identity bridge to
    /// sign it. The signature field is intentionally ignored here.
    pub fn validate_signing_claim(&self) -> Result<(), ValidationError> {
        crate::validation::schema("DeletionRequestEnvelopeV1", self.schema_version)?;
        self.challenge.validate()?;
        if self.schema_version != self.challenge.schema_version {
            return Err(ValidationError::ClaimMismatch {
                field: "deletion_request.challenge.schema_version",
            });
        }
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        crate::canonical::domain_separated_bytes(
            DELETION_REQUEST_SIGNATURE_DOMAIN_V1,
            &DeletionRequestSignable {
                schema_version: self.schema_version,
                challenge: &self.challenge,
            },
        )
    }
}

impl Validate for DeletionRequestEnvelopeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.validate_signing_claim()?;
        if self.signature.is_zero() {
            return Err(ValidationError::Zero {
                field: "deletion_request.signature",
            });
        }
        Ok(())
    }
}

/// Immediate result of a successful owner deletion request. Ranking and
/// replay visibility have already been removed when this is returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionReceiptV1 {
    pub schema_version: u32,
    pub deletion_request_id: OpaqueId,
    pub target: DeletionTargetV1,
    pub tombstoned_at_unix_ms: u64,
    /// Earliest time physical garbage collection may occur. `None` means the
    /// operator configured no automatic purge. Shared replay objects remain
    /// until every live reference is gone regardless of this timestamp.
    pub purge_eligible_at_unix_ms: Option<u64>,
}

impl Validate for DeletionReceiptV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("DeletionReceiptV1", self.schema_version)?;
        if self.tombstoned_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "deletion_receipt.tombstoned_at_unix_ms",
            });
        }
        if self
            .purge_eligible_at_unix_ms
            .is_some_and(|timestamp| timestamp <= self.tombstoned_at_unix_ms)
        {
            return Err(ValidationError::CountOutOfRange {
                field: "deletion_receipt.purge_eligible_at_unix_ms",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbuseReportCategoryV1 {
    SuspectedCheating,
    OffensiveIdentity,
    Privacy,
    Copyright,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AbuseReportTargetV1 {
    Run { run_id: OpaqueId },
    Player { public_key: PublicKey32 },
}

/// Bounded moderation signal. Acceptance records a report only; it never
/// changes verification, ranking, identity, or replay visibility by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbuseReportV1 {
    pub schema_version: u32,
    pub target: AbuseReportTargetV1,
    pub category: AbuseReportCategoryV1,
    pub detail: String,
}

impl Validate for AbuseReportV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("AbuseReportV1", self.schema_version)?;
        if matches!(
            &self.target,
            AbuseReportTargetV1::Player { public_key } if public_key.is_zero()
        ) {
            return Err(ValidationError::Zero {
                field: "abuse_report.target.public_key",
            });
        }
        crate::validation::text("abuse_report.detail", &self.detail, 2_000)
    }
}

/// Queue acknowledgement only. It deliberately makes no moderation-outcome
/// or visibility claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbuseReportAcceptedV1 {
    pub schema_version: u32,
    pub report_id: OpaqueId,
    pub received_at_unix_ms: u64,
}

impl Validate for AbuseReportAcceptedV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("AbuseReportAcceptedV1", self.schema_version)?;
        if self.received_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "abuse_report_accepted.received_at_unix_ms",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SCHEMA_VERSION_V1;

    fn id(value: &str) -> OpaqueId {
        OpaqueId::new(value).unwrap()
    }

    fn challenge() -> DeletionChallengeV1 {
        DeletionChallengeV1 {
            schema_version: SCHEMA_VERSION_V1,
            deletion_challenge_id: id("delete-challenge-1"),
            deletion_challenge_nonce: ChallengeNonce32::from_bytes([1; 32]),
            expires_at_unix_ms: 10,
            public_key: PublicKey32::from_bytes([2; 32]),
            target: DeletionTargetV1::Run {
                run_id: id("run-1"),
            },
        }
    }

    #[test]
    fn deletion_signature_binds_exact_server_challenge_and_target() {
        let mut request = DeletionRequestEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge: challenge(),
            signature: Signature64::from_bytes([3; 64]),
        };
        assert!(request.validate().is_ok());
        let original = request.signing_bytes().unwrap();
        request.signature = Signature64::from_bytes([4; 64]);
        assert_eq!(original, request.signing_bytes().unwrap());
        request.challenge.target = DeletionTargetV1::Submission {
            submission_id: id("submission-1"),
        };
        assert_ne!(original, request.signing_bytes().unwrap());
    }

    #[test]
    fn deletion_bridge_can_validate_before_signature_exists() {
        let request = DeletionRequestEnvelopeV1 {
            schema_version: SCHEMA_VERSION_V1,
            challenge: challenge(),
            signature: Signature64::from_bytes([0; 64]),
        };
        assert!(request.validate_signing_claim().is_ok());
        assert!(request.validate().is_err());
    }

    #[test]
    fn deletion_retention_is_server_configured_and_strictly_future() {
        let mut receipt = DeletionReceiptV1 {
            schema_version: SCHEMA_VERSION_V1,
            deletion_request_id: id("delete-request-1"),
            target: DeletionTargetV1::Run {
                run_id: id("run-1"),
            },
            tombstoned_at_unix_ms: 10,
            purge_eligible_at_unix_ms: None,
        };
        assert!(receipt.validate().is_ok());
        receipt.purge_eligible_at_unix_ms = Some(10);
        assert!(receipt.validate().is_err());
        receipt.purge_eligible_at_unix_ms = Some(11);
        assert!(receipt.validate().is_ok());
    }

    #[test]
    fn report_is_bounded_and_has_no_visibility_outcome() {
        let mut report = AbuseReportV1 {
            schema_version: SCHEMA_VERSION_V1,
            target: AbuseReportTargetV1::Run {
                run_id: id("run-1"),
            },
            category: AbuseReportCategoryV1::SuspectedCheating,
            detail: "The playback diverges from the displayed result.".into(),
        };
        assert!(report.validate().is_ok());
        report.detail = "x".repeat(2_001);
        assert!(report.validate().is_err());
    }
}
