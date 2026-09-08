//! Worker outcomes and the exact request/result binding contract.
use crate::{
    CampaignContentManifestV1, CampaignSessionKindV1, CanonicalDocument as _, Digest32,
    InputProvenanceStatusV1, OfficialContentSubjectV1, OpaqueId, RulesetManifestV1, RunScopeKindV1,
    SubmissionArtifactsV1, TerminalOutcomeV1, Validate, ValidationError, VerificationRequestV1,
    VerifiedRunV1,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRejectionCodeV1 {
    MalformedReplay,
    ResourceLimit,
    UnsupportedSchema,
    BuildNotAllowed,
    ContentNotAllowed,
    ConfigMismatch,
    StartingStateMismatch,
    CommandNotAllowed,
    TimelineInvalid,
    StateHashMismatch,
    TerminalInvalid,
    ResultInvariantMismatch,
    InputProvenanceIneligible,
    /// Deterministic in-engine simulation/tick budget exhaustion. Host wall
    /// clock or supervisor timeouts are infrastructure failures instead.
    SimulationBudgetExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRejectionV1 {
    pub code: VerificationRejectionCodeV1,
    /// Stable, non-sensitive implementation detail such as
    /// `campaign_team_index_out_of_range`; never a path or panic text.
    pub detail_code: Option<String>,
}

impl Validate for VerificationRejectionV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(detail) = &self.detail_code {
            crate::validation::text("verification_rejection.detail_code", detail, 128)?;
        }
        Ok(())
    }
}

/// Infrastructure faults are never run rejections and must never be admitted
/// to a board. A worker retries them according to bounded backend policy; only
/// the exhausted terminal result uses `FailedInfrastructure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationInfrastructureFailureCodeV1 {
    WorkerInternalFailure,
    WorkerUnavailable,
    VerifierProcessFailure,
    ArtifactIoFailure,
    InfrastructureTimeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationInfrastructureFailureV1 {
    pub code: VerificationInfrastructureFailureCodeV1,
    /// Private, stable operator diagnostic. Never raw panic text, a file path,
    /// a command line, or another secret-bearing implementation string.
    pub private_detail_code: Option<String>,
}

impl Validate for VerificationInfrastructureFailureV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(detail) = &self.private_detail_code {
            crate::validation::text(
                "verification_infrastructure_failure.private_detail_code",
                detail,
                128,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "result", rename_all = "snake_case")]
pub enum VerificationStatusV1 {
    Verified(VerifiedRunV1),
    Rejected(VerificationRejectionV1),
    FailedInfrastructure(VerificationInfrastructureFailureV1),
}

/// Stable worker-boundary failures which happen before a trustworthy
/// [`VerificationResultV1`] can be constructed.
///
/// In particular, a malformed request has no authenticated request id or
/// rules/content identities. The worker must report only the digest of the
/// exact untrusted request artifact instead of inventing those fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierAdmissionFailureCodeV1 {
    RequestTooLarge,
    MalformedRequest,
    UnsupportedRequestSchema,
    RequestAuthenticationFailed,
    WorkerInternalFailure,
}

/// Exact result document written by the isolated verifier child.
///
/// A decoded request always produces `VerificationResult`. Admission failures
/// are reserved for the boundary where the worker cannot safely populate the
/// proof fields required by [`VerificationResultV1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifierWorkerOutputV1 {
    VerificationResult {
        schema_version: u32,
        result: VerificationResultV1,
    },
    AdmissionFailure {
        schema_version: u32,
        request_artifact_sha256: Digest32,
        code: VerifierAdmissionFailureCodeV1,
        bounded_detail: Option<String>,
    },
}

impl Validate for VerifierWorkerOutputV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::VerificationResult {
                schema_version,
                result,
            } => {
                crate::validation::schema("VerifierWorkerOutputV1", *schema_version)?;
                result.validate()
            }
            Self::AdmissionFailure {
                schema_version,
                request_artifact_sha256,
                bounded_detail,
                ..
            } => {
                crate::validation::schema("VerifierWorkerOutputV1", *schema_version)?;
                if request_artifact_sha256.is_zero() {
                    return Err(ValidationError::Zero {
                        field: "verifier_worker_output.request_artifact_sha256",
                    });
                }
                if let Some(detail) = bounded_detail {
                    crate::validation::text("verifier_worker_output.bounded_detail", detail, 128)?;
                }
                Ok(())
            }
        }
    }
}

