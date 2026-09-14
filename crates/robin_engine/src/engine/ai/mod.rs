//! Enemy AI engine integration.
//!
//! Tick orchestration and engine-owned side effects are grouped by domain:
//!  - [`tick_data`] and [`snapshots`] build owner-boundary tactical views.
//!  - [`tick_scheduling`] and [`owner_scheduling`] drive per-NPC work.
//!  - [`detection`] and [`post_detection`] implement the visibility phases.
//!  - [`event_dispatch`] handles animation, noise, detection, and speech events.
//!  - [`patrol_coordination`] and [`patrol_dispatch`] coordinate patrol members.
//!  - [`cross_npc_actions`] closes synchronous interactions between NPCs.

mod alert_execution;
mod archery_execution;
mod battle_approach;
mod battle_archery;
mod battle_cover;
mod battle_decision_execution;
#[cfg(test)]
mod battle_decision_observation_tests;
#[cfg(test)]
mod battle_decision_panic_tests;
mod battle_execution;
mod body_execution;
mod body_observation;
mod cross_npc_actions;
mod detection;
mod duty_callers;
mod duty_common;
mod duty_execution;
mod enemy_report_execution;
mod event_dispatch;
#[cfg(test)]
mod event_handler_live_tests;
mod execution;
mod friend_check_execution;
mod friendly_execution;
mod initialization;
mod live_visibility;
mod macro_execution;
mod money_execution;
mod officer_rendezvous_execution;
mod owner_scheduling;
mod panic_execution;
mod patrol_assembly;
mod patrol_coordination;
mod patrol_dispatch;
mod phalanx_execution;
mod protection_execution;
mod seek_execution;
mod shot_selection;
mod swordfight_candidates;
mod swordfight_execution;
mod wondering_execution;
#[cfg(test)]
pub(crate) use detection::capture_heard_callbacks;
pub(crate) use detection::debug_detectable_mutation_load_snapshot;
mod post_detection;
mod snapshots;
mod tick_data;
mod tick_scheduling;

#[cfg(test)]
pub(crate) use post_detection::{
    NpcPostDetectionTailPhase, capture_npc_post_detection_tail_phases,
};

use super::*;
use crate::ai::{AiContext, StimulusType};
use crate::ai_entity_view::{self, AiEntityViewMap, AiEntityViews, SharedAiEntityViews};
use crate::ai_vision;
use crate::coordinates::MapPoint;
use crate::element::{
    Camp, Detectable, DetectableType, Entity, EntityId, Human as _, PcId, SoldierId,
};
use crate::engine::SimScratch;
use crate::entities::Entities;
use serde::{Deserialize, Serialize};

fn beam_door_waypoints_into_houses(
    paths: &mut [crate::level_data::RawHikingPath],
    mut waypoint_sectors: Option<&mut Vec<Vec<crate::position_interface::SectorHandle>>>,
    doors: &[crate::ai::DoorSeekInfo],
) {
    if let Some(sectors) = waypoint_sectors.as_deref() {
        assert_eq!(
            sectors.len(),
            paths.len(),
            "exact hiking-waypoint sector rows do not match the authored path count"
        );
        for (path_index, (sector_row, path)) in sectors.iter().zip(paths.iter()).enumerate() {
            assert_eq!(
                sector_row.len(),
                path.waypoints.len(),
                "exact hiking-waypoint sector row {path_index} does not match the authored waypoint count"
            );
        }
    }

    for (path_index, path) in paths.iter_mut().enumerate() {
        for (waypoint_index, waypoint) in path.waypoints.iter_mut().enumerate() {
            for door in doors {
                if door.door_type != crate::gate::DoorType::Building {
                    continue;
                }
                let dx = (waypoint.x as f32 - door.point_out.x).abs();
                let dy = (waypoint.y as f32 - door.point_out.y).abs();
                if dx.max(dy) > 5.0 {
                    continue;
                }

                // The original game assigns the complete position returned by
                // the door's inside position, including its sector identity.
                // Rewriting only the public number leaves an impossible
                // outside-arena/interior-number pair on overlapping sectors.
                let inside_sector = door.position_in.sector.unwrap_or_else(|| {
                    panic!(
                        "building door {} has no required interior sector",
                        door.door_index.get()
                    )
                });
                if waypoint_sectors.is_some() {
                    assert!(
                        inside_sector.arena_index().is_some(),
                        "building door {} interior sector has no exact arena identity",
                        door.door_index.get()
                    );
                }
                waypoint.x = door.position_in.x as i16;
                waypoint.y = door.position_in.y as i16;
                waypoint.sector = u16::from(inside_sector);
                waypoint.level = door.position_in.level;
                if let Some(sectors) = waypoint_sectors.as_deref_mut() {
                    sectors[path_index][waypoint_index] = inside_sector;
                }
                break;
            }
        }
    }
}

#[cfg(test)]
mod building_door_membership_tests {
    use super::{beam_door_waypoints_into_houses, door_belongs_to_ai_house};
    use crate::ai::{DoorSeekInfo, Position};
    use crate::coordinates::MapPoint;
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::{DoorIndex, DoorType};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};
    use crate::position_interface::SectorHandle;

    #[test]
    fn building_trap_remains_in_complete_ai_house_gate_list() {
        assert!(door_belongs_to_ai_house(DoorType::Building));
        assert!(door_belongs_to_ai_house(DoorType::BuildingTrap));

        for unrelated in [
            DoorType::Default,
            DoorType::Gate,
            DoorType::LiftHigh,
            DoorType::LiftLow,
            DoorType::LiftHighCrenel,
            DoorType::Trap,
            DoorType::Reinforcement,
        ] {
            assert!(
                !door_belongs_to_ai_house(unrelated),
                "{unrelated:?} must not create an AI house association"
            );
        }
    }

    #[test]
    fn building_waypoint_beam_replaces_public_and_exact_sector_together() {
        let outside = SectorHandle::new(0)
            .unwrap()
            .with_arena_index(SectorIndex::new(0).unwrap());
        let inside = SectorHandle::new(146)
            .unwrap()
            .with_arena_index(SectorIndex::new(146).unwrap());
        let mut paths = vec![RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 1955,
                y: 1992,
                sector: 0,
                level: 0,
                command: WaypointCommand::None,
            }],
        }];
        let mut sectors = vec![vec![outside]];
        let doors = vec![DoorSeekInfo {
            door_index: DoorIndex::new(5).expect("valid door index"),
            door_type: DoorType::Building,
            point_out: MapPoint::new(1955.0, 1992.0),
            position_in: Position {
                x: 1938.0,
                y: 1964.0,
                sector: Some(inside),
                level: 8,
            },
            sector_out: 0,
            sector_out_index: Some(SectorIndex::new(0).unwrap()),
            sector_in: 146,
            layer_out: 0,
            npc_villain_authorized_direct: true,
        }];

        beam_door_waypoints_into_houses(&mut paths, Some(&mut sectors), &doors);

        let waypoint = &paths[0].waypoints[0];
        assert_eq!(
            (waypoint.x, waypoint.y, waypoint.sector, waypoint.level),
            (1938, 1964, 146, 8)
        );
        assert_eq!(sectors[0][0].get(), 146);
        assert_eq!(sectors[0][0].arena_index(), SectorIndex::new(146));
    }
}

/// `[from frame (default 0), through frame (default u32::MAX), creation order]`,
/// all optional.
fn refresh_view_lifecycle_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<3> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<3>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE",
            [
                "PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_FROM",
                "PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_THROUGH",
                "PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_CREATION_ORDER",
            ],
        )
    })
}

/// Opt-in provenance for the two-draw building-exit wait used by parity
/// audits. This is deliberately process-local diagnostic state: it must not
/// enter snapshots, state hashes, or the simulation RNG stream.
pub(super) fn building_exit_wait_owner_debug_enabled() -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<0>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| ParityGate::from_env("PARITY_DEBUG_BUILDING_EXIT_WAIT_OWNER", []))
        .enabled()
}

/// Select the fleeing side for a two-allegiance doorway battle.
///
/// Original always puts Royalists in the fleeing list when the legacy camps
/// are tied during in-house enemy alerts. Generalized
/// allegiances retain that source-independent ordering by using their authored
/// allegiance IDs as the tie-breaker.
fn doorway_battle_source_side_flees(
    source_camp: crate::element::Camp,
    source_count: usize,
    opposing_camp: crate::element::Camp,
    opposing_count: usize,
) -> bool {
    if source_count != opposing_count {
        return source_count < opposing_count;
    }
    let source_id = source_camp
        .allegiance_id()
        .expect("doorway battle source must have a valid allegiance");
    let opposing_id = opposing_camp
        .allegiance_id()
        .expect("doorway battle opponent must have a valid allegiance");
    source_id < opposing_id
}

fn authoritative_house_occupants(
    building_sector: u32,
    canonical_handles: Option<&[i32]>,
    mirrored: &[EntityId],
    mut resolve: impl FnMut(i32) -> Option<EntityId>,
) -> Vec<EntityId> {
    canonical_handles.map_or_else(
        || mirrored.to_vec(),
        |handles| {
            handles
                .iter()
                .map(|&handle| {
                    resolve(handle).unwrap_or_else(|| {
                        panic!(
                            "building {building_sector} occupant handle {handle} has no live actor"
                        )
                    })
                })
                .collect()
        },
    )
}

#[cfg(test)]
mod doorway_battle_side_tests {
    use super::{authoritative_house_occupants, doorway_battle_source_side_flees};
    use crate::element::{Camp, EntityId};
    use crate::entity_id::{CivilianId, PcId, SoldierId};

    #[test]
    fn tied_legacy_battle_always_makes_royalists_flee() {
        assert!(doorway_battle_source_side_flees(
            Camp::Royalists,
            1,
            Camp::Lacklandists,
            1,
        ));
        assert!(!doorway_battle_source_side_flees(
            Camp::Lacklandists,
            1,
            Camp::Royalists,
            1,
        ));
    }

    #[test]
    fn canonical_building_occupants_override_a_lagging_ai_house_mirror() {
        let pc = EntityId::Pc(PcId(169));
        let soldier = EntityId::Soldier(SoldierId(55));
        let civilian = EntityId::Civilian(CivilianId(33));
        let mirrored = [pc, soldier];
        let canonical = [33, 169, 55];

        let occupants =
            authoritative_house_occupants(
                126,
                Some(&canonical),
                &mirrored,
                |handle| match handle {
                    33 => Some(civilian),
                    169 => Some(pc),
                    55 => Some(soldier),
                    _ => None,
                },
            );

        assert_eq!(occupants, [civilian, pc, soldier]);
    }
}

/// Original attaches both ordinary building doors and building-trap doors to
/// the building gate list. AI house initialization must preserve that
/// ownership because both rally-point creation and door-fight placement walk
/// the complete building gate list.
fn door_belongs_to_ai_house(door_type: crate::gate::DoorType) -> bool {
    matches!(
        door_type,
        crate::gate::DoorType::Building | crate::gate::DoorType::BuildingTrap
    )
}

/// `[frame, creation order]`, both required.
fn civilian_random_speech_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env_required(
            "PARITY_DEBUG_CIVILIAN_RANDOM_SPEECH",
            [
                "PARITY_DEBUG_CIVILIAN_RANDOM_SPEECH_FRAME",
                "PARITY_DEBUG_CIVILIAN_RANDOM_SPEECH_CREATION_ORDER",
            ],
        )
    })
}

/// `[frame, actor slot]`, both optional.
fn speech_lifecycle_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_SPEECH_LIFECYCLE",
            [
                "PARITY_DEBUG_SPEECH_LIFECYCLE_FRAME",
                "PARITY_DEBUG_SPEECH_LIFECYCLE_ACTOR",
            ],
        )
    })
}

/// `[frame, creation order]`, both optional.
fn patrol_turn_lifecycle_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_PATROL_TURN_LIFECYCLE",
            [
                "PARITY_DEBUG_PATROL_TURN_FRAME",
                "PARITY_DEBUG_PATROL_TURN_CREATION_ORDER",
            ],
        )
    })
}

/// `[frame, creation order, owner handle]`, all optional.
fn archer_step_back_lifecycle_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<3> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<3>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_ARCHER_STEP_BACK_LIFECYCLE",
            [
                "PARITY_DEBUG_ARCHER_STEP_BACK_FRAME",
                "PARITY_DEBUG_ARCHER_STEP_BACK_CREATION_ORDER",
                "PARITY_DEBUG_ARCHER_STEP_BACK_OWNER_HANDLE",
            ],
        )
    })
}

/// Opt-in, stderr-only trace for the Save049 timer-driven archer step-back.
/// The original game executes the actor update before the NPC timer tail, so
/// the authoritative installed order and sprite completion counters at context
/// construction distinguish a late actor retirement from an AI movement issue.
fn archer_step_back_lifecycle_debug_matches(
    frame: u32,
    creation_order: Option<u32>,
    owner_handle: u32,
) -> bool {
    archer_step_back_lifecycle_debug_gate().matches_required([
        Some(frame),
        creation_order,
        Some(owner_handle),
    ])
}

/// `[installed, concrete]` animations.
#[inline(never)]
fn trace_archer_step_back_context(
    frame: u32,
    original_creation_order: Option<u32>,
    elem: &crate::element::ElementData,
    actor: Option<&crate::element::ActorData>,
    [self_animation, concrete_self_animation]: [crate::order::OrderType; 2],
    self_action_state: impl std::fmt::Debug,
    self_animation_reached_action_done: bool,
) {
    let sprite = &elem.sprite;
    eprintln!(
        "[ARCHERSTEP frame={frame} co={original_creation_order:?} me={} phase=context installed={self_animation:?} concrete={concrete_self_animation:?} action_state={self_action_state:?} motion_state={:?} order_id={:?} last_execute_order_id={:?} sprite_action={:?} row={} sprite_frame={} frame_count={} done_frame={} done_counter={} reached_done={self_animation_reached_action_done}]",
        elem.index_in_elements_list,
        actor.map(|actor| actor.continuation.motion_state),
        actor.and_then(|actor| actor.installed_order.map(|order| order.order_id)),
        actor.and_then(|actor| actor.last_execute_order_id),
        sprite.last_action,
        sprite.current_row,
        sprite.current_frame,
        sprite.frame_count,
        sprite.action_done_frame,
        sprite.action_done_counter,
    );
}

impl EngineInner {
    fn patrol_turn_lifecycle_debug_matches(&self, owner: EntityId) -> bool {
        let gate = patrol_turn_lifecycle_debug_gate();
        if !gate.matches([Some(self.control.frame_counter), None]) {
            return false;
        }
        let creation_order = self.world.original_creation_order(owner);
        gate.matches([None, Some(creation_order)])
    }

    /// Opt-in, process-local trace of patrol Turn registration and ownership.
    /// It deliberately reads only live state and writes only stderr, so it
    /// cannot affect serialization, state hashes, ordering, or RNG.
    #[inline(never)]
    pub(super) fn debug_patrol_turn_lifecycle(&self, boundary: &'static str, owner: EntityId) {
        if !self.patrol_turn_lifecycle_debug_matches(owner) {
            return;
        }
        let creation_order = self.world.original_creation_order(owner);
        let current = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(sequence_id, element_index)| {
                self.orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .map(|element| {
                        (
                            sequence_id,
                            element_index,
                            element.command,
                            element.state,
                            element.current_order().map(|order| order.order_id),
                        )
                    })
            });
        let (installed, last_execute, sprite_order) = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::actor_data)
            .map(|actor| {
                let sprite_order = self
                    .world
                    .entities
                    .get(owner)
                    .map(|entity| entity.sprite().last_processed_order_id);
                (
                    actor.installed_order,
                    actor.last_execute_order_id,
                    sprite_order,
                )
            })
            .unwrap_or((None, None, None));
        let deferred_turns = self
            .orders
            .sequence_manager
            .deferred_elements_to_go()
            .into_iter()
            .filter_map(|(sequence_id, element_index)| {
                self.orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
                    .filter(|element| {
                        element.owner == Some(owner)
                            && matches!(
                                element.command,
                                crate::element::Command::Turn | crate::element::Command::TurnFast
                            )
                    })
                    .map(|element| (sequence_id, element_index, element.state))
            })
            .collect::<Vec<_>>();
        eprintln!(
            "PATROLTURN frame={} creation_order={} owner={owner:?} boundary={boundary} current={current:?} installed={installed:?} last_execute={last_execute:?} sprite_order={sprite_order:?} deferred_turns={deferred_turns:?}",
            self.control.frame_counter, creation_order,
        );
    }

    #[inline(never)]
    pub(super) fn debug_patrol_turn_instruct(
        &self,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) {
        if !self.patrol_turn_lifecycle_debug_matches(owner) {
            return;
        }
        let is_turn = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .is_some_and(|element| {
                matches!(
                    element.command,
                    crate::element::Command::Turn | crate::element::Command::TurnFast
                )
            });
        if is_turn {
            self.debug_patrol_turn_lifecycle("manager_instruct_turn", owner);
            eprintln!(
                "PATROLTURN frame={} owner={owner:?} boundary=manager_instruct_ref sequence={sequence_id:?} element={element_index}",
                self.control.frame_counter,
            );
        }
    }
}

