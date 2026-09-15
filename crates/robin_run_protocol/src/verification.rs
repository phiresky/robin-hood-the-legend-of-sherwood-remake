//! Verifier job input, verifier output and achievement catalog.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    BoardSimulationPolicyV1, CanonicalValue, Digest32, InputProvenanceStatusV1,
    OfficialContentEditionV1, OpaqueId, ReplayArtifactV1, TerminalOutcomeV1, Validate,
    ValidationError,
};

/// Pre-decode ceiling for the job document handed to one verifier process.
pub const MAX_VERIFIER_JOB_BYTES_V2: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationLimitsV1 {
    pub max_input_bytes: u64,
    pub max_compressed_bytes: u64,
    pub max_decompressed_bytes: u64,
    pub max_base64_payload_bytes: u64,
    pub max_campaign_bytes: u64,
    pub max_frames: u32,
    pub max_version_bytes: u32,
    pub max_mission_id_bytes: u32,
    pub max_metadata_records: u32,
    pub max_entries_per_frame: u32,
}

impl Validate for VerificationLimitsV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        for (field, value) in [
            ("limits.max_input_bytes", self.max_input_bytes),
            ("limits.max_compressed_bytes", self.max_compressed_bytes),
            ("limits.max_decompressed_bytes", self.max_decompressed_bytes),
            (
                "limits.max_base64_payload_bytes",
                self.max_base64_payload_bytes,
            ),
            ("limits.max_campaign_bytes", self.max_campaign_bytes),
            ("limits.max_frames", u64::from(self.max_frames)),
            (
                "limits.max_version_bytes",
                u64::from(self.max_version_bytes),
            ),
            (
                "limits.max_mission_id_bytes",
                u64::from(self.max_mission_id_bytes),
            ),
            (
                "limits.max_metadata_records",
                u64::from(self.max_metadata_records),
            ),
            (
                "limits.max_entries_per_frame",
                u64::from(self.max_entries_per_frame),
            ),
        ] {
            crate::validation::nonzero(field, &value)?;
        }
        Ok(())
    }
}

/// Exact job written by the queue worker for one verifier process. The worker
/// is trusted; only the replay bytes are hostile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierJobV2 {
    pub schema_version: u32,
    pub job_id: OpaqueId,
    pub edition: OfficialContentEditionV1,
    pub mission_id: String,
    pub simulation_policy: BoardSimulationPolicyV1,
    pub allow_state_load: bool,
    pub replay: ReplayArtifactV1,
    /// Single numeric locale directory below the raw content root, e.g. `1033`.
    pub resource_locale_root: String,
    pub limits: VerificationLimitsV1,
}

