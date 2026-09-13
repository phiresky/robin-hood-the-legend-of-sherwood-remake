//! Simulation compatibility facade for level data shared with asset loading.

pub use robin_level_data::level_data::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human_control::{CombatStance, CommandInterface, DecisionPolicy, MissionRole};

    #[test]
    fn hackable_descriptor_authors_multiple_soldier_and_pc_allegiances() {
        let level = LoadedLevel::hackable_from_json(
            br#"{
                "map_filename": "Arena",
                "spawn": [50, 50],
                "spawn_player": false,
                "diplomacy": {
                    "player_coalition": [0, 7],
                    "relationships": [
                        {"first": 2, "second": 9, "relationship": "neutral"}
                    ]
                },
                "walkable_polygon": [[0, 0], [100, 0], [100, 100]],
                "soldiers": [
                    {"position": [20, 20], "profile": 0, "allegiance": 2, "command_interface": "tactical_orders", "mission_role": "tactical_ally", "combat_stance": "defensive"},
                    {"position": [80, 20], "profile": 1, "allegiance": 9}
                ],
                "pcs": [
                    {"position": [50, 80], "profile": 2, "allegiance": 7, "autonomous": true, "aggressive_combat": true, "ai_profile": "soldier_b04"}
                ]
            }"#,
        )
        .expect("multi-team hackable descriptor");
        assert!(level.mission.reserve_null_ai_handle);
        assert!(level.mission.beam_mes.is_empty());
        assert_eq!(level.mission.soldiers.len(), 2);
        assert_eq!(level.mission.soldiers[0].allegiance, Some(2));
        assert_eq!(
            level.mission.soldiers[0].command_interface,
            CommandInterface::TacticalOrders
        );
        assert_eq!(
            level.mission.soldiers[0].mission_role,
            MissionRole::TacticalAlly
        );
        assert_eq!(level.mission.soldiers[1].allegiance, Some(9));
        assert!(level.mission.soldiers.iter().all(|soldier| {
            soldier.path_id == NO_HIKING_PATH_ID && soldier.alert_path_id == NO_HIKING_PATH_ID
        }));
        assert!(crate::ai::PathId::new(level.mission.soldiers[0].path_id).is_none());
        assert_eq!(level.mission.pcs_to_rescue[0].allegiance, Some(7));
        assert_eq!(
            level.mission.pcs_to_rescue[0].decision_policy,
            DecisionPolicy::EnemyAi
        );
        assert_eq!(
            level.mission.pcs_to_rescue[0].combat_stance,
            CombatStance::Aggressive
        );
        let diplomacy = level.diplomacy.expect("authored diplomacy");
        assert_eq!(diplomacy.player_coalition, vec![0, 7]);
        assert_eq!(diplomacy.relationships.len(), 1);
        assert_eq!(
            diplomacy.relationships[0].relationship,
            crate::diplomacy::Relationship::Neutral
        );
        assert_eq!(
            level.mission.pcs_to_rescue[0].ai_profile.as_deref(),
            Some("soldier_b04")
        );
    }
}
