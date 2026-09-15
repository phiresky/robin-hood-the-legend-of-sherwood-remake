use serde::{Deserialize, Serialize};

use crate::signed_request::{SignedRequestClaim, SignedRequestV2};
use crate::{OpaqueId, PublicKey32, Validate, ValidationError};

pub const DELETION_REQUEST_SIGNATURE_DOMAIN_V2: &[u8] =
    b"robinhood/leaderboards/2/deletion-request\0";

/// An owner may tombstone either an upload still in its submission lifecycle
/// or a published verified run. A run target also covers its published replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeletionTargetV1 {
    Submission { submission_id: OpaqueId },
    Run { run_id: OpaqueId },
}

/// Owner-signed deletion. Deletion is idempotent, so replaying a captured
/// request within the signing window has no further effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionRequestV2 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
    pub signed_at_unix_ms: u64,
    pub target: DeletionTargetV1,
}

impl Validate for DeletionRequestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "DeletionRequestV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::nonzero("deletion_request.public_key", &self.public_key)?;
        crate::validation::nonzero(
            "deletion_request.signed_at_unix_ms",
            &self.signed_at_unix_ms,
        )
    }
}

impl SignedRequestClaim for DeletionRequestV2 {
    const DOMAIN: &'static [u8] = DELETION_REQUEST_SIGNATURE_DOMAIN_V2;

    fn signer_public_key(&self) -> PublicKey32 {
        self.public_key
    }

    fn signed_at_unix_ms(&self) -> u64 {
        self.signed_at_unix_ms
    }
}

pub type SignedDeletionRequestV2 = SignedRequestV2<DeletionRequestV2>;

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

    #[test]
    fn deletion_signing_bytes_bind_target_key_and_time() {
        let request = DeletionRequestV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            public_key: PublicKey32::from_bytes([2; 32]),
            signed_at_unix_ms: 10,
            target: DeletionTargetV1::Run {
                run_id: id("run-1"),
            },
        };
        let original = SignedDeletionRequestV2::signing_bytes(&request).unwrap();
        assert!(original.starts_with(DELETION_REQUEST_SIGNATURE_DOMAIN_V2));
        let mut other_target = request.clone();
        other_target.target = DeletionTargetV1::Submission {
            submission_id: id("submission-1"),
        };
        assert_ne!(
            original,
            SignedDeletionRequestV2::signing_bytes(&other_target).unwrap()
        );
        let mut other_time = request;
        other_time.signed_at_unix_ms = 11;
        assert_ne!(
            original,
            SignedDeletionRequestV2::signing_bytes(&other_time).unwrap()
        );
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