/// Record-only trace of synchronous `EVENT_GALOPP_LOOP_END` dispatches.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::engine) enum GalloppProbeEvent {
    /// Recorded after the dispatch's Think/script/order drain has closed.
    Dispatched {
        owner: EntityId,
        owned_elements: Vec<GalloppOwnedElement>,
    },
    /// Test-authored ordering marker (for example from an owner-slot hook),
    /// interleaved with dispatches in the same capture.
    Marker(EntityId),
}

/// Snapshot of one sequence element owned by the dispatched rider.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::engine) struct GalloppOwnedElement {
    pub sequence: crate::sequence::SequenceId,
    pub element: usize,
    pub state: crate::sequence::SequenceState,
    pub current_order: Option<std::num::NonZeroU32>,
}

#[cfg(test)]
thread_local! {
    static GALOPP_DISPATCH_PROBE: crate::engine::test_support::Probe<GalloppProbeEvent> =
        const { crate::engine::test_support::Probe::new() };
}

/// Exact frame phase computed by the NPC update.
/// `register_number` is the original creation/register ordering value.
pub(super) fn npc_hourglass_frame_phase(frame: u32, register_number: u32) -> u8 {
    (frame as u8).wrapping_sub((register_number as u8).wrapping_add(100))
}

/// Number of arrows given to Merry Man archers in forest levels.
const MERRY_MAN_ARROWS: u16 = 3;

/// Match NPC direction-vector behavior for the directed-panic
/// front-facing test. Original builds the facing vector with
/// aspect-corrected direction-sector assignment, which compresses its Y component
/// before taking the ordinary 2D dot product.
fn directed_panic_center_is_in_front(
    direction: i16,
    actor_x: f32,
    actor_y: f32,
    center_x: f32,
    center_y: f32,
) -> bool {
    let (face_x, face_y) = crate::element::direction_vector_16(direction);
    let dx = center_x - actor_x;
    let dy = center_y - actor_y;
    face_x * dx + face_y * crate::position_interface::ASPECT_RATIO * dy > 0.0
}

#[cfg(test)]
mod directed_panic_front_tests {
    use super::*;

    const ACTOR_X: f32 = 807.457_64;
    const ACTOR_Y: f32 = 767.533_3;
    const CENTER_X: f32 = 866.094_1;
    const CENTER_Y: f32 = 592.307_6;

    #[test]
    fn task338_aspect_scaled_facing_treats_recorded_center_as_in_front() {
        let (face_x, face_y) = crate::element::direction_vector_16(5);
        let dx = CENTER_X - ACTOR_X;
        let dy = CENTER_Y - ACTOR_Y;

        // The unscaled unit-vector dot has the opposite sign, so this fixture
        // specifically guards the original game's aspect-ratio-compressed facing Y.
        assert!(face_x * dx + face_y * dy < 0.0);
        assert!(directed_panic_center_is_in_front(
            5, ACTOR_X, ACTOR_Y, CENTER_X, CENTER_Y
        ));
    }

    #[test]
    fn task338_aspect_scaled_facing_rejects_mirrored_center() {
        let mirrored_x = ACTOR_X - (CENTER_X - ACTOR_X);
        let mirrored_y = ACTOR_Y - (CENTER_Y - ACTOR_Y);

        assert!(!directed_panic_center_is_in_front(
            5, ACTOR_X, ACTOR_Y, mirrored_x, mirrored_y
        ));
    }
}

#[cfg(test)]
mod panic_boundary_tests {
    use super::*;
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, AiActorData, AiBrain, ElementData, ElementKind,
        HumanData, NpcData, PcData, Posture, SoldierData,
    };

    #[test]
    fn panic_door_search_rereads_locks_occupants_and_wrapped_distance() {
        use crate::coordinates::MapPoint;
        use crate::fast_find_grid::{GridSector, SectorIndex};
        use crate::gate::{Door, DoorType, GateType};
        use crate::sector::{BuildingIdx, SectorNumber, SectorType};
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(enemy_soldier());
        let pc = engine.add_test_entity(enemy_ai_hero());
        for index in 0..2usize {
            let sector = GridSector {
                points: vec![],
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: SectorType::MOTION | SectorType::AREA | SectorType::BUILDING,
                layer: 0,
                sector_number: SectorNumber::new(index as i16 + 1),
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: BuildingIdx::new(index as u16),
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: vec![],
                gate_indices: vec![],
                underlying_sector: None,
            };
            let level = engine.world.fast_grid_mut().level_mut();
            level.sector_number_map.insert(sector.sector_number, index);
            level.sectors.push(sector);
            engine.script_domains.interactables.doors.push(Door {
                gate_type: GateType::Door,
                door_type: DoorType::Building,
                point_out: MapPoint::new(10.0 + index as f32 * 10.0, 0.0),
                sector_in: SectorNumber::new(index as i16 + 1),
                sector_in_index: SectorIndex::new(index as u32),
                ..Door::default()
            });
        }
        engine.script_domains.buildings.occupants = vec![vec![], vec![]];
        assert_eq!(engine.nearest_panic_door(owner, None), Some(0));
        engine.script_domains.interactables.doors[0].locked_npc_villain = true;
        assert_eq!(engine.nearest_panic_door(owner, None), Some(1));
        engine.script_domains.interactables.doors[0].locked_npc_villain = false;
        engine.script_domains.buildings.occupants[0]
            .push(crate::natives::ScriptHandleCodec::actor_handle(pc));
        assert_eq!(engine.nearest_panic_door(owner, None), Some(1));
        engine.script_domains.buildings.occupants[0].clear();
        engine.script_domains.interactables.doors[1].point_out.x = 65_040.0;
        assert_eq!(
            engine.nearest_panic_door(owner, None),
            Some(1),
            "the sector penalty wraps the sixteen-bit score"
        );
    }

    fn enemy_soldier() -> Entity {
        let enemy_ai = crate::ai_enemy::EnemyAi {
            hth_weapon_id: 1,
            ..Default::default()
        };
        Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai: crate::element::AiActorData {
                    ai_brain: AiBrain::Enemy(Box::new(enemy_ai)),
                    ..Default::default()
                },
                ..NpcData::default()
            },
            soldier: SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..SoldierData::default()
            },
        })
    }

    fn enemy_ai_hero() -> Entity {
        let enemy_ai = crate::ai_enemy::EnemyAi {
            hth_weapon_id: 1,
            ..Default::default()
        };
        Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                command_interface: crate::human_control::CommandInterface::None,
                mission_role: crate::human_control::MissionRole::Combatant,
                combat_stance: crate::human_control::CombatStance::Aggressive,
                ai: Some(Box::new(AiActorData {
                    ai_brain: AiBrain::Enemy(Box::new(enemy_ai)),
                    ..AiActorData::default()
                })),
                ..PcData::default()
            },
        })
    }

    #[test]
    fn panic_state_dispatch_accepts_enemy_ai_hero_enemy_ai() {
        let mut engine = EngineInner::new();
        let pc_id = engine.add_test_entity(enemy_ai_hero());

        engine.set_typed_npc_state(
            pc_id,
            crate::ai::AiState::Fleeing,
            crate::ai::Substate::FleeingPanic,
            "Panic run entry",
        );

        let ai = engine
            .get_entity(pc_id)
            .and_then(Entity::enemy_ai)
            .expect("AI-controlled hero must retain its Enemy AI");
        assert_eq!(ai.base.current_state, crate::ai::AiState::Fleeing);
        assert_eq!(ai.base.current_substate, crate::ai::Substate::FleeingPanic);
    }

    #[test]
    fn script_think_entry_dispatches_to_enemy_ai_hero_enemy_ai() {
        let mut engine = EngineInner::new();
        let pc_id = engine.add_test_entity(enemy_ai_hero());

        engine.start_script_ai_native_think_pre_filter(pc_id);

        assert_eq!(engine.ai_think_depth(), 1);
    }

    #[test]
    fn enemy_ai_hero_completes_new_no_door_panic_boundary() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let pc_id = engine.add_test_entity(enemy_ai_hero());
        let mut assets = LevelAssets::default();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
        let request = crate::ai::PanicRequest {
            center: None,
            runs: crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
            alert: crate::ai::AlertLevel::Red,
            is_new_panic: true,
        };

        engine.begin_panic_no_door_branch(&sim, &assets, pc_id, &request, false);

        let ai = engine
            .get_entity(pc_id)
            .and_then(Entity::enemy_ai)
            .expect("AI-controlled hero must retain its Enemy AI");
        assert_eq!(ai.base.current_state, crate::ai::AiState::Fleeing);
        assert_eq!(ai.base.current_substate, crate::ai::Substate::FleeingHiding);
    }

    #[test]
    fn new_panic_request_keeps_outgoing_state_until_live_door_selection() {
        let mut friendly = crate::ai_friendly::FriendlyAi::new(1);
        friendly.base.outbox.actor.set_unfocus();
        friendly.panic_undirected(8);
        assert_eq!(friendly.base.current_state, crate::ai::AiState::Default);
        assert_eq!(
            friendly.base.current_substate,
            crate::ai::Substate::DefaultOnPost
        );
        assert!(friendly.base.outbox.reentrant.owner_work.is_empty());
        assert!(friendly.base.outbox.actor.unfocus);
        assert!(friendly.base.outbox.actor.begin_panic.unwrap().is_new_panic);
    }

    #[test]
    fn enemy_ai_hero_recovers_from_stuck_ladder_through_enemy_ai() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let mut pc = enemy_ai_hero();
        pc.set_posture(Posture::OnLadder);
        pc.ai_actor_data_mut()
            .expect("AI-controlled hero has AI actor data")
            .stuck_on_ladder_emergency_counter = 25;
        let pc_id = engine.add_test_entity(pc);
        let mut assets = LevelAssets::default();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());

        engine.tick_npc_stuck_on_ladder_for_npc(&sim, pc_id, &assets);

        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::ai_actor_data)
                .expect("AI-controlled hero retains its AI actor data")
                .stuck_on_ladder_emergency_counter,
            0
        );
        assert!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::enemy_ai)
                .is_some()
        );
    }

    #[test]
    fn new_no_door_panic_boundary_closes_recursive_reachpoint() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let npc_id = engine.add_test_entity(enemy_soldier());
        let mut assets = LevelAssets::default();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
        let runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
        let request = crate::ai::PanicRequest {
            center: None,
            runs,
            alert: crate::ai::AlertLevel::Red,
            is_new_panic: true,
        };

        engine.begin_panic_no_door_branch(&sim, &assets, npc_id, &request, false);

        let ai = engine.get_entity(npc_id).unwrap().ai_controller().unwrap();
        assert_eq!(ai.current_state, crate::ai::AiState::Fleeing);
        assert_eq!(ai.current_substate, crate::ai::Substate::FleeingHiding);
        assert_eq!(ai.view_alert_status, crate::ai::AlertLevel::Yellow);
        assert_eq!(ai.current_music_alert_status, crate::ai::AlertLevel::Yellow);
        assert_eq!(ai.lasting_panic_runs, 0);
    }

    #[test]
    fn repeated_no_door_panic_boundary_preserves_red_and_larger_run_count() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let npc_id = engine.add_test_entity(enemy_soldier());
        {
            let ai = engine
                .get_entity_mut(npc_id)
                .unwrap()
                .ai_controller_mut()
                .unwrap();
            ai.current_state = crate::ai::AiState::Fleeing;
            ai.current_substate = crate::ai::Substate::FleeingPanic;
            ai.set_alert_status(crate::ai::AlertLevel::Red);
            ai.lasting_panic_runs = 11;
        }
        let request = crate::ai::PanicRequest {
            center: None,
            runs: 8,
            alert: crate::ai::AlertLevel::Red,
            is_new_panic: false,
        };

        engine.begin_panic_no_door_branch(&sim, &LevelAssets::default(), npc_id, &request, false);

        let ai = engine.get_entity(npc_id).unwrap().ai_controller().unwrap();
        assert_eq!(ai.current_state, crate::ai::AiState::Fleeing);
        assert_eq!(ai.current_substate, crate::ai::Substate::FleeingPanic);
        assert_eq!(ai.view_alert_status, crate::ai::AlertLevel::Red);
        assert_eq!(ai.current_music_alert_status, crate::ai::AlertLevel::Red);
        assert_eq!(ai.lasting_panic_runs, 11);
        assert!(ai.outbox.reentrant.owner_work.is_empty());
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    #[test]
    fn synchronous_panic_boundary_consumes_recursive_reach_point_rng() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let npc_id = engine.add_test_entity(enemy_soldier());
        let mut assets = LevelAssets::default();
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
        let request = crate::ai::PanicRequest {
            center: Some(AiContext::test_fixture().position),
            runs: crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8,
            alert: crate::ai::AlertLevel::Red,
            is_new_panic: true,
        };

        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            engine.begin_panic_no_door_branch(&sim, &assets, npc_id, &request, false);
        });

        assert!(
            draws.len() >= 2
                && draws
                    .iter()
                    .all(|site| *site == crate::sim_rng::RngSite::AiPanic),
            "the deferred boundary must retain Panic's direction/distance draws: {draws:?}"
        );
        let ai = engine.get_entity(npc_id).unwrap().ai_controller().unwrap();
        assert_eq!(ai.current_state, crate::ai::AiState::Fleeing);
    }
}

fn append_detectable(
    list: &mut Vec<Detectable>,
    entity_id: EntityId,
    detectable_type: DetectableType,
    preserve_duplicate: bool,
) {
    if preserve_duplicate
        || !list
            .iter()
            .any(|detectable| detectable.element == Some(entity_id))
    {
        list.push(Detectable {
            element: Some(entity_id),
            detectable_type,
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod detectable_append_tests {
    use super::*;

    #[test]
    fn alert_officer_append_preserves_preseeded_friend_duplicate() {
        let officer = EntityId::Soldier(SoldierId(97));
        let mut friends = vec![Detectable {
            element: Some(officer),
            detectable_type: DetectableType::Friend,
            ..Default::default()
        }];

        append_detectable(&mut friends, officer, DetectableType::Friend, true);

        assert_eq!(friends.len(), 2);
        assert!(friends.iter().all(|detectable| {
            detectable.element == Some(officer)
                && detectable.detectable_type == DetectableType::Friend
        }));
    }

    #[test]
    fn ordinary_detectable_add_remains_unique() {
        let officer = EntityId::Soldier(SoldierId(97));
        let mut friends = vec![Detectable {
            element: Some(officer),
            detectable_type: DetectableType::Friend,
            ..Default::default()
        }];

        append_detectable(&mut friends, officer, DetectableType::Friend, false);

        assert_eq!(friends.len(), 1);
    }
}

/// Snapshot of a potential detectable human at level-load time.
///
/// Used by [`EngineInner::init_one_ai`] to filter which other humans each
/// NPC should start with in its `detectable_lists[Enemy]` array —
/// the "create list of detectable enemies" pass inside the per-NPC
/// init for both enemy and friendly AI.
#[derive(Debug, Clone, Copy)]
#[cfg(test)]
pub(super) struct PotentialDetectable {
    id: EntityId,
    is_pc: bool,
    is_soldier: bool,
    camp: Camp,
}

/// Apply the numeric tail of original-game NPC hearing volume.
///
/// The distance remainder is truncated to 16 bits before deafness is tested
/// and subtracted. A positive fractional remainder is therefore inaudible;
/// testing the float first can incorrectly dispatch `EVENT_HEAR` with a
/// zero-volume payload.
fn subjective_hear_volume(modified_volume: f32, distance: f32, deafness: u16) -> u16 {
    let remainder = modified_volume - distance;
    if remainder <= 0.0 {
        return 0;
    }
    let truncated = remainder as u16;
    truncated.saturating_sub(deafness)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct NpcSpeechSettlement {
    pub(super) invoke_finished_callback: bool,
    pub(super) category_rejection: Option<CategorySpeechRejectionFinalization>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CategorySpeechRejectionFinalization {
    reason_after_callback: Option<u16>,
}

/// Build a snapshot of every authored human in the engine. Called once at
/// the start of [`EngineInner::init_ai`] and handed to every per-NPC init
/// pass.
#[cfg(test)]
pub(super) fn build_potential_detectables(engine: &EngineInner) -> Vec<PotentialDetectable> {
    let mut out = Vec::new();
    for (id, entity) in engine.world.entities.humans() {
        // The original game walks the complete engine element array during AI initialization and
        // tests only human/PC identity and camp; it does not gate this bootstrap list
        // on whether the element is active. Authored rescue PCs commonly
        // begin inactive but must already occur in every applicable Enemy
        // detectable list so activation does not alter list identity/order.
        match entity {
            Entity::Pc(pc) => {
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: true,
                    is_soldier: false,
                    camp: pc.pc.cached_camp,
                });
            }
            Entity::Soldier(s) => {
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: false,
                    is_soldier: true,
                    camp: s.soldier.cached_camp,
                });
            }
            Entity::Civilian(c) => {
                // Civilians are tracked in the snapshot so the friendship
                // filter below can consider them, but the non-civilian
                // guard and the per-self filters in `add_detectable`
                // (Good/Evil branches) end up excluding every civilian
                // from every NPC's enemy list anyway.
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: false,
                    is_soldier: false,
                    camp: c.civilian.cached_camp,
                });
            }
            _ => {}
        }
    }
    out
}

