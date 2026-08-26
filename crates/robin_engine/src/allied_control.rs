//! Deterministic state for the optional direct-control allied soldier system.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{coordinates::MapPoint, element::EntityId};

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AlliedStance {
    /// Do not acquire targets or leave the assigned position.
    Hold,
    /// Return fire, but resume the assigned patrol/follow duty afterwards.
    Defensive,
    /// Let the soldier AI pursue threats without a defensive leash.
    #[default]
    Aggressive,
}

impl AlliedStance {
    pub fn next(self) -> Self {
        match self {
            Self::Hold => Self::Defensive,
            Self::Defensive => Self::Aggressive,
            Self::Aggressive => Self::Hold,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AlliedFormation {
    #[default]
    #[serde(alias = "PatrolColumn", alias = "Battle", alias = "Column")]
    Line,
    #[serde(alias = "Compact", alias = "Ring")]
    Box,
    #[serde(alias = "Wedge")]
    Staggered,
    Flank,
}

impl AlliedFormation {
    pub fn next(self) -> Self {
        match self {
            Self::Line => Self::Box,
            Self::Box => Self::Staggered,
            Self::Staggered => Self::Flank,
            Self::Flank => Self::Line,
        }
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AlliedDuty {
    Hold { anchor: MapPoint },
    Patrol { points: [MapPoint; 2], next: u8 },
    Follow { hero: EntityId, offset: MapPoint },
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AlliedSoldierOrder {
    pub stance: AlliedStance,
    pub formation: AlliedFormation,
    pub duty: AlliedDuty,
    /// Last destination sent to the path system. Follow orders use this to
    /// avoid replacing an active route for insignificant hero movement.
    pub last_destination: MapPoint,
    /// One-shot reachable fallback for a formation slot. A failed slot moves
    /// toward the command's shared center instead of leaving the actor frozen
    /// in `MoveWaiting` for the generic 100-frame retry window.
    #[serde(default)]
    pub path_fallback: Option<MapPoint>,
    /// Final Line-formation slot used after a long move first gathers the
    /// group into its two-wide marching column.
    #[serde(default)]
    pub deploy_destination: Option<MapPoint>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AlliedPinnedGroup {
    pub id: u32,
    pub members: Vec<EntityId>,
}

#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AlliedSeatState {
    pub selection: Vec<EntityId>,
    pub pinned_groups: Vec<AlliedPinnedGroup>,
    pub first_visible_portrait: usize,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AlliedControlState {
    pub seats: Vec<AlliedSeatState>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    pub orders: BTreeMap<EntityId, AlliedSoldierOrder>,
    pub next_group_id: u32,
}

impl Default for AlliedControlState {
    fn default() -> Self {
        Self {
            seats: vec![AlliedSeatState::default()],
            orders: BTreeMap::new(),
            next_group_id: 1,
        }
    }
}

impl AlliedControlState {
    pub fn ensure_seat(&mut self, seat: usize) -> &mut AlliedSeatState {
        if self.seats.len() <= seat {
            self.seats.resize_with(seat + 1, AlliedSeatState::default);
        }
        &mut self.seats[seat]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_id::SoldierId;

    #[test]
    fn default_stance_preserves_normal_autonomous_ai() {
        assert_eq!(AlliedStance::default(), AlliedStance::Aggressive);
    }

    #[test]
    fn controlled_orders_serialize_to_json() {
        let soldier = EntityId::Soldier(SoldierId(17));
        let point = MapPoint::new(12.0, 34.0);
        let mut state = AlliedControlState::default();
        state.orders.insert(
            soldier,
            AlliedSoldierOrder {
                stance: AlliedStance::Defensive,
                formation: AlliedFormation::Line,
                duty: AlliedDuty::Hold { anchor: point },
                last_destination: point,
                path_fallback: None,
                deploy_destination: None,
            },
        );

        let json = serde_json::to_string(&state).expect("allied control state serializes");
        let restored: AlliedControlState =
            serde_json::from_str(&json).expect("allied control state deserializes");
        assert_eq!(restored.orders.get(&soldier), state.orders.get(&soldier));
    }

    #[test]
    fn older_controlled_orders_default_missing_path_fallback() {
        let soldier = EntityId::Soldier(SoldierId(17));
        let point = MapPoint::new(12.0, 34.0);
        let mut state = AlliedControlState::default();
        state.orders.insert(
            soldier,
            AlliedSoldierOrder {
                stance: AlliedStance::Defensive,
                formation: AlliedFormation::Line,
                duty: AlliedDuty::Hold { anchor: point },
                last_destination: point,
                path_fallback: Some(MapPoint::new(50.0, 60.0)),
                deploy_destination: None,
            },
        );

        let mut value = serde_json::to_value(state).expect("allied control state serializes");
        let orders = value
            .get_mut("orders")
            .and_then(serde_json::Value::as_object_mut)
            .expect("serialized allied orders are a JSON object");
        let order = orders
            .values_mut()
            .next()
            .and_then(serde_json::Value::as_object_mut)
            .expect("serialized allied order is a JSON object");
        order.remove("path_fallback");
        order.remove("deploy_destination");

        let restored: AlliedControlState =
            serde_json::from_value(value).expect("older allied order deserializes");
        assert_eq!(restored.orders[&soldier].path_fallback, None);
        assert_eq!(restored.orders[&soldier].deploy_destination, None);
    }

    #[test]
    fn previous_formation_names_remain_loadable() {
        assert_eq!(
            serde_json::from_str::<AlliedFormation>("\"Compact\"").unwrap(),
            AlliedFormation::Box
        );
        assert_eq!(
            serde_json::from_str::<AlliedFormation>("\"PatrolColumn\"").unwrap(),
            AlliedFormation::Line
        );
        assert_eq!(
            serde_json::from_str::<AlliedFormation>("\"Battle\"").unwrap(),
            AlliedFormation::Line
        );
    }
}
