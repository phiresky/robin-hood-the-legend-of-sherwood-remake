//! Owner-authenticated submission status, lifecycle documents and exact response binding.

use super::SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V1;
use crate::CanonicalDocument as _;
use crate::{
    CampaignChainReceiptV1, ChallengeNonce32, Digest32, OpaqueId, PublicKey32, Signature64,
    SignatureAlgorithmV1, Validate, ValidationError, VerificationRejectionCodeV1,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionFailureCodeV1 {
    VerificationInfrastructure,
}

/// Requests a one-use owner-status challenge. Servers must return the same
/// challenge shape whether or not the submission exists or is owned by this
/// key; this request is not an ownership/existence oracle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOwnerStatusChallengeRequestV1 {
    pub schema_version: u32,
    pub controller_public_key: PublicKey32,
    pub submission_id: OpaqueId,
}

impl Validate for SubmissionOwnerStatusChallengeRequestV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema(
            "SubmissionOwnerStatusChallengeRequestV1",
            self.schema_version,
        )?;
        if self.controller_public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "submission_owner_status_challenge_request.controller_public_key",
            });
        }
        Ok(())
    }
}

/// Server-authored one-use challenge. Issuance proves no ownership fact; the
/// server performs the ownership check only after verifying the signed
/// envelope and must keep unauthorized/not-found failures indistinguishable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOwnerStatusChallengeV1 {
    pub schema_version: u32,
    pub owner_status_challenge_id: OpaqueId,
    pub owner_status_challenge_nonce: ChallengeNonce32,
    pub expires_at_unix_ms: u64,
    pub controller_public_key: PublicKey32,
    pub submission_id: OpaqueId,
}

impl Validate for SubmissionOwnerStatusChallengeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionOwnerStatusChallengeV1", self.schema_version)?;
        if self.owner_status_challenge_nonce.is_zero() {
            return Err(ValidationError::Zero {
                field: "submission_owner_status_challenge.nonce",
            });
        }
        if self.expires_at_unix_ms == 0 {
            return Err(ValidationError::Zero {
                field: "submission_owner_status_challenge.expires_at_unix_ms",
            });
        }
        if self.controller_public_key.is_zero() {
            return Err(ValidationError::Zero {
                field: "submission_owner_status_challenge.controller_public_key",
            });
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct SubmissionOwnerStatusSignable<'a> {
    schema_version: u32,
    challenge: &'a SubmissionOwnerStatusChallengeV1,
    algorithm: SignatureAlgorithmV1,
}

/// Domain-separated Ed25519 proof authorizing one private status read. The
/// complete challenge is signed, so key, submission, nonce, expiry, and
/// challenge namespace cannot be transplanted independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionOwnerStatusEnvelopeV1 {
    pub schema_version: u32,
    pub challenge: SubmissionOwnerStatusChallengeV1,
    pub algorithm: SignatureAlgorithmV1,
    pub signature: Signature64,
}

impl SubmissionOwnerStatusEnvelopeV1 {
    pub fn validate_signing_claim(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionOwnerStatusEnvelopeV1", self.schema_version)?;
        self.challenge.validate()?;
        if self.schema_version != self.challenge.schema_version {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_owner_status.challenge.schema_version",
            });
        }
        Ok(())
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>, crate::canonical::CanonicalError> {
        crate::canonical::domain_separated_bytes(
            SUBMISSION_OWNER_STATUS_SIGNATURE_DOMAIN_V1,
            &SubmissionOwnerStatusSignable {
                schema_version: self.schema_version,
                challenge: &self.challenge,
                algorithm: self.algorithm,
            },
        )
    }
}

impl Validate for SubmissionOwnerStatusEnvelopeV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        self.validate_signing_claim()?;
        if self.signature.is_zero() {
            return Err(ValidationError::Zero {
                field: "submission_owner_status.signature",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubmissionLifecycleV1 {
    Queued,
    Verifying,
    RetryPending,
    Accepted {
        run_id: OpaqueId,
        campaign_chain_receipt: Option<CampaignChainReceiptV1>,
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
            Self::Accepted {
                campaign_chain_receipt,
                ..
            } => {
                if let Some(receipt) = campaign_chain_receipt {
                    receipt.validate()?;
                }
            }
            Self::Rejected { safe_message, .. } => {
                crate::validation::text("submission.safe_message", safe_message, 500)?;
            }
            Self::Failed { safe_message, .. } => {
                crate::validation::text("submission.failure_safe_message", safe_message, 500)?;
            }
            Self::Queued | Self::Verifying | Self::RetryPending => {}
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
pub struct SubmissionOwnerStatusResponseV1 {
    pub schema_version: u32,
    pub submission_id: OpaqueId,
    pub controller_public_key: PublicKey32,
    /// Digest of the exact signed `SubmissionOwnerStatusEnvelopeV1` accepted
    /// for this one private read.
    pub owner_status_envelope_sha256: Digest32,
    pub state: SubmissionLifecycleV1,
}

impl Validate for SubmissionOwnerStatusResponseV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("SubmissionOwnerStatusResponseV1", self.schema_version)?;
        if self.controller_public_key.is_zero() || self.owner_status_envelope_sha256.is_zero() {
            return Err(ValidationError::Zero {
                field: "submission_owner_status_response.owner_binding",
            });
        }
        self.state.validate()?;
        if let SubmissionLifecycleV1::Accepted {
            run_id,
            campaign_chain_receipt: Some(receipt),
        } = &self.state
            && (receipt.predecessor_run_id != *run_id
                || receipt.campaign_controller_public_key != self.controller_public_key)
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_owner_status_response.campaign_chain_receipt",
            });
        }
        Ok(())
    }
}

impl SubmissionOwnerStatusResponseV1 {
    pub fn validate_against_envelope(
        &self,
        envelope: &SubmissionOwnerStatusEnvelopeV1,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        envelope.validate()?;
        let envelope_sha256 =
            envelope
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "submission_owner_status_response.envelope",
                })?;
        if self.submission_id != envelope.challenge.submission_id
            || self.controller_public_key != envelope.challenge.controller_public_key
            || self.owner_status_envelope_sha256 != envelope_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "submission_owner_status_response.envelope",
            });
        }
        Ok(())
    }
}
