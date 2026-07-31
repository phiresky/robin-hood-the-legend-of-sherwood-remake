#![allow(unused_mut)]

use super::movement::mercenary_formation_destinations;
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
mod selected_melee_owner;
mod sequence;
mod serialization;
mod snapshot;
mod world_entity;

use sequence::{bind_test_action_point, bind_test_bow_release_action};
use world_entity::install_test_building_sector;
use world_entity::{make_test_ai_soldier, make_test_civilian, make_test_pc, make_test_soldier};
