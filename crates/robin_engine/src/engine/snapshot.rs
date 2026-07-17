//! Stable serialization and deterministic-hash boundary for [`EngineInner`].
//!
//! The in-memory engine is going to be split into cohesive owned state groups.
//! Save files, multiplayer bincode snapshots, and replay hashes must not change
//! merely because those fields move. This flat remote-serde schema deliberately
//! repeats the current field names, types, attributes, and declaration order.
//! Future in-memory regrouping maps through this boundary instead of deriving a
//! new external schema from the runtime layout.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::EngineInner;

/// The compatibility snapshot shape present before the logical Engine split.
///
/// `serde(remote)` generates conversion directly to/from `EngineInner`, so
/// there is no second authoritative state value. Once `EngineInner` fields move
/// into owner structs, this definition stays flat and gains explicit getters /
/// conversion code while retaining this exact order.
#[derive(Serialize, Deserialize)]
#[serde(remote = "EngineInner")]
struct FlatEngineSnapshot {
    sim_config: std::cell::Cell<Option<super::SimConfig>>,
    mission: super::MissionState,
    frame_counter: u32,
    sound_sim: crate::sound::SoundSimState,
    simulation_gates: super::SimulationGateState,
    speed: f32,
    speed_int: u16,
    weather: super::WeatherState,
    shield: super::ShieldState,
    script_globals: Vec<i32>,
    cheat_used_flags: u32,
    standard_view_polygon_radius: u16,
    next_order_id: u32,
    chorus_timer: u16,
    force_check: bool,
    messenger: crate::messenger::Messenger,
    fast_grid: crate::fast_find_grid::FastFindGrid,
    pathfinder: crate::pathfinder::PathFinder,
    short_briefings: crate::short_briefings::ShortBriefings,
    mission_stat: crate::mission_stat::MissionStat,
    ground_mark: crate::markers::GroundMark,
    entities: crate::entities::Entities,
    pc_ids: Vec<crate::element::EntityId>,
    titbit_manager: crate::titbit::TitbitManager,
    seats: Vec<super::SeatState>,
    cutscene_camera: super::CameraState,
    rng: super::SimulationRng,
    pending_side_effects: super::SideEffects,
    user_locked: bool,
    qa_recording_for: Vec<crate::element::EntityId>,
    qa_recording_slot: u8,
    action_before_recording_macro: crate::profiles::Action,
    fast_forward: bool,
    pending_move_requests: Vec<(crate::element::EntityId, crate::order::AiOrderIntent)>,
    #[serde(default)]
    pending_path_requests: super::movement::PendingPathRequestQueue,
    failed_path_requests: Vec<super::movement::FailedPathRequest>,
    ai_global: crate::ai::AiGlobalState,
    macro_store: crate::macro_store::MacroStore,
    dead_pc: Option<crate::element::EntityId>,
    timer_elements: Vec<super::TimerEntry>,
    sequence_manager: crate::sequence::SequenceManager,
    pending_reinforcements: Vec<Option<crate::element::EntityId>>,
    pending_scroll_amulets: Vec<super::PendingScrollAmulet>,
    pending_hero_speeches: Vec<(crate::element::EntityId, u16)>,
    pending_hades_kills: Vec<crate::element::EntityId>,
    pending_concussion_side_effects:
        Vec<(crate::element::EntityId, crate::combat::ConcussionOutcome)>,
    mission_script: Option<super::MissionScript>,
    script_zone_data: Vec<crate::sector::ScriptSectorData>,
    dynamic_sight_obstacles: Vec<crate::sight_obstacle::SightObstacle>,
    static_sight_obstacle_active: Vec<bool>,
    campaign: Option<crate::campaign::Campaign>,
}

impl Serialize for EngineInner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        FlatEngineSnapshot::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for EngineInner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        FlatEngineSnapshot::deserialize(deserializer)
    }
}

/// Hash fields in the pre-split declaration order.
///
/// The derive macro does not hash field names or struct boundaries; it invokes
/// `StateHash` for each field in declaration order and writes a fixed marker for
/// `#[state_hash(skip)]`. Keeping that exact sequence here preserves recorded
/// replay hashes while allowing the runtime fields to be grouped later.
impl robin_util::state_hash::StateHash for EngineInner {
    fn state_hash<H: std::hash::Hasher>(&self, state: &mut H) {
        robin_util::state_hash::hash_skipped_field(state); // sim_config
        self.mission.state_hash(state);
        self.frame_counter.state_hash(state);
        self.sound_sim.state_hash(state);
        self.simulation_gates.state_hash(state);
        self.speed.state_hash(state);
        self.speed_int.state_hash(state);
        self.weather.state_hash(state);
        self.shield.state_hash(state);
        self.script_globals.state_hash(state);
        self.cheat_used_flags.state_hash(state);
        self.standard_view_polygon_radius.state_hash(state);
        self.next_order_id.state_hash(state);
        self.chorus_timer.state_hash(state);
        self.force_check.state_hash(state);
        self.messenger.state_hash(state);
        self.fast_grid.state_hash(state);
        self.pathfinder.state_hash(state);
        self.short_briefings.state_hash(state);
        self.mission_stat.state_hash(state);
        self.ground_mark.state_hash(state);
        self.entities.state_hash(state);
        self.pc_ids.state_hash(state);
        self.titbit_manager.state_hash(state);
        self.seats.state_hash(state);
        self.cutscene_camera.state_hash(state);
        self.rng.state_hash(state);
        self.pending_side_effects.state_hash(state);
        self.user_locked.state_hash(state);
        self.qa_recording_for.state_hash(state);
        self.qa_recording_slot.state_hash(state);
        self.action_before_recording_macro.state_hash(state);
        self.fast_forward.state_hash(state);
        self.pending_move_requests.state_hash(state);
        self.pending_path_requests.state_hash(state);
        self.failed_path_requests.state_hash(state);
        self.ai_global.state_hash(state);
        self.macro_store.state_hash(state);
        self.dead_pc.state_hash(state);
        self.timer_elements.state_hash(state);
        self.sequence_manager.state_hash(state);
        self.pending_reinforcements.state_hash(state);
        self.pending_scroll_amulets.state_hash(state);
        self.pending_hero_speeches.state_hash(state);
        self.pending_hades_kills.state_hash(state);
        self.pending_concussion_side_effects.state_hash(state);
        self.mission_script.state_hash(state);
        self.script_zone_data.state_hash(state);
        self.dynamic_sight_obstacles.state_hash(state);
        self.static_sight_obstacle_active.state_hash(state);
        self.campaign.state_hash(state);
    }
}