/// Build this NPC's initial `detectable_lists[Enemy]` from a
/// [`PotentialDetectable`] snapshot.
///
/// Applies the combined filter of the enemy/friendly per-NPC init
/// (the outer loop over all humans, skipping friends and civilians in
/// the enemy case; adding PCs and opposing soldiers in the friendly
/// case) and then the per-self-type filter in `add_detectable`.
/// The net result for each self class:
///
/// - Royalist soldier (Merry Man): detects Lacklandist soldiers.
/// - Lacklandist soldier: detects Royalist soldiers + PCs.
/// - Royalist civilian: detects PCs.
/// - Lacklandist civilian (hostile civ): detects PCs.
/// - Custom soldier: detects PCs and soldiers of every other allegiance.
#[cfg(test)]
pub(super) fn build_detectable_enemies_for(
    self_camp: Camp,
    self_is_civilian: bool,
    self_id: EntityId,
    snapshot: &[PotentialDetectable],
) -> Vec<Detectable> {
    build_detectable_enemies_for_with(
        &crate::diplomacy::DiplomacyState::default(),
        self_camp,
        self_is_civilian,
        self_id,
        snapshot,
    )
}

#[cfg(test)]
pub(super) fn build_detectable_enemies_for_with(
    diplomacy: &crate::diplomacy::DiplomacyState,
    self_camp: Camp,
    self_is_civilian: bool,
    self_id: EntityId,
    snapshot: &[PotentialDetectable],
) -> Vec<Detectable> {
    let mut out = Vec::new();
    for pd in snapshot {
        if pd.id == self_id {
            continue;
        }
        // Civilians are never added as detectables on any list (both
        // malignity and bonhomie init paths skip them via the kind
        // check / detectable-admission type filter).
        let pd_is_civilian = !pd.is_pc && !pd.is_soldier;
        if pd_is_civilian {
            continue;
        }
        let is_detectable = crate::ai_detectable_filter::should_add_enemy_detectable_with(
            diplomacy,
            self_camp,
            !self_is_civilian,
            pd.is_pc,
            pd.is_soldier,
            pd.camp,
        );
        if is_detectable {
            out.push(Detectable {
                element: Some(pd.id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: false,
                heard_last_frame: false,
                seen_now: false,
                shadow_seen_now: false,
                shadow_seen_last_frame: false,
                last_visibility: 0.0,
            });
        }
    }
    out
}

/// Preserve the original game's left-to-right patrol initialization evaluation.
///
/// The visibility operand precedes the member-state predicates, so an active
/// outdoor member can emit an authoritative LOS query even when it is not in
/// `STATE_DEFAULT` and will therefore not be admitted.
fn patrol_member_admitted(
    both_active: bool,
    detect_360: impl FnOnce() -> bool,
    ai_state: crate::ai::AiState,
    is_civilian: bool,
    is_able_to_fight: bool,
) -> bool {
    let detected = both_active && detect_360();
    detected && ai_state == crate::ai::AiState::Default && (is_civilian || is_able_to_fight)
}

/// Preserve patrol initialization's insertion-loop comparison exactly.
///
/// The original game advances past an existing member only while
/// `new_distance > existing_distance`.  Spelling the stopping condition as
/// `new_distance <= existing_distance` is not equivalent for unordered IEEE
/// values: a NaN distance stops the original-game loop immediately and is inserted at
/// that position.
fn patrol_distance_inserts_before(new_distance: f32, existing_distance: f32) -> bool {
    !(new_distance > existing_distance)
}

/// Preserve the look-there broadcast's positive, strict range admission.
///
/// The original game sends the look-there callback only within the squared radius.
/// Rewriting this as an early rejection with `distance² >= radius²` is not
/// equivalent for unordered IEEE values: a soldier with a NaN position must
/// not receive the call.
fn look_there_target_is_inside_radius(distance_squared: f32, radius_squared: f32) -> bool {
    distance_squared < radius_squared
}

/// The original game's patrol initialization uses actor positions for sorting and
/// formation, but its admission ray calls the actor overload of
/// omnidirectional detection. That variant builds both endpoints from the
/// actors' literal stored 3-D positions. In particular, a door-passing member
/// must not substitute the committed gate-side AI position here.
fn patrol_member_visible_from_raw_world(
    chief_world: crate::coordinates::WorldPoint3D,
    chief_is_rider: bool,
    chief_view_radius: u16,
    chief_in_building: bool,
    member_world: crate::coordinates::WorldPoint3D,
    member_posture: crate::element::Posture,
    member_is_rider: bool,
    member_direction: i16,
    member_in_building: bool,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    let mut chief_eye = chief_world;
    chief_eye.z +=
        crate::stealth::eye_z_for_posture(crate::element::Posture::Upright, chief_is_rider);
    let member_detection = crate::stealth::detection_point_world(
        member_world,
        member_posture,
        member_direction,
        member_is_rider,
    );
    crate::ai_enemy::soldier_detects_detection_point_360(
        chief_eye,
        chief_view_radius,
        chief_in_building,
        member_detection,
        member_in_building,
        obstacles,
    )
}

/// Preserve patrol refresh's missed-member `&&` evaluation order.
///
/// The original game checks all-around detection before help eligibility and the AI
/// state check. A visible civilian therefore emits the LOS query even though
/// its default ability-to-help check rejects re-acquisition.
fn missed_patrol_member_reacquired(
    both_active: bool,
    detect_360: impl FnOnce() -> bool,
    is_able_to_help: bool,
    ai_state: crate::ai::AiState,
) -> bool {
    let detected = both_active && detect_360();
    detected && is_able_to_help && ai_state == crate::ai::AiState::Default
}

/// The original game's nearby-civilian panic asks every active outdoor civilian whether
/// it detects the source. Dead and unconscious civilians are not rejected
/// before omnidirectional detection, so they can still emit its LOS query.
fn nearby_panic_civilian_reaches_visibility(active: bool, in_building: bool) -> bool {
    active && !in_building
}

/// Original's money-brawl inline panic sweep uses the civilian's 180-degree
/// detector. Keep LOS lazy so actors outside the forward cone do not emit an
/// obstacle query. The shared nearby-civilian panic callback must not use
/// this helper: its source implementation explicitly uses 360 degrees.
#[cfg(test)]
fn brawl_panic_civilian_detects_source(
    viewer: crate::ai::Position,
    viewer_direction: u16,
    source: crate::ai::Position,
    sq_view_radius: f32,
    los_clear: impl FnOnce() -> bool,
) -> bool {
    crate::ai_enemy::detects_position_180_raw(viewer, viewer_direction, source, sq_view_radius)
        && los_clear()
}

#[cfg(test)]
fn nearby_panic_civilian_detects_source(
    use_180_degree_detection: bool,
    viewer: crate::ai::Position,
    viewer_direction: u16,
    source: crate::ai::Position,
    sq_view_radius: f32,
    los_clear: impl FnOnce() -> bool,
) -> bool {
    if use_180_degree_detection {
        brawl_panic_civilian_detects_source(
            viewer,
            viewer_direction,
            source,
            sq_view_radius,
            los_clear,
        )
    } else {
        // Shared NearbyCiviliansPanic is explicitly 360 degrees in Original.
        los_clear()
    }
}

/// Whether the installed actor order owns the sprite's exact completion
/// boundary. A newly installed order can temporarily coexist with the prior
/// sprite action and must not inherit that action's terminal frame/counter.
fn installed_animation_has_reached_action_done(
    concrete_animation: crate::order::OrderType,
    sprite: &crate::sprite::Sprite,
) -> bool {
    sprite.last_action == concrete_animation
        && (sprite.current_frame > sprite.action_done_frame
            || (sprite.current_frame == sprite.action_done_frame
                && sprite.frame_count >= sprite.action_done_counter))
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn potential_detectables_include_inactive_authored_pcs() {
        let mut engine = EngineInner::new();
        let add_pc = |engine: &mut EngineInner, active, camp| {
            engine.add_test_entity(Entity::Pc(crate::element::ActorPc {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::ActorPc;
                    initial_element.active = active;
                    initial_element
                },
                actor: Default::default(),
                human: Default::default(),
                pc: crate::element::PcData {
                    cached_camp: camp,
                    ..Default::default()
                },
            }))
        };
        let inactive = add_pc(&mut engine, false, Camp::Custom(7));
        let active = add_pc(&mut engine, true, Camp::Custom(8));

        let candidates = build_potential_detectables(&engine)
            .into_iter()
            .map(|candidate| (candidate.id, candidate.camp))
            .collect::<Vec<_>>();

        assert_eq!(
            candidates,
            vec![(inactive, Camp::Custom(7)), (active, Camp::Custom(8))]
        );
    }

    #[test]
    fn patrol_distance_insertion_preserves_unordered_and_tie_semantics() {
        assert!(patrol_distance_inserts_before(4.0, 4.0));
        assert!(patrol_distance_inserts_before(f32::NAN, 4.0));
        assert!(patrol_distance_inserts_before(4.0, f32::NAN));
        assert!(!patrol_distance_inserts_before(5.0, 4.0));

        let mut sorted = vec![1.0, 3.0];
        for distance in [2.0, f32::NAN] {
            let insert_at = sorted
                .iter()
                .position(|&existing| patrol_distance_inserts_before(distance, existing))
                .unwrap_or(sorted.len());
            sorted.insert(insert_at, distance);
        }
        assert!(sorted[0].is_nan());
        assert_eq!(&sorted[1..], &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn look_there_range_admission_preserves_nan_and_boundary_semantics() {
        let radius_squared = 100.0;

        assert!(!look_there_target_is_inside_radius(
            f32::NAN,
            radius_squared
        ));
        assert!(!look_there_target_is_inside_radius(
            radius_squared,
            radius_squared
        ));
        assert!(look_there_target_is_inside_radius(
            radius_squared - 1.0,
            radius_squared
        ));
    }

    #[test]
    fn patrol_visibility_precedes_member_admission_predicates() {
        let calls = std::cell::Cell::new(0);
        let admitted = patrol_member_admitted(
            true,
            || {
                calls.set(calls.get() + 1);
                true
            },
            crate::ai::AiState::Attacking,
            false,
            false,
        );
        assert!(!admitted);
        assert_eq!(calls.get(), 1, "visibility must run before state rejection");

        let admitted = patrol_member_admitted(
            false,
            || {
                calls.set(calls.get() + 1);
                true
            },
            crate::ai::AiState::Default,
            false,
            true,
        );
        assert!(!admitted);
        assert_eq!(calls.get(), 1, "inactive actors return before LOS");
    }

    #[test]
    fn patrol_visibility_uses_literal_world_position_during_door_pass() {
        let chief_world = crate::coordinates::WorldPoint3D::new(1033.5859, 2061.8677, 25.10078);
        let member_world = crate::coordinates::WorldPoint3D::new(1021.8682, 2079.0342, 0.4859238);

        crate::sight_obstacle::begin_parity_visibility_capture();
        assert!(patrol_member_visible_from_raw_world(
            chief_world,
            false,
            400,
            false,
            member_world,
            crate::element::Posture::Upright,
            false,
            1,
            false,
            crate::sight_obstacle::ObstacleList::empty(),
        ));
        let queries = crate::sight_obstacle::take_parity_visibility_capture();
        assert_eq!(queries.len(), 1);
        let mut expected_origin = chief_world;
        expected_origin.z += 45.0;
        let mut expected_destination = member_world;
        expected_destination.z += 45.0;
        assert_eq!(
            queries[0].origin,
            [expected_origin.x, expected_origin.y, expected_origin.z]
        );
        assert_eq!(
            queries[0].destination,
            [
                expected_destination.x,
                expected_destination.y,
                expected_destination.z,
            ]
        );
        assert_ne!(
            queries[0].destination[1], 2060.4858,
            "the patrol visibility ray must not use the door's gate-side AI position"
        );
    }

    #[test]
    fn missed_patrol_visibility_uses_literal_world_position_during_door_pass() {
        let chief_world = crate::coordinates::WorldPoint3D::new(1033.5859, 2061.8677, 25.10078);
        // Soldier 48's literal position at the chief's frame-34864 owner
        // boundary. The AI Position(actor) helper instead reports the door's
        // committed gate side at map y=2060.
        let member_world = crate::coordinates::WorldPoint3D::new(1022.0, 2074.301, 7.101156);

        crate::sight_obstacle::begin_parity_visibility_capture();
        assert!(patrol_member_visible_from_raw_world(
            chief_world,
            false,
            400,
            false,
            member_world,
            crate::element::Posture::Upright,
            false,
            0,
            false,
            crate::sight_obstacle::ObstacleList::empty(),
        ));
        let queries = crate::sight_obstacle::take_parity_visibility_capture();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].destination, [1022.0, 2074.301, 52.101_16]);
        assert_ne!(
            queries[0].destination[1], 2067.1012,
            "missed-member reacquisition must not use the door's gate-side AI position"
        );
    }

    #[test]
    fn missed_patrol_visibility_precedes_ability_and_state_predicates() {
        let calls = std::cell::Cell::new(0);
        let reacquired = missed_patrol_member_reacquired(
            true,
            || {
                calls.set(calls.get() + 1);
                true
            },
            false,
            crate::ai::AiState::Attacking,
        );
        assert!(!reacquired);
        assert_eq!(
            calls.get(),
            1,
            "visibility must run before ability rejection"
        );

        let reacquired = missed_patrol_member_reacquired(
            false,
            || {
                calls.set(calls.get() + 1);
                true
            },
            true,
            crate::ai::AiState::Default,
        );
        assert!(!reacquired);
        assert_eq!(calls.get(), 1, "inactive actors return before LOS");
    }

    #[test]
    fn nearby_panic_uses_active_outdoor_gate_without_life_filter() {
        // A dead body's element can remain active outdoors. Life and
        // consciousness are intentionally absent from Original's gate.
        let mut civilian = crate::element::ActorCivilian {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            civilian: Default::default(),
        };
        civilian.npc.life_points = 0;
        civilian.human.unconscious = true;

        assert!(nearby_panic_civilian_reaches_visibility(
            civilian.element.active,
            false,
        ));
        assert!(!nearby_panic_civilian_reaches_visibility(false, false));
        assert!(!nearby_panic_civilian_reaches_visibility(true, true));
    }

    #[test]
    fn nearby_panic_uses_civilian_forward_half_plane_and_los() {
        use std::cell::Cell;

        let viewer = crate::ai::Position::default();
        let position = |x, y| crate::ai::Position {
            x,
            y,
            ..Default::default()
        };
        let los_calls = Cell::new(0);
        let clear_los = || {
            los_calls.set(los_calls.get() + 1);
            true
        };

        // Direction 0 faces north (-Y). Ahead and either 180-degree boundary
        // are accepted; directly behind is rejected without consulting LOS.
        assert!(brawl_panic_civilian_detects_source(
            viewer,
            0,
            position(0.0, -100.0),
            40_000.0,
            clear_los,
        ));
        assert!(brawl_panic_civilian_detects_source(
            viewer,
            0,
            position(100.0, 0.0),
            40_000.0,
            clear_los,
        ));
        assert!(brawl_panic_civilian_detects_source(
            viewer,
            0,
            position(-100.0, 0.0),
            40_000.0,
            clear_los,
        ));
        assert!(!brawl_panic_civilian_detects_source(
            viewer,
            0,
            position(0.0, 100.0),
            40_000.0,
            clear_los,
        ));
        assert_eq!(los_calls.get(), 3);

        // An actor in front still fails when opaque sight obstacles block it.
        assert!(!brawl_panic_civilian_detects_source(
            viewer,
            0,
            position(0.0, -100.0),
            40_000.0,
            || false,
        ));
    }

    #[test]
    fn generic_nearby_panic_keeps_360_degree_detection() {
        let viewer = crate::ai::Position::default();
        let behind = crate::ai::Position {
            y: 100.0,
            ..Default::default()
        };

        assert!(nearby_panic_civilian_detects_source(
            false,
            viewer,
            0,
            behind,
            40_000.0,
            || true,
        ));
        assert!(!nearby_panic_civilian_detects_source(
            true,
            viewer,
            0,
            behind,
            40_000.0,
            || true,
        ));
    }

    #[test]
    fn action_done_projection_requires_sprite_to_match_installed_animation() {
        use crate::element::ActionState;
        use crate::order::OrderType as OT;

        let sprite = crate::sprite::Sprite {
            last_action: OT::TransitionRunningAlertedWaitingAlerted,
            current_frame: 5,
            frame_count: 1,
            action_done_frame: 5,
            action_done_counter: 1,
            ..Default::default()
        };
        let resolved = super::super::animation::soldier_movement_animation(
            OT::TransitionRunningUprightWaitingUpright,
            true,
            ActionState::Waiting,
        );
        assert!(installed_animation_has_reached_action_done(
            resolved, &sprite,
        ));

        let past_done = crate::sprite::Sprite {
            current_frame: 6,
            ..sprite.clone()
        };
        assert!(installed_animation_has_reached_action_done(
            resolved, &past_done,
        ));

        let before_done = crate::sprite::Sprite {
            current_frame: 4,
            ..sprite.clone()
        };
        assert!(!installed_animation_has_reached_action_done(
            resolved,
            &before_done,
        ));
        let unrelated_prior = super::super::animation::soldier_movement_animation(
            OT::TransitionWalkingUprightWaitingUpright,
            true,
            ActionState::Waiting,
        );
        assert!(
            !installed_animation_has_reached_action_done(unrelated_prior, &sprite),
            "a newly installed transition must not inherit the prior sprite's terminal frame"
        );
    }

    fn lift_grid(
        lift_type: crate::sector::LiftType,
        doors: &[crate::gate::Door],
    ) -> crate::fast_find_grid::FastFindGrid {
        let mut grid = crate::fast_find_grid::FastFindGrid::new();
        let sector_number = crate::sector::SectorNumber::new(42);
        let level = std::sync::Arc::make_mut(&mut grid.level);
        level.door_projection_infos = doors
            .iter()
            .map(|door| crate::fast_find_grid::DoorProjectionInfo {
                point_in: door.point_in,
                point_out: door.point_out,
                sector_out: door.sector_out,
                sector_out_index: door.sector_out_index,
                layer_out: door.layer_out,
            })
            .collect();
        level.sector_number_map.insert(sector_number, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number,
            door_index: None,
            lift_type: Some(lift_type),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: doors
                .iter()
                .enumerate()
                .filter(|(_, door)| {
                    door.sector_in == sector_number || door.sector_out == sector_number
                })
                .map(|(index, _)| {
                    crate::gate::DoorIndex::new(index as u32).expect("valid door index")
                })
                .collect(),
            underlying_sector: None,
        });
        grid
    }

    fn lift_doors() -> Vec<crate::gate::Door> {
        vec![
            crate::gate::Door {
                door_type: crate::gate::DoorType::LiftLow,
                sector_in: crate::sector::SectorNumber::new(42),
                sector_out: crate::sector::SectorNumber::new(5),
                sector_out_index: crate::fast_find_grid::SectorIndex::new(5),
                point_out: MapPoint::new(10.0, 20.0),
                layer_out: 1,
                ..Default::default()
            },
            crate::gate::Door {
                door_type: crate::gate::DoorType::LiftHigh,
                sector_in: crate::sector::SectorNumber::new(42),
                sector_out: crate::sector::SectorNumber::new(8),
                sector_out_index: crate::fast_find_grid::SectorIndex::new(8),
                point_out: MapPoint::new(30.0, 40.0),
                layer_out: 3,
                ..Default::default()
            },
        ]
    }

    #[test]
    fn lift_approach_uses_high_entry_only_from_the_high_layer() {
        let doors = lift_doors();
        let grid = lift_grid(crate::sector::LiftType::Ladder, &doors);
        let sector = crate::position_interface::SectorHandle::new(42).unwrap();
        let target = crate::ai::Position {
            sector: Some(sector),
            ..crate::ai::Position::default()
        };

        // High/low is decided by point_out screen-Y (smallest Y = high door),
        // never by the authored door-type tags. In this fixture the door at
        // (10, 20) / layer 1 is therefore the high door even though it is
        // tagged LiftLow.
        let high = crate::ai::AiContext::enemy_lift_approach_for_position(&grid, target, Some(1))
            .expect("target is in a lift")
            .expect("ladder has an approach entry");
        assert_eq!((high.x, high.y, high.level), (10.0, 20.0, 1));
        assert_eq!(high.sector.map(u16::from), Some(5));
        assert_eq!(
            high.sector.and_then(|sector| sector.arena_index()),
            crate::fast_find_grid::SectorIndex::new(5)
        );

        // Every layer other than the high door's layer falls back to the low
        // entry, including layers matching neither door.
        for attacker_layer in [2, 3] {
            let low = crate::ai::AiContext::enemy_lift_approach_for_position(
                &grid,
                target,
                Some(attacker_layer),
            )
            .expect("target is in a lift")
            .expect("ladder has an approach entry");
            assert_eq!((low.x, low.y, low.level), (30.0, 40.0, 3));
            assert_eq!(low.sector.map(u16::from), Some(8));
        }
    }

    #[test]
    fn lift_approach_prefers_exact_arena_over_duplicate_public_sector() {
        let mut doors = lift_doors();
        let mut second_high = doors[0].clone();
        second_high.point_out = MapPoint::new(100.0, 120.0);
        second_high.sector_out = crate::sector::SectorNumber::new(15);
        second_high.sector_out_index = crate::fast_find_grid::SectorIndex::new(15);
        let mut second_low = doors[1].clone();
        second_low.point_out = MapPoint::new(130.0, 300.0);
        second_low.sector_out = crate::sector::SectorNumber::new(18);
        second_low.sector_out_index = crate::fast_find_grid::SectorIndex::new(18);
        doors.extend([second_high, second_low]);

        let mut grid = lift_grid(crate::sector::LiftType::Ladder, &doors);
        let level = std::sync::Arc::make_mut(&mut grid.level);
        level.sectors[0].gate_indices = vec![
            crate::gate::DoorIndex::new(0).expect("valid door index"),
            crate::gate::DoorIndex::new(1).expect("valid door index"),
        ];
        let mut duplicate = level.sectors[0].clone();
        duplicate.gate_indices = vec![
            crate::gate::DoorIndex::new(2).expect("valid door index"),
            crate::gate::DoorIndex::new(3).expect("valid door index"),
        ];
        level.sectors.push(duplicate);
        level
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(42), 1);

        let public = crate::position_interface::SectorHandle::new(42).unwrap();
        let exact = public.with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap());
        let exact_entry = crate::ai::AiContext::enemy_lift_approach_for_position(
            &grid,
            crate::ai::Position {
                sector: Some(exact),
                ..crate::ai::Position::default()
            },
            Some(1),
        )
        .expect("exact target is a lift")
        .expect("exact target has an entry");
        assert_eq!((exact_entry.x, exact_entry.y), (10.0, 20.0));
        assert_eq!(
            exact_entry.sector.and_then(|sector| sector.arena_index()),
            crate::fast_find_grid::SectorIndex::new(5)
        );

        let numeric_entry = crate::ai::AiContext::enemy_lift_approach_for_position(
            &grid,
            crate::ai::Position {
                sector: Some(public),
                ..crate::ai::Position::default()
            },
            Some(1),
        )
        .expect("number-only target is a lift")
        .expect("number-only target has an entry");
        assert_eq!((numeric_entry.x, numeric_entry.y), (100.0, 120.0));
        assert_eq!(
            numeric_entry.sector.and_then(|sector| sector.arena_index()),
            crate::fast_find_grid::SectorIndex::new(15)
        );
    }

    #[test]
    fn lift_approach_uses_point_out_geometry_when_both_doors_are_tagged_low() {
        let mut doors = lift_doors();
        doors[1].door_type = crate::gate::DoorType::LiftLow;
        let grid = lift_grid(crate::sector::LiftType::Ladder, &doors);
        let target = crate::ai::Position {
            sector: crate::position_interface::SectorHandle::new(42),
            ..crate::ai::Position::default()
        };

        let high = crate::ai::AiContext::enemy_lift_approach_for_position(&grid, target, Some(3))
            .expect("target is in a lift")
            .expect("ladder has an approach entry");
        assert_eq!((high.x, high.y, high.level), (30.0, 40.0, 3));

        let low = crate::ai::AiContext::enemy_lift_approach_for_position(&grid, target, Some(1))
            .expect("target is in a lift")
            .expect("ladder has an approach entry");
        assert_eq!((low.x, low.y, low.level), (10.0, 20.0, 1));
    }

    #[test]
    fn stairs_are_still_lifts_but_have_no_entry_detour() {
        let grid = lift_grid(crate::sector::LiftType::Stairs, &[]);
        let target = crate::ai::Position {
            sector: crate::position_interface::SectorHandle::new(42),
            ..crate::ai::Position::default()
        };
        assert_eq!(
            crate::ai::AiContext::enemy_lift_approach_for_position(&grid, target, Some(3)),
            Some(None)
        );
    }

    #[test]
    fn generic_owner_zero_context_may_lack_an_ai_entity_view() {
        let views = crate::ai_entity_view::shared_entity_views(
            crate::ai_entity_view::AiEntityViewMap::new(),
        );
        assert_eq!(context_original_creation_order(0, &views), None);
    }

    #[test]
    #[should_panic(expected = "has no authored entry doors")]
    fn non_stairs_lift_does_not_fake_a_missing_entry() {
        let grid = lift_grid(crate::sector::LiftType::Ladder, &[]);
        let target = crate::ai::Position {
            sector: crate::position_interface::SectorHandle::new(42),
            ..crate::ai::Position::default()
        };
        let _ = crate::ai::AiContext::enemy_lift_approach_for_position(&grid, target, Some(3));
    }

    #[test]
    fn detectable_initialization_preserves_creation_order_for_mixed_enemy_kinds() {
        let self_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let snapshot = vec![
            PotentialDetectable {
                id: EntityId::Soldier(crate::entity_id::SoldierId(7)),
                is_pc: false,
                is_soldier: true,
                camp: Camp::Royalists,
            },
            PotentialDetectable {
                id: EntityId::Pc(crate::entity_id::PcId(3)),
                is_pc: true,
                is_soldier: false,
                camp: Camp::Royalists,
            },
            PotentialDetectable {
                id: EntityId::Soldier(crate::entity_id::SoldierId(9)),
                is_pc: false,
                is_soldier: true,
                camp: Camp::Lacklandists,
            },
        ];

        let detectables =
            build_detectable_enemies_for(Camp::Lacklandists, false, self_id, &snapshot);
        assert_eq!(
            detectables
                .iter()
                .map(|detectable| detectable.element.unwrap().index())
                .collect::<Vec<_>>(),
            vec![7, 3]
        );
    }

    #[test]
    fn custom_retinue_detects_hostile_champions_but_not_its_own() {
        let self_id = EntityId::Soldier(crate::entity_id::SoldierId(5));
        let allied_champion = EntityId::Pc(crate::entity_id::PcId(1));
        let hostile_champion = EntityId::Pc(crate::entity_id::PcId(2));
        let snapshot = vec![
            PotentialDetectable {
                id: allied_champion,
                is_pc: true,
                is_soldier: false,
                camp: Camp::Custom(2),
            },
            PotentialDetectable {
                id: hostile_champion,
                is_pc: true,
                is_soldier: false,
                camp: Camp::Custom(3),
            },
        ];

        let detectables = build_detectable_enemies_for(Camp::Custom(2), false, self_id, &snapshot);

        assert_eq!(
            detectables
                .iter()
                .map(|detectable| detectable.element)
                .collect::<Vec<_>>(),
            vec![Some(hostile_champion)]
        );
    }

    #[test]
    fn custom_civilian_detects_hostile_champion_but_not_allied_champion() {
        let self_id = EntityId::Civilian(crate::entity_id::CivilianId(5));
        let allied_champion = EntityId::Pc(crate::entity_id::PcId(1));
        let hostile_champion = EntityId::Pc(crate::entity_id::PcId(2));
        let snapshot = vec![
            PotentialDetectable {
                id: allied_champion,
                is_pc: true,
                is_soldier: false,
                camp: Camp::Custom(2),
            },
            PotentialDetectable {
                id: hostile_champion,
                is_pc: true,
                is_soldier: false,
                camp: Camp::Custom(3),
            },
        ];

        let detectables = build_detectable_enemies_for(Camp::Custom(2), true, self_id, &snapshot);

        assert_eq!(
            detectables
                .iter()
                .map(|detectable| detectable.element)
                .collect::<Vec<_>>(),
            vec![Some(hostile_champion)]
        );
    }
}

