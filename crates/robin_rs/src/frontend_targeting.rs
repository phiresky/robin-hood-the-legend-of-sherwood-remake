//! One-shot world targeting for tactical portrait actions.
//!
//! Arming captures the exact selected members/formation. Resolving consumes
//! that capture and constructs its command atomically; a later selection
//! change cannot silently retarget the pending action.
use robin_engine::coordinates::MapPoint;
use robin_engine::element::EntityId;
use robin_engine::player_command::PlayerCommand;
use robin_engine::tactical_control::TacticalFormation;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct TacticalTargeting {
    pending: Option<PendingPatrol>,
}

#[derive(Serialize, Deserialize)]
struct PendingPatrol {
    soldiers: Vec<EntityId>,
    formation: TacticalFormation,
}

impl TacticalTargeting {
    pub fn arm_patrol(&mut self, soldiers: Vec<EntityId>, formation: TacticalFormation) {
        self.pending = Some(PendingPatrol {
            soldiers,
            formation,
        });
    }

    pub fn resolve_world_click(&mut self, destination: MapPoint) -> Option<PlayerCommand> {
        self.pending.take().map(
            |PendingPatrol {
                 soldiers,
                 formation,
             }| {
                PlayerCommand::SetTacticalPatrol {
                    soldiers,
                    destination,
                    formation,
                }
            },
        )
    }

    /// Reports whether the cancel consumed the click (without dispatching).
    pub fn cancel(&mut self) -> bool {
        self.pending.take().is_some()
    }

    pub fn is_armed(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_is_one_shot_and_rearming_replaces_the_capture() {
        let mut targeting = TacticalTargeting::default();
        assert!(targeting.resolve_world_click(MapPoint::ZERO).is_none());
        let first = EntityId::Soldier(robin_engine::entity_id::SoldierId(3));
        let second = EntityId::Soldier(robin_engine::entity_id::SoldierId(7));
        targeting.arm_patrol(vec![first], TacticalFormation::Line);
        targeting.arm_patrol(vec![second, first], TacticalFormation::Box);
        let destination = MapPoint::new(12.0, 34.0);
        assert!(matches!(targeting.resolve_world_click(destination),
            Some(PlayerCommand::SetTacticalPatrol { destination: p, formation, soldiers })
                if p == destination && formation == TacticalFormation::Box && soldiers == vec![second, first]));
        assert!(!targeting.is_armed());
        assert!(targeting.resolve_world_click(destination).is_none());
        targeting.arm_patrol(vec![], TacticalFormation::Line);
        assert!(targeting.cancel());
        assert!(!targeting.cancel());
        assert!(targeting.resolve_world_click(destination).is_none());
    }
}
