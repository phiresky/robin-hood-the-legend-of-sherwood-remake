//! Pure campaign graph and Sherwood museum presentation models.

use std::collections::HashMap;

use robin_engine::campaign::Campaign;
use robin_engine::campaign_history::{MissionAttempt, MissionAttemptOutcome};
use robin_engine::mission::MissionStatus;
use robin_engine::profiles::{MissionLocation, ProfileManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionProgressState {
    Locked,
    Available,
    Completed,
    Lost,
    Expired,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub badge_count: usize,
    pub lifetime_attempt_count: usize,
    pub lifetime_win_count: usize,
    pub selectable: bool,
    pub history_replay: bool,
}

impl CampaignProgressNode {
    pub fn summary(&self, include_history: bool) -> String {
        if !include_history {
            return self.name.clone();
        }
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
        if self.badge_count != 0 {
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CampaignProgressGraph {
    pub nodes: Vec<CampaignProgressNode>,
    pub completed_missions: usize,
    pub known_missions: usize,
    /// True when cyclic prerequisite input was detected. The graph remains
    /// inspectable; cyclic nodes are placed after the resolved frontier.
    pub cyclic_prerequisites: bool,
}

impl CampaignProgressGraph {
    pub fn build(
        campaign: &Campaign,
        profiles: &ProfileManager,
        allow_replays: bool,
        lifetime: Option<&robin_engine::campaign_history::ProfileCampaignHistory>,
    ) -> Self {
        let sherwood = campaign.get_sherwood_mission_idx();
        let mut mission_to_node = HashMap::new();
        let mut nodes = Vec::new();
        for (mission_idx, mission) in campaign.missions.iter().enumerate() {
            if mission_idx == sherwood {
                continue;
            }
            let profile = mission.profile(profiles);
            let attempts = mission.attempt_history().attempts();
            let has_win = attempts
                .iter()
                .any(|attempt| attempt.outcome() == MissionAttemptOutcome::Won)
                || mission.status == MissionStatus::Won;
            let accessible = campaign.accessible_mission_indices.contains(&mission_idx);
            let expired = mission.age >= profile.life_time;
            let state = if has_win {
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
                badge_count: mission.attempt_history().badges().len(),
                lifetime_attempt_count,
                lifetime_win_count,
                selectable: accessible || (allow_replays && has_win),
                history_replay: !accessible && allow_replays && has_win,
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

/// Deterministic grid navigation used by the walkable museum presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MuseumNavigator {
    pub selected: usize,
    columns: usize,
    len: usize,
}

impl MuseumNavigator {
    pub fn new(len: usize, initial: usize) -> Self {
        let columns = 4;
        Self {
            selected: initial.min(len.saturating_sub(1)),
            columns,
            len,
        }
    }

    pub fn walk(&mut self, dx: isize, dy: isize) {
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
        let graph = CampaignProgressGraph::build(&campaign, &profiles, true, None);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].depth, 0);
        assert_eq!(graph.nodes[1].depth, 1);
        assert!(graph.nodes[0].selectable);
        assert!(!graph.nodes[1].selectable);
    }

    #[test]
    fn museum_navigation_clamps_to_real_exhibits() {
        let mut nav = MuseumNavigator::new(6, 0);
        nav.walk(3, 1);
        assert_eq!(nav.selected, 5);
        nav.walk(-1, -1);
        assert_eq!(nav.selected, 0);
    }
}
