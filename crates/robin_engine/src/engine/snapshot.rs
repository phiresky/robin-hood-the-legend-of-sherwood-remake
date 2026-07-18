//! Stable serialization boundary for [`EngineInner`].
//!
//! The in-memory engine is going to be split into cohesive owned state groups.
//! JSON save fields remain compatible while in-memory ownership changes. This
//! flat schema deliberately repeats the historical top-level field names and
//! normalizes legacy nested owners. Bincode rollback/replay bytes are locked
//! within a build, but historical replay compatibility is intentionally not a
//! constraint on the runtime layout. Deterministic hashing likewise follows
//! the current runtime ownership layout.

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

use super::{
    EngineInner,
    state::{
        AiRuntime, FeedbackRuntime, MissionDomain, OrderRuntime, PlayerRuntime, ScriptRuntime,
        SimulationControl, WorldState,
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
    #[serde(default)]
    script_zone_data: Option<Vec<crate::sector::ScriptSectorData>>,
    dynamic_sight_obstacles: Vec<crate::sight_obstacle::SightObstacle>,
    static_sight_obstacle_active: Vec<bool>,
    #[serde(default)]
    mobile_elements: Vec<crate::mobile::MobileElement>,
    campaign: Option<crate::campaign::Campaign>,
    #[serde(default)]
    script_domains: Option<super::state::ScriptDomains>,
}

impl Serialize for EngineInner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut snapshot = serializer.serialize_struct("EngineInner", 53)?;
        snapshot.serialize_field("sim_config", &self.sim_config)?;
        snapshot.serialize_field("mission", &self.mission_domain.state)?;
        snapshot.serialize_field("frame_counter", &self.control.frame_counter)?;
        snapshot.serialize_field("sound_sim", &self.feedback.sound_sim)?;
        snapshot.serialize_field("simulation_gates", &self.control.simulation_gates)?;
        snapshot.serialize_field("speed", &self.control.speed)?;
        snapshot.serialize_field("speed_int", &self.control.speed_int)?;
        snapshot.serialize_field("weather", &self.world.weather)?;
        snapshot.serialize_field("shield", &self.world.shield)?;
        snapshot.serialize_field("script_globals", &self.scripts.globals)?;
        snapshot.serialize_field("cheat_used_flags", &self.mission_domain.cheat_used_flags)?;
        snapshot.serialize_field(
            "standard_view_polygon_radius",
            &self.ai.standard_view_polygon_radius,
        )?;
        snapshot.serialize_field("next_order_id", &self.orders.next_order_id)?;
        snapshot.serialize_field("chorus_timer", &self.control.chorus_timer)?;
        snapshot.serialize_field("force_check", &self.script_domains.mission_ui.force_check)?;
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
        snapshot.serialize_field("mission_script", &self.scripts.mission)?;
        snapshot.serialize_field(
            "script_zone_data",
            &Option::<&Vec<crate::sector::ScriptSectorData>>::None,
        )?;
        snapshot.serialize_field(
            "dynamic_sight_obstacles",
            &self.world.dynamic_sight_obstacles,
        )?;
        snapshot.serialize_field(
            "static_sight_obstacle_active",
            &self.world.static_sight_obstacle_active,
        )?;
        snapshot.serialize_field("mobile_elements", &self.world.mobile_elements)?;
        snapshot.serialize_field("campaign", &self.mission_domain.campaign)?;
        snapshot.serialize_field("script_domains", &Some(&self.script_domains))?;
        snapshot.end()
    }
}

