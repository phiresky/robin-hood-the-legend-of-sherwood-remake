//! Optional top-left achievement, speedrun, and detailed-XP trackers.

use robin_engine::{
    achievement::{
        AchievementAggregationProgress, AchievementAggregationStatus,
        AchievementAggregationSummary, AchievementEvaluation, AchievementId,
        AchievementTrackingProvenance, MissionAchievementResults,
    },
    engine::PresentationView,
    gameplay_config::GameplayConfig,
    player_command::PlayerId,
};

use crate::{hud_text::HudFonts, renderer::Renderer};

const TRACKER_X: i32 = 2;
const TRACKER_Y: i32 = 32;

/// Stable presentation metadata for one campaign-history badge. String keys
/// are asset/localization seams; callers render the fallback when a pack has
/// not supplied an override yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AchievementBadgePresentation {
    pub id: AchievementId,
    pub localization_key: &'static str,
    pub icon_key: &'static str,
    pub label: String,
    pub earned: bool,
}

fn badge_presentation(
    id: AchievementId,
    earned: robin_engine::achievement::AchievementSet,
    localize: &mut impl FnMut(&str) -> Option<String>,
) -> AchievementBadgePresentation {
    let (localization_key, icon_key) = match id {
        AchievementId::CleanHands => (
            "achievement.clean_hands.name",
            "achievement.clean_hands.icon",
        ),
        AchievementId::Ghost => ("achievement.ghost.name", "achievement.ghost.icon"),
        AchievementId::PileOBones => (
            "achievement.pile_o_bones.name",
            "achievement.pile_o_bones.icon",
        ),
        // TODO: supply translated names and dedicated art for the expanded
        // catalogue; protocol IDs are stable override keys in the meantime.
        _ => (id.protocol_id(), id.protocol_id()),
    };
    AchievementBadgePresentation {
        id,
        localization_key,
        icon_key,
        label: localize(localization_key).unwrap_or_else(|| id.name().to_owned()),
        earned: earned.contains(id),
    }
}

pub fn mission_badge_presentations(
    earned: robin_engine::achievement::AchievementSet,
    mut localize: impl FnMut(&str) -> Option<String>,
) -> Vec<AchievementBadgePresentation> {
    AchievementId::ALL
        .into_iter()
        .filter(|id| !id.campaign_only())
        .map(|id| badge_presentation(id, earned, &mut localize))
        .collect()
}

/// Stable catalogue order without constructing labels or other presentation data.
pub(crate) fn permanent_badge_ids() -> impl Iterator<Item = AchievementId> {
    AchievementId::ALL.into_iter().filter(|id| {
        id.aggregation_policy()
            != robin_engine::achievement::AchievementAggregationPolicy::MissionOnly
    })
}

pub fn permanent_badge_presentations(
    earned: robin_engine::achievement::AchievementSet,
) -> impl Iterator<Item = AchievementBadgePresentation> {
    permanent_badge_ids().map(move |id| badge_presentation(id, earned, &mut |_| None))
}

fn aggregation_status_text(status: AchievementAggregationStatus) -> &'static str {
    match status {
        AchievementAggregationStatus::InProgress => "IN PROGRESS",
        AchievementAggregationStatus::Unverifiable => "N/A",
        AchievementAggregationStatus::MissingRequirements => "MISSING",
        AchievementAggregationStatus::Earned => "MET",
    }
}

/// Compact, honest envelope status. A ratio is shown only when the engine has
/// a concrete requirement count; legacy/in-progress envelopes with no frozen
/// requirement set remain textual instead of displaying a made-up `0/0`.
pub fn format_aggregation_progress(progress: AchievementAggregationProgress) -> String {
    let status = aggregation_status_text(progress.status);
    if progress.required_missions == 0 {
        status.to_owned()
    } else {
        format!(
            "{status} {}/{}",
            progress.earned_missions, progress.required_missions
        )
    }
}

/// Count only permanent-envelope achievements, without formatting badge labels.
pub(crate) fn permanent_achievement_counts(
    summary: AchievementAggregationSummary,
) -> (usize, usize) {
    permanent_badge_ids().fold((0, 0), |(earned, total), id| {
        (earned + usize::from(summary.get(id).earned()), total + 1)
    })
}

fn evaluation_mark(evaluation: AchievementEvaluation) -> &'static str {
    match evaluation {
        AchievementEvaluation::Unverifiable => "N/A",
        AchievementEvaluation::NotApplicable => "UNAVAILABLE",
        AchievementEvaluation::Failed => "FAILED",
        AchievementEvaluation::Earned => "MET",
    }
}