/// Per-segment obstacle check against a hiking path's waypoints.
///
/// Each adjacent pair of waypoints that stays on the same sector/level
/// is tested for both raw motion reachability and thick-mobile
/// straight-movement authorization using the NPC's move box.  Returns
/// `true` when every applicable segment passes both checks.
///
/// Uses the "set `path_is_ok = false`, continue the loop" idiom so every
/// bad segment is logged rather than only the first. The debug-overlay
/// side effect (bad path visualisation) is dev-only and not yet
/// implemented — log emission is the equivalent.
fn test_hiking_path_fine(
    grid: &crate::fast_find_grid::FastFindGrid,
    waypoints: &[crate::level_data::RawWaypoint],
    move_box: &crate::coordinates::MoveBox,
) -> bool {
    if waypoints.len() < 2 {
        return true;
    }
    let mut ok = true;
    let mut prev = &waypoints[0];
    for (i, wp) in waypoints.iter().enumerate().skip(1) {
        if wp.level == prev.level && wp.sector == prev.sector {
            let p1 = MapPoint::new(prev.x as f32, prev.y as f32);
            let p2 = MapPoint::new(wp.x as f32, wp.y as f32);
            if !grid.is_reachable_thin(p1, p2, wp.level) {
                tracing::debug!(
                    wp_idx = i,
                    p1 = ?p1,
                    p2 = ?p2,
                    layer = wp.level,
                    "TestIfPathIsFine: segment not reachable (obstacle)"
                );
                ok = false;
            }
            // Split the authorized check into its two components
            // (destination-box auth check, then thick-corridor check) so
            // diagnostics pinpoint which half of the test rejects.
            let dest_box = move_box.translated(p2);
            if !grid.is_position_authorized(&dest_box, wp.level) {
                tracing::debug!(
                    wp_idx = i,
                    p1 = ?p1,
                    p2 = ?p2,
                    layer = wp.level,
                    ?dest_box,
                    "TestIfPathIsFine: destination move-box overlaps obstacle \
                     (IsPositionAutorized)"
                );
                ok = false;
            }
            let hd =
                crate::coordinates::MoveBoxHalfDiagonal::new(move_box.x_max(), move_box.y_max());
            if !grid.is_reachable_thick(p1, p2, wp.level, hd) {
                tracing::debug!(
                    wp_idx = i,
                    p1 = ?p1,
                    p2 = ?p2,
                    layer = wp.level,
                    ?hd,
                    "TestIfPathIsFine: thick-corridor too close to obstacle \
                     (IsReachableThick)"
                );
                ok = false;
            }
        }
        prev = wp;
    }
    ok
}

/// Whether the actor's selected sequence command is PassDoor, matching
/// the actor door-passing check.
pub(super) fn selected_actor_is_passing_door(
    sequence_manager: &crate::sequence::SequenceManager,
    entity_id: EntityId,
) -> bool {
    sequence_manager
        .current_element_for_actor(entity_id)
        .and_then(|(sequence_id, element_index)| {
            sequence_manager.get_element(sequence_id, element_index)
        })
        .is_some_and(|element| element.command == crate::element::Command::PassDoor)
}

/// Return the gate and direction carried by the selected PassDoor movement
/// element. AI position reads these fields from the
/// sequence element itself; unlike AI destination forecasting, it does not
/// consult the sprite position interface's live door pointer.
fn selected_pass_door_movement(
    sequence_manager: &crate::sequence::SequenceManager,
    entity_id: EntityId,
) -> Option<(crate::gate::DoorIndex, i16)> {
    let element = sequence_manager
        .current_element_for_actor(entity_id)
        .and_then(|(sequence_id, element_index)| {
            sequence_manager.get_element(sequence_id, element_index)
        })?;
    if element.command != crate::element::Command::PassDoor {
        return None;
    }
    let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &element.data
    else {
        panic!("selected PassDoor for {entity_id:?} is not a movement element")
    };
    Some((
        gate_id.unwrap_or_else(|| panic!("selected PassDoor for {entity_id:?} has no gate")),
        *direction,
    ))
}

/// Extract a [`ForecastInput`] from an entity for destination prediction.
///
/// Returns `None` for entities without actor data (e.g. objects, FX).
pub(super) fn extract_forecast_input(
    entity: &Entity,
    is_passing_door: bool,
) -> Option<crate::ai::ForecastInput> {
    let elem = entity.element_data();
    let actor = entity.actor_data()?;
    // AI destination forecasting gates the serialized door pointer on
    // door-passing state, then uses the independent direct-passage latch
    // for the destination side. A legacy save restores the selected PassDoor
    // element and both serialized actor fields even though Rust's runtime-only
    // ActiveDoorPass choreography is not reconstructed.
    let live_door = entity.position_iface().get_door();
    let door_pass = is_passing_door
        .then_some(live_door)
        .flatten()
        .map(|door| (door, actor.passing_door_directly));
    let forecasted_z = entity.position_iface().get_forecasted_movement().z;
    Some(crate::ai::ForecastInput {
        position_map_x: elem.position_map().x,
        position_map_y: elem.position_map().y,
        sector: elem.sector().map(u16::from).unwrap_or(0),
        sector_handle: elem.sector(),
        layer: elem.layer(),
        direction: elem.direction() as u16,
        forecasted_movement_z: forecasted_z,
        door_pass,
        passing_door_directly: actor.passing_door_directly,
    })
}