impl<'de> Deserialize<'de> for EngineInner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let snapshot = FlatEngineSnapshot::deserialize(deserializer)?;
        let mut mission_script = snapshot.mission_script;
        let legacy_script_domains = mission_script
            .as_mut()
            .and_then(super::MissionScript::take_legacy_script_domains);
        let legacy_native_owners = mission_script
            .as_mut()
            .map(super::MissionScript::take_legacy_native_owners);
        let mut entities = snapshot.entities;
        if let Some(legacy) = legacy_native_owners
            .as_ref()
            .and_then(|owners| owners.entities.clone())
            && !legacy.is_empty()
        {
            if entities.is_empty() {
                entities = legacy;
            } else {
                return Err(serde::de::Error::custom(
                    "Engine snapshot contains contradictory canonical and legacy GameHost entities",
                ));
            }
        }
        let mut ai_global = snapshot.ai_global;
        if let Some(legacy) = legacy_native_owners
            .as_ref()
            .and_then(|owners| owners.ai_global.clone())
            && !legacy.is_default_for_legacy_merge()
        {
            if ai_global.is_default_for_legacy_merge() {
                ai_global = legacy;
            } else {
                return Err(serde::de::Error::custom(
                    "Engine snapshot contains contradictory canonical and legacy GameHost AI state",
                ));
            }
        }
        let mut fast_grid = snapshot.fast_grid;
        if let Some(legacy) = legacy_native_owners.and_then(|owners| owners.fast_grid)
            && !legacy.runtime_is_empty_for_legacy_merge()
        {
            if fast_grid.runtime_is_empty_for_legacy_merge() {
                fast_grid = legacy;
            } else {
                return Err(serde::de::Error::custom(
                    "Engine snapshot contains contradictory canonical and legacy GameHost grid state",
                ));
            }
        }
        let mut script_domains = snapshot.script_domains.unwrap_or_default();
        if let Some(legacy) = legacy_script_domains {
            if let Some(buildings) = legacy.buildings {
                let current = &script_domains.buildings;
                if !current.occupants.is_empty()
                    || !current.arrow_reserves.is_empty()
                    || !current.actor_building.is_empty()
                    || !current.active.is_empty()
                    || !current.gates.is_empty()
                {
                    return Err(serde::de::Error::custom(
                        "Engine snapshot contains contradictory new and legacy building state",
                    ));
                }
                script_domains.buildings = buildings;
            }
            if let Some(interactables) = legacy.interactables {
                if !script_domains.interactables.doors.is_empty()
                    || !script_domains.interactables.patches.is_empty()
                {
                    return Err(serde::de::Error::custom(
                        "Engine snapshot contains contradictory new and legacy interactable state",
                    ));
                }
                script_domains.interactables = interactables;
            }
            if let Some(mission_ui) = legacy.mission_ui {
                if !script_domains.mission_ui.is_default_for_legacy_merge() {
                    return Err(serde::de::Error::custom(
                        "Engine snapshot contains contradictory new and legacy mission UI state",
                    ));
                }
                script_domains.mission_ui = mission_ui;
            }
            if let Some(scrolls) = legacy.scrolls {
                let current = &script_domains.scrolls;
                if !current.status.is_empty()
                    || !current.attachments.is_empty()
                    || !current.attachment_dirty.is_empty()
                {
                    return Err(serde::de::Error::custom(
                        "Engine snapshot contains contradictory new and legacy scroll state",
                    ));
                }
                script_domains.scrolls = scrolls;
            }
        }
        if let Some(legacy_zones) = snapshot.script_zone_data {
            if !script_domains.zones.scripts.is_empty() {
                return Err(serde::de::Error::custom(
                    "Engine snapshot contains contradictory new and legacy script zones",
                ));
            }
            script_domains.zones.scripts = legacy_zones;
        }
        script_domains.mission_ui.force_check |= snapshot.force_check;
        Ok(Self {
            sim_config: snapshot.sim_config,
            mission_domain: MissionDomain {
                state: snapshot.mission,
                cheat_used_flags: snapshot.cheat_used_flags,
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
                global: ai_global,
                standard_view_polygon_radius: snapshot.standard_view_polygon_radius,
            },
            world: WorldState {
                entities,
                pc_ids: snapshot.pc_ids,
                fast_grid,
                pathfinder: snapshot.pathfinder,
                weather: snapshot.weather,
                shield: snapshot.shield,
                dynamic_sight_obstacles: snapshot.dynamic_sight_obstacles,
                static_sight_obstacle_active: snapshot.static_sight_obstacle_active,
                mobile_elements: snapshot.mobile_elements,
            },
            script_domains,
            scripts: ScriptRuntime::from_snapshot(snapshot.script_globals, mission_script),
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
        })
    }
}