/// Human-readable frozen attempt details for terminal debriefing/history UI.
pub fn format_attempt_summary(results: MissionAchievementResults) -> String {
    let metrics = results.metrics();
    let evaluation = |id| evaluation_mark(results.evaluation(id));
    let mut lines = vec!["Achievement conditions".to_owned()];
    if results.provenance() == AchievementTrackingProvenance::LegacyImportIncomplete {
        lines.push("Evidence unavailable for imported Original save".to_owned());
    }

    let clean_hands = results.evaluation(AchievementId::CleanHands);
    lines.push(if clean_hands == AchievementEvaluation::Unverifiable {
        format!(
            "{}: {}",
            "Clean Hands",
            evaluation(AchievementId::CleanHands)
        )
    } else {
        format!(
            "{}: {} ({} {}, {} {})",
            "Clean Hands",
            evaluation(AchievementId::CleanHands),
            "player-caused deaths",
            metrics.player_caused_deaths,
            "NPC-caused deaths",
            metrics.npc_caused_deaths,
        )
    });

    let ghost = results.evaluation(AchievementId::Ghost);
    lines.push(if ghost == AchievementEvaluation::Unverifiable {
        format!("{}: {}", "Ghost", evaluation(AchievementId::Ghost))
    } else {
        format!(
            "{}: {} ({} {}, {} {})",
            "Ghost",
            evaluation(AchievementId::Ghost),
            metrics.unique_hostile_observers,
            "observers",
            metrics.unique_observed_player_characters,
            "heroes",
        )
    });

    let pile = results.evaluation(AchievementId::PileOBones);
    lines.push(if pile == AchievementEvaluation::Unverifiable {
        format!(
            "{}: {}",
            "Pile-o-Bones",
            evaluation(AchievementId::PileOBones)
        )
    } else {
        format!(
            "{}: {} ({}/10)",
            "Pile-o-Bones",
            evaluation(AchievementId::PileOBones),
            metrics.max_bodies_in_one_building,
        )
    });

    for id in AchievementId::ALL.into_iter().skip(3) {
        if results.evaluation(id) == AchievementEvaluation::NotApplicable {
            continue;
        }
        lines.push(format!("{}: {}", id.name(), evaluation(id)));
        lines.push(id.description().to_owned());
    }
    if results.provenance() == AchievementTrackingProvenance::MissionStart {
        lines.push(format!("Enemies killed: {}/{}; rich civilians knocked out: {}/{}; beggars exhausted: {}/{}; banners purchased: {}/{}", metrics.dead_enemies, metrics.encountered_hostiles, metrics.rich_civilians_knocked_out, metrics.rich_civilians, metrics.beggars_exhausted, metrics.beggars, metrics.banners_purchased, metrics.purchasable_banners));
    }

    if results.provenance() == AchievementTrackingProvenance::MissionStart {
        lines.push(format!(
            "{}: {}",
            "Time",
            format_speedrun_clock(metrics.duration_frames),
        ));
    }
    lines.join("\n")
}

fn format_speedrun_clock(frames: u32) -> String {
    // The deterministic Original hourglass runs at exactly 25 Hz.
    let total_seconds = frames / 25;
    let hundredths = (frames % 25) * 4;
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    if total_minutes >= 60 {
        format!(
            "{:02}:{:02}:{:02}.{:02}",
            total_minutes / 60,
            total_minutes % 60,
            seconds,
            hundredths
        )
    } else {
        format!("{total_minutes:02}:{seconds:02}.{hundredths:02}")
    }
}

fn format_speedrun_time(frames: u32) -> String {
    format!("{} {}", "Time", format_speedrun_clock(frames))
}