/// Extract the live AI destination-forecast input with the complete
/// exact sector identity that the original game reads from the target actor.
///
/// Legacy saves can leave the entity's compact sector handle number-only.
/// AI-facing forecasts must restore that omitted identity from the loaded
/// arena at the same boundary as other `Position(actor)` snapshots; otherwise
/// a perfectly valid forecast becomes an unroutable mixed exact/number-only
/// movement destination.
pub(super) fn extract_exact_forecast_input(
    engine: &EngineInner,
    entity: &Entity,
    is_passing_door: bool,
) -> Option<crate::ai::ForecastInput> {
    let mut input = extract_forecast_input(entity, is_passing_door)?;
    input.sector_handle = ai_view_position_sector(engine, entity.element_data());
    Some(input)
}

impl EngineInner {
    pub(in crate::engine) fn ai_bored_time(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
    ) -> u16 {
        let entity = self.expect_entity(owner, "bored timer owner");
        let (rank, pride) = entity
            .enemy_ai()
            .map(|ai| (ai.soldier_profile_rank, ai.soldier_profile_pride))
            .unwrap_or((crate::profiles::ProfileRank::None, 0));
        entity
            .ai_controller()
            .expect("bored timer requires AI")
            .get_bored_time_for(sim, self.control.frame_counter, rank, pride)
    }

    /// Build a dispatch context from the selected observation, preserving the
    /// caller's frame and building-sector boundary rather than resampling them.
    pub(in crate::engine) fn ai_context_from_entity(
        &self,
        entity: &Entity,
        frame: u32,
        building_sector: Option<crate::position_interface::SectorHandle>,
        scratch: &SimScratch,
        assets: &LevelAssets,
    ) -> AiContext {
        build_ai_context_from_entity(
            entity,
            frame,
            building_sector,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.navigation.hiking_paths,
            &assets.navigation.hiking_waypoint_sectors,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
            self.ai_think_depth(),
        )
    }

    /// Resolve an NPC and its current building sector at the dispatch boundary.
    /// Split-borrow translators continue to use the entity-based primitive.
    #[track_caller]
    pub(in crate::engine) fn ai_context_for(
        &self,
        npc_id: EntityId,
        frame: u32,
        scratch: &SimScratch,
        assets: &LevelAssets,
    ) -> AiContext {
        let entity = self.expect_entity(npc_id, "building AI dispatch context");
        let building_sector = self.entity_building_sector(entity.element_data().sector());
        self.ai_context_from_entity(entity, frame, building_sector, scratch, assets)
    }
}

/// Build an [`AiContext`] from a generic [`Entity`] reference.
///
/// Extracts position, direction, posture, camp, building status, and
/// swordfighting flag from the live human opponent list so the AI think method
/// sees a consistent, non-stale snapshot each call.
///
/// Also threads the per-tick [`SharedAiEntityViews`] map into the
/// context so handlers can resolve arbitrary entity handles to live
/// position / state without a mutable engine borrow.  Callers grab
/// the map from [`SimScratch`], built by
/// [`EngineInner::build_sim_scratch`] before each dispatch pass.
pub(super) fn build_ai_context_from_entity(
    entity: &Entity,
    frame: u32,
    building_sector: Option<crate::position_interface::SectorHandle>,
    is_forest_level: bool,
    ambiance: crate::engine::types::Ambiance,
    standard_view_polygon_radius: u16,
    entity_views: &SharedAiEntityViews,
    sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
    fast_grid: &std::sync::Arc<crate::fast_find_grid::FastFindGrid>,
    hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
    hiking_waypoint_sectors: &Option<
        std::sync::Arc<Vec<Vec<crate::position_interface::SectorHandle>>>,
    >,
    all_soldier_handles: &std::sync::Arc<Vec<u32>>,
    difficulty: crate::player_profile::DifficultyLevel,
    think_depth: u8,
) -> AiContext {
    let elem = entity.element_data();
    let actor = entity.actor_data();
    let original_creation_order =
        context_original_creation_order(elem.index_in_elements_list as u32, entity_views);
    // The actor's AI position uses the committed gate
    // side while the sprite interpolates along a door rail. The shared view
    // has already applied that override; raw sprite coordinates here made
    // self-relative AI geometry disagree with target lookups during PassDoor.
    let self_position = if actor.is_some_and(|actor| actor.active_door_pass.is_some()) {
        entity_views
            .get(&(elem.index_in_elements_list as u32))
            .unwrap_or_else(|| {
                panic!(
                    "door-passing AI owner {} is missing its required entity view",
                    elem.index_in_elements_list
                )
            })
            .position
    } else {
        crate::ai::Position {
            x: elem.position_map().x,
            y: elem.position_map().y,
            sector: elem.sector(),
            level: elem.layer(),
        }
    };
    build_ai_owner_scalars(
        entity,
        self_position,
        original_creation_order,
        frame,
        building_sector,
        is_forest_level,
        ambiance,
        standard_view_polygon_radius,
        entity_views,
        sight_obstacles,
        fast_grid,
        hiking_paths,
        hiking_waypoint_sectors,
        all_soldier_handles,
        difficulty,
        think_depth,
    )
}

fn build_ai_owner_scalars(
    entity: &Entity,
    self_position: crate::ai::Position,
    original_creation_order: Option<u32>,
    frame: u32,
    building_sector: Option<crate::position_interface::SectorHandle>,
    is_forest_level: bool,
    ambiance: crate::engine::types::Ambiance,
    standard_view_polygon_radius: u16,
    entity_views: &SharedAiEntityViews,
    sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
    fast_grid: &std::sync::Arc<crate::fast_find_grid::FastFindGrid>,
    hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
    hiking_waypoint_sectors: &Option<
        std::sync::Arc<Vec<Vec<crate::position_interface::SectorHandle>>>,
    >,
    all_soldier_handles: &std::sync::Arc<Vec<u32>>,
    difficulty: crate::player_profile::DifficultyLevel,
    think_depth: u8,
) -> AiContext {
    let elem = entity.element_data();
    let camp = entity.camp();
    let actor = entity.actor_data();
    // `is_swordfighting` is "opponents list is non-empty"; do not proxy
    // it through action_state.
    let is_swordfighting = entity
        .human_data()
        .map(|h| !h.opponents.is_empty())
        .unwrap_or(false);
    let move_box = if actor.is_some() {
        *entity.position_iface().get_move_box()
    } else {
        Default::default()
    };
    let remaining_arrows = entity
        .ai_actor_data()
        .map(|ai| ai.number_of_arrows)
        .unwrap_or(0);
    // `self_is_beggar` / `self_is_child` are civilian-type checks.
    // Non-civilian NPCs always read false (callers cast to civilian
    // first).
    let (self_is_beggar, self_is_child) = match entity {
        Entity::Civilian(c) => (
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar,
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Child,
        ),
        _ => (false, false),
    };
    // Soldier vs civilian — drives the soldier-only macro opcodes
    // (CMD_CHECK_4, CMD_LOOK_LEFT, CMD_BEND, CMD_PATROL_*) which error
    // on civilians.
    let self_is_soldier = entity.enemy_ai().is_some();
    // `self_is_rider` is the cached `SoldierData.rider` flag from the
    // soldier profile, set at level load.  Non-soldier NPCs are never
    // riders.
    let self_is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
    // `self_rank` / `self_pride` are the soldier's profile rank and
    // pride, used by the bored-time picker for longer officer/pride
    // bored intervals.  `ProfileRank::None` for non-soldiers makes the
    // officer check fall through.
    let (self_rank, self_pride) = entity
        .enemy_ai()
        .map(|ai| (ai.soldier_profile_rank, ai.soldier_profile_pride))
        .unwrap_or((crate::profiles::ProfileRank::None, 0));
    // Number of detectables of type Friend — the
    // `return_to_duty_common_stuff` guard uses this to decide whether
    // to clear the stashed detected body.
    let self_detectable_friend_count = entity
        .ai_actor_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::Friend as usize)
        })
        .map(|lst| lst.len() as u16)
        .unwrap_or(0);
    // Number of detectables of type MissedFriend — enemy
    // `return_to_duty` uses this to know whether to record the
    // abandoned checkpoint Charly in the missed-in-action list.
    let self_detectable_missed_friend_count = entity
        .ai_actor_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::MissedFriend as usize)
        })
        .map(|lst| lst.len() as u16)
        .unwrap_or(0);
    let self_seen_enemy_handles = entity
        .ai_actor_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::Enemy as usize)
        })
        .into_iter()
        .flatten()
        .filter(|detectable| detectable.seen_now)
        .filter_map(|detectable| detectable.element.map(|target| target.index()))
        .collect();
    // Actor animation selection reads the current order, not the
    // sprite's background animation. In particular, boredom can play a
    // WAITING_UPRIGHT_BORED sprite while the authoritative actor order remains
    // WAITING_UPRIGHT; movement's close-point shortcut must still recognize that
    // idle order and synchronously advance the patrol waypoint.
    //
    // `installed_order` is Rust's exact current-order pointer mirror. A null
    // pointer is the NonanimationEnd sentinel; sequence selection and the
    // visible sprite are not substitutes for an installed actor order.
    let self_animation = actor
        .and_then(|actor| actor.installed_order)
        .map(|order| order.order_type)
        .unwrap_or(crate::order::OrderType::NonanimationEnd);
    let self_action_state = actor.map(|a| a.action_state).unwrap_or_default();
    let concrete_self_animation = match entity {
        Entity::Soldier(soldier) => super::animation::soldier_movement_animation(
            self_animation,
            soldier
                .npc
                .ai_brain
                .enemy()
                .is_some_and(|enemy| enemy.attentive),
            self_action_state,
        ),
        _ => self_animation,
    };
    let self_animation_reached_action_done = installed_animation_has_reached_action_done(
        concrete_self_animation,
        &entity.element_data().sprite,
    );
    if archer_step_back_lifecycle_debug_matches(
        frame,
        original_creation_order,
        elem.index_in_elements_list as u32,
    ) {
        trace_archer_step_back_context(
            frame,
            original_creation_order,
            elem,
            actor,
            [self_animation, concrete_self_animation],
            self_action_state,
            self_animation_reached_action_done,
        );
    }
    tracing::trace!(
        target: "robin_engine::ai::goto",
        me = elem.index_in_elements_list,
        frame,
        ?self_animation,
        "build_ai_context: installed mpOrder animation"
    );
    // Only soldiers can be forced-attentive; civilians always read
    // `false`.  Threaded into AiContext so
    // `set_alert_status_with_flags` can apply the view-override from
    // inside shared `AiController` paths.
    let self_forced_attentive = entity
        .enemy_ai()
        .is_some_and(|enemy| enemy.forced_attentive);
    let self_view_radius = entity
        .ai_actor_data()
        .map(|npc| npc.view_radius as f32)
        .unwrap_or(standard_view_polygon_radius as f32);
    let self_eye = entity.compute_eyes_point(None);
    let self_eye_position = self_eye
        .map(|eye| {
            crate::coordinates::MapPoint::from_world_xyz(
                eye.x,
                eye.y,
                entity.element_data().position().z,
            )
        })
        .unwrap_or_else(|| elem.position_map());
    let self_eye_z = self_eye.map(|eye| eye.z).unwrap_or(elem.position().z);
    let self_upright_eye_world = entity
        .compute_eyes_point(Some(crate::element::Posture::Upright))
        .unwrap_or(elem.position());
    let self_stare_point = entity
        .ai_actor_data()
        .map(|npc| npc.stare_point)
        .unwrap_or_else(|| {
            crate::coordinates::GroundPoint::from_map_and_z(elem.position_map(), elem.position().z)
        });
    let self_view_direction = entity
        .ai_actor_data()
        .map(|npc| npc.view_direction)
        .unwrap_or_else(|| {
            let (x, y) = crate::ai_vision::sector_to_forward(elem.direction());
            [x, y]
        });
    let self_real_half_aperture = entity
        .ai_actor_data()
        .map(|npc| npc.real_half_aperture)
        .unwrap_or(crate::ai_vision::NORMAL_HALF_APERTURE);
    let self_eye_status = entity
        .ai_actor_data()
        .map(|npc| npc.eye_status)
        .unwrap_or_default();
    AiContext {
        think_depth,
        difficulty,
        original_creation_order,
        position: self_position,
        self_layer: elem.layer(),
        self_body_position_world: elem.position(),
        frame,
        direction: elem.direction() as u16,
        posture: elem.posture(),
        self_eye_position,
        self_eye_z,
        self_upright_eye_world,
        self_stare_point,
        self_view_direction,
        self_view_radius: self_view_radius as u16,
        self_real_half_aperture,
        self_eye_status,
        is_night_or_fog: matches!(
            ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        ),
        in_uninterruptible_command: false,
        // Every AI-side building test resolves the actor's *sector*: the
        // indoor early-outs, the 180°/360° detection short-circuits, and the
        // outdoor question gate all ask whether the current sector is a
        // building. A soldier standing on a door rail has no building sector
        // yet and must still behave as an outdoor actor, so the door-transit
        // branch (which only governs whether the view polygon is drawn) must
        // not leak into this flag.
        in_building: building_sector.is_some(),
        self_is_active: elem.active,
        building_sector,
        camp,
        is_swordfighting,
        enter_swordfight_pending: false,
        is_forest_level,
        move_box,
        remaining_arrows,
        sq_standard_view_radius: (standard_view_polygon_radius as f32)
            * (standard_view_polygon_radius as f32),
        sq_self_view_radius: self_view_radius * self_view_radius,
        elevation: if actor.is_some() {
            entity.position_iface().get_elevation()
        } else {
            elem.position().z
        },
        self_is_beggar,
        self_is_child,
        self_is_soldier,
        self_is_rider,
        self_action_state,
        self_rank,
        self_pride,
        self_life_points: entity.human_life_points(),
        self_max_life_points: entity.human_max_life_points(),
        self_is_dead: entity.is_dead(),
        self_is_unconscious: entity.is_unconscious(),
        self_detectable_friend_count,
        self_detectable_missed_friend_count,
        self_seen_enemy_handles,
        self_forced_attentive,
        self_animation_reached_action_done,
        self_animation,
        self_animation_motion_state: actor
            .map(|actor| actor.continuation.motion_state)
            .unwrap_or_default(),
        self_selected_element_is_default_wait: None,
        self_selected_element_priority: None,
        antagonist: None,
        entity_views: entity_views.clone(),
        sight_obstacles: sight_obstacles.clone(),
        view_radius_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        fast_grid: fast_grid.clone(),
        hiking_paths: hiking_paths.clone(),
        hiking_waypoint_sectors: hiking_waypoint_sectors.clone(),
        all_soldier_handles: all_soldier_handles.clone(),
    }
}

fn context_original_creation_order(
    entity_index: u32,
    entity_views: &SharedAiEntityViews,
) -> Option<u32> {
    entity_views
        .get(&entity_index)
        .map(|view| view.original_creation_order)
}

pub(super) struct AiPositionResolution {
    /// Target's own position after the door-first arm, before an optional
    /// carried-PC substitution.
    pub(super) target: crate::ai::Position,
    /// Final AI position for the entity.
    pub(super) effective: crate::ai::Position,
}

pub(super) fn resolve_ai_position_with(
    entities: &crate::entities::Entities,
    doors: &[crate::gate::Door],
    sequence_manager: &crate::sequence::SequenceManager,
    target_id: crate::element::EntityId,
    position_of: impl FnMut(crate::element::EntityId) -> crate::ai::Position,
) -> AiPositionResolution {
    let selected_door = selected_pass_door_movement(sequence_manager, target_id);
    resolve_ai_position_with_selected(entities, doors, target_id, selected_door, position_of)
}

/// Resolve AI position from an already-sampled selected PassDoor element.
/// Callers constructing multiple fields at one synchronous boundary use this
/// to avoid repeating the same sequence-manager lookup.
fn resolve_ai_position_with_selected(
    entities: &crate::entities::Entities,
    doors: &[crate::gate::Door],
    target_id: crate::element::EntityId,
    selected_door: Option<(crate::gate::DoorIndex, i16)>,
    mut position_of: impl FnMut(crate::element::EntityId) -> crate::ai::Position,
) -> AiPositionResolution {
    let target = entities
        .get(target_id)
        .unwrap_or_else(|| panic!("AI position target {target_id:?} disappeared"));
    if target.actor_data().is_some()
        && let Some((gate_id, direction)) = selected_door
    {
        let door = doors.get(usize::from(gate_id)).unwrap_or_else(|| {
            panic!(
                "AI position target {target_id:?} references missing door {}",
                gate_id
            )
        });
        let position = if direction != 0 {
            crate::ai::Position {
                x: door.point_in.x,
                y: door.point_in.y,
                sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_in))
                    .map(|handle| {
                        handle.with_arena_index(door.sector_in_index.unwrap_or_else(|| {
                            panic!(
                                "selected pass-door {} interior sector has no exact arena identity",
                                gate_id.0
                            )
                        }))
                    }),
                level: door.layer_in,
            }
        } else {
            crate::ai::Position {
                x: door.point_out.x,
                y: door.point_out.y,
                sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_out))
                    .map(|handle| {
                        handle.with_arena_index(door.sector_out_index.unwrap_or_else(|| {
                            panic!(
                                "selected pass-door {} exterior sector has no exact arena identity",
                                gate_id.0
                            )
                        }))
                    }),
                level: door.layer_out,
            }
        };
        return AiPositionResolution {
            target: position,
            effective: position,
        };
    }

    let target_position = position_of(target_id);
    let carrier_id = match target {
        Entity::Pc(pc) if pc.element.posture() == crate::element::Posture::OnShoulders => {
            Some(pc.human.carrier.unwrap_or_else(|| {
                panic!("on-shoulders PC {target_id:?} has no carrier for AI Position")
            }))
        }
        _ => None,
    };
    let carrier = carrier_id.map(&mut position_of);
    AiPositionResolution {
        target: target_position,
        effective: carrier.unwrap_or(target_position),
    }
}

