//! Campaign progress embedded in the trace header.
use super::scalar::TraceFloat;
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceCampaign {
    pub(crate) version: u32,
    pub(crate) values: Vec<i32>,
    pub(crate) ares: i8,
    pub(crate) missions: Vec<TraceCampaignMission>,
    pub(crate) accessible_mission_indices: Vec<usize>,
    pub(crate) pending_accessible_mission_indices: Vec<usize>,
    pub(crate) last_mission_index: Option<usize>,
    pub(crate) current_mission_index: Option<usize>,
    pub(crate) next_mission_index: Option<usize>,
    pub(crate) blazon_mission_index: Option<usize>,
    pub(crate) last_played_mission_indices: Vec<usize>,
    pub(crate) last_pseudo_mission_status: u32,
    pub(crate) last_pseudo_mission_id: u32,
    pub(crate) characters: Vec<TraceCampaignCharacter>,
    pub(crate) gang_indices: Vec<usize>,
    pub(crate) reservist_indices: Vec<usize>,
    pub(crate) mission_team_indices: Vec<usize>,
    pub(crate) peasant_names: Vec<String>,
    pub(crate) reservists_are_back: bool,
    pub(crate) collected_relics: Vec<u32>,
    pub(crate) production_sectors: Vec<TraceProductionSector>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceCampaignMission {
    pub(crate) profile_index: u32,
    pub(crate) profile_id: u32,
    pub(crate) mission: String,
    pub(crate) proto_level: String,
    pub(crate) age: u16,
    pub(crate) blazon_price: u16,
    pub(crate) status: u32,
    pub(crate) ares_state_succeeded: i8,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceCampaignCharacter {
    pub(crate) profile_index: u32,
    pub(crate) profile_name: String,
    pub(crate) instanced: bool,
    pub(crate) status: TracePcStatus,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TracePcStatus {
    pub(crate) hand_to_hand: TraceSkill,
    pub(crate) bow: TraceSkill,
    pub(crate) life_points: i16,
    pub(crate) in_coma: bool,
    pub(crate) ales: u16,
    pub(crate) arrows: u16,
    pub(crate) apples: u16,
    pub(crate) rations: u16,
    pub(crate) stones: u16,
    pub(crate) wasp_nests: u16,
    pub(crate) nets: u16,
    pub(crate) plants: u16,
    pub(crate) purses: u16,
    pub(crate) name: String,
    pub(crate) beam_me_index_in_sherwood: i16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSkill {
    pub(crate) capacity: u32,
    pub(crate) experience: u32,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceProductionSector {
    pub(crate) r#type: u32,
    pub(crate) speed: u16,
    pub(crate) amount: u16,
    pub(crate) produced_amount: u16,
    pub(crate) max_amount_reached: bool,
    pub(crate) occupants: Vec<TraceProductionOccupant>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceProductionOccupant {
    pub(crate) character_index: usize,
    pub(crate) x: TraceFloat,
    pub(crate) y: TraceFloat,
    pub(crate) obstacle: u16,
}