/// Build the text independently of rendering so settings and exact counter
/// semantics remain straightforward to test.
pub fn tracker_lines(
    engine: &PresentationView<'_>,
    seat: PlayerId,
    config: GameplayConfig,
) -> Result<Vec<String>, String> {
    let progress = engine.achievement_progress();
    let metrics = progress.metrics;
    let mut lines = Vec::new();

    if config.show_speedrun_tracker {
        lines.push(format_speedrun_time(metrics.duration_frames));
    }
    if config.show_clean_hands_tracker {
        let evaluation = progress.evaluations.get(AchievementId::CleanHands);
        let npc_suffix = config
            .clean_hands_npc_kills_invalidate
            .then(|| format!(", {} {}", "NPC-caused deaths", metrics.npc_caused_deaths));
        lines.push(format!(
            "{}: {} ({} {}{})",
            "Clean Hands",
            evaluation_mark(evaluation),
            "player-caused deaths",
            metrics.player_caused_deaths,
            npc_suffix.unwrap_or_default()
        ));
    }
    if config.show_ghost_tracker {
        lines.push(format!(
            "{}: {} ({} {}, {} {})",
            "Ghost",
            evaluation_mark(progress.evaluations.get(AchievementId::Ghost)),
            metrics.unique_hostile_observers,
            "observers",
            metrics.unique_observed_player_characters,
            "heroes",
        ));
    }
    if config.show_pile_o_bones_tracker {
        lines.push(format!(
            "{}: {} ({}/10)",
            "Pile-o-Bones",
            evaluation_mark(progress.evaluations.get(AchievementId::PileOBones)),
            metrics.max_bodies_in_one_building.min(10)
        ));
    }
    if config.show_new_achievement_trackers {
        for id in AchievementId::ALL
            .into_iter()
            .skip(3)
            .filter(|id| !id.campaign_only())
        {
            let result = progress.evaluations.get(id);
            if result != AchievementEvaluation::NotApplicable {
                lines.push(format!("{}: {}", id.name(), evaluation_mark(result)));
            }
        }
        lines.push(format!(
            "Dead enemies {}/{}; rich civilians {}/{}; beggars {}/{}",
            metrics.dead_enemies,
            metrics.encountered_hostiles,
            metrics.rich_civilians_knocked_out,
            metrics.rich_civilians,
            metrics.beggars_exhausted,
            metrics.beggars
        ));
    }
    if config.show_detailed_xp {
        for &pc in engine.hero_selection(seat) {
            if !matches!(pc, robin_engine::element::EntityId::Pc(_)) {
                continue;
            }
            let xp = engine.pc_experience_snapshot(pc)?;
            let name = engine.pc_character_kind(pc).ok_or_else(|| {
                format!(
                    "player character {} has no required character identity",
                    pc.index()
                )
            })?;
            lines.push(format!(
                "{} XP: {} {} ({}/100), {} {} ({}/100)",
                name.profile_name(),
                "Sword",
                xp.hand_to_hand.capacity,
                xp.hand_to_hand.experience,
                "Bow",
                xp.bow.capacity,
                xp.bow.experience
            ));
        }
    }
    Ok(lines)
}