pub(super) fn lookup_primary_target_position(
    engine: &EngineInner,
    target_id: crate::element::EntityId,
) -> Option<crate::ai::Position> {
    if target_id.index() == 0 {
        return None;
    }
    engine.world.entities.get(target_id)?;
    let resolved = resolve_ai_position_with(
        &engine.world.entities,
        engine.script_domains.interactables.doors.as_slice(),
        &engine.orders.sequence_manager,
        target_id,
        |id| {
            let element = engine
                .expect_entity(id, "AI metadata position owner")
                .element_data();
            crate::ai::Position {
                x: element.position_map().x,
                y: element.position_map().y,
                sector: ai_view_position_sector(engine, element),
                level: element.layer(),
            }
        },
    );
    Some(resolved.target)
}

/// Run the "avenger on the roof" wait-position lookup for the
/// evaluating NPC, if its `couldnt_reachpoint` flag is set.
///
/// The pre-dispatch wiring for
/// `get_avenger_on_the_roof_wait_position`.  The gate-chain walker
/// itself lives in [`crate::gate::compute_avenger_wait_position`];
/// this helper extracts the per-actor state the walker needs from
/// the live entity store.
///
/// Returns `None` when any input is missing or the walker finds no
/// blocking gate — the caller should record no entry in
/// `tick.avenger_on_roof_wait_positions` for that target in that case.
pub(super) fn precompute_avenger_on_roof_wait_position(
    entities: &crate::entities::Entities,
    doors: &[crate::gate::Door],
    sequence_manager: &crate::sequence::SequenceManager,
    me_id: impl Into<crate::element::EntityId>,
    target_id: impl Into<crate::element::EntityId>,
    position_sector: impl Fn(
        &crate::element::ElementData,
    ) -> Option<crate::position_interface::SectorHandle>,
    building_is_authorized: &impl Fn(crate::sector::SectorNumber) -> bool,
    sector_lift_type: &impl Fn(crate::sector::SectorNumber) -> Option<crate::sector::LiftType>,
) -> Option<crate::ai::Position> {
    let me_id = me_id.into();
    let target_id = target_id.into();
    if doors.is_empty() {
        return None;
    }
    let me = entities.get(me_id)?;
    let target = entities.get(target_id)?;

    // The original game's rooftop-avenger wait calculation queries AI positions
    // for both actors. That
    // position commits a selected PassDoor actor to the destination gate
    // endpoint before falling back to its sprite coordinates
    // during door traversal.
    let resolve_position = |id| {
        resolve_ai_position_with(entities, doors, sequence_manager, id, |position_id| {
            let element = entities
                .get(position_id)
                .unwrap_or_else(|| panic!("roof-wait position owner {position_id:?} disappeared"))
                .element_data();
            crate::ai::Position {
                x: element.position_map().x,
                y: element.position_map().y,
                // The original game's element position retains the exact sector reference.
                // Restored actors may still carry only its public number, so
                // recover the arena identity before entering the exact gate
                // graph instead of silently mixing number-only and exact
                // endpoint keys.
                sector: position_sector(element),
                level: element.layer(),
            }
        })
        .effective
    };
    let me_position = resolve_position(me_id);
    let target_position = resolve_position(target_id);
    let me_sector = me_position.sector?;
    let target_sector = target_position.sector?;
    if me_sector.get() == target_sector.get()
        && me_sector.arena_index() == target_sector.arena_index()
    {
        return None;
    }

    let me_auth = me.actor_auth_info();
    let target_auth = target.actor_auth_info();

    let wait = crate::gate::compute_avenger_wait_position(
        doors,
        (target_position.x, target_position.y),
        target_sector,
        &target_auth,
        (me_position.x, me_position.y),
        me_sector,
        &me_auth,
        building_is_authorized,
        sector_lift_type,
    )?;

    Some(crate::ai::Position {
        x: wait.x,
        y: wait.y,
        sector: Some(wait.sector),
        level: wait.layer,
    })
}

impl EngineInner {
    /// Read AI Position directly, including selected door endpoints
    /// and carried-PC substitution, without constructing an entity-view world.
    pub(in crate::engine) fn live_ai_position(&self, id: EntityId) -> crate::ai::Position {
        let entity = self.expect_entity(id, "live AI position");
        assert!(
            entity_has_ai_view(entity),
            "live AI position unavailable for {id:?}"
        );
        let doors = self
            .scripts
            .mission
            .as_ref()
            .map(|_| self.script_domains.interactables.doors.as_slice())
            .unwrap_or(&[]);
        resolve_ai_position_with(
            &self.world.entities,
            doors,
            &self.orders.sequence_manager,
            id,
            |position_id| {
                let element = self
                    .expect_entity(position_id, "live AI position owner")
                    .element_data();
                crate::ai::Position {
                    x: element.position_map().x,
                    y: element.position_map().y,
                    sector: ai_view_position_sector(self, element),
                    level: element.layer(),
                }
            },
        )
        .effective
    }
}

fn build_one_entity_view(
    engine: &EngineInner,
    doors_ref: &[crate::gate::Door],
    entity_id: EntityId,
    entity: &Entity,
) -> ai_entity_view::AiEntityView {
    let building_sector = engine.entity_building_sector(entity.element_data().sector());
    let current_animation = engine
        .live_actor_animation(entity_id)
        .unwrap_or(crate::order::OrderType::NonanimationEnd);
    let selected_door = matches!(
        entity,
        Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
    )
    .then(|| selected_pass_door_movement(&engine.orders.sequence_manager, entity_id))
    .flatten();
    let mut view = ai_entity_view::entity_view_from_entity(
        entity,
        engine.world.original_creation_order(entity_id),
        building_sector.is_some(),
        building_sector,
        Some(&engine.mission_domain.campaign),
        current_animation,
    );

    if matches!(
        entity,
        Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
    ) {
        view.position = resolve_ai_position_with_selected(
            &engine.world.entities,
            doors_ref,
            entity_id,
            selected_door,
            |position_id| {
                let position_element = engine
                    .expect_entity(position_id, "AI entity-view position owner")
                    .element_data();
                crate::ai::Position {
                    x: position_element.position_map().x,
                    y: position_element.position_map().y,
                    sector: ai_view_position_sector(engine, position_element),
                    level: position_element.layer(),
                }
            },
        )
        .effective;
    }

    if matches!(
        entity,
        Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
    ) && let Some(input) = extract_exact_forecast_input(engine, entity, selected_door.is_some())
    {
        view.forecasted_destination = crate::ai::prepare_forecast_destination_for_ia(
            &input,
            doors_ref,
            &engine.world.fast_grid.level.sectors,
            &engine.world.fast_grid.level.sector_number_map,
        );
    }
    view
}

/// Preserve the sector identity carried by original-game element position
/// when constructing an AI-visible `Position(element)`.
///
/// Legacy compatibility entities can still carry only a public sector
/// number.  For a loaded spatial grid, recover that omitted pointer from the
/// actor's current point and layer, never from the lossy public-number map.
/// A wholly empty test/compatibility grid cannot prove an arena identity and
/// deliberately remains number-only. Once topology exists, a missing or
/// ambiguous identity is an invariant failure rather than an invitation to
/// guess through the lossy public-number map.
pub(super) fn ai_view_position_sector(
    engine: &EngineInner,
    element: &crate::element::ElementData,
) -> Option<crate::position_interface::SectorHandle> {
    let sector = element.sector()?;
    if let Some(index) = sector.arena_index() {
        let exact =
            super::movement::grid_sector_for_position_handle(&engine.world.fast_grid.level, sector)
                .unwrap_or_else(|| panic!("AI Position carries missing exact sector {index:?}"));
        assert_eq!(
            exact.sector_number,
            crate::sector::SectorNumber::new(i16::from(sector)),
            "AI Position exact sector identity disagrees with its public number"
        );
        return Some(sector);
    }

    let public = crate::sector::SectorNumber::new(i16::from(sector));
    let point = element.position_map();
    let layer = element.layer();
    let candidates = engine
        .world
        .fast_grid
        .level
        .sectors
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.sector_number == public && candidate.layer == layer)
        .collect::<Vec<_>>();
    let matches = candidates
        .iter()
        .copied()
        .filter(|(_, candidate)| candidate.contains_point(point))
        .collect::<Vec<_>>();
    let index = match matches.as_slice() {
        [(index, _)] => *index,
        [] => match candidates.as_slice() {
            [(index, _)] => *index,
            [] if engine.world.fast_grid.level.sectors.is_empty() => return Some(sector),
            [] => panic!(
                "AI Position sector {public} layer {layer} is absent from the loaded exact arena"
            ),
            _ => unique_gate_endpoint_sector(engine, public, layer, &candidates).unwrap_or_else(
                || {
                    panic!(
                        "AI Position sector {public} at {point:?} has no containing sector and is ambiguous in the exact arena"
                    )
                },
            ),
        },
        _ => panic!("AI Position sector {public} at {point:?} is ambiguous in the exact arena"),
    };
    let index = crate::fast_find_grid::SectorIndex::new(index as u32)
        .expect("AI Position exact sector index exceeds the arena range");
    Some(sector.with_arena_index(index))
}

/// Recover the identity the original game carries when an actor is standing on
/// a gate endpoint just outside both adjacent sector polygons. Public sector
/// numbers remain lossy, so this is valid only when every matching gate
/// endpoint names the same exact arena sector.
fn unique_gate_endpoint_sector(
    engine: &EngineInner,
    public: crate::sector::SectorNumber,
    layer: u16,
    candidates: &[(usize, &crate::fast_find_grid::GridSector)],
) -> Option<usize> {
    let mut unique = None;
    for door in &engine.script_domains.interactables.doors {
        for (endpoint_public, endpoint_layer, endpoint_index) in [
            (door.sector_out, door.layer_out, door.sector_out_index),
            (door.sector_in, door.layer_in, door.sector_in_index),
        ] {
            if endpoint_public != public || endpoint_layer != layer {
                continue;
            }
            let Some(endpoint_index) = endpoint_index else {
                continue;
            };
            let endpoint_index = usize::from(endpoint_index);
            if !candidates
                .iter()
                .any(|(candidate_index, _)| *candidate_index == endpoint_index)
            {
                panic!(
                    "gate endpoint sector {endpoint_index} disagrees with public sector {public} layer {layer}"
                );
            }
            match unique {
                None => unique = Some(endpoint_index),
                Some(previous) if previous == endpoint_index => {}
                Some(_) => return None,
            }
        }
    }
    unique
}

#[cfg(test)]
mod ai_view_position_sector_tests {
    use super::*;
    use crate::coordinates::MapPoint;
    use crate::engine::test_support::square_sector;
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::Door;
    use crate::sector::SectorNumber;

    #[test]
    fn entity_view_recovers_duplicate_public_goal_for_exact_gate_route() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        let wrong = engine.world.fast_grid_mut().add_sector(
            square_sector(
                88,
                2,
                MapPoint::new(300.0, 300.0),
                MapPoint::new(350.0, 350.0),
            ),
            2,
        );
        let goal = engine.world.fast_grid_mut().add_sector(
            square_sector(
                88,
                2,
                MapPoint::new(100.0, 100.0),
                MapPoint::new(200.0, 200.0),
            ),
            2,
        );
        let source = engine.world.fast_grid_mut().add_sector(
            square_sector(77, 2, MapPoint::new(10.0, 10.0), MapPoint::new(60.0, 60.0)),
            2,
        );
        assert_ne!(wrong, goal);

        let _legacy_null_slot =
            engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::ActorPc;
                    initial_element
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            }));
        let target = engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: {
                let mut initial_element = crate::element::ElementData::from_initial_posture(
                    crate::element::Posture::Upright,
                );
                initial_element.kind = crate::element::ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }));
        let element = engine
            .get_entity_mut(target)
            .expect("test PC exists")
            .element_data_mut();
        element.active = true;
        element.set_position_map(MapPoint::new(150.0, 150.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(88));

        let views = build_entity_views(&engine);
        let goal_position = views.get(&target.index()).expect("PC view exists").position;
        assert_eq!(
            goal_position.sector.and_then(|sector| sector.arena_index()),
            SectorIndex::new(goal)
        );
        let metadata_position = lookup_primary_target_position(&engine, target)
            .expect("live primary target metadata exists");
        assert_eq!(
            metadata_position
                .sector
                .and_then(|sector| sector.arena_index()),
            SectorIndex::new(goal)
        );
        let forecast_input = extract_exact_forecast_input(
            &engine,
            engine.get_entity(target).expect("test PC exists"),
            false,
        )
        .expect("PC has forecast input");
        assert_eq!(
            forecast_input
                .sector_handle
                .and_then(|sector| sector.arena_index()),
            SectorIndex::new(goal),
            "AI destination forecasting must receive the same exact live sector as the PC's AI position",
        );
        let forecast = crate::ai::prepare_forecast_destination_for_ia(
            &forecast_input,
            &[],
            &engine.world.fast_grid.level.sectors,
            &engine.world.fast_grid.level.sector_number_map,
        )
        .resolve(&crate::sim_rng::SimulationContext::with_seed(1));
        assert_eq!(
            forecast
                .position
                .sector
                .and_then(|sector| sector.arena_index()),
            SectorIndex::new(goal),
        );

        let door = Door {
            sector_out: SectorNumber::new(77),
            sector_in: SectorNumber::new(88),
            sector_out_index: SectorIndex::new(source),
            sector_in_index: SectorIndex::new(goal),
            point_out: MapPoint::new(50.0, 50.0),
            point_in: MapPoint::new(150.0, 150.0),
            layer_out: 2,
            layer_in: 2,
            ..Door::default()
        };
        let route = crate::gate::find_path_gates_with_sector_indices(
            &[door],
            (25.0, 25.0),
            77,
            SectorIndex::new(source),
            (goal_position.x, goal_position.y),
            goal_position.sector.unwrap().get(),
            goal_position.sector.unwrap().arena_index(),
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .expect("exact EventView goal must remain routable through the indexed gate graph");
        assert_eq!(route.len(), 1);
    }

    #[test]
    fn empty_compatibility_grid_keeps_number_only_ai_position() {
        let mut engine = EngineInner::new();
        let mut element = crate::element::ElementData::default();
        element.set_position_map(MapPoint::new(150.0, 150.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(88));
        assert_eq!(
            ai_view_position_sector(&engine, &element)
                .unwrap()
                .arena_index(),
            None
        );

        let _legacy_null_slot =
            engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::ActorPc;
                    initial_element
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            }));
        let target = engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: {
                let mut initial_element = crate::element::ElementData::from_initial_posture(
                    crate::element::Posture::Upright,
                );
                initial_element.kind = crate::element::ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }));
        let target_element = engine
            .get_entity_mut(target)
            .expect("test PC exists")
            .element_data_mut();
        target_element.set_position_map(MapPoint::new(150.0, 150.0));
        target_element.set_layer(2);
        target_element.set_sector(crate::position_interface::SectorHandle::new(88));
        let metadata_position = lookup_primary_target_position(&engine, target)
            .expect("compatibility primary target metadata exists");
        assert_eq!(
            metadata_position
                .sector
                .and_then(|sector| sector.arena_index()),
            None
        );
        let forecast_input = extract_exact_forecast_input(
            &engine,
            engine.get_entity(target).expect("test PC exists"),
            false,
        )
        .expect("PC has forecast input");
        assert_eq!(
            forecast_input
                .sector_handle
                .and_then(|sector| sector.arena_index()),
            None,
            "an empty compatibility grid cannot invent exact forecast topology",
        );
    }

    #[test]
    #[should_panic(expected = "ambiguous in the exact arena")]
    fn duplicate_public_noncontaining_position_does_not_guess_an_identity() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        engine.world.fast_grid_mut().add_sector(
            square_sector(
                88,
                2,
                MapPoint::new(250.0, 250.0),
                MapPoint::new(300.0, 300.0),
            ),
            2,
        );
        engine.world.fast_grid_mut().add_sector(
            square_sector(
                88,
                2,
                MapPoint::new(350.0, 350.0),
                MapPoint::new(400.0, 400.0),
            ),
            2,
        );
        let mut element = crate::element::ElementData::default();
        element.set_position_map(MapPoint::new(150.0, 150.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(88));
        let _ = ai_view_position_sector(&engine, &element);
    }

    #[test]
    fn gate_endpoint_recovers_actor_outside_duplicate_sector_polygons() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        let wrong = engine.world.fast_grid_mut().add_sector(
            square_sector(
                58,
                2,
                MapPoint::new(300.0, 300.0),
                MapPoint::new(350.0, 350.0),
            ),
            2,
        );
        let endpoint = engine.world.fast_grid_mut().add_sector(
            square_sector(
                58,
                2,
                MapPoint::new(100.0, 100.0),
                MapPoint::new(200.0, 200.0),
            ),
            2,
        );
        assert_ne!(wrong, endpoint);
        engine.script_domains.interactables.doors.push(Door {
            sector_out: SectorNumber::new(58),
            sector_out_index: SectorIndex::new(endpoint),
            layer_out: 2,
            ..Door::default()
        });

        let mut element = crate::element::ElementData::default();
        element.set_position_map(MapPoint::new(250.0, 250.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(58));

        assert_eq!(
            ai_view_position_sector(&engine, &element)
                .expect("gate endpoint identity is recoverable")
                .arena_index(),
            SectorIndex::new(endpoint)
        );
    }
}

