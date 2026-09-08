//! Lossless projection of engine-derived terminal facts into verifier output.
//!
//! This module does not construct [`robin_run_protocol::VerifiedRunV1`]. It
//! only converts facts already frozen by the deterministic engine and checks
//! them against the exact ruleset catalog. The worker may consume the result
//! only after the sealed resimulation path has independently succeeded.

use std::collections::BTreeMap;

use robin_engine::achievement::{
    AchievementAttemptMetrics, AchievementEvaluation, AchievementId, AchievementTrackingProvenance,
    MissionAchievementResults,
};
use robin_run_protocol::{
    CanonicalValue, OpaqueId, RulesetManifestV1, VerifiedAchievementEvaluationV1,
    VerifiedAchievementV1,
};

#[derive(Debug, thiserror::Error)]
pub enum AchievementProjectionError {
    #[error("ruleset contains an achievement ID unknown to this verifier: {0}")]
    UnknownRulesetAchievement(String),
    #[error("engine achievement ID is not a valid protocol identifier: {0}")]
    InvalidEngineAchievementId(String),
    #[error("authoritative achievement results violate the exact ruleset catalog: {0}")]
    RulesetValidation(String),
}

/// Project every and only achievement configured by `ruleset`, retaining the
/// engine's exact three-way decision and every frozen attempt counter.
///
/// `RulesetManifestV1::validate_authoritative_achievements` deliberately runs
/// last. It therefore rejects a missing/extra ID and a `Required` result that
/// the engine marked `Unverifiable`; this function never substitutes
/// `NotEarned` for missing evidence.
pub fn project_authoritative_achievements(
    results: MissionAchievementResults,
    ruleset: &RulesetManifestV1,
) -> Result<Vec<VerifiedAchievementV1>, AchievementProjectionError> {
    let evidence = achievement_evidence(results.provenance(), results.metrics());
    let mut projected = Vec::with_capacity(ruleset.achievement_policies.len());
    for policy in &ruleset.achievement_policies {
        let engine_id = engine_achievement_id(policy.achievement_id.as_str())?;
        let achievement_id = OpaqueId::new(engine_id.protocol_id()).map_err(|error| {
            AchievementProjectionError::InvalidEngineAchievementId(error.to_string())
        })?;
        projected.push(VerifiedAchievementV1 {
            achievement_id,
            evaluation: project_evaluation(results.evaluation(engine_id)),
            evidence: evidence.clone(),
        });
    }
    ruleset
        .validate_authoritative_achievements(&projected)
        .map_err(|error| AchievementProjectionError::RulesetValidation(error.to_string()))?;
    Ok(projected)
}

fn engine_achievement_id(id: &str) -> Result<AchievementId, AchievementProjectionError> {
    AchievementId::ALL
        .into_iter()
        .find(|achievement| achievement.protocol_id() == id)
        .ok_or_else(|| AchievementProjectionError::UnknownRulesetAchievement(id.to_owned()))
}

const fn project_evaluation(evaluation: AchievementEvaluation) -> VerifiedAchievementEvaluationV1 {
    match evaluation {
        AchievementEvaluation::Unverifiable => VerifiedAchievementEvaluationV1::Unverifiable,
        AchievementEvaluation::Failed => VerifiedAchievementEvaluationV1::NotEarned,
        AchievementEvaluation::Earned => VerifiedAchievementEvaluationV1::Earned,
    }
}