pub fn render_trackers(
    engine: &PresentationView<'_>,
    seat: PlayerId,
    config: GameplayConfig,
    renderer: &mut Renderer,
    fonts: &HudFonts,
) {
    let lines = tracker_lines(engine, seat, config)
        .unwrap_or_else(|error| panic!("cannot render achievement HUD: {error}"));
    let line_height = i32::try_from(fonts.tooltip_font.height())
        .expect("achievement HUD font height exceeds i32")
        .max(12);
    for (index, line) in lines.iter().enumerate() {
        let y = TRACKER_Y
            + i32::try_from(index).expect("achievement HUD line count exceeds i32") * line_height;
        crate::hud_text::render_text_background(
            &fonts.tooltip_font,
            fonts.shadow_font.as_ref(),
            line,
            TRACKER_X,
            y,
            |font, text, x, y| {
                crate::ingame_menu::layout::render_text_screen_font(renderer, font, text, x, y)
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_aggregation_progress, format_attempt_summary, format_speedrun_time,
        mission_badge_presentations, permanent_achievement_counts,
    };

    #[test]
    fn permanent_badge_catalogue_and_lazy_pages_share_order_and_earned_state() {
        use robin_engine::achievement::{
            AchievementAggregationPolicy, AchievementId, AchievementSet,
        };
        let expected_ids: Vec<_> = AchievementId::ALL
            .into_iter()
            .filter(|id| id.aggregation_policy() != AchievementAggregationPolicy::MissionOnly)
            .collect();
        assert_eq!(
            super::permanent_badge_ids().collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(super::permanent_badge_ids().count(), expected_ids.len());
        for earned in [
            AchievementSet::empty(),
            AchievementSet::from_ids([AchievementId::Ghost]),
            AchievementSet::from_ids(AchievementId::ALL),
        ] {
            let badges: Vec<_> = super::permanent_badge_presentations(earned).collect();
            assert_eq!(
                badges.iter().map(|badge| badge.id).collect::<Vec<_>>(),
                expected_ids
            );
            for badge in &badges {
                assert_eq!(badge.earned, earned.contains(badge.id));
                assert_eq!(badge.label, badge.id.name());
            }
            for offset in 0..=badges.len() + 1 {
                let page: Vec<_> = super::permanent_badge_presentations(earned)
                    .skip(offset)
                    .take(4)
                    .collect();
                assert_eq!(
                    page,
                    badges
                        .iter()
                        .skip(offset)
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn speedrun_time_uses_exact_25_hz_clock() {
        assert_eq!(format_speedrun_time(0), "Time 00:00.00");
        assert_eq!(format_speedrun_time(24), "Time 00:00.96");
        assert_eq!(format_speedrun_time(25 * 65 + 12), "Time 01:05.48");
        assert_eq!(format_speedrun_time(25 * 3_661), "Time 01:01:01.00");
    }

    #[test]
    fn campaign_badges_remain_individual_and_localizable() {
        use robin_engine::achievement::{AchievementId, AchievementSet};

        let earned = AchievementSet::from_ids([AchievementId::Ghost]);
        let badges = mission_badge_presentations(earned, |key| {
            (key == "achievement.clean_hands.name").then(|| "Saubere Hände".to_string())
        });
        assert_eq!(badges.len(), 10);
        assert_eq!(badges[0].label, "Saubere Hände");
        assert!(!badges[0].earned);
        assert_eq!(badges[1].id, AchievementId::Ghost);
        assert!(badges[1].earned);
        assert_eq!(badges[2].id, AchievementId::Ruthless);
        assert_eq!(badges[3].id, AchievementId::ImOffHome);
        assert!(badges.iter().all(|badge| !badge.id.campaign_only()));
    }

    #[test]
    fn aggregate_counts_and_formatting_keep_four_typed_statuses_distinct() {
        use robin_engine::achievement::{
            AchievementAggregationInput, AchievementAggregationStatus, AchievementId,
            AchievementSet,
        };

        let summary =
            robin_engine::achievement::AchievementAggregationSummary::from_inputs(|id| match id {
                AchievementId::CleanHands => AchievementAggregationInput {
                    required_missions: 3,
                    earned_missions: 1,
                    ..Default::default()
                },
                AchievementId::Ghost => AchievementAggregationInput {
                    envelope_complete: true,
                    required_missions: 3,
                    earned_missions: 3,
                    ..Default::default()
                },
                AchievementId::PileOBones => AchievementAggregationInput {
                    required_missions: 1,
                    unverifiable_missions: 1,
                    ..Default::default()
                },
                _ => AchievementAggregationInput {
                    envelope_complete: true,
                    required_missions: 1,
                    ..Default::default()
                },
            });
        for (id, status, text) in [
            (
                AchievementId::CleanHands,
                AchievementAggregationStatus::InProgress,
                "IN PROGRESS 1/3",
            ),
            (
                AchievementId::Ghost,
                AchievementAggregationStatus::Earned,
                "MET 3/3",
            ),
            (
                AchievementId::PileOBones,
                AchievementAggregationStatus::Unverifiable,
                "N/A 0/1",
            ),
            (
                super::permanent_badge_ids().nth(3).unwrap(),
                AchievementAggregationStatus::MissingRequirements,
                "MISSING 0/1",
            ),
        ] {
            let progress = summary.get(id);
            assert_eq!(progress.status, status);
            assert_eq!(format_aggregation_progress(progress), text);
        }
        assert_eq!(
            permanent_achievement_counts(summary),
            (1, super::permanent_badge_ids().count())
        );
        assert_eq!(
            summary.earned(),
            AchievementSet::from_ids([AchievementId::Ghost])
        );
    }

    #[test]
    fn aggregate_progress_without_an_envelope_never_prints_zero_over_zero() {
        use robin_engine::achievement::{AchievementAggregationInput, AchievementId};

        let progress = robin_engine::achievement::aggregate_achievement(
            AchievementId::CleanHands,
            AchievementAggregationInput::default(),
        );
        assert_eq!(format_aggregation_progress(progress), "IN PROGRESS");
    }

    #[test]
    fn incomplete_import_summary_does_not_present_unknown_counters_as_zero() {
        let mut state =
            robin_engine::achievement::MissionAchievementState::from_incomplete_legacy_import();
        let summary = format_attempt_summary(*state.finalize_success());

        assert!(summary.contains("Evidence unavailable"));
        assert!(summary.contains("Clean Hands: N/A"));
        assert!(summary.contains("Ruthless: N/A"));
        assert!(!summary.contains("0/10"));
        assert!(!summary.contains("0/0"));
        assert!(!summary.contains("Time:"));
    }
}