/// Capture spatial entities for callers that still need a complete observation.
pub(super) fn build_entity_views(engine: &EngineInner) -> AiEntityViewMap {
    let _detail =
        super::tick::entity_system_detail_guard(super::tick::EntitySystemDetail::BuildEntityViews);
    // Scratch views are also built by empty/pre-script engine fixtures.  Door
    // state is intentionally unavailable during that phase; `init_ai` emits a
    // warning when a real level reaches AI initialization without a script.
    let doors_ref = engine
        .scripts
        .mission
        .as_ref()
        .map(|_| engine.script_domains.interactables.doors.as_slice())
        .unwrap_or(&[]);

    let mut map = ai_entity_view::take_entity_view_map(engine.world.entities.len());
    for (entity_id, entity) in engine.world.entities.occupied() {
        if !entity_has_ai_view(entity) {
            continue;
        }
        let view = build_one_entity_view(engine, doors_ref, entity_id, entity);

        // AI handle == entity slot index (see `FighterSnapshot.handle =
        // target_id.index()` elsewhere, and `self.world.entities.get_mut(target as
        // usize)` for `CrossNpcAction` handlers).
        map.insert(entity_id.index(), view);
    }
    map
}

fn entity_has_ai_view(entity: &Entity) -> bool {
    // A cleared Original layer (0xFFFF) means the entity is outside spatial
    // membership. This occurs transiently for projectiles and can be retained
    // by loaded actor state; neither has a valid AI Position until a real
    // layer is installed again.
    crate::ai_entity_view::entity_view_unavailable(entity).is_none()
}

impl EngineInner {
    /// Make nearby civilians panic.
    ///
    /// Iterates every civilian within `view_radius` of `source`,
    /// dispatches `EventPanic` through the civilian's
    /// [`crate::ai_friendly::FriendlyAi::think`] — which sets
    /// `FleeingPanic` and records a [`crate::ai::PanicRequest`] on the
    /// AI base — then drains the request against
    /// `ai_global.door_seek_infos` so a matching door gets picked and
    /// movement to the door entrance queued.
    /// Orchestrate a building-wide enemy alert.
    ///
    /// Walks the building's occupant list, splits it into royalists /
    /// lacklandists / civilians, panics the civilians, and — if both
    /// camps are present — stages the outnumbered side to flee the
    /// building while the stronger side pursues
    /// (`init_battle_before_door` follow-on).
    ///
    /// `send_before_door_to_fight` is implemented as
    /// [`EngineInner::send_before_door_to_fight`], and the
    /// `init_battle_before_door` orchestration — pick nearest door,
    /// compute defender/attacker positions, fan out
    /// `send_before_door_to_fight` per occupant — is implemented as
    /// [`EngineInner::init_battle_before_door`] and called below.
    #[tracing::instrument(level = "trace", skip_all, fields(source = source.index()))]
    pub(crate) fn dispatch_enemy_in_house_alert(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source: EntityId,
        assets: &LevelAssets,
    ) {
        // Find the source NPC's building sector.
        let source_sector = {
            let Some(entity) = self.world.entities.get(source) else {
                return;
            };
            let sector = entity.element_data().sector();
            match self.entity_building_sector(sector) {
                Some(_) => sector, // real building
                None => return,    // source left the building already
            }
        };

        let building_sector_num = match source_sector {
            Some(s) => u32::from(s),
            None => return,
        };

        // Look up the matching House to get the occupant list.
        let Some(house) = self
            .ai
            .global
            .houses
            .iter()
            .find(|h| h.sector_index == building_sector_num)
        else {
            return;
        };
        let door_indices = house.door_indices.clone();
        let building_index = house.building_index;
        let mirrored_occupant_ids = house.occupant_ids.clone();
        // The building occupant list is the authority read by the original
        // game here. The Rust `House` mirror can temporarily lag it when
        // legacy adoption restores the serialized building list before a
        // later entity/topology phase rebuilds the AI houses. Resolve the
        // canonical actor-handle list at the call boundary so civilians are
        // not silently omitted from the alert and its synchronous Panic.
        let canonical_handles = building_index
            .and_then(|index| {
                self.script_domains
                    .buildings
                    .occupants
                    .get(usize::from(index))
            })
            .cloned();
        let occupant_ids = authoritative_house_occupants(
            building_sector_num,
            canonical_handles.as_deref(),
            &mirrored_occupant_ids,
            |handle| self.entity_id_for_actor_handle(handle),
        );

        // Group live fighters by allegiance. Original used two fixed lists;
        // custom missions may have any number of groups in one building.
        let mut fighter_ids: std::collections::BTreeMap<crate::element::Camp, Vec<EntityId>> =
            std::collections::BTreeMap::new();
        let mut civilian_ids: Vec<EntityId> = Vec::new();
        for &eid in &occupant_ids {
            let Some(entity) = self.world.entities.get(eid) else {
                continue;
            };
            match entity {
                Entity::Soldier(s) => {
                    if s.npc.life_points <= 0 || s.human.unconscious {
                        continue;
                    }
                    if s.soldier.cached_camp.allegiance_id().is_some() {
                        fighter_ids
                            .entry(s.soldier.cached_camp)
                            .or_default()
                            .push(eid);
                    }
                }
                Entity::Civilian(c) => {
                    if c.npc.life_points <= 0 || c.human.unconscious {
                        continue;
                    }
                    civilian_ids.push(eid);
                }
                Entity::Pc(pc) if pc.pc.life_points > 0 && !pc.human.unconscious => {
                    fighter_ids.entry(entity.camp()).or_default().push(eid);
                }
                _ => {}
            }
        }

        if building_exit_wait_owner_debug_enabled() {
            self.trace_enemy_in_house_alert(
                source,
                building_sector_num,
                &occupant_ids,
                [
                    fighter_ids
                        .get(&crate::element::Camp::Royalists)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    fighter_ids
                        .get(&crate::element::Camp::Lacklandists)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    &civilian_ids,
                ],
            );
        }

        let source_camp = self.expect_entity(source, "building alert source").camp();
        let source_ids = fighter_ids
            .iter()
            .filter(|(camp, _)| self.camps_are_allied(source_camp, **camp))
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect::<Vec<_>>();
        if source_ids.is_empty() {
            return;
        }
        let Some((opposing_camp, opposing_ids)) = fighter_ids
            .iter()
            .filter(|(camp, _)| self.camps_are_hostile(source_camp, **camp))
            .map(|(camp, _)| {
                let coalition = fighter_ids
                    .iter()
                    .filter(|(ally, _)| {
                        self.camps_are_allied(*camp, **ally)
                            && self.camps_are_hostile(source_camp, **ally)
                    })
                    .flat_map(|(_, ids)| ids.iter().copied())
                    .collect::<Vec<_>>();
                (*camp, coalition)
            })
            .filter(|(_, ids)| !ids.is_empty())
            .max_by_key(|(_, ids)| ids.len())
        else {
            return;
        };

        // Every live civilian panics.
        let panic_runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
        for civ_id in civilian_ids {
            self.process_building_civilian_panic(sim, assets, civ_id, panic_runs);
        }

        // Outnumbered side flees; the stronger side pursues.
        // A doorway battle has two sides. Dispatch the alerting allegiance
        // against its largest hostile group; other allegiances remain valid
        // independent combatants and can dispatch their own alerts.
        // TODO(multi-team-door-battles): schedule every hostile pair when the
        // door coordinator can own more than one simultaneous battle.
        let (fleeing, pursuing) = if doorway_battle_source_side_flees(
            source_camp,
            source_ids.len(),
            opposing_camp,
            opposing_ids.len(),
        ) {
            (source_ids, opposing_ids)
        } else {
            (opposing_ids, source_ids)
        };

        self.init_battle_before_door(sim, assets, &door_indices, &fleeing, &pursuing);

        tracing::debug!(
            source = source.index(),
            building = building_sector_num,
            fleeing = fleeing.len(),
            pursuing = pursuing.len(),
            "indoor enemy alert: civilians panicked, door-battle dispatched"
        );
    }

    /// Make a single civilian panic from the building alert.
    /// Equivalent to the inline
    /// `civilians[i].panic(AI_STANDARD_PANIC_RUNS)` loop body in
    /// `enemy_in_house_alert`.
    #[tracing::instrument(level = "trace", skip_all, fields(civ = civ_id.index(), runs))]
    fn process_building_civilian_panic(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        civ_id: EntityId,
        runs: u8,
    ) {
        self.world
            .entities
            .get_mut(civ_id)
            .and_then(Entity::friendly_ai_mut)
            .expect("building panic civilian has no friendly AI")
            .panic_undirected(runs);
        self.process_pending_begin_panic_for(sim, assets, civ_id);
    }

    #[tracing::instrument(level = "trace", skip_all, fields(source = source.index()))]
    pub(crate) fn nearby_civilians_panic(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source: EntityId,
    ) {
        self.nearby_civilians_panic_generic(sim, assets, source);
    }

    pub(crate) fn nearby_civilians_panic_180(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source: EntityId,
    ) {
        self.brawl_nearby_civilians_panic_exact(sim, assets, source);
    }

    /// Exact inline sweep from `WonderingBrawlHitting::EVENT_DONE`.
    /// Unlike the shared callback, Original has no standard-view AABB and
    /// does not require the brawler to be outdoors; every civilian delegates
    /// directly to its own forward-half-plane detection.
    fn brawl_nearby_civilians_panic_exact(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source: EntityId,
    ) {
        let scratch = self.build_sim_scratch(assets);
        let Some(source_entity) = self.world.entities.get(source) else {
            tracing::trace!(target: "parity_nearby_panic", "brawl source missing");
            return;
        };
        let source_map = source_entity.element_data().position_map();
        let panic_center = crate::ai::Position {
            x: source_map.x,
            y: source_map.y,
            sector: None,
            level: 0,
        };

        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            let ctx = {
                let Some(Entity::Civilian(civilian)) = self.world.entities.get(npc_id) else {
                    continue;
                };
                // Forward-half-plane detection checks both actors' raw active
                // flags. The target/source check remains inside the shared
                // detector so its gate ordering stays source-exact.
                if !civilian.element.active {
                    continue;
                }
                let building_sector = self.entity_building_sector(civilian.element.sector());
                self.ai_context_from_entity(
                    self.world
                        .entities
                        .get(npc_id)
                        .expect("civilian disappeared"),
                    self.control.frame_counter,
                    building_sector,
                    &scratch,
                    assets,
                )
            };
            ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
            let detected =
                crate::ai_enemy::context_detects_180_degrees(npc_id.index(), source.index(), &ctx);
            ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);
            if !detected {
                continue;
            }

