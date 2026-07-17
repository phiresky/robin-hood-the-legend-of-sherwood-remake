//! Stable serialization and deterministic-hash boundary for [`EngineInner`].
//!
//! The in-memory engine is going to be split into cohesive owned state groups.
//! Save files, multiplayer bincode snapshots, and replay hashes must not change
//! merely because those fields move. This flat remote-serde schema deliberately
//! repeats the current field names, types, attributes, and declaration order.
//! Future in-memory regrouping maps through this boundary instead of deriving a
//! new external schema from the runtime layout.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

use super::{
    EngineInner,
    state::{FeedbackRuntime, PlayerRuntime, SimulationControl},
};

/// The compatibility snapshot shape present before the logical Engine split.
///
/// Deserialization first reads the historical flat shape, then moves each
/// value into its runtime owner. Serialization below writes fields manually in
/// this same order so no second authoritative state value exists.
#[derive(Deserialize)]
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
        let mut snapshot = serializer.serialize_struct("EngineInner", 51)?;
        snapshot.serialize_field("sim_config", &self.sim_config)?;
        snapshot.serialize_field("mission", &self.mission)?;
        snapshot.serialize_field("frame_counter", &self.control.frame_counter)?;
        snapshot.serialize_field("sound_sim", &self.feedback.sound_sim)?;
        snapshot.serialize_field("simulation_gates", &self.control.simulation_gates)?;
        snapshot.serialize_field("speed", &self.control.speed)?;
        snapshot.serialize_field("speed_int", &self.control.speed_int)?;
        snapshot.serialize_field("weather", &self.weather)?;
        snapshot.serialize_field("shield", &self.shield)?;
        snapshot.serialize_field("script_globals", &self.script_globals)?;
        snapshot.serialize_field("cheat_used_flags", &self.cheat_used_flags)?;
        snapshot.serialize_field(
            "standard_view_polygon_radius",
            &self.standard_view_polygon_radius,
        )?;
        snapshot.serialize_field("next_order_id", &self.next_order_id)?;
        snapshot.serialize_field("chorus_timer", &self.control.chorus_timer)?;
        snapshot.serialize_field("force_check", &self.force_check)?;
        snapshot.serialize_field("messenger", &self.messenger)?;
        snapshot.serialize_field("fast_grid", &self.fast_grid)?;
        snapshot.serialize_field("pathfinder", &self.pathfinder)?;
        snapshot.serialize_field("short_briefings", &self.short_briefings)?;
        snapshot.serialize_field("mission_stat", &self.mission_stat)?;
        snapshot.serialize_field("ground_mark", &self.feedback.ground_mark)?;
        snapshot.serialize_field("entities", &self.entities)?;
        snapshot.serialize_field("pc_ids", &self.pc_ids)?;
        snapshot.serialize_field("titbit_manager", &self.feedback.titbit_manager)?;
        snapshot.serialize_field("seats", &self.players.seats)?;
        snapshot.serialize_field("cutscene_camera", &self.feedback.cutscene_camera)?;
        snapshot.serialize_field("rng", &self.control.rng)?;
        snapshot.serialize_field("pending_side_effects", &self.feedback.pending_side_effects)?;
        snapshot.serialize_field("user_locked", &self.players.user_locked)?;
        snapshot.serialize_field("qa_recording_for", &self.players.qa_recording_for)?;
        snapshot.serialize_field("qa_recording_slot", &self.players.qa_recording_slot)?;
        snapshot.serialize_field(
            "action_before_recording_macro",
            &self.players.action_before_recording_macro,
        )?;
        snapshot.serialize_field("fast_forward", &self.control.fast_forward)?;
        snapshot.serialize_field("pending_move_requests", &self.pending_move_requests)?;
        snapshot.serialize_field("pending_path_requests", &self.pending_path_requests)?;
        snapshot.serialize_field("failed_path_requests", &self.failed_path_requests)?;
        snapshot.serialize_field("ai_global", &self.ai_global)?;
        snapshot.serialize_field("macro_store", &self.players.macro_store)?;
        snapshot.serialize_field("dead_pc", &self.dead_pc)?;
        snapshot.serialize_field("timer_elements", &self.timer_elements)?;
        snapshot.serialize_field("sequence_manager", &self.sequence_manager)?;
        snapshot.serialize_field("pending_reinforcements", &self.pending_reinforcements)?;
        snapshot.serialize_field("pending_scroll_amulets", &self.pending_scroll_amulets)?;
        snapshot.serialize_field("pending_hero_speeches", &self.pending_hero_speeches)?;
        snapshot.serialize_field("pending_hades_kills", &self.pending_hades_kills)?;
        snapshot.serialize_field(
            "pending_concussion_side_effects",
            &self.pending_concussion_side_effects,
        )?;
        snapshot.serialize_field("mission_script", &self.mission_script)?;
        snapshot.serialize_field("script_zone_data", &self.script_zone_data)?;
        snapshot.serialize_field("dynamic_sight_obstacles", &self.dynamic_sight_obstacles)?;
        snapshot.serialize_field(
            "static_sight_obstacle_active",
            &self.static_sight_obstacle_active,
        )?;
        snapshot.serialize_field("campaign", &self.campaign)?;
        snapshot.end()
    }
}

