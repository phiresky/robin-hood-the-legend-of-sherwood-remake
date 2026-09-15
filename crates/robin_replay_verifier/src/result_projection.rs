//! Lossless projection of engine-derived achievement results into verifier output.
//!
//! This module only converts facts already frozen by the deterministic engine
//! and checks them against the official achievement catalog.

use std::collections::BTreeMap;

use robin_engine::achievement::{
    AchievementAttemptMetrics, AchievementEvaluation, AchievementId, AchievementTrackingProvenance,
    MissionAchievementResults,
};
use robin_run_protocol::{
    CanonicalValue, OpaqueId, VerifiedAchievementEvaluationV1, VerifiedAchievementV1,
    official_achievement_policies_v1, validate_authoritative_achievements,
};

#[derive(Debug, thiserror::Error)]
pub enum AchievementProjectionError {
    #[error("official catalog contains an achievement ID unknown to this verifier: {0}")]
    UnknownCatalogAchievement(String),
    #[error("engine achievement ID is not a valid protocol identifier: {0}")]
    InvalidEngineAchievementId(String),
    #[error("achievement results violate the official catalog: {0}")]
    CatalogValidation(String),
}

/// Project every achievement in the official catalog, retaining the engine's
/// award decision, applicability, and every frozen attempt counter.
///
/// Catalog validation runs last. It rejects a `Required` result that the
/// engine marked `Unverifiable`; this function never substitutes `NotEarned`
/// for missing evidence.
pub fn project_authoritative_achievements(
    results: MissionAchievementResults,
) -> Result<Vec<VerifiedAchievementV1>, AchievementProjectionError> {
    let evidence = achievement_evidence(results.provenance(), results.metrics());
    let policies = official_achievement_policies_v1();
    let mut projected = Vec::with_capacity(policies.len());
    for policy in &policies {
        let engine_id = engine_achievement_id(policy.achievement_id.as_str())?;
        let achievement_id = OpaqueId::new(engine_id.protocol_id()).map_err(|error| {
            AchievementProjectionError::InvalidEngineAchievementId(error.to_string())
        })?;
        let mut evidence = evidence.clone();
        evidence.insert(
            "applicable".into(),
            CanonicalValue::Bool(
                results.evaluation(engine_id) != AchievementEvaluation::NotApplicable,
            ),
        );
        projected.push(VerifiedAchievementV1 {
            achievement_id,
            evaluation: project_evaluation(results.evaluation(engine_id)),
            evidence,
        });
    }
    validate_authoritative_achievements(&projected)
        .map_err(|error| AchievementProjectionError::CatalogValidation(error.to_string()))?;
    Ok(projected)
}

fn engine_achievement_id(id: &str) -> Result<AchievementId, AchievementProjectionError> {
    AchievementId::ALL
        .into_iter()
        .find(|achievement| achievement.protocol_id() == id)
        .ok_or_else(|| AchievementProjectionError::UnknownCatalogAchievement(id.to_owned()))
}

const fn project_evaluation(evaluation: AchievementEvaluation) -> VerifiedAchievementEvaluationV1 {
    match evaluation {
        AchievementEvaluation::Unverifiable => VerifiedAchievementEvaluationV1::Unverifiable,
        // The wire has three award states. Unavailable is not earned; retain
        // the distinction in per-achievement evidence.
        AchievementEvaluation::Failed | AchievementEvaluation::NotApplicable => {
            VerifiedAchievementEvaluationV1::NotEarned
        }
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
    let counter = |value: u32| CanonicalValue::Unsigned(u64::from(value));
    BTreeMap::from([
        (
            "baseline_dead_npcs".into(),
            counter(metrics.baseline_dead_npcs),
        ),
        (
            "baseline_living_npcs".into(),
            counter(metrics.baseline_living_npcs),
        ),
        ("duration_frames".into(), counter(metrics.duration_frames)),
        (
            "encountered_hostiles".into(),
            counter(metrics.encountered_hostiles),
        ),
        ("dead_enemies".into(), counter(metrics.dead_enemies)),
        ("rich_civilians".into(), counter(metrics.rich_civilians)),
        (
            "rich_civilians_knocked_out".into(),
            counter(metrics.rich_civilians_knocked_out),
        ),
        ("beggars".into(), counter(metrics.beggars)),
        (
            "beggars_exhausted".into(),
            counter(metrics.beggars_exhausted),
        ),
        (
            "charitable_payments".into(),
            counter(metrics.charitable_payments),
        ),
        (
            "banners_purchased".into(),
            counter(metrics.banners_purchased),
        ),
        (
            "purchasable_banners".into(),
            counter(metrics.purchasable_banners),
        ),
        (
            "max_bodies_in_one_building".into(),
            counter(metrics.max_bodies_in_one_building),
        ),
        (
            "npc_caused_deaths".into(),
            counter(metrics.npc_caused_deaths),
        ),
        (
            "player_caused_deaths".into(),
            counter(metrics.player_caused_deaths),
        ),
        (
            "tracking_provenance".into(),
            CanonicalValue::String(provenance.into()),
        ),
        (
            "unique_hostile_observers".into(),
            counter(metrics.unique_hostile_observers),
        ),
        (
            "unique_observed_player_characters".into(),
            counter(metrics.unique_observed_player_characters),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::achievement::MissionAchievementState;

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
            dead_enemies: 10,
            rich_civilians: 11,
            rich_civilians_knocked_out: 12,
            beggars: 13,
            beggars_exhausted: 14,
            charitable_payments: 15,
            banners_purchased: 16,
            purchasable_banners: 17,
        };
        let evidence = achievement_evidence(AchievementTrackingProvenance::MissionStart, metrics);
        assert_eq!(evidence.len(), 18);
        assert_eq!(evidence["duration_frames"], CanonicalValue::Unsigned(1));
        assert_eq!(
            evidence["purchasable_banners"],
            CanonicalValue::Unsigned(17)
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
    fn every_official_catalog_achievement_is_known_to_the_engine() {
        for policy in official_achievement_policies_v1() {
            engine_achievement_id(policy.achievement_id.as_str()).unwrap();
        }
        assert!(matches!(
            engine_achievement_id("future-achievement"),
            Err(AchievementProjectionError::UnknownCatalogAchievement(_))
        ));
    }

    #[test]
    fn required_unverifiable_engine_result_is_rejected_without_substitution() {
        let mut incomplete = MissionAchievementState::from_incomplete_legacy_import();
        let results = *incomplete.finalize_success();
        assert!(matches!(
            project_authoritative_achievements(results),
            Err(AchievementProjectionError::CatalogValidation(_))
        ));
    }
}
