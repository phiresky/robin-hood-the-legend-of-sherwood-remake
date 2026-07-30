//! Persistent actor state which Original keeps outside its current order.

use serde::{Deserialize, Serialize};

use crate::{
    coordinates::MapPoint, element::EntityId, position_interface::SectorHandle, sprite::MotionState,
};

/// Actor-owned continuation state serialized by `RHElementActor::Serialize`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ActorContinuationState {
    pub about_to_surrender: bool,
    pub surrendering: bool,
    pub menacer: Option<EntityId>,
    pub distance_to_boundary: f32,
    pub position_at_last_distance_request: MapPoint,
    pub motion_state: MotionState,
    pub seek_layer: u16,
    pub seek_to_point: bool,
    pub seek_sector: Option<SectorHandle>,
    pub check_for_jump: bool,
    pub bypassing: bool,
    pub on_railroad: bool,
    pub bypass_exit: MapPoint,
    pub bypass_reference: Option<EntityId>,
    pub bypass_points: Vec<MapPoint>,
    pub material_sector: Option<SectorHandle>,
}