impl<'de> Deserialize<'de> for EngineInner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let snapshot = FlatEngineSnapshot::deserialize(deserializer)?;
        Ok(Self {
            sim_config: snapshot.sim_config,
            mission: snapshot.mission,
            control: SimulationControl {
                frame_counter: snapshot.frame_counter,
                simulation_gates: snapshot.simulation_gates,
                speed: snapshot.speed,
                speed_int: snapshot.speed_int,
                chorus_timer: snapshot.chorus_timer,
                rng: snapshot.rng,
                fast_forward: snapshot.fast_forward,
            },
            weather: snapshot.weather,
            shield: snapshot.shield,
            script_globals: snapshot.script_globals,
            cheat_used_flags: snapshot.cheat_used_flags,
            standard_view_polygon_radius: snapshot.standard_view_polygon_radius,
            next_order_id: snapshot.next_order_id,
            force_check: snapshot.force_check,
            messenger: snapshot.messenger,
            fast_grid: snapshot.fast_grid,
            pathfinder: snapshot.pathfinder,
            short_briefings: snapshot.short_briefings,
            mission_stat: snapshot.mission_stat,
            entities: snapshot.entities,
            pc_ids: snapshot.pc_ids,
            players: PlayerRuntime {
                seats: snapshot.seats,
                macro_store: snapshot.macro_store,
                user_locked: snapshot.user_locked,
                qa_recording_for: snapshot.qa_recording_for,
                qa_recording_slot: snapshot.qa_recording_slot,
                action_before_recording_macro: snapshot.action_before_recording_macro,
            },
            feedback: FeedbackRuntime {
                sound_sim: snapshot.sound_sim,
                ground_mark: snapshot.ground_mark,
                titbit_manager: snapshot.titbit_manager,
                cutscene_camera: snapshot.cutscene_camera,
                pending_side_effects: snapshot.pending_side_effects,
            },
            pending_move_requests: snapshot.pending_move_requests,
            pending_path_requests: snapshot.pending_path_requests,
            failed_path_requests: snapshot.failed_path_requests,
            ai_global: snapshot.ai_global,
            dead_pc: snapshot.dead_pc,
            timer_elements: snapshot.timer_elements,
            sequence_manager: snapshot.sequence_manager,
            pending_reinforcements: snapshot.pending_reinforcements,
            pending_scroll_amulets: snapshot.pending_scroll_amulets,
            pending_hero_speeches: snapshot.pending_hero_speeches,
            pending_hades_kills: snapshot.pending_hades_kills,
            pending_concussion_side_effects: snapshot.pending_concussion_side_effects,
            mission_script: snapshot.mission_script,
            script_zone_data: snapshot.script_zone_data,
            dynamic_sight_obstacles: snapshot.dynamic_sight_obstacles,
            static_sight_obstacle_active: snapshot.static_sight_obstacle_active,
            campaign: snapshot.campaign,
        })
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
        self.control.frame_counter.state_hash(state);
        self.feedback.sound_sim.state_hash(state);
        self.control.simulation_gates.state_hash(state);
        self.control.speed.state_hash(state);
        self.control.speed_int.state_hash(state);
        self.weather.state_hash(state);
        self.shield.state_hash(state);
        self.script_globals.state_hash(state);
        self.cheat_used_flags.state_hash(state);
        self.standard_view_polygon_radius.state_hash(state);
        self.next_order_id.state_hash(state);
        self.control.chorus_timer.state_hash(state);
        self.force_check.state_hash(state);
        self.messenger.state_hash(state);
        self.fast_grid.state_hash(state);
        self.pathfinder.state_hash(state);
        self.short_briefings.state_hash(state);
        self.mission_stat.state_hash(state);
        self.feedback.ground_mark.state_hash(state);
        self.entities.state_hash(state);
        self.pc_ids.state_hash(state);
        self.feedback.titbit_manager.state_hash(state);
        self.players.seats.state_hash(state);
        self.feedback.cutscene_camera.state_hash(state);
        self.control.rng.state_hash(state);
        self.feedback.pending_side_effects.state_hash(state);
        self.players.user_locked.state_hash(state);
        self.players.qa_recording_for.state_hash(state);
        self.players.qa_recording_slot.state_hash(state);
        self.players.action_before_recording_macro.state_hash(state);
        self.control.fast_forward.state_hash(state);
        self.pending_move_requests.state_hash(state);
        self.pending_path_requests.state_hash(state);
        self.failed_path_requests.state_hash(state);
        self.ai_global.state_hash(state);
        self.players.macro_store.state_hash(state);
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
