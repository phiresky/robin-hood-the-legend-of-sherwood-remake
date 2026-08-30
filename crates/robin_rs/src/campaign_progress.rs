//! Pure campaign graph and modal Hall-of-Deeds presentation models.

use std::collections::HashMap;

use robin_engine::achievement::{AchievementAggregationSummary, AchievementSet};
use robin_engine::campaign::Campaign;
use robin_engine::campaign_history::{MissionAttempt, MissionAttemptOutcome};
use robin_engine::mission::MissionStatus;
use robin_engine::profiles::{MissionLocation, ProfileManager};
use serde::{Deserialize, Serialize};

/// Effective per-mission badge row shown by every campaign presentation.
/// Current-campaign evidence is retained, while the profile archive restores
/// badges after a replaceable campaign slot is reset.
pub(crate) fn combined_mission_badges(
    mut current: AchievementSet,
    mission_id: u32,
    lifetime: Option<&robin_engine::campaign_history::ProfileCampaignHistory>,
) -> AchievementSet {
    if let Some(history) = lifetime {
        current.union_with(history.eligible_badges_for_mission(mission_id));
    }
    current
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionProgressState {
    Locked,
    Available,
    Completed,
    Lost,
    Expired,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionBestStats {
    pub fastest_win_seconds: Option<u32>,
    pub highest_score: Option<u32>,
    pub most_money: Option<u32>,
    pub most_soldiers_preserved: Option<u32>,
}

impl MissionBestStats {
    fn include(&mut self, attempt: &MissionAttempt) {
        if attempt.outcome() == MissionAttemptOutcome::Won
            && let Some(duration) = attempt.duration_seconds()
        {
            self.fastest_win_seconds = Some(
                self.fastest_win_seconds
                    .map_or(duration, |best| best.min(duration)),
            );
        }
        if let Some(score) = attempt.stats().added_score {
            self.highest_score = Some(self.highest_score.map_or(score, |best| best.max(score)));
        }
        if let Some(money) = attempt.stats().collected_money {
            self.most_money = Some(self.most_money.map_or(money, |best| best.max(money)));
        }
        if let (Some(living), Some(total)) = (
            attempt.stats().living_soldiers,
            attempt.stats().total_soldiers,
        ) && total != 0
        {
            let preserved = living.saturating_mul(100) / total;
            self.most_soldiers_preserved = Some(
                self.most_soldiers_preserved
                    .map_or(preserved, |best| best.max(preserved)),
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignProgressNode {
    pub mission_idx: usize,
    pub mission_id: u32,
    pub name: String,
    pub location: MissionLocation,
    pub state: MissionProgressState,
    pub prerequisite_nodes: Vec<usize>,
    pub depth: usize,
    pub lane: usize,
    pub attempt_count: usize,
    pub win_count: usize,
    pub best: MissionBestStats,
    /// Policy-attested badges earned for this mission across eligible runs.
    pub badges: AchievementSet,
    pub badge_count: usize,
    pub lifetime_attempt_count: usize,
    pub lifetime_win_count: usize,
    pub selectable: bool,
    pub history_replay: bool,
}

impl CampaignProgressNode {
    pub fn summary(&self, include_badges: bool) -> String {
        let mut summary = format!(
            "{}  |  {:?}  |  {} attempt{} / {} win{}",
            self.name,
            self.state,
            self.attempt_count,
            if self.attempt_count == 1 { "" } else { "s" },
            self.win_count,
            if self.win_count == 1 { "" } else { "s" },
        );
        if let Some(seconds) = self.best.fastest_win_seconds {
            summary.push_str(&format!("  |  best {seconds}s"));
        }
        if let Some(score) = self.best.highest_score {
            summary.push_str(&format!("  |  score {score}"));
        }
        if include_badges && self.badge_count != 0 {
            summary.push_str(&format!("  |  {} badge(s)", self.badge_count));
        }
        if self.lifetime_attempt_count != self.attempt_count {
            summary.push_str(&format!(
                "  |  lifetime {} attempt(s) / {} win(s)",
                self.lifetime_attempt_count, self.lifetime_win_count
            ));
        }
        summary
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignProgressGraph {
    pub nodes: Vec<CampaignProgressNode>,
    pub completed_missions: usize,
    pub known_missions: usize,
    /// Derived achievement envelope for the replaceable current campaign.
    pub campaign_achievements: AchievementAggregationSummary,
    /// Derived all-time envelope retained by the active player profile.
    pub lifetime_achievements: AchievementAggregationSummary,
    /// True when cyclic prerequisite input was detected. The graph remains
    /// inspectable; cyclic nodes are placed after the resolved frontier.
    pub cyclic_prerequisites: bool,
}

impl CampaignProgressGraph {
    pub fn build(
        campaign: &Campaign,
        profiles: &ProfileManager,
        lifetime: Option<&robin_engine::campaign_history::ProfileCampaignHistory>,
    ) -> Self {
        let campaign_achievements = campaign.achievement_aggregation(profiles);
        let lifetime_achievements = lifetime
            .map(|history| history.achievement_aggregation())
            .unwrap_or(campaign_achievements);
        let sherwood = campaign.get_sherwood_mission_idx();
        let mut mission_to_node = HashMap::new();
        let mut nodes = Vec::new();
        for (mission_idx, mission) in campaign.missions.iter().enumerate() {
            if mission_idx == sherwood {
                continue;
            }
            let profile = mission.profile(profiles);
            let attempts = mission.attempt_history().attempts();
            let current_has_win = attempts
                .iter()
                .any(|attempt| attempt.outcome() == MissionAttemptOutcome::Won)
                || mission.status == MissionStatus::Won;
            let accessible = campaign.accessible_mission_indices.contains(&mission_idx);
            let expired = mission.age >= profile.life_time;
            let state = if current_has_win {
                MissionProgressState::Completed
            } else if accessible {
                MissionProgressState::Available
            } else if mission.status == MissionStatus::Lost {
                MissionProgressState::Lost
            } else if expired {
                MissionProgressState::Expired
            } else {
                MissionProgressState::Locked
            };
            let mut best = MissionBestStats::default();
            for attempt in attempts {
                best.include(attempt);
            }
            let node_idx = nodes.len();
            mission_to_node.insert(profile.id, node_idx);
            let lifetime_attempt_count = lifetime
                .map(|history| {
                    history
                        .attempts()
                        .iter()
                        .filter(|entry| entry.mission_id() == profile.id)
                        .count()
                })
                .unwrap_or(attempts.len());
            let lifetime_win_count = lifetime
                .map(|history| {
                    history
                        .attempts()
                        .iter()
                        .filter(|entry| {
                            entry.mission_id() == profile.id
                                && entry.attempt().outcome() == MissionAttemptOutcome::Won
                        })
                        .count()
                })
                .unwrap_or_else(|| {
                    attempts
                        .iter()
                        .filter(|attempt| attempt.outcome() == MissionAttemptOutcome::Won)
                        .count()
                });
            let badges =
                combined_mission_badges(mission.achievement_badges(), profile.id, lifetime);
            nodes.push(CampaignProgressNode {
                mission_idx,
                mission_id: profile.id,
                name: profile.mission_name.clone(),
                location: profile.location,
                state,
                prerequisite_nodes: Vec::new(),
                depth: 0,
                lane: 0,
                attempt_count: attempts.len(),
                win_count: attempts
                    .iter()
                    .filter(|attempt| attempt.outcome() == MissionAttemptOutcome::Won)
                    .count(),
                best,
                // Raw calculated results remain on every immutable attempt for
                // debrief/audit. Only the host-policy-approved achievement
                // history is allowed to drive awarded badge presentation.
                badges,
                badge_count: badges.len(),
                lifetime_attempt_count,
                lifetime_win_count,
                selectable: accessible || current_has_win,
                history_replay: !accessible && current_has_win,
            });
        }

        for node in &mut nodes {
            let profile = campaign.missions[node.mission_idx].profile(profiles);
            node.prerequisite_nodes = profile
                .missions_required_to_be_done
                .iter()
                .filter_map(|mission_id| mission_to_node.get(mission_id).copied())
                .collect();
            node.prerequisite_nodes.sort_unstable();
            node.prerequisite_nodes.dedup();
        }

        let mut resolved = vec![false; nodes.len()];
        let mut cyclic = false;
        for _ in 0..nodes.len() {
            let mut advanced = false;
            for idx in 0..nodes.len() {
                if resolved[idx]
                    || !nodes[idx]
                        .prerequisite_nodes
                        .iter()
                        .all(|&parent| resolved[parent])
                {
                    continue;
                }
                nodes[idx].depth = nodes[idx]
                    .prerequisite_nodes
                    .iter()
                    .map(|&parent| nodes[parent].depth + 1)
                    .max()
                    .unwrap_or(0);
                resolved[idx] = true;
                advanced = true;
            }
            if !advanced {
                break;
            }
        }
        if resolved.iter().any(|done| !done) {
            cyclic = true;
            let fallback_depth = nodes.iter().map(|node| node.depth).max().unwrap_or(0) + 1;
            for (idx, done) in resolved.into_iter().enumerate() {
                if !done {
                    nodes[idx].depth = fallback_depth;
                }
            }
        }

        let mut next_lane_by_depth: HashMap<usize, usize> = HashMap::new();
        for node in &mut nodes {
            let lane = next_lane_by_depth.entry(node.depth).or_default();
            node.lane = *lane;
            *lane += 1;
        }
        let completed_missions = nodes
            .iter()
            .filter(|node| node.state == MissionProgressState::Completed)
            .count();
        let known_missions = nodes.len();
        Self {
            nodes,
            completed_missions,
            known_missions,
            campaign_achievements,
            lifetime_achievements,
            cyclic_prerequisites: cyclic,
        }
    }

    pub fn first_selectable(&self) -> Option<usize> {
        self.nodes.iter().position(|node| node.selectable)
    }

    pub fn next_selectable(&self, current: usize, forward: bool) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }
        for step in 1..=self.nodes.len() {
            let idx = if forward {
                (current + step) % self.nodes.len()
            } else {
                (current + self.nodes.len() - step % self.nodes.len()) % self.nodes.len()
            };
            if self.nodes[idx].selectable {
                return idx;
            }
        }
        current.min(self.nodes.len() - 1)
    }
}

/// Deterministic keyboard navigation used by the modal exhibit grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExhibitGridNavigator {
    pub selected: usize,
    columns: usize,
    len: usize,
}

impl ExhibitGridNavigator {
    pub fn new(len: usize, initial: usize) -> Self {
        let columns = 4;
        Self {
            selected: initial.min(len.saturating_sub(1)),
            columns,
            len,
        }
    }

    pub fn navigate(&mut self, dx: isize, dy: isize) {
        if self.len == 0 {
            return;
        }
        let col = self.selected % self.columns;
        let row = self.selected / self.columns;
        let max_row = (self.len - 1) / self.columns;
        let next_col = (col as isize + dx).clamp(0, self.columns as isize - 1) as usize;
        let next_row = (row as isize + dy).clamp(0, max_row as isize) as usize;
        self.selected = (next_row * self.columns + next_col).min(self.len - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::mission::Mission;
    use robin_engine::profiles::MissionProfile;

    #[test]
    fn graph_uses_required_mission_ids_for_depth() {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 1,
            mission_name: "Sherwood".into(),
            location: MissionLocation::Sherwood,
            ..Default::default()
        });
        profiles.missions.push(MissionProfile {
            id: 10,
            mission_name: "First".into(),
            ..Default::default()
        });
        profiles.missions.push(MissionProfile {
            id: 20,
            mission_name: "Second".into(),
            missions_required_to_be_done: vec![10],
            ..Default::default()
        });
        let mut campaign = Campaign::default();
        for idx in 0..3 {
            campaign.missions.push(Mission {
                profile_idx: Some(idx),
                ..Mission::new()
            });
        }
        campaign.accessible_mission_indices.push(1);
        let graph = CampaignProgressGraph::build(&campaign, &profiles, None);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].depth, 0);
        assert_eq!(graph.nodes[1].depth, 1);
        assert!(graph.nodes[0].selectable);
        assert!(!graph.nodes[1].selectable);
    }

    #[test]
    fn museum_navigation_clamps_to_real_exhibits() {
        let mut nav = ExhibitGridNavigator::new(6, 0);
        nav.navigate(3, 1);
        assert_eq!(nav.selected, 5);
        nav.navigate(-1, -1);
        assert_eq!(nav.selected, 0);
    }

    #[test]
    fn graph_keeps_current_run_and_lifetime_attempt_counts_distinct() {
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 1,
            mission_name: "Sherwood".into(),
            location: MissionLocation::Sherwood,
            ..Default::default()
        });
        profiles.missions.push(MissionProfile {
            id: 10,
            mission_name: "The Rescue".into(),
            ..Default::default()
        });
        let campaign_with_attempt = |run_id| {
            let mut campaign = Campaign::default();
            for idx in 0..2 {
                campaign.missions.push(Mission {
                    profile_idx: Some(idx),
                    ..Mission::new()
                });
            }
            campaign.current_mission_idx = Some(1);
            campaign.record_mission_attempt(
                1,
                MissionAttemptOutcome::Won,
                Some(100),
                Some(run_id),
                60,
                robin_engine::engine::SimConfig::default(),
                &robin_engine::mission_stat::MissionStat::default(),
                None,
            );
            campaign
        };

        let previous_campaign = campaign_with_attempt(1);
        let current_campaign = campaign_with_attempt(2);
        let mut lifetime = robin_engine::campaign_history::ProfileCampaignHistory::default();
        assert_eq!(
            lifetime
                .promote_campaign(&previous_campaign, &profiles)
                .unwrap(),
            1
        );
        assert_eq!(
            lifetime
                .promote_campaign(&current_campaign, &profiles)
                .unwrap(),
            1
        );

        let graph = CampaignProgressGraph::build(&current_campaign, &profiles, Some(&lifetime));
        assert_eq!(graph.nodes[0].attempt_count, 1);
        assert_eq!(graph.nodes[0].lifetime_attempt_count, 2);
        assert_eq!(graph.nodes[0].lifetime_win_count, 2);
    }
    #[test]
    fn lifetime_badge_survives_reset_without_unlocking_an_archived_replay() {
        use robin_engine::achievement::{
            AchievementEvaluation, AchievementId, AchievementRunContext, AchievementUnlockPolicy,
            MissionAchievementState,
        };

        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 1,
            mission_name: "Sherwood".into(),
            location: MissionLocation::Sherwood,
            ..Default::default()
        });
        profiles.missions.push(MissionProfile {
            id: 10,
            mission_name: "The Rescue".into(),
            ..Default::default()
        });

        let mut completed_campaign = Campaign::default();
        for idx in 0..2 {
            completed_campaign.missions.push(Mission {
                profile_idx: Some(idx),
                ..Mission::new()
            });
        }
        let mut tracker = MissionAchievementState::from_mission_start();
        tracker
            .record_evaluation(AchievementId::PileOBones, AchievementEvaluation::Earned)
            .unwrap();
        completed_campaign.current_mission_idx = Some(1);
        completed_campaign.record_mission_attempt(
            1,
            MissionAttemptOutcome::Won,
            Some(100),
            Some(7),
            60,
            robin_engine::engine::SimConfig::default(),
            &robin_engine::mission_stat::MissionStat::default(),
            Some(*tracker.finalize_success()),
        );
        completed_campaign
            .attest_mission_achievement_attempt(
                completed_campaign.latest_mission_attempt_key().unwrap(),
                AchievementUnlockPolicy::default(),
                AchievementRunContext::default(),
                &profiles,
            )
            .unwrap();

        let mut lifetime = robin_engine::campaign_history::ProfileCampaignHistory::default();
        lifetime
            .promote_campaign(&completed_campaign, &profiles)
            .unwrap();

        let mut reset_campaign = Campaign::default();
        for idx in 0..2 {
            reset_campaign.missions.push(Mission {
                profile_idx: Some(idx),
                ..Mission::new()
            });
        }
        let graph = CampaignProgressGraph::build(&reset_campaign, &profiles, Some(&lifetime));
        let node = &graph.nodes[0];

        assert!(node.badges.contains(AchievementId::PileOBones));
        assert_eq!(node.badge_count, 1);
        assert!(!node.selectable);
        assert!(!node.history_replay);
        assert!(
            graph
                .lifetime_achievements
                .get(AchievementId::PileOBones)
                .earned()
        );
        assert!(
            !graph
                .campaign_achievements
                .get(AchievementId::PileOBones)
                .earned()
        );
    }

    #[test]
    fn node_summary_can_hide_badges_without_hiding_attempt_history() {
        let node = CampaignProgressNode {
            mission_idx: 0,
            mission_id: 10,
            name: "The Rescue".into(),
            location: MissionLocation::Nottingham,
            state: MissionProgressState::Completed,
            prerequisite_nodes: Vec::new(),
            depth: 0,
            lane: 0,
            attempt_count: 2,
            win_count: 1,
            best: MissionBestStats::default(),
            badges: robin_engine::achievement::AchievementSet::empty(),
            badge_count: 3,
            lifetime_attempt_count: 2,
            lifetime_win_count: 1,
            selectable: true,
            history_replay: false,
        };

        assert!(node.summary(true).contains("3 badge"));
        let hidden = node.summary(false);
        assert!(hidden.contains("2 attempts"));
        assert!(!hidden.contains("badge"));
    }
}