impl Validate for VerifierJobV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "VerifierJobV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::text("verifier_job.mission_id", &self.mission_id, 256)?;
        self.simulation_policy.validate()?;
        self.replay.validate_current_schema()?;
        if self.resource_locale_root.is_empty()
            || self.resource_locale_root.len() > 8
            || !self
                .resource_locale_root
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verifier_job.resource_locale_root",
            });
        }
        self.limits.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRejectionCodeV1 {
    MalformedReplay,
    ResourceLimit,
    UnsupportedSchema,
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
/// to a board. A worker retries them according to bounded backend policy.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedAchievementEvaluationV1 {
    Unverifiable,
    NotEarned,
    Earned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedAchievementV1 {
    pub achievement_id: OpaqueId,
    /// Lossless verifier decision. In particular, an unavailable dependency
    /// is not silently collapsed into a legitimate `not_earned` result.
    pub evaluation: VerifiedAchievementEvaluationV1,
    /// Exact bounded verifier evidence. Empty evidence is valid when the
    /// achievement definition is itself a single terminal predicate.
    pub evidence: BTreeMap<String, CanonicalValue>,
}

impl VerifiedAchievementV1 {
    pub const fn is_awarded(&self) -> bool {
        matches!(self.evaluation, VerifiedAchievementEvaluationV1::Earned)
    }
}

impl Validate for VerifiedAchievementV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.evidence.len() > 64 {
            return Err(ValidationError::CountOutOfRange {
                field: "verified_achievement.evidence",
            });
        }
        for (key, value) in &self.evidence {
            crate::validation::text("verified_achievement.evidence.key", key, 128)?;
            value.validate_depth(16)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AchievementPolicyModeV1 {
    /// Failure to produce a definitive decision rejects verification.
    Required,
    /// A typed `unverifiable` result may be reported but is never an award.
    Reported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AchievementPolicyV1 {
    pub achievement_id: OpaqueId,
    pub mode: AchievementPolicyModeV1,
}

/// The achievement catalog verified for every ranked board, in canonical
/// stable-ID order.
pub fn official_achievement_policies_v1() -> Vec<AchievementPolicyV1> {
    [
        "a-legend-is-born",
        "all-banners-purchased",
        "all-beggar-info",
        "charity",
        "clean-hands",
        "different-kind-of-scarlet",
        "for-king-richard",
        "ghost",
        "im-off-home",
        "kill-a-civilian",
        "leave-everyone-standing",
        "many-hands",
        "no-banners-purchased",
        "no-empty-places",
        "not-a-scratch",
        "on-my-mark",
        "people-behind-the-legend",
        "pile-o-bones",
        "round-on-the-friar",
        "ruthless",
        "something-in-the-air",
        "string-theory",
        "whole-merry-company",
        "you-never-saw-us-leave",
    ]
    .into_iter()
    .map(|achievement_id| AchievementPolicyV1 {
        achievement_id: OpaqueId::new(achievement_id)
            .expect("official achievement IDs are protocol constants"),
        mode: AchievementPolicyModeV1::Required,
    })
    .collect()
}

/// Require exactly the official catalog, in order, with no `Required`
/// achievement left unverifiable.
pub fn validate_authoritative_achievements(
    achievements: &[VerifiedAchievementV1],
) -> Result<(), ValidationError> {
    let policies = official_achievement_policies_v1();
    if achievements.len() != policies.len()
        || achievements
            .iter()
            .zip(&policies)
            .any(|(result, policy)| result.achievement_id != policy.achievement_id)
    {
        return Err(ValidationError::ClaimMismatch {
            field: "verified_run.achievements.catalog",
        });
    }
    for (result, policy) in achievements.iter().zip(&policies) {
        result.validate()?;
        if policy.mode == AchievementPolicyModeV1::Required
            && result.evaluation == VerifiedAchievementEvaluationV1::Unverifiable
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verified_run.achievements.required_unverifiable",
            });
        }
    }
    Ok(())
}

/// Facts derived from one successful resimulation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedRunV2 {
    /// Engine version recorded in the compact replay envelope; the viewer
    /// selects its runtime build by this value.
    pub recorded_engine_version: String,
    /// Exact replayed `SimConfig`.
    pub sim_config: CanonicalValue,
    pub max_concurrent_players: u16,
    pub participant_instance_count: u16,
    pub outcome: TerminalOutcomeV1,
    pub starting_campaign_score: i32,
    pub final_campaign_score: i32,
    pub original_score_delta: i64,
    /// SHA-256 of the terminal engine snapshot.
    pub final_state_sha256: Digest32,
    pub replay_frames: u32,
    pub active_simulation_ticks: u64,
    pub ransom_collected: u64,
    /// Complete verifier-derived achievement decisions, sorted by stable ID.
    pub achievements: Vec<VerifiedAchievementV1>,
}

impl Validate for VerifiedRunV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text(
            "verified_run.recorded_engine_version",
            &self.recorded_engine_version,
            64,
        )?;
        if !matches!(self.sim_config, CanonicalValue::Object(_)) {
            return Err(ValidationError::NotObject {
                field: "verified_run.sim_config",
            });
        }
        self.sim_config.validate_depth(64)?;
        if self.max_concurrent_players == 0
            || self.max_concurrent_players > crate::MAX_REPLAY_SEATS_V1
            || self.participant_instance_count < self.max_concurrent_players
            || self.participant_instance_count > crate::MAX_PARTICIPANT_INSTANCES_V1
        {
            return Err(ValidationError::EmptyPlayerCount);
        }
        crate::validation::nonzero("verified_run.final_state_sha256", &self.final_state_sha256)?;
        crate::validation::nonzero("verified_run.replay_frames", &self.replay_frames)?;
        let wrapped_score_delta = i64::from(
            self.final_campaign_score
                .wrapping_sub(self.starting_campaign_score) as u32,
        );
        if self.original_score_delta != wrapped_score_delta {
            return Err(ValidationError::InvalidOriginalScore);
        }
        validate_authoritative_achievements(&self.achievements)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "result", rename_all = "snake_case")]