fn achievement_evidence(
    provenance: AchievementTrackingProvenance,
    metrics: AchievementAttemptMetrics,
) -> BTreeMap<String, CanonicalValue> {
    let provenance = match provenance {
        AchievementTrackingProvenance::MissionStart => "mission_start",
        AchievementTrackingProvenance::LegacyImportIncomplete => "legacy_import_incomplete",
    };
    BTreeMap::from([
        (
            "baseline_dead_npcs".into(),
            CanonicalValue::Unsigned(u64::from(metrics.baseline_dead_npcs)),
        ),
        (
            "baseline_living_npcs".into(),
            CanonicalValue::Unsigned(u64::from(metrics.baseline_living_npcs)),
        ),
        (
            "duration_frames".into(),
            CanonicalValue::Unsigned(u64::from(metrics.duration_frames)),
        ),
        (
            "encountered_hostiles".into(),
            CanonicalValue::Unsigned(u64::from(metrics.encountered_hostiles)),
        ),
        (
            "enemies_in_stash_building".into(),
            CanonicalValue::Unsigned(u64::from(metrics.enemies_in_stash_building)),
        ),
        (
            "enemies_required_for_stash".into(),
            CanonicalValue::Unsigned(u64::from(metrics.enemies_required_for_stash)),
        ),
        (
            "max_bodies_in_one_building".into(),
            CanonicalValue::Unsigned(u64::from(metrics.max_bodies_in_one_building)),
        ),
        (
            "npc_caused_deaths".into(),
            CanonicalValue::Unsigned(u64::from(metrics.npc_caused_deaths)),
        ),
        (
            "player_caused_deaths".into(),
            CanonicalValue::Unsigned(u64::from(metrics.player_caused_deaths)),
        ),
        (
            "tracking_provenance".into(),
            CanonicalValue::String(provenance.into()),
        ),
        (
            "unique_hostile_observers".into(),
            CanonicalValue::Unsigned(u64::from(metrics.unique_hostile_observers)),
        ),
        (
            "unique_observed_player_characters".into(),
            CanonicalValue::Unsigned(u64::from(metrics.unique_observed_player_characters)),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::achievement::MissionAchievementState;
    use robin_run_protocol::{
        AchievementPolicyModeV1, ActiveTimeDefinitionV1, AnonymousParticipantPolicyV1,
        BoardMetricV1, CampaignAggregationConsentPolicyV1, CampaignCompletionPolicyRequirementV1,
        CampaignRosterContinuityV1, CanonicalCampaignStateKindV1,
        CanonicalCampaignStateRequirementV1, CanonicalStartPolicyV1, FrameCountingPolicyV1,
        FullCampaignChainPolicyV1, FullCampaignTimeAggregationV1, ImmutablePolicyIdentityV1,
        ImmutablePolicyKindV1, InputProvenanceEligibilityV1, MetricRankingPolicyV1,
        NamedParticipantPolicyV1, PaginationTieBreakV1, ParticipantEligibilityV1,
        RulesConfigConstraintV1, RulesetBoardScopeV1, RulesetSeedPolicyV1, RunCompositionPolicyV1,
        ScoreAlgorithmV1, ScoreOverflowPolicyV1, TerminalResultPolicyV1, TickDurationV1,
        VisibleTiePolicyV1, official_achievement_policies_v1,
        official_full_campaign_completion_policy_v1,
    };

    fn policy_identity(kind: ImmutablePolicyKindV1, byte: u8) -> ImmutablePolicyIdentityV1 {
        ImmutablePolicyIdentityV1 {
            kind,
            version: 1,
            manifest_sha256: robin_run_protocol::Digest32::from_bytes([byte; 32]),
        }
    }

    fn official_ruleset() -> RulesetManifestV1 {
        RulesetManifestV1 {
            schema_version: 1,
            display_name: "Standard / Normal".into(),
            preset_id: OpaqueId::new("standard").unwrap(),
            preset_name: "Standard".into(),
            difficulty_id: OpaqueId::new("normal").unwrap(),
            difficulty_name: "Normal".into(),
            rules_config_sha256: robin_run_protocol::Digest32::from_bytes([7; 32]),
            rules_config_constraint: RulesConfigConstraintV1::ExactCanonicalDigestOnly,
            allowed_build_manifest_sha256: vec![robin_run_protocol::Digest32::from_bytes([8; 32])],
            allowed_content_manifest_sha256: vec![robin_run_protocol::Digest32::from_bytes([
                9; 32
            ])],
            allowed_campaign_content_manifest_sha256: vec![
                robin_run_protocol::Digest32::from_bytes([10; 32]),
            ],
            board_scopes: vec![
                RulesetBoardScopeV1::IndividualLevel,
                RulesetBoardScopeV1::CampaignMission,
                RulesetBoardScopeV1::FullCampaign,
            ],
            metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
            metric_ranking: vec![
                MetricRankingPolicyV1::OriginalScoreDescending,
                MetricRankingPolicyV1::FastestSuccessAscending,
            ],
            achievement_policies: official_achievement_policies_v1(),
            campaign_completion_policy: CampaignCompletionPolicyRequirementV1::Required(
                official_full_campaign_completion_policy_v1(),
            ),
            canonical_start_policy:
                CanonicalStartPolicyV1::RulesConfigBoundOperatorStateAndVerifiedPredecessor,
            canonical_campaign_state: CanonicalCampaignStateRequirementV1 {
                edition: robin_run_protocol::OfficialContentEditionV1::Full,
                kind: CanonicalCampaignStateKindV1::FullCampaignGenesis,
                rules_config_sha256: robin_run_protocol::Digest32::from_bytes([7; 32]),
            },
            run_preflight_grant_public_key: robin_run_protocol::PublicKey32::from_bytes([11; 32]),
            full_campaign_chain_policy:
                FullCampaignChainPolicyV1::CanonicalGenesisEveryFieldAndHeadquartersSessionIndependentCompletion,
            campaign_roster_continuity:
                CampaignRosterContinuityV1::UnionOfVerifiedSessionSubsets,
            campaign_aggregation_consent_policy:
                CampaignAggregationConsentPolicyV1::EveryAuthenticatedKeyFinalCosignsEachSession,
            participant_eligibility: ParticipantEligibilityV1 {
                allow_single_player: true,
                allow_multiplayer: true,
                named_policy:
                    NamedParticipantPolicyV1::HostGenesisGuestTransportJoinAttestationAndFinalCosign,
                anonymous_policy:
                    AnonymousParticipantPolicyV1::AllowedAuthenticatedButPubliclyRedacted,
                minimum_max_concurrent_players: 1,
                maximum_max_concurrent_players: robin_run_protocol::MAX_REPLAY_SEATS_V1,
                maximum_participant_instances:
                    robin_run_protocol::MAX_PARTICIPANT_INSTANCES_V1,
            },
            replay_schema_versions: vec![
                robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            ],
            network_protocol_versions: vec![robin_engine::multiplayer::NET_PROTOCOL_VERSION],
            input_provenance_policy: policy_identity(
                ImmutablePolicyKindV1::InputProvenance,
                11,
            ),
            command_admission_policy: policy_identity(
                ImmutablePolicyKindV1::CommandAdmission,
                12,
            ),
            submission_admission_policy: policy_identity(
                ImmutablePolicyKindV1::SubmissionAdmission,
                13,
            ),
            verifier_policy: policy_identity(ImmutablePolicyKindV1::Verification, 14),
            input_provenance_eligibility:
                InputProvenanceEligibilityV1::CurrentSchemaCanonicalReplayOnly,
            terminal_result_policy: TerminalResultPolicyV1::IndependentlyReachedWonOnly,
            score_algorithm:
                ScoreAlgorithmV1::OriginalMissionAttemptWrappingSubtotalCampaignDeltaV1,
            score_overflow_policy: ScoreOverflowPolicyV1::RejectCampaignOrAggregateOverflow,
            visible_tie_policy: VisibleTiePolicyV1::EqualPrimaryMetricSharesRank,
            pagination_tie_break:
                PaginationTieBreakV1::AcceptedSequenceThenVerificationTimeThenRunIdOnly,
            tick_duration: TickDurationV1 {
                numerator_micros: 50_000,
                denominator: 1,
            },
            active_time_definition: ActiveTimeDefinitionV1::SuccessfulSimulationTicks,
            frame_counting_policy:
                FrameCountingPolicyV1::ZeroBasedEventsBeforeExclusiveReplayFrameCount,
            full_campaign_time_aggregation:
                FullCampaignTimeAggregationV1::CheckedSumEveryVerifiedFieldAndHeadquartersSession,
            run_composition_policy:
                RunCompositionPolicyV1::MissionSingleReplayFullCampaignOrderedSessionsNoSyntheticReplay,
            main_board_seed_policy: RulesetSeedPolicyV1::Open,
            competition_seed_policy: RulesetSeedPolicyV1::ServerPinned,
            allow_save_creation: true,
            allow_autosave: true,
            allow_state_load: false,
            allow_mission_restart: false,
        }
    }

    #[test]
    fn evidence_projects_every_counter_without_loss() {
        let metrics = AchievementAttemptMetrics {
            duration_frames: 1,
            baseline_living_npcs: 2,
            baseline_dead_npcs: 3,
            encountered_hostiles: 4,
            player_caused_deaths: 5,
            npc_caused_deaths: 6,
            unique_hostile_observers: 7,
            unique_observed_player_characters: 8,
            max_bodies_in_one_building: 9,
            enemies_in_stash_building: 10,
            enemies_required_for_stash: 11,
        };
        let evidence = achievement_evidence(AchievementTrackingProvenance::MissionStart, metrics);
        assert_eq!(evidence.len(), 12);
        assert_eq!(evidence["duration_frames"], CanonicalValue::Unsigned(1));
        assert_eq!(
            evidence["enemies_required_for_stash"],
            CanonicalValue::Unsigned(11)
        );
        assert_eq!(
            evidence["tracking_provenance"],
            CanonicalValue::String("mission_start".into())
        );
    }

    #[test]
    fn engine_evaluations_are_mapped_losslessly() {
        assert_eq!(
            project_evaluation(AchievementEvaluation::Unverifiable),
            VerifiedAchievementEvaluationV1::Unverifiable
        );
        assert_eq!(
            project_evaluation(AchievementEvaluation::Failed),
            VerifiedAchievementEvaluationV1::NotEarned
        );
        assert_eq!(
            project_evaluation(AchievementEvaluation::Earned),
            VerifiedAchievementEvaluationV1::Earned
        );
    }

    #[test]
    fn unknown_ruleset_achievement_fails_closed() {
        assert!(matches!(
            engine_achievement_id("future-achievement"),
            Err(AchievementProjectionError::UnknownRulesetAchievement(_))
        ));
    }

    #[test]
    fn required_unverifiable_engine_result_is_rejected_without_substitution() {
        let mut incomplete = MissionAchievementState::from_incomplete_legacy_import();
        let results = *incomplete.finalize_success();
        let ruleset = official_ruleset();
        assert!(matches!(
            project_authoritative_achievements(results, &ruleset),
            Err(AchievementProjectionError::RulesetValidation(_))
        ));

        let mut diagnostic_ruleset = ruleset;
        for policy in &mut diagnostic_ruleset.achievement_policies {
            policy.mode = AchievementPolicyModeV1::Reported;
        }
        let projected = project_authoritative_achievements(results, &diagnostic_ruleset)
            .expect("reported achievements retain typed unverifiable evidence");
        assert!(projected.iter().all(|achievement| {
            achievement.evaluation == VerifiedAchievementEvaluationV1::Unverifiable
        }));
    }
}