            let stimulus = crate::ai::Stimulus::with_position(
                crate::ai::StimulusType::EventPanic,
                panic_center,
            );
            self.dispatch_think_with_drain(sim, npc_id, &stimulus, None, assets);
        }
    }

    fn nearby_civilians_panic_generic(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source: EntityId,
    ) {
        let scratch = self.build_sim_scratch(assets);
        let view_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS as f32
        };
        // `nearby_civilians_panic` builds an aspect-ratio-stretched
        // axis-aligned box (radius, radius * ASPECT_RATIO) around
        // self, then walks every NPC asking:
        // The shared callback uses omnidirectional detection. The separate
        // money-brawl completion sweep uses forward-half-plane detection. Both use
        // the civilian's upright eye point, the source actor's detection
        // point, the civilian's live view radius, and opaque 3D LOS.
        let radius_y = view_radius * crate::position_interface::ASPECT_RATIO;

        let (source_map, source_ground, source_detection_point) = {
            let Some(entity) = self.world.entities.get(source) else {
                tracing::trace!(target: "parity_nearby_panic", "source missing");
                return;
            };
            // Source must be active and outside a building for
            // Either actor detector requires an active, outdoor source.
            if !entity.element_data().active {
                tracing::trace!(target: "parity_nearby_panic", "source inactive");
                return;
            }
            if self
                .entity_building_sector(entity.element_data().sector())
                .is_some()
            {
                tracing::trace!(target: "parity_nearby_panic", sector = ?entity.element_data().sector(), "source classified in building");
                return;
            }
            let Some(detection_point) = entity.compute_detection_point() else {
                tracing::trace!(target: "parity_nearby_panic", "source has no detection point");
                return;
            };
            (
                entity.element_data().position_map(),
                entity.ground_position(),
                detection_point,
            )
        };

        let panic_center = crate::ai::Position {
            x: source_map.x,
            y: source_map.y,
            sector: None,
            level: 0,
        };

        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        // Clone the Arc-shared snapshot so the per-civilian filter can
        // call `los_clear` without holding an immutable borrow on
        // `self.ai.global` across the later `process_pending_*` mutable
        // borrows.
        let obstacles_owned = scratch.ai_sight_obstacles.clone();
        for npc_id in npc_ids {
            let obstacles = obstacles_owned.list();
            let eligible = {
                let Some(entity) = self.world.entities.get(npc_id) else {
                    continue;
                };
                let Entity::Civilian(c) = entity else {
                    continue;
                };
                // Both actor detectors test only active/outside-building
                // lifecycle here. In particular, it does not reject dead or
                // unconscious civilians before its distance and LOS work.
                let civilian_in_building =
                    self.entity_building_sector(c.element.sector()).is_some();
                if !nearby_panic_civilian_reaches_visibility(c.element.active, civilian_in_building)
                {
                    continue;
                }
                // Ground position is the cached world-space X/Y pair,
                // not map-space X/Y. Elevation therefore contributes to Y
                // before the aspect-ratio bounding-box test.
                let p = entity.ground_position();
                let dx = source_ground.x - p.x;
                let dy = source_ground.y - p.y;
                // Aspect-ratio bounding box: |dx| <= r,
                // |dy| <= r * ASPECT_RATIO.
                if dx.abs() > view_radius || dy.abs() > radius_y {
                    continue;
                }
                let Some(viewer_eye) =
                    entity.compute_eyes_point(Some(crate::element::Posture::Upright))
                else {
                    continue;
                };
                // omnidirectional actor detection's stretched-Y 3D distance
                // gate: civilian upright eye to source detection point,
                // clamped by the civilian's live real view radius.
                let dx = source_detection_point.x - viewer_eye.x;
                let dy = (source_detection_point.y - viewer_eye.y)
                    * crate::position_interface::INVERSE_ASPECT_RATIO;
                let dz = source_detection_point.z - viewer_eye.z;
                let sq_view_radius = {
                    let radius = c.npc.view_radius as f32;
                    radius * radius
                };
                if dx * dx + dy * dy + dz * dz > sq_view_radius {
                    continue;
                }
                crate::sight_obstacle::is_reachable_3d(
                    obstacles,
                    [viewer_eye.x, viewer_eye.y, viewer_eye.z],
                    [
                        source_detection_point.x,
                        source_detection_point.y,
                        source_detection_point.z,
                    ],
                    crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                )
            };
            if !eligible {
                continue;
            }

            let stimulus = crate::ai::Stimulus::with_position(
                crate::ai::StimulusType::EventPanic,
                panic_center,
            );
            // nearby-civilian panic directly sends the stimulus to the NPC.
            // Close that recipient's complete owner-local Think boundary:
            // EVENT_PANIC chooses a door and queues movement, whose
            // element and synchronous path request must exist before the
            // caller resumes. A raw dispatch plus manual PanicRequest drain
            // left the movement stranded in the civilian outbox until its next
            // owner slot.
            self.dispatch_think_with_drain(sim, npc_id, &stimulus, None, assets);
        }
    }

    /// Re-issue in-flight patrol movement so a freshly-changed
    /// `default_path_walking_flags` (typically RUN ↔ WALK from
    /// the `SetPathWalkingStyle` script native) takes effect
    /// immediately rather than at the next waypoint pickup.
    /// The relaunch tail of `set_path_walking_flags`:
    ///
    /// ```ignore
    /// if has_patrol_path && substate in {DefaultGotoRoute, DefaultEnroute} {
    ///     let mut flags = default_path_walking_flags;
    ///     if !will_stop_at_next_waypoint(sim, ) { flags |= GotoFlags::DONT_STOP; }
    ///     go_to(current_waypoint_position, flags);
    /// }
    /// ```
    pub(crate) fn relaunch_path_at_new_speed(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        let frame = self.control.frame_counter;
        let creation_order = self.world.original_creation_order(npc_id);
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("patrol speed owner"));
        if !ai.has_patrol_path
            || !matches!(
                ai.current_substate,
                crate::ai::Substate::DefaultGotoRoute | crate::ai::Substate::DefaultEnroute
            )
        {
            return;
        }
        let will_stop = ai.will_stop_at_next_waypoint_at(
            sim,
            &assets.navigation.hiking_paths,
            frame,
            Some(creation_order),
            crate::ai::WillStopCaller::SetPathWalkingFlags,
        );
        let path = ai
            .patrol_path
            .as_ref()
            .expect("patrol speed change requires initialized path");
        let waypoint = path
            .current_waypoint(&assets.navigation.hiking_paths)
            .expect("patrol speed change requires current waypoint");
        let destination = crate::ai::Position {
            x: waypoint.x as f32,
            y: waypoint.y as f32,
            sector: assets.navigation.hiking_waypoint_sector(
                usize::from(path.hiking_path_index),
                usize::from(path.current_waypoint_index),
                waypoint.sector,
            ),
            level: waypoint.level,
        };
        let mut flags = ai.default_path_walking_flags;
        if !will_stop {
            flags |= crate::ai::GotoFlags::DONT_STOP;
        }
        self.duty_go_to(sim, assets, npc_id, destination, flags);
    }
    /// Complete a panic request against live actor and door state.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_begin_panic_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        let Some(request) = self
            .world
            .entities
            .expect_ai_controller_mut(npc_id, format_args!("panic request owner"))
            .outbox
            .actor
            .begin_panic
            .take()
        else {
            return;
        };
        let directed = request.center.is_some();
        let mut door = self.nearest_panic_door(npc_id, request.center);
        {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("panic direction"));
            ai.directed_panic = directed;
            if let Some(center) = request.center {
                ai.panic_center_x = center.x;
                ai.panic_center_y = center.y;
            }
        }
        if directed && door.is_none() {
            door = self.nearest_panic_door(npc_id, None);
            self.world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("panic direction fallback"))
                .directed_panic = false;
        }
        let frame = self.control.frame_counter;
        let is_civilian = self.expect_entity(npc_id, "panic speaker").is_civilian();
        {
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("panic macro"));
            ai.break_macro();
            ai.set_transient_emoticon(crate::ai::EmoticonType::XMark, 0, frame);
        }
        if let Some(door) = door {
            if is_civilian {
                self.world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("panic speech"))
                    .say(crate::ai::Remark::CivPanic);
                self.drain_ai_owner_work_for(sim, assets, npc_id);
            }
            self.duty_set_state(
                sim,
                assets,
                npc_id,
                crate::ai::AiState::Fleeing,
                crate::ai::Substate::FleeingRunToDoor,
            );
            {
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("panic door"));
                ai.set_alert_status(request.alert);
                ai.lasting_panic_runs = 0;
            }
            let position = self.panic_door_position(door);
            self.duty_go_to(sim, assets, npc_id, position, crate::ai::GotoFlags::RUN);
            let ai = self
                .world
                .entities
                .expect_ai_controller_mut(npc_id, format_args!("panic route result"));
            if !ai.couldnt_reachpoint {
                return;
            }
            ai.couldnt_reachpoint = false;
            if ai.directed_panic {
                let retry = self
                    .nearest_panic_door(npc_id, None)
                    .expect("directed panic retry has an accessible building door");
                self.world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("panic retry direction"))
                    .directed_panic = false;
                let position = self.panic_door_position(retry);
                self.duty_go_to(sim, assets, npc_id, position, crate::ai::GotoFlags::RUN);
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("panic retry result"));
                if !ai.couldnt_reachpoint {
                    return;
                }
                ai.couldnt_reachpoint = false;
            }
        }
        self.begin_panic_no_door_branch(sim, assets, npc_id, &request, is_civilian);
    }

    fn panic_door_position(&self, index: usize) -> crate::ai::Position {
        let door = &self.script_domains.interactables.doors[index];
        crate::ai::Position {
            x: door.point_in.x,
            y: door.point_in.y,
            level: door.layer_in,
            sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_in)).map(
                |sector| {
                    door.sector_in_index
                        .map_or(sector, |index| sector.with_arena_index(index))
                },
            ),
        }
    }

    fn nearest_panic_door(
        &self,
        owner: EntityId,
        center: Option<crate::ai::Position>,
    ) -> Option<usize> {
        let entity = self.expect_entity(owner, "panic door owner");
        let element = entity.element_data();
        let raw = element.position_map();
        let position = self.live_ai_position(owner);
        let raw_sector = ai_view_position_sector(self, element);
        let building = self.entity_building_sector(raw_sector);
        let auth = entity.actor_auth_info();
        let mut minimum = u16::MAX;
        let mut selected = None;
        for (index, door) in self.script_domains.interactables.doors.iter().enumerate() {
            let inside = crate::position_interface::SectorHandle::from_number(door.sector_in);
            let inside = door
                .sector_in_index
                .map_or(inside, |index| inside.with_arena_index(index));
            if door.door_type != crate::gate::DoorType::Building
                || building.is_some_and(|building| building.reference() == inside.reference())
                || !door.is_actor_authorized(
                    true,
                    &auth,
                    self.building_sector_is_authorized(door.sector_in),
                    false,
                )
            {
                continue;
            }
            if center.is_some_and(|center| {
                (door.point_out.x - raw.x) * (center.x - position.x)
                    + (door.point_out.y - raw.y) * (center.y - position.y)
                    >= 0.0
            }) {
                continue;
            }
            let mut distance = ((door.point_out.x - position.x)
                .abs()
                .max((door.point_out.y - position.y).abs()) as u32)
                as u16;
            let sector = crate::position_interface::SectorHandle::new(u16::from(door.sector_out))
                .map(|sector| {
                    door.sector_out_index
                        .map_or(sector, |index| sector.with_arena_index(index))
                });
            if sector.map(|sector| sector.reference())
                != raw_sector.map(|sector| sector.reference())
            {
                distance = distance.wrapping_add(500);
            }
            if door.layer_out != element.layer() {
                distance = distance.wrapping_add(300);
            }
            if distance >= minimum {
                continue;
            }
            if entity.camp() == crate::element::Camp::Lacklandists {
                let building_index = door
                    .sector_in_index
                    .and_then(|index| self.world.fast_grid.level.sectors.get(usize::from(index)))
                    .and_then(|sector| sector.building_index)
                    .expect("panic building door has no building");
                let occupants = self
                    .script_domains
                    .buildings
                    .occupants
                    .get(usize::from(building_index))
                    .expect("panic building occupants missing");
                if occupants.iter().any(|&handle| {
                    let id = self
                        .entity_id_for_actor_handle(handle)
                        .expect("panic building occupant missing");
                    matches!(
                        self.expect_entity(id, "panic building occupant"),
                        Entity::Pc(_)
                    )
                }) {
                    continue;
                }
            }
            minimum = distance;
            selected = Some(index);
        }
        selected
    }

    #[inline(never)]
    fn trace_enemy_in_house_alert(
        &self,
        source: EntityId,
        building_sector_num: impl std::fmt::Display,
        occupant_ids: &[EntityId],
        [royalists, lacklandists, civilian_ids]: [&[EntityId]; 3],
    ) {
        static INVOCATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let invocation = INVOCATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let describe = |ids: &[EntityId]| {
            ids.iter()
                .map(|&id| (id, self.world.original_creation_order(id)))
                .collect::<Vec<_>>()
        };
        eprintln!(
            "BEXITWAIT {{\"event\":\"enemy_in_house_alert\",\"invocation\":{invocation},\"frame\":{},\"source\":{:?},\"source_creation_order\":{},\"building_sector\":{building_sector_num},\"occupants\":{:?},\"royalists\":{:?},\"lacklandists\":{:?},\"civilians\":{:?}}}",
            self.control.frame_counter,
            source,
            self.world.original_creation_order(source),
            describe(occupant_ids),
            describe(royalists),
            describe(lacklandists),
            describe(civilian_ids),
        );
        for &eid in occupant_ids {
            let Some(entity) = self.world.entities.get(eid) else {
                continue;
            };
            let detail = match entity {
                Entity::Soldier(s) => format!(
                    "soldier lp={} unconscious={} camp={:?} posture={:?}",
                    s.npc.life_points,
                    s.human.unconscious,
                    s.soldier.cached_camp,
                    entity.element_data().posture()
                ),
                Entity::Civilian(c) => format!(
                    "civilian lp={} unconscious={}",
                    c.npc.life_points, c.human.unconscious
                ),
                Entity::Pc(p) => {
                    format!(
                        "pc lp={} unconscious={}",
                        p.pc.life_points, p.human.unconscious
                    )
                }
                _ => "other".to_string(),
            };
            eprintln!(
                "BEXITWAIT_OCC {:?} co={} {detail}",
                eid,
                self.world.original_creation_order(eid)
            );
        }
    }

    /// No-door branch of `panic`.  Split out so the door-found
    /// branch can fall through on a post-movement-request unreachable-point
    /// error.
    fn begin_panic_no_door_branch(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
        request: &crate::ai::PanicRequest,
        is_civilian: bool,
    ) {
        // If directed, OR in the "panic center is in front of me"
        // dot-product test so a center that has flipped in front
        // during a prior run still counts as a new panic.
        let mut is_new_panic = request.is_new_panic;
        if request.center.is_some() && !is_new_panic {
            let position = self.live_ai_position(npc_id);
            let direction = self
                .expect_entity(npc_id, "panic facing")
                .element_data()
                .direction();
            let ai = self
                .world
                .entities
                .expect_ai_controller(npc_id, format_args!("panic owner"));
            if directed_panic_center_is_in_front(
                direction as i16,
                position.x,
                position.y,
                ai.panic_center_x,
                ai.panic_center_y,
            ) {
                is_new_panic = true;
            }
        }

        if is_new_panic {
            // New panic — full side-effect set.
            self.duty_set_state(
                sim,
                assets,
                npc_id,
                crate::ai::AiState::Fleeing,
                crate::ai::Substate::FleeingPanic,
            );
            self.world
                .entities
                .expect_ai_controller_mut(
                    npc_id,
                    format_args!("panic owner {} has no AI", npc_id.index()),
                )
                .say(if is_civilian {
                    crate::ai::Remark::CivPanic
                } else {
                    crate::ai::Remark::Panic
                });
            self.drain_ai_owner_work_for(sim, assets, npc_id);
            let deferred_self_stimuli = {
                let ai = self
                    .world
                    .entities
                    .expect_ai_controller_mut(npc_id, format_args!("panic owner after speech"));
                ai.set_alert_status(request.alert);
                ai.lasting_panic_runs = request.runs.wrapping_add(1);

                // A pre-existing Rust self-stimulus is deferred work from an
                // enclosing boundary. It is not part of the original game's panic handling
                // direct recursive Think call and must not be pulled into it.
                let deferred = std::mem::take(&mut ai.outbox.reentrant.self_stimuli);
                deferred
            };

            // AI panic calls
            // `Think(EVENT_REACHPOINT)` directly here.  This is a recursive
            // owner-local boundary, not a deferred event: in particular, a
            // retained sibling stimulus must not run first and replace the
            // freshly installed `FLEEING_PANIC` substate.  Close the generated
            // Think (and its two direction/distance RNG draws) before Panic
            // returns to its caller.
            self.execute_ai_callback(
                sim,
                assets,
                npc_id,
                &crate::ai::Stimulus::new(crate::ai::StimulusType::EventReachPoint),
            );
            self.world
                .entities
                .expect_ai_controller_mut(
                    npc_id,
                    format_args!(
                        "panic owner {} lost AI after recursive Think",
                        npc_id.index()
                    ),
                )
                .outbox
                .reentrant
                .self_stimuli
                .extend(deferred_self_stimuli);
        } else {
            // Not new: upgrade-only bump of `lasting_panic_runs`
            // (`if lasting_panic_runs < runs`).  No state change, no
            // `say()`, no self-fire.
            let ai = self.world.entities.expect_ai_controller_mut(
                npc_id,
                format_args!("panic owner {} has no AI", npc_id.index()),
            );
            if ai.lasting_panic_runs < request.runs {
                ai.lasting_panic_runs = request.runs;
            }
        }
    }

    /// Enter an enemy/friendly state change after releasing the
    /// engine's prior controller borrow. Dispatch follows the actor's AI brain,
    /// not its entity kind: custom-mission PCs may own the same [`EnemyAi`] as
    /// soldiers. Required callers must not degrade a missing owner or
    /// mismatched brain into a silent no-op.
    #[cfg(test)]
    pub(super) fn set_typed_npc_state(
        &mut self,
        npc_id: EntityId,
        state: crate::ai::AiState,
        substate: crate::ai::Substate,
        context: &'static str,
    ) {
        let entity = self.expect_entity_mut(npc_id, context);
        if let Some(enemy) = entity.enemy_ai_mut() {
            enemy.set_state(state, substate);
            return;
        }
        if let Some(friendly) = entity.friendly_ai_mut() {
            friendly.set_state(state, substate);
            return;
        }
        panic!(
            "{context} owner {} has entity kind {:?} but no typed AI brain",
            npc_id.index(),
            entity.element_data().kind
        );
    }

    /// Enter the pre-filter half of typed no-event decision-tick admission.
    pub(super) fn start_script_ai_native_think_pre_filter(&mut self, npc_id: EntityId) {
        use crate::ai::AiRole;
        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::NoEvent);
        self.enter_ai_think_frame(npc_id);
        let entity = self.expect_entity_mut(npc_id, "SetAIState decision-entry owner");
        if let Some(enemy) = entity.enemy_ai_mut() {
            enemy.start_think_pre_filter(&stimulus);
        } else if let Some(friendly) = entity.friendly_ai_mut() {
            friendly.start_think_pre_filter(&stimulus);
        } else {
            panic!(
                "SetAIState decision-entry owner {} has no typed AI for entity kind {:?}",
                npc_id.index(),
                entity.element_data().kind
            );
        }
    }

    /// Run the post-filter half of typed no-event decision-tick admission and return
    /// its normal Think admission decision. SetAIState deliberately ignores
    /// this bool, but the lock/freeze/special-state side effects still occur.
    pub(super) fn start_script_ai_native_think_post_filter(&mut self, npc_id: EntityId) -> bool {
        let (self_is_dead, self_is_unconscious) = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| (entity.is_dead(), entity.is_unconscious()))
            .unwrap_or_else(|| {
                panic!(
                    "SetAIState post-filter decision-entry owner {} disappeared",
                    npc_id.index()
                )
            });
        let static_ai_frozen = self.ai.global.freeze;
        self.world
            .entities
            .expect_ai_controller_mut(
                npc_id,
                format_args!(
                    "SetAIState post-filter decision-entry owner {} lost its typed AI",
                    npc_id.index()
                ),
            )
            .start_no_event_post_filter(static_ai_frozen, self_is_dead, self_is_unconscious)
    }

    /// Close typed decision-tick completion after area seeking/panic and their recursively
    /// produced owner work have stabilized.
    pub(super) fn end_script_ai_native_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        self.execute_ai_end_think(sim, assets, npc_id);
    }

    /// Drain a pending script-driven area-search request. Consumes
    /// `AiController::outbox.actor.script_seek_area` set by
    /// `script_set_ai_state` when a script fires
    /// `SetAIState(actor, STATE_SEEKING)` into live soldier area search.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_script_seek_area_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        let request = {
            let ai = self.world.entities.expect_ai_controller_mut(
                npc_id,
                format_args!("accepted SetAIState SEEKING owner before area search"),
            );
            ai.outbox.actor.script_seek_area.take().unwrap_or_else(|| {
                panic!(
                    "accepted SetAIState SEEKING owner {} lost its required area-search request",
                    npc_id.index()
                )
            })
        };

        let frame = self.control.frame_counter;
        let creation_order = Some(self.world.original_creation_order(npc_id));
        if crate::ai_enemy::EnemyAi::seek_area_phase6_caller_debug_enabled()
            && crate::ai_enemy::EnemyAi::seek_area_phase6_caller_debug_matches(
                frame,
                creation_order,
            )
        {
            Self::trace_seek_area_script_caller(npc_id, frame, creation_order);
        }
        self.execute_ai_seek_area(
            sim,
            assets,
            npc_id,
            request.center,
            request.radius,
            crate::ai_enemy::SeekFlags::empty(),
            crate::ai_enemy::UNDEFINED_DIRECTION,
        );
        // Area seeking's typed state-change callback is inside the decision-tick
        // scope and must finish before its later movement/order tail is
        // exposed to the enclosing native barrier.
        self.drain_ai_owner_work_for(sim, assets, npc_id);
    }

    #[inline(never)]
    fn trace_seek_area_script_caller(npc_id: EntityId, frame: u32, creation_order: Option<u32>) {
        eprintln!(
            "SEEKAREA_CALLER {{\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"caller\":\"script_set_ai_state\",\"stimulus\":\"no_event\"}}",
            frame,
            npc_id.index(),
            creation_order
                .expect("phase6 caller diagnostic matched an owner without creation order"),
        );
    }
}