pub enum VerificationStatusV2 {
    Verified(VerifiedRunV2),
    Rejected(VerificationRejectionV1),
    FailedInfrastructure(VerificationInfrastructureFailureV1),
}

/// Exact result document written by the verifier process. A job the verifier
/// cannot decode exits non-zero instead of writing this document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierOutputV2 {
    pub schema_version: u32,
    pub job_sha256: Digest32,
    /// SHA-256 of the replay bytes the verifier actually read.
    pub replay_sha256: Digest32,
    /// `None` only when verification failed before provenance could be decoded.
    pub input_provenance: Option<InputProvenanceStatusV1>,
    pub status: VerificationStatusV2,
}

impl Validate for VerifierOutputV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "VerifierOutputV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        crate::validation::nonzero("verifier_output.job_sha256", &self.job_sha256)?;
        crate::validation::nonzero("verifier_output.replay_sha256", &self.replay_sha256)?;
        if let Some(provenance) = &self.input_provenance {
            provenance.validate()?;
        }
        match &self.status {
            VerificationStatusV2::Verified(run) => {
                if self
                    .input_provenance
                    .as_ref()
                    .is_none_or(|provenance| !provenance.is_rankable())
                {
                    return Err(ValidationError::VerifiedRunNotRankable);
                }
                run.validate()
            }
            VerificationStatusV2::Rejected(rejection) => {
                if rejection.code == VerificationRejectionCodeV1::InputProvenanceIneligible
                    && self
                        .input_provenance
                        .as_ref()
                        .is_none_or(InputProvenanceStatusV1::is_rankable)
                {
                    return Err(ValidationError::InvalidInputProvenanceRejection);
                }
                rejection.validate()
            }
            VerificationStatusV2::FailedInfrastructure(failure) => failure.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn verified_run() -> VerifiedRunV2 {
        VerifiedRunV2 {
            recorded_engine_version: "0123456789ab".into(),
            sim_config: CanonicalValue::Object(BTreeMap::from([(
                "difficulty".into(),
                CanonicalValue::String("Medium".into()),
            )])),
            max_concurrent_players: 2,
            participant_instance_count: 3,
            outcome: TerminalOutcomeV1::Won,
            starting_campaign_score: 10,
            final_campaign_score: 25,
            original_score_delta: 15,
            final_state_sha256: Digest32::from_bytes([1; 32]),
            replay_frames: 100,
            active_simulation_ticks: 90,
            ransom_collected: 4,
            achievements: official_achievement_policies_v1()
                .into_iter()
                .map(|policy| VerifiedAchievementV1 {
                    achievement_id: policy.achievement_id,
                    evaluation: VerifiedAchievementEvaluationV1::NotEarned,
                    evidence: BTreeMap::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn verified_run_checks_counts_score_and_achievement_catalog() {
        assert!(verified_run().validate().is_ok());
        let mut score = verified_run();
        score.original_score_delta += 1;
        assert!(score.validate().is_err());
        let mut counts = verified_run();
        counts.participant_instance_count = 1;
        assert!(counts.validate().is_err());
        let mut catalog = verified_run();
        catalog.achievements.pop();
        assert!(catalog.validate().is_err());
        let mut unverifiable = verified_run();
        unverifiable.achievements[0].evaluation = VerifiedAchievementEvaluationV1::Unverifiable;
        assert!(unverifiable.validate().is_err());
    }

    #[test]
    fn verified_output_requires_rankable_provenance() {
        let mut output = VerifierOutputV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            job_sha256: Digest32::from_bytes([2; 32]),
            replay_sha256: Digest32::from_bytes([3; 32]),
            input_provenance: Some(InputProvenanceStatusV1::Rankable),
            status: VerificationStatusV2::Verified(verified_run()),
        };
        assert!(output.validate().is_ok());
        output.input_provenance = None;
        assert!(output.validate().is_err());
        output.status = VerificationStatusV2::Rejected(VerificationRejectionV1 {
            code: VerificationRejectionCodeV1::InputProvenanceIneligible,
            detail_code: None,
        });
        assert!(output.validate().is_err());
    }
}
