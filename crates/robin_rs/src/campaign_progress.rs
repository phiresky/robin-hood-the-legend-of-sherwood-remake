//! Pure campaign graph and modal Hall-of-Deeds presentation models.

use std::collections::HashMap;

use robin_engine::achievement::{AchievementAggregationSummary, AchievementSet};
use robin_engine::campaign::Campaign;
use robin_engine::campaign_history::{MissionAttempt, MissionAttemptOutcome};
use robin_engine::mission::MissionStatus;
use robin_engine::profiles::{MissionLocation, MissionProfile, MissionType, ProfileManager};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionKind {
    Story,
    Optional,
    Ambush,
    Training,
    CampaignEvent,
    Epilogue,
    Unavailable,
}

impl MissionKind {
    fn for_profile(profile: &MissionProfile) -> Self {
        if profile
            .mission_filename
            .eq_ignore_ascii_case("SherwoodOutro")
        {
            Self::Epilogue
        } else if profile
            .mission_filename
            .eq_ignore_ascii_case("EmbTut_FoC_EC")
        {
            Self::Training
        } else if profile.mission_type == MissionType::Pseudo {
            Self::CampaignEvent
        } else if profile
            .mission_filename
            .eq_ignore_ascii_case("Impossible_mission")
        {
            Self::Unavailable
        } else {
            match profile.mission_type {
                MissionType::Historical | MissionType::Attack => Self::Story,
                MissionType::Ambush => Self::Ambush,
                MissionType::Rescue | MissionType::Tactical => Self::Optional,
                MissionType::Hq | MissionType::End | MissionType::Pseudo => Self::CampaignEvent,
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Story => "Story",
            Self::Optional => "Optional",
            Self::Ambush => "Ambush",
            Self::Training => "Training",
            Self::CampaignEvent => "Campaign event",
            Self::Epilogue => "Epilogue",
            Self::Unavailable => "Archived / unavailable",
        }
    }

    pub fn is_field_mission(self) -> bool {
        matches!(
            self,
            Self::Story | Self::Optional | Self::Ambush | Self::Training
        )
    }
}

/// Explain unmet conditions with the same values used by mission accessibility.
/// TODO: Share a typed condition list with Mission::is_accessible_why instead
/// of maintaining its player-facing formatting alongside the engine checks.
fn availability_notes(
    campaign: &Campaign,
    profiles: &ProfileManager,
    mission_idx: usize,
) -> Vec<String> {
    use robin_engine::campaign::CampaignValue;
    let mission = &campaign.missions[mission_idx];
    let profile = mission.profile(profiles);
    let money = campaign.get_value(CampaignValue::Ransom);
    let gang = campaign.get_size_of_gang();
    let mut notes = Vec::new();
    match MissionKind::for_profile(profile) {
        MissionKind::Epilogue => notes.push("Ending scene after The Sheriff of Nottingham.".into()),
        MissionKind::CampaignEvent => notes.push(
            "Campaign event, resolved through campaign progression rather than a field mission."
                .into(),
        ),
        MissionKind::Unavailable => {
            notes.push("This archived mission has no playable map in this content.".into())
        }
        _ => {}
    }
    if money < profile.min_ransom as i32 {
        notes.push(format!(
            "Need at least {} ransom money (currently {money}).",
            profile.min_ransom
        ));
    }
    if profile.max_ransom != 200000 && profile.max_ransom < money as u32 {
        notes.push(format!(
            "Requires no more than {} ransom money (currently {money}).",
            profile.max_ransom
        ));
    }
    if gang < usize::from(profile.min_gang_size) {
        notes.push(format!(
            "Need at least {} gang members (currently {gang}).",
            profile.min_gang_size
        ));
    }
    if gang > usize::from(profile.max_gang_size) {
        notes.push(format!(
            "Requires no more than {} gang members (currently {gang}).",
            profile.max_gang_size
        ));
    }
    if mission.age >= profile.life_time {
        notes.push("This mission's availability window has expired.".into());
    }
    let ares = campaign.get_ares();
    if profile.ares_sensible && ares != -1 {
        let stage = usize::try_from(ares).expect("negative campaign story state");
        if !profile.available_in_ares_state[stage] {
            notes.push("Not available at the current point in the story.".into());
        }
    }
    for (&id, must_be_done) in profile
        .missions_required_to_be_done
        .iter()
        .map(|id| (id, true))
        .chain(
            profile
                .missions_required_not_to_be_done
                .iter()
                .map(|id| (id, false)),
        )
    {
        let required = campaign
            .get_mission(id, profiles)
            .expect("campaign mission prerequisite is missing");
        if required.is_done() != must_be_done {
            let name = &required.profile(profiles).mission_name;
            notes.push(if must_be_done {
                format!("Finish {name} first.")
            } else if id == profile.id {
                "This mission has already been resolved in this campaign.".into()
            } else {
                format!("Only offered before {name} is resolved.")
            });
        }
    }
    if notes.is_empty() && !campaign.accessible_mission_indices.contains(&mission_idx) {
        notes.push(if campaign.pending_accessible_mission_indices.contains(&mission_idx) {
            "This mission is pending the next mission selection.".into()
        } else {
            "Entry conditions met; this mission is not currently offered. Sherwood selects from eligible missions.".into()
        });
    }
    notes
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
    pub kind: MissionKind,
    /// Primary route stays visible while browsing the side branches.
    pub on_spine: bool,
    pub availability_notes: Vec<String>,
    /// Display connections, including the finale-to-epilogue story transition.
    /// Mission launch eligibility still comes from the campaign itself.
    pub prerequisite_nodes: Vec<usize>,
    pub depth: usize,
    pub lane: usize,
    pub attempt_count: usize,
    pub win_count: usize,
    /// Best recorded results across the current save and archived attempts.
    pub best: MissionBestStats,
    pub campaign_badges: AchievementSet,
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
            let kind = MissionKind::for_profile(profile);
            // These descriptor slots have no playable map. Preserve any old
            // recorded results, but don't fill the tree with unused content.
            if kind == MissionKind::Unavailable && attempts.is_empty() && !mission.is_done() {
                continue;
            }
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
            if let Some(history) = lifetime {
                for entry in history
                    .attempts()
                    .iter()
                    .filter(|entry| entry.mission_id() == profile.id)
                {
                    best.include(entry.attempt());
                }
            }
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
                name: if profile
                    .mission_filename
                    .eq_ignore_ascii_case("SherwoodOutro")
                {
                    // TODO: Localize the epilogue label with the other campaign manager text.
                    "Epilogue".to_owned()
                } else {
                    profile.mission_name.clone()
                },
                location: profile.location,
                state,
                kind,
                on_spine: false,
                availability_notes: availability_notes(campaign, profiles, mission_idx),
                prerequisite_nodes: Vec::new(),
                depth: 0,
                lane: 0,
                attempt_count: attempts.len(),
                win_count: attempts
                    .iter()
                    .filter(|attempt| attempt.outcome() == MissionAttemptOutcome::Won)
                    .count(),
                best,
                campaign_badges: mission.achievement_badges(),
                // Raw calculated results remain on every immutable attempt for
                // debrief/audit. Only the host-policy-approved achievement
                // history is allowed to drive awarded badge presentation.
                badges,
                badge_count: badges.len(),
                lifetime_attempt_count,
                lifetime_win_count,
                selectable: kind != MissionKind::Unavailable && (accessible || current_has_win),
                history_replay: kind != MissionKind::Unavailable && !accessible && current_has_win,
            });
        }

        let is_epilogue = |node: &CampaignProgressNode| {
            campaign.missions[node.mission_idx]
                .profile(profiles)
                .mission_filename
                .eq_ignore_ascii_case("SherwoodOutro")
        };
        // The outro is a scripted story transition rather than a normal
        // prerequisite-gated mission. Keep it last in the gallery as well.
        nodes.sort_by_key(is_epilogue);
        for (index, node) in nodes.iter().enumerate() {
            mission_to_node.insert(node.mission_id, index);
        }
        let finale = nodes.iter().position(|node| {
            campaign.missions[node.mission_idx]
                .profile(profiles)
                .mission_filename
                .eq_ignore_ascii_case("H12_Not_MP")
        });
        for node in &mut nodes {
            let profile = campaign.missions[node.mission_idx].profile(profiles);
            node.prerequisite_nodes = profile
                .missions_required_to_be_done
                .iter()
                .filter_map(|mission_id| mission_to_node.get(mission_id).copied())
                .collect();
            if is_epilogue(node) {
                if let Some(finale) = finale {
                    node.prerequisite_nodes.push(finale);
                } else {
                    // TODO: Resolve story transitions from content metadata when available.
                    tracing::warn!(
                        "Epilogue has no H12_Not_MP finale in this content; placing it after the mission tree without a story connection"
                    );
                }
            }
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

        if let Some(last_mission_depth) = nodes
            .iter()
            .filter(|node| !is_epilogue(node))
            .map(|node| node.depth)
            .max()
        {
            for node in nodes.iter_mut().filter(|node| is_epilogue(node)) {
                node.depth = node.depth.max(last_mission_depth + 1);
            }
        }
        // Trace the longest prerequisite route backwards from the story finale.
        // Other required missions remain genuine branches, not invented linear
        // prerequisites. Partial/demo content uses its deepest story endpoint.
        let mut cursor = nodes
            .iter()
            .position(|node| node.kind == MissionKind::Epilogue)
            .or(finale)
            .or_else(|| {
                nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.kind == MissionKind::Story)
                    .max_by_key(|(_, node)| node.depth)
                    .map(|(index, _)| index)
            });
        while let Some(index) = cursor {
            if nodes[index].on_spine {
                break;
            }
            nodes[index].on_spine = true;
            cursor = nodes[index]
                .prerequisite_nodes
                .iter()
                .copied()
                .filter(|&parent| nodes[parent].depth < nodes[index].depth)
                .max_by_key(|&parent| {
                    (
                        nodes[parent].depth,
                        nodes[parent].kind == MissionKind::Story,
                    )
                });
        }
        let mut branch_order: Vec<_> = (0..nodes.len()).collect();
        branch_order.sort_by_key(|&index| {
            let node = &nodes[index];
            (
                node.depth,
                !node.on_spine,
                match node.kind {
                    MissionKind::Story => 0,
                    MissionKind::Training => 1,
                    MissionKind::CampaignEvent => 2,
                    MissionKind::Ambush => 3,
                    MissionKind::Optional => 4,
                    MissionKind::Epilogue => 5,
                    MissionKind::Unavailable => 6,
                },
            )
        });
        let mut next_lane_by_depth: HashMap<usize, usize> = HashMap::new();
        for &index in &branch_order {
            let node = &mut nodes[index];
            if node.on_spine {
                node.lane = 0;
            } else {
                let lane = next_lane_by_depth.entry(node.depth).or_insert(1);
                node.lane = *lane;
                *lane += 1;
            }
        }
        // Gallery order follows story stages too. Remap every display edge;
        // mission_idx continues to identify the original campaign mission.
        let mut remap = vec![0; nodes.len()];
        for (new, &old) in branch_order.iter().enumerate() {
            remap[old] = new;
        }
        nodes = branch_order.iter().map(|&old| nodes[old].clone()).collect();
        for node in &mut nodes {
            for parent in &mut node.prerequisite_nodes {
                *parent = remap[*parent];
            }
        }
        let completed_missions = nodes
            .iter()
            .filter(|node| {
                node.kind.is_field_mission() && node.state == MissionProgressState::Completed
            })
            .count();
        let known_missions = nodes
            .iter()
            .filter(|node| node.kind.is_field_mission())
            .count();
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
    fn story_route_and_branches_keep_real_edges_and_hide_unused_slots() {
        let mut profiles = ProfileManager::new();
        for (id, filename, kind, parents) in [
            (1, "Sherwood", MissionType::Hq, vec![]),
            (10, "Opening", MissionType::Historical, vec![]),
            (20, "Next", MissionType::Historical, vec![10]),
            (30, "Side", MissionType::Tactical, vec![10]),
            (40, "EmbTut_FoC_EC", MissionType::Ambush, vec![10]),
            (50, "H12_Not_MP", MissionType::Historical, vec![20]),
            (60, "Impossible_mission", MissionType::Ambush, vec![10]),
            (70, "SherwoodOutro", MissionType::Ambush, vec![]),
        ] {
            profiles.missions.push(MissionProfile {
                id,
                mission_filename: filename.into(),
                mission_name: filename.into(),
                mission_type: kind,
                missions_required_to_be_done: parents,
                max_ransom: 200000,
                max_gang_size: u16::MAX,
                life_time: u16::MAX,
                location: if id == 1 {
                    MissionLocation::Sherwood
                } else {
                    MissionLocation::Nottingham
                },
                ..Default::default()
            });
        }
        let mut campaign = Campaign::default();
        for idx in 0..profiles.missions.len() {
            campaign.missions.push(Mission {
                profile_idx: Some(idx as u32),
                ..Mission::new()
            });
        }
        let graph = CampaignProgressGraph::build(&campaign, &profiles, None);
        assert_eq!(graph.nodes.len(), 6);
        assert_eq!(graph.known_missions, 5);
        for node in &graph.nodes {
            assert_eq!(profiles.missions[node.mission_idx].id, node.mission_id);
            assert_eq!(node.on_spine, [10, 20, 50, 70].contains(&node.mission_id));
            if node.on_spine {
                assert_eq!(node.lane, 0);
            } else {
                assert!(node.lane > 0);
            }
            for &parent in &node.prerequisite_nodes {
                assert!(graph.nodes[parent].depth < node.depth);
            }
        }
        let training = graph
            .nodes
            .iter()
            .find(|node| node.mission_id == 40)
            .unwrap();
        assert_eq!(training.kind, MissionKind::Training);
        let side = graph
            .nodes
            .iter()
            .find(|node| node.mission_id == 30)
            .unwrap();
        assert_eq!(side.depth, training.depth);
        assert_ne!(side.lane, training.lane);
        assert_eq!(graph.nodes.last().unwrap().kind, MissionKind::Epilogue);
        assert_eq!(campaign.missions.len(), 8);
        campaign.missions[6].status = MissionStatus::Won;
        let with_archive = CampaignProgressGraph::build(&campaign, &profiles, None);
        let archived = with_archive
            .nodes
            .iter()
            .find(|node| node.mission_id == 60)
            .unwrap();
        assert_eq!(archived.kind, MissionKind::Unavailable);
        assert!(!archived.selectable);
        assert!(!archived.history_replay);
    }

    #[test]
    fn availability_explains_resource_story_and_mission_conditions() {
        use robin_engine::campaign::CampaignValue;
        let mut profiles = ProfileManager::new();
        profiles.missions.push(MissionProfile {
            id: 1,
            mission_name: "First mission".into(),
            mission_type: MissionType::Historical,
            max_ransom: 200000,
            max_gang_size: u16::MAX,
            life_time: 10,
            ..Default::default()
        });
        profiles.missions.push(MissionProfile {
            id: 2,
            mission_name: "Next mission".into(),
            mission_type: MissionType::Historical,
            min_ransom: 100,
            max_ransom: 200000,
            min_gang_size: 2,
            max_gang_size: u16::MAX,
            life_time: 10,
            ares_sensible: true,
            available_in_ares_state: [false; 10],
            missions_required_to_be_done: vec![1],
            ..Default::default()
        });
        let mut campaign = Campaign::default();
        campaign.missions = (0..2)
            .map(|idx| Mission {
                profile_idx: Some(idx),
                ..Mission::new()
            })
            .collect();
        campaign.set_value(CampaignValue::Ransom, 50);
        campaign.set_ares(1);
        let notes = availability_notes(&campaign, &profiles, 1);
        assert_eq!(notes.len(), 4);
        assert!(notes[0].contains("100 ransom money (currently 50)"));
        assert!(notes[1].contains("2 gang members (currently 0)"));
        assert!(notes[2].contains("point in the story"));
        assert_eq!(notes[3], "Finish First mission first.");
        assert!(
            campaign.missions[1]
                .is_accessible_why(&campaign, &profiles)
                .is_err()
        );
        campaign.set_value(CampaignValue::Ransom, 100);
        profiles.missions[1].min_gang_size = 0;
        profiles.missions[1].available_in_ares_state[1] = true;
        campaign.missions[0].status = MissionStatus::Lost; // Engine prerequisites require resolved, not necessarily won.
        assert!(
            campaign.missions[1]
                .is_accessible_why(&campaign, &profiles)
                .is_ok()
        );
        let notes = availability_notes(&campaign, &profiles, 1);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("not currently offered"));
        campaign.accessible_mission_indices.push(1);
        assert!(availability_notes(&campaign, &profiles, 1).is_empty());
        profiles.missions[1].missions_required_not_to_be_done = vec![1];
        assert_eq!(
            availability_notes(&campaign, &profiles, 1),
            ["Only offered before First mission is resolved."]
        );
    }

    #[test]
    fn epilogue_follows_finale_without_changing_mission_access() {
        let mut profiles = ProfileManager::new();
        for (id, filename, required) in [
            (1, "Sherwood", vec![]),
            (10, "Godfather", vec![]),
            (20, "SherwoodOutro", vec![]),
            (30, "H12_Not_MP", vec![10]),
        ] {
            profiles.missions.push(MissionProfile {
                id,
                mission_filename: filename.into(),
                mission_name: filename.into(),
                location: if id == 1 {
                    MissionLocation::Sherwood
                } else {
                    MissionLocation::Nottingham
                },
                missions_required_to_be_done: required,
                ..Default::default()
            });
        }
        let mut campaign = Campaign::default();
        for idx in 0..4 {
            campaign.missions.push(Mission {
                profile_idx: Some(idx),
                ..Mission::new()
            });
        }
        campaign.accessible_mission_indices.push(1);
        let graph = CampaignProgressGraph::build(&campaign, &profiles, None);
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.mission_id)
                .collect::<Vec<_>>(),
            [10, 30, 20]
        );
        let epilogue = &graph.nodes[2];
        assert_eq!(epilogue.name, "Epilogue");
        assert_eq!(epilogue.mission_idx, 2);
        assert_eq!(epilogue.prerequisite_nodes, [1]);
        assert_eq!(graph.nodes[1].prerequisite_nodes, [0]);
        assert!(epilogue.depth > graph.nodes[1].depth);
        assert!(!epilogue.selectable);
        assert!(profiles.missions[2].missions_required_to_be_done.is_empty());
        assert_eq!(campaign.accessible_mission_indices, [1]);

        campaign.missions.pop();
        let partial = CampaignProgressGraph::build(&campaign, &profiles, None);
        assert_eq!(partial.nodes[1].name, "Epilogue");
        assert!(partial.nodes[1].depth > partial.nodes[0].depth);
        assert!(partial.nodes[1].prerequisite_nodes.is_empty());
    }

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
        assert!(node.campaign_badges.is_empty());
        assert_eq!(node.badge_count, 1);
        assert_eq!(node.best.fastest_win_seconds, Some(60));
        assert_eq!(node.lifetime_attempt_count, 1);
        assert_eq!(node.attempt_count, 0);
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
            kind: MissionKind::Story,
            on_spine: true,
            availability_notes: Vec::new(),
            prerequisite_nodes: Vec::new(),
            depth: 0,
            lane: 0,
            attempt_count: 2,
            win_count: 1,
            best: MissionBestStats::default(),
            campaign_badges: AchievementSet::empty(),
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