impl Validate for VerificationStatusV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Verified(result) => result.validate(),
            Self::Rejected(result) => result.validate(),
            Self::FailedInfrastructure(result) => result.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultV1 {
    pub schema_version: u32,
    pub request_id: OpaqueId,
    pub verification_request_sha256: Digest32,
    /// Exact artifact tuple. `artifacts.replay` is the same byte identity used
    /// for verifier resimulation, retention, and public download.
    pub artifacts: SubmissionArtifactsV1,
    pub session_genesis_sha256: Digest32,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
    /// `None` is permitted only when verification failed before replay
    /// provenance could be decoded (for example, a malformed upload).
    pub input_provenance: Option<InputProvenanceStatusV1>,
    pub status: VerificationStatusV1,
}

impl Validate for VerificationResultV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerificationResultV1", self.schema_version)?;
        for digest in [
            self.verification_request_sha256,
            self.session_genesis_sha256,
            self.build_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ] {
            if digest.is_zero() {
                return Err(ValidationError::Zero {
                    field: "verification_result.proof_digest",
                });
            }
        }
        self.artifacts.validate()?;
        if let Some(provenance) = &self.input_provenance {
            provenance.validate()?;
        }
        if self
            .competition_manifest_sha256
            .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "verification_result.competition_manifest_sha256",
            });
        }
        match &self.status {
            VerificationStatusV1::Verified(_) => {
                if self
                    .input_provenance
                    .as_ref()
                    .is_none_or(|provenance| !provenance.is_rankable())
                {
                    return Err(ValidationError::VerifiedRunNotRankable);
                }
                let VerificationStatusV1::Verified(run) = &self.status else {
                    unreachable!()
                };
                if run.replay_session_transcript.session_genesis_sha256
                    != self.session_genesis_sha256
                    || run.starting_campaign != self.artifacts.starting_campaign
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "verification_result.session_genesis_sha256",
                    });
                }
                if let Some(evidence) = &run.campaign_complete_evidence
                    && (evidence.verification_request_sha256 != self.verification_request_sha256
                        || evidence.replay_sha256 != self.artifacts.replay.artifact.sha256
                        || evidence.content_manifest_sha256 != self.content_manifest_sha256
                        || evidence.rules_config_sha256 != self.rules_config_sha256
                        || evidence.ruleset_manifest_sha256 != self.ruleset_manifest_sha256
                        || evidence.final_campaign_sha256 != run.final_campaign.sha256
                        || evidence.final_state_sha256 != run.final_state_sha256)
                {
                    return Err(ValidationError::ClaimMismatch {
                        field: "verification_result.campaign_complete_evidence",
                    });
                }
            }
            VerificationStatusV1::Rejected(rejection)
                if rejection.code == VerificationRejectionCodeV1::InputProvenanceIneligible =>
            {
                if self
                    .input_provenance
                    .as_ref()
                    .is_none_or(InputProvenanceStatusV1::is_rankable)
                {
                    return Err(ValidationError::InvalidInputProvenanceRejection);
                }
            }
            VerificationStatusV1::Rejected(_) | VerificationStatusV1::FailedInfrastructure(_) => {}
        }
        self.status.validate()
    }
}

