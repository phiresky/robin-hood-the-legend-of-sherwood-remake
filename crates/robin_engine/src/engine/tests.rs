use super::movement::{
    assign_circular_dispatch_candidates, circular_dispatch_candidate_points,
    circular_dispatch_destinations, mercenary_formation_destinations,
    uses_mercenary_group_formation,
};
use super::tick::{
    HourglassPhase, begin_hourglass_phase_capture, capture_ordered_gameplay_entities,
    end_hourglass_phase_capture,
};
use super::*;
use crate::campaign::{Campaign, CampaignValue};
use crate::coordinates::{MapBBox, MapPoint, MapSize, MapVec, SpriteFrameOffset};
use crate::game_operation::GameCode;

mod ai_detection;
mod commands;
mod lifecycle;
mod macros;
mod messages;
mod movement;
mod scenario_properties;
pub(in crate::engine) mod scenarios;
mod selected_melee_owner;
mod sequence;
mod serialization;
mod snapshot;
mod world_entity;

use scenarios::{make_test_ai_soldier, make_test_civilian, make_test_pc, make_test_soldier};
use sequence::{bind_test_action_point, bind_test_bow_release_action};
use world_entity::install_test_building_sector;
