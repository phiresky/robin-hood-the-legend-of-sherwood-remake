//! Public submission status, owner-signed private status and lifecycle documents.

use crate::CanonicalDocument as _;
use crate::signed_request::{SignedRequestClaim, SignedRequestV2};
use crate::{
    Digest32, OpaqueId, PublicKey32, Validate, ValidationError, VerificationRejectionCodeV1,
};
use serde::{Deserialize, Serialize};

pub const SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V2: &[u8] =
    b"robinhood/leaderboards/2/submission-owner-status\0";

/// Minimal shareable progress. Detailed failures remain available only through
/// the owner-signed status endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSubmissionStatusV1 {
    pub schema_version: u32,
    pub submission_id: OpaqueId,
    pub state: PublicSubmissionStateV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublicSubmissionStateV1 {
    Queued,
    Verifying,
    RetryPending,
    Verified { run_id: OpaqueId },
    Rejected,
    Failed,
}

impl Validate for PublicSubmissionStatusV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("PublicSubmissionStatusV1", self.schema_version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionFailureCodeV1 {
    VerificationInfrastructure,
}

/// Owner-signed request for one submission's private lifecycle. Replaying a
/// captured request within the signing window only returns what the key
/// owner can already see; responses to unknown or foreign submissions are
/// indistinguishable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOwnerStatusRequestV2 {
    pub schema_version: u32,
    pub public_key: PublicKey32,
    pub signed_at_unix_ms: u64,
    pub submission_id: OpaqueId,
}

impl Validate for SubmissionOwnerStatusRequestV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "SubmissionOwnerStatusRequestV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::nonzero("submission_owner_status.public_key", &self.public_key)?;
        crate::validation::nonzero(
            "submission_owner_status.signed_at_unix_ms",
            &self.signed_at_unix_ms,
        )
    }
}

impl SignedRequestClaim for SubmissionOwnerStatusRequestV2 {
    const DOMAIN: &'static [u8] = SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V2;

    fn signer_public_key(&self) -> PublicKey32 {
        self.public_key
    }

    fn signed_at_unix_ms(&self) -> u64 {
        self.signed_at_unix_ms
    }
}

pub type SignedSubmissionOwnerStatusRequestV2 = SignedRequestV2<SubmissionOwnerStatusRequestV2>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionLifecycleV1 {
    Queued,
    Verifying,
    RetryPending,
    Accepted {
        run_id: OpaqueId,
    },
    Rejected {
        code: VerificationRejectionCodeV1,
        safe_message: String,
    },
    /// Terminal infrastructure failure after the bounded worker retry policy
    /// is exhausted. This is not evidence that the submitted run was invalid
    /// and must never be mapped to a replay rejection.
    Failed {
        code: SubmissionFailureCodeV1,
        safe_message: String,
    },
}

impl Validate for SubmissionLifecycleV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Rejected { safe_message, .. } => {
                crate::validation::text("submission.safe_message", safe_message, 500)?;
            }
            Self::Failed { safe_message, .. } => {
                crate::validation::text("submission.failure_safe_message", safe_message, 500)?;
            }
            Self::Accepted { .. } | Self::Queued | Self::Verifying | Self::RetryPending => {}
        }
        Ok(())
    }
}

/// Immediate `202 Accepted` response from `POST /api/v1/submissions`.
///
/// "Accepted" here means accepted into the bounded verification queue, not
/// accepted as a ranked run; callers must follow the lifecycle resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionAcceptedV1 {
    pub schema_version: u32,
    pub submission_id: OpaqueId,
    pub state: SubmissionLifecycleV1,
    pub retry_after_ms: u64,
}

impl Validate for SubmissionAcceptedV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionAcceptedV1", self.schema_version)?;
        if self.retry_after_ms == 0 {
            return Err(ValidationError::Zero {
                field: "submission_accepted.retry_after_ms",
            });
        }
        match &self.state {
            SubmissionLifecycleV1::Queued
            | SubmissionLifecycleV1::Verifying
            | SubmissionLifecycleV1::RetryPending => self.state.validate(),
            SubmissionLifecycleV1::Accepted { .. }
            | SubmissionLifecycleV1::Rejected { .. }
            | SubmissionLifecycleV1::Failed { .. } => Err(ValidationError::ClaimMismatch {
                field: "submission_accepted.state",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOwnerStatusResponseV2 {
    pub schema_version: u32,
    pub submission_id: OpaqueId,
    pub public_key: PublicKey32,
    /// Canonical digest of the exact signed request answered by this response.
    pub request_sha256: Digest32,
    pub state: SubmissionLifecycleV1,
}

impl Validate for SubmissionOwnerStatusResponseV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "SubmissionOwnerStatusResponseV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::nonzero(
            "submission_owner_status_response.public_key",
            &self.public_key,
        )?;
        crate::validation::nonzero(
            "submission_owner_status_response.request_sha256",
            &self.request_sha256,
        )?;
        self.state.validate()
    }
}

impl SubmissionOwnerStatusResponseV2 {
    pub fn validate_against_request(
        &self,
        request: &SignedSubmissionOwnerStatusRequestV2,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        request.validate()?;
        let request_sha256 =
            request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "submission_owner_status_response.request",
                })?;
        if self.submission_id != request.request.submission_id
            || self.public_key != request.request.public_key
            || self.request_sha256 != request_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_owner_status_response.request",
            });
        }
        Ok(())
    }
}
