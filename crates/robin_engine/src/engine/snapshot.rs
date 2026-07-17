//! Stable serialization boundary for [`EngineInner`].
//!
//! The in-memory engine is going to be split into cohesive owned state groups.
//! Save files and multiplayer bincode snapshots must not change merely because
//! fields move. This flat schema deliberately repeats the current field names,
//! types, attributes, and declaration order. Future in-memory regrouping maps
//! through this boundary instead of deriving a new external schema from the
//! runtime layout. Deterministic hashing follows the current runtime ownership
//! layout and is intentionally separate from this compatibility adapter.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

use super::{
    EngineInner,
    state::{
        AiRuntime, FeedbackRuntime, MissionDomain, OrderRuntime, PlayerRuntime, SimulationControl,
        WorldState,
    },
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
        snapshot.serialize_field("mission", &self.mission_domain.state)?;
        snapshot.serialize_field("frame_counter", &self.control.frame_counter)?;
        snapshot.serialize_field("sound_sim", &self.feedback.sound_sim)?;
        snapshot.serialize_field("simulation_gates", &self.control.simulation_gates)?;
        snapshot.serialize_field("speed", &self.control.speed)?;
        snapshot.serialize_field("speed_int", &self.control.speed_int)?;
        snapshot.serialize_field("weather", &self.world.weather)?;
        snapshot.serialize_field("shield", &self.world.shield)?;
        snapshot.serialize_field("script_globals", &self.script_globals)?;
        snapshot.serialize_field("cheat_used_flags", &self.mission_domain.cheat_used_flags)?;
        snapshot.serialize_field(
            "standard_view_polygon_radius",
            &self.ai.standard_view_polygon_radius,
        )?;
        snapshot.serialize_field("next_order_id", &self.orders.next_order_id)?;
        snapshot.serialize_field("chorus_timer", &self.control.chorus_timer)?;
        snapshot.serialize_field("force_check", &self.mission_domain.force_check)?;
        snapshot.serialize_field("messenger", &self.orders.messenger)?;
        snapshot.serialize_field("fast_grid", &self.world.fast_grid)?;
        snapshot.serialize_field("pathfinder", &self.world.pathfinder)?;
        snapshot.serialize_field("short_briefings", &self.mission_domain.short_briefings)?;
        snapshot.serialize_field("mission_stat", &self.mission_domain.mission_stat)?;
        snapshot.serialize_field("ground_mark", &self.feedback.ground_mark)?;
        snapshot.serialize_field("entities", &self.world.entities)?;
        snapshot.serialize_field("pc_ids", &self.world.pc_ids)?;
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
        snapshot.serialize_field("pending_move_requests", &self.orders.pending_move_requests)?;
        snapshot.serialize_field("pending_path_requests", &self.orders.pending_path_requests)?;
        snapshot.serialize_field("failed_path_requests", &self.orders.failed_path_requests)?;
        snapshot.serialize_field("ai_global", &self.ai.global)?;
        snapshot.serialize_field("macro_store", &self.players.macro_store)?;
        snapshot.serialize_field("dead_pc", &self.mission_domain.dead_pc)?;
        snapshot.serialize_field("timer_elements", &self.orders.timer_elements)?;
        snapshot.serialize_field("sequence_manager", &self.orders.sequence_manager)?;
        snapshot.serialize_field(
            "pending_reinforcements",
            &self.orders.pending_reinforcements,
        )?;
        snapshot.serialize_field(
            "pending_scroll_amulets",
            &self.orders.pending_scroll_amulets,
        )?;
        snapshot.serialize_field("pending_hero_speeches", &self.orders.pending_hero_speeches)?;
        snapshot.serialize_field("pending_hades_kills", &self.orders.pending_hades_kills)?;
        snapshot.serialize_field(
            "pending_concussion_side_effects",
            &self.orders.pending_concussion_side_effects,
        )?;
        snapshot.serialize_field("mission_script", &self.mission_script)?;
        snapshot.serialize_field("script_zone_data", &self.world.script_zones)?;
        snapshot.serialize_field(
            "dynamic_sight_obstacles",
            &self.world.dynamic_sight_obstacles,
        )?;
        snapshot.serialize_field(
            "static_sight_obstacle_active",
            &self.world.static_sight_obstacle_active,
        )?;
        snapshot.serialize_field("campaign", &self.mission_domain.campaign)?;
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
            mission_domain: MissionDomain {
                state: snapshot.mission,
                cheat_used_flags: snapshot.cheat_used_flags,
                force_check: snapshot.force_check,
                short_briefings: snapshot.short_briefings,
                mission_stat: snapshot.mission_stat,
                dead_pc: snapshot.dead_pc,
                campaign: snapshot.campaign,
            },
            control: SimulationControl {
                frame_counter: snapshot.frame_counter,
                simulation_gates: snapshot.simulation_gates,
                speed: snapshot.speed,
                speed_int: snapshot.speed_int,
                chorus_timer: snapshot.chorus_timer,
                rng: snapshot.rng,
                fast_forward: snapshot.fast_forward,
            },
            ai: AiRuntime {
                global: snapshot.ai_global,
                standard_view_polygon_radius: snapshot.standard_view_polygon_radius,
            },
            world: WorldState {
                entities: snapshot.entities,
                pc_ids: snapshot.pc_ids,
                fast_grid: snapshot.fast_grid,
                pathfinder: snapshot.pathfinder,
                weather: snapshot.weather,
                shield: snapshot.shield,
                script_zones: snapshot.script_zone_data,
                dynamic_sight_obstacles: snapshot.dynamic_sight_obstacles,
                static_sight_obstacle_active: snapshot.static_sight_obstacle_active,
            },
            script_globals: snapshot.script_globals,
            orders: OrderRuntime {
                next_order_id: snapshot.next_order_id,
                messenger: snapshot.messenger,
                pending_move_requests: snapshot.pending_move_requests,
                pending_path_requests: snapshot.pending_path_requests,
                failed_path_requests: snapshot.failed_path_requests,
                timer_elements: snapshot.timer_elements,
                sequence_manager: snapshot.sequence_manager,
                pending_reinforcements: snapshot.pending_reinforcements,
                pending_scroll_amulets: snapshot.pending_scroll_amulets,
                pending_hero_speeches: snapshot.pending_hero_speeches,
                pending_hades_kills: snapshot.pending_hades_kills,
                pending_concussion_side_effects: snapshot.pending_concussion_side_effects,
            },
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
            mission_script: snapshot.mission_script,
        })
    }
}