impl VerificationResultV1 {
    /// Validate the verifier-only campaign-completion document against every
    /// content-addressed object needed to interpret it. The ordinary
    /// [`Validate`] implementation covers result-local bindings; this method
    /// is required at verifier/backend boundaries which possess the signed
    /// request, immutable ruleset, and selected campaign catalog.
    pub fn validate_campaign_complete_evidence(
        &self,
        request: &VerificationRequestV1,
        ruleset: &RulesetManifestV1,
        campaign_content: Option<&CampaignContentManifestV1>,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        request.validate()?;
        ruleset.validate()?;

        let request_sha256 =
            request
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "verification_result.verification_request_sha256",
                })?;
        let ruleset_sha256 =
            ruleset
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "verification_result.ruleset_manifest_sha256",
                })?;
        let submission = &request.submission.submission;
        let offer = &submission.offer;
        let ranked = &offer.session_genesis.claim.ranked_session;
        let session_genesis_sha256 = offer.session_genesis.canonical_digest().map_err(|_| {
            ValidationError::ClaimMismatch {
                field: "verification_result.session_genesis_sha256",
            }
        })?;
        if self.request_id != request.request_id
            || self.verification_request_sha256 != request_sha256
            || self.artifacts != submission.artifacts
            || self.session_genesis_sha256 != session_genesis_sha256
            || self.build_manifest_sha256 != offer.build_manifest_sha256
            || self.content_manifest_sha256 != offer.content_manifest_sha256
            || self.rules_config_sha256 != offer.rules_config_sha256
            || self.ruleset_manifest_sha256 != offer.ruleset_manifest_sha256
            || self.ruleset_manifest_sha256 != ruleset_sha256
            || self.competition_manifest_sha256 != offer.competition_manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verification_result.request_tuple",
            });
        }

        let VerificationStatusV1::Verified(run) = &self.status else {
            return Ok(());
        };
        if run.replay_session_transcript != submission.replay_session_transcript {
            return Err(ValidationError::ClaimMismatch {
                field: "verification_result.replay_session_transcript",
            });
        }
        if run.starting_campaign != self.artifacts.starting_campaign {
            return Err(ValidationError::ClaimMismatch {
                field: "verification_result.starting_campaign",
            });
        }
        let policy = ruleset.campaign_completion_policy.required();
        let Some(evidence) = &run.campaign_complete_evidence else {
            if policy.is_some_and(|policy| {
                run.scope_kind == RunScopeKindV1::Campaign
                    && run.outcome == TerminalOutcomeV1::Won
                    && ranked.content_subject == policy.terminal_subject
            }) {
                return Err(ValidationError::ClaimMismatch {
                    field: "campaign_complete_evidence.missing_terminal_evidence",
                });
            }
            return Ok(());
        };
        let policy = policy.ok_or(ValidationError::ClaimMismatch {
            field: "ruleset.campaign_completion_policy",
        })?;
        let campaign_content = campaign_content.ok_or(ValidationError::ClaimMismatch {
            field: "campaign_complete_evidence.campaign_content_manifest",
        })?;
        ruleset.validate_campaign_completion_catalog(campaign_content)?;
        let campaign_content_sha256 =
            campaign_content
                .canonical_digest()
                .map_err(|_| ValidationError::ClaimMismatch {
                    field: "campaign_complete_evidence.campaign_content_manifest_sha256",
                })?;
        let session_campaign_content =
            ranked
                .campaign_content_manifest_sha256
                .ok_or(ValidationError::ClaimMismatch {
                    field: "campaign_complete_evidence.campaign_content_manifest_sha256",
                })?;
        let terminal_content_sha256 = campaign_content
            .content_for(&policy.terminal_subject)
            .ok_or(ValidationError::ClaimMismatch {
                field: "campaign_complete_evidence.terminal_subject",
            })?;
        let session_kind_matches = match (&run.campaign_session_kind, &policy.terminal_subject) {
            (
                Some(CampaignSessionKindV1::FieldMission {
                    mission_id: session,
                }),
                OfficialContentSubjectV1::FieldMission {
                    mission_id: terminal,
                },
            ) => session == terminal,
            (
                Some(CampaignSessionKindV1::Headquarters { .. }),
                OfficialContentSubjectV1::Headquarters { .. },
            ) => true,
            _ => false,
        };
        if run.scope_kind != RunScopeKindV1::Campaign
            || run.outcome != TerminalOutcomeV1::Won
            || !session_kind_matches
            || campaign_content.edition != ranked.content_edition
            || ruleset
                .allowed_campaign_content_manifest_sha256
                .binary_search(&campaign_content_sha256)
                .is_err()
            || evidence.campaign_content_manifest_sha256 != campaign_content_sha256
            || evidence.campaign_content_manifest_sha256 != session_campaign_content
            || evidence.content_manifest_sha256 != terminal_content_sha256
            || evidence.terminal_subject != policy.terminal_subject
            || ranked.content_subject != policy.terminal_subject
            || evidence.observed_progression_percent != policy.required_progression_percent
        {
            return Err(ValidationError::ClaimMismatch {
                field: "campaign_complete_evidence.policy_binding",
            });
        }
        Ok(())
    }
}
