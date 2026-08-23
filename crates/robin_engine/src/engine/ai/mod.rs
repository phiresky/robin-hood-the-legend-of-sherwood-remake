//! Enemy AI engine integration.
//!
//! Tick orchestration and engine-owned side effects are grouped by domain:
//!  - [`tick_data`] and [`snapshots`] build owner-boundary tactical views.
//!  - [`tick_scheduling`] and [`owner_scheduling`] drive per-NPC work.
//!  - [`detection`] and [`post_detection`] implement the visibility phases.
//!  - [`event_dispatch`] handles animation, noise, detection, and speech events.
//!  - [`patrol_coordination`] and [`patrol_dispatch`] coordinate patrol members.
//!  - [`cross_npc_actions`] closes synchronous interactions between NPCs.

mod cross_npc_actions;
mod detection;
mod event_dispatch;
mod initialization;
mod owner_scheduling;
mod patrol_coordination;
mod patrol_dispatch;
pub(crate) use detection::debug_detectable_mutation_load_snapshot;
#[cfg(test)]
pub(crate) use detection::set_heard_callback_observer;
mod post_detection;
mod snapshots;
mod tick_data;
mod tick_scheduling;

#[cfg(test)]
pub(crate) use post_detection::{
    NpcPostDetectionTailPhase, capture_npc_post_detection_tail_phases,
};

use super::*;
use crate::ai::{AiContext, AiPerTickData, StimulusType};
use crate::ai_entity_view::{self, AiEntityViewMap, AiEntityViews, SharedAiEntityViews};
use crate::ai_vision;
use crate::coordinates::MapPoint;
use crate::element::{
    Camp, Detectable, DetectableType, Entity, EntityId, Human as _, PcId, SoldierId,
};
use crate::engine::SimScratch;
use crate::entities::{Entities, EntitySlots};
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

                // Original assigns the complete RHposition returned by
                // RHDoor::GetPositionIn, including its RHSector pointer.
                // Rewriting only the public number leaves an impossible
                // outside-arena/interior-number pair on overlapping sectors.
                let inside_sector = door.position_in.sector.unwrap_or_else(|| {
                    panic!(
                        "building door {} has no required interior sector",
                        door.door_index.0
                    )
                });
                if waypoint_sectors.is_some() {
                    assert!(
                        inside_sector.arena_index().is_some(),
                        "building door {} interior sector has no exact arena identity",
                        door.door_index.0
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

#[derive(Debug)]
struct RefreshViewLifecycleDebugConfig {
    enabled: bool,
    from_frame: u32,
    through_frame: u32,
    creation_order: Option<u32>,
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
            door_index: DoorIndex(5),
            door_type: DoorType::Building,
            point_out: MapPoint::new(1955.0, 1992.0),
            position_in: Position {
                x: 1938.0,
                y: 1964.0,
                sector: Some(inside),
                level: 8,
            },
            sector_out: 0,
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

fn refresh_view_lifecycle_debug_config() -> &'static RefreshViewLifecycleDebugConfig {
    static CONFIG: std::sync::OnceLock<RefreshViewLifecycleDebugConfig> =
        std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let enabled = std::env::var_os("PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE").is_some();
        if !enabled {
            return RefreshViewLifecycleDebugConfig {
                enabled: false,
                from_frame: 0,
                through_frame: u32::MAX,
                creation_order: None,
            };
        }
        let parse = |name: &str, default: u32| {
            std::env::var(name).map_or(default, |value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for RVLIFE diagnostic: {error}")
                })
            })
        };
        RefreshViewLifecycleDebugConfig {
            enabled: true,
            from_frame: parse("PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_FROM", 0),
            through_frame: parse("PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_THROUGH", u32::MAX),
            creation_order: std::env::var("PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_CREATION_ORDER")
                .ok()
                .map(|value| {
                    value.parse::<u32>().unwrap_or_else(|error| {
                        panic!(
                            "invalid PARITY_DEBUG_REFRESH_VIEW_LIFECYCLE_CREATION_ORDER={value:?}: {error}"
                        )
                    })
                }),
        }
    })
}

/// Opt-in provenance for the two-draw building-exit wait used by parity
/// audits. This is deliberately process-local diagnostic state: it must not
/// enter snapshots, state hashes, or the simulation RNG stream.
pub(super) fn building_exit_wait_owner_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PARITY_DEBUG_BUILDING_EXIT_WAIT_OWNER").is_some())
}

/// Original attaches both ordinary building doors and building-trap doors to
/// `RHSectorBuilding::GetGates()`. AI house initialization must preserve that
/// ownership because both rally-point creation and door-fight placement walk
/// the complete building gate list.
fn door_belongs_to_ai_house(door_type: crate::gate::DoorType) -> bool {
    matches!(
        door_type,
        crate::gate::DoorType::Building | crate::gate::DoorType::BuildingTrap
    )
}

/// Narrow, process-local diagnostics for the Save050 SeekArea point-count
/// mismatch. Environment reads and stderr output must remain outside engine
/// state so enabling this cannot affect snapshots, hashes, or simulation RNG.
fn seek_area_owner_position_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_SEEK_AREA_OWNER_POSITION").is_some()
}

fn seek_area_owner_position_debug_matches(frame: u32, creation_order: u32) -> bool {
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for SEEKAREA diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_SEEK_AREA_FRAME").is_none_or(|expected| frame == expected)
        && parse_filter("PARITY_DEBUG_SEEK_AREA_CREATION_ORDER")
            .is_none_or(|expected| creation_order == expected)
}

/// Opt-in, stderr-only provenance for the Save024
/// `ReconsiderSwordfightObservation` fighter-list mismatch. Keep the enable
/// check ahead of identity lookup so the disabled path performs no additional
/// world reads.
fn reconsider_observation_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_RECONSIDER_OBSERVATION").is_some()
}

fn reconsider_observation_debug_matches(frame: u32, creation_order: u32, handle: u32) -> bool {
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for RECONSIDER diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_RECONSIDER_OBSERVATION_FRAME")
        .is_none_or(|expected| frame == expected)
        && parse_filter("PARITY_DEBUG_RECONSIDER_OBSERVATION_CREATION_ORDER")
            .is_none_or(|expected| creation_order == expected)
        && parse_filter("PARITY_DEBUG_RECONSIDER_OBSERVATION_OWNER_HANDLE")
            .is_none_or(|expected| handle == expected)
}

#[derive(Debug)]
struct CivilianRandomSpeechDebugConfig {
    enabled: bool,
    frame: u32,
    creation_order: u32,
}

fn civilian_random_speech_debug_config() -> &'static CivilianRandomSpeechDebugConfig {
    static CONFIG: std::sync::OnceLock<CivilianRandomSpeechDebugConfig> =
        std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let enabled = std::env::var_os("PARITY_DEBUG_CIVILIAN_RANDOM_SPEECH").is_some();
        if !enabled {
            return CivilianRandomSpeechDebugConfig {
                enabled: false,
                frame: 0,
                creation_order: 0,
            };
        }
        let parse = |name: &str| {
            let value = std::env::var(name).unwrap_or_else(|error| {
                panic!("CIVRANDSPEECH diagnostic requires {name}: {error}")
            });
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for CIVRANDSPEECH diagnostic: {error}")
            })
        };
        CivilianRandomSpeechDebugConfig {
            enabled,
            frame: parse("PARITY_DEBUG_CIVILIAN_RANDOM_SPEECH_FRAME"),
            creation_order: parse("PARITY_DEBUG_CIVILIAN_RANDOM_SPEECH_CREATION_ORDER"),
        }
    })
}

#[derive(Debug)]
struct SpeechLifecycleDebugConfig {
    enabled: bool,
    frame: Option<u32>,
    actor: Option<u32>,
}

fn speech_lifecycle_debug_config() -> &'static SpeechLifecycleDebugConfig {
    static CONFIG: std::sync::OnceLock<SpeechLifecycleDebugConfig> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let enabled = std::env::var_os("PARITY_DEBUG_SPEECH_LIFECYCLE").is_some();
        if !enabled {
            return SpeechLifecycleDebugConfig {
                enabled: false,
                frame: None,
                actor: None,
            };
        }
        let parse = |name: &str| {
            std::env::var(name).ok().map(|value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for SPEECHLIFE diagnostic: {error}")
                })
            })
        };
        SpeechLifecycleDebugConfig {
            enabled: true,
            frame: parse("PARITY_DEBUG_SPEECH_LIFECYCLE_FRAME"),
            actor: parse("PARITY_DEBUG_SPEECH_LIFECYCLE_ACTOR"),
        }
    })
}

#[derive(Debug)]
struct PatrolTurnLifecycleDebugConfig {
    enabled: bool,
    frame: Option<u32>,
    creation_order: Option<u32>,
}

fn patrol_turn_lifecycle_debug_config() -> &'static PatrolTurnLifecycleDebugConfig {
    static CONFIG: std::sync::OnceLock<PatrolTurnLifecycleDebugConfig> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let enabled = std::env::var_os("PARITY_DEBUG_PATROL_TURN_LIFECYCLE").is_some();
        if !enabled {
            return PatrolTurnLifecycleDebugConfig {
                enabled: false,
                frame: None,
                creation_order: None,
            };
        }
        let parse = |name: &str| {
            std::env::var(name).ok().map(|value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for PATROLTURN diagnostic: {error}")
                })
            })
        };
        PatrolTurnLifecycleDebugConfig {
            enabled,
            frame: parse("PARITY_DEBUG_PATROL_TURN_FRAME"),
            creation_order: parse("PARITY_DEBUG_PATROL_TURN_CREATION_ORDER"),
        }
    })
}

#[derive(Debug)]
struct ArcherStepBackLifecycleDebugConfig {
    enabled: bool,
    frame: Option<u32>,
    creation_order: Option<u32>,
    owner_handle: Option<u32>,
}

fn archer_step_back_lifecycle_debug_config() -> &'static ArcherStepBackLifecycleDebugConfig {
    static CONFIG: std::sync::OnceLock<ArcherStepBackLifecycleDebugConfig> =
        std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let enabled = std::env::var_os("PARITY_DEBUG_ARCHER_STEP_BACK_LIFECYCLE").is_some();
        if !enabled {
            return ArcherStepBackLifecycleDebugConfig {
                enabled: false,
                frame: None,
                creation_order: None,
                owner_handle: None,
            };
        }
        let parse = |name: &str| {
            std::env::var(name).ok().map(|value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for ARCHERSTEP diagnostic: {error}")
                })
            })
        };
        ArcherStepBackLifecycleDebugConfig {
            enabled,
            frame: parse("PARITY_DEBUG_ARCHER_STEP_BACK_FRAME"),
            creation_order: parse("PARITY_DEBUG_ARCHER_STEP_BACK_CREATION_ORDER"),
            owner_handle: parse("PARITY_DEBUG_ARCHER_STEP_BACK_OWNER_HANDLE"),
        }
    })
}

/// Opt-in, stderr-only trace for the Save049 timer-driven archer step-back.
/// Original executes `RHElementActor::Hourglass` before the NPC timer tail, so
/// the authoritative installed order and sprite completion counters at context
/// construction distinguish a late actor retirement from an AI `GoTo` issue.
fn archer_step_back_lifecycle_debug_matches(
    frame: u32,
    creation_order: Option<u32>,
    owner_handle: u32,
) -> bool {
    let config = archer_step_back_lifecycle_debug_config();
    config.enabled
        && config.frame.is_none_or(|expected| expected == frame)
        && config
            .creation_order
            .is_none_or(|expected| Some(expected) == creation_order)
        && config
            .owner_handle
            .is_none_or(|expected| expected == owner_handle)
}

impl EngineInner {
    fn patrol_turn_lifecycle_debug_matches(&self, owner: EntityId) -> bool {
        let config = patrol_turn_lifecycle_debug_config();
        if !config.enabled
            || config
                .frame
                .is_some_and(|frame| frame != self.control.frame_counter)
        {
            return false;
        }
        let creation_order = self.world.original_creation_order(owner);
        !config
            .creation_order
            .is_some_and(|expected| expected != creation_order)
    }

    /// Opt-in, process-local trace of patrol Turn registration and ownership.
    /// It deliberately reads only live state and writes only stderr, so it
    /// cannot affect serialization, state hashes, ordering, or RNG.
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

#[cfg(test)]
thread_local! {
    static GALOPP_DISPATCH_OBSERVER: std::cell::RefCell<
        Option<Box<dyn FnMut(&EngineInner, EntityId)>>
    > = std::cell::RefCell::new(None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AiEntityViewStamp {
    entity_generation: u64,
    position_dependency_generation: u64,
    current_animation: crate::order::OrderType,
    selected_door: Option<(crate::gate::DoorIndex, i16)>,
    building_sector: Option<crate::position_interface::SectorHandle>,
    in_coma: bool,
    nets_generation: u64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PreparedAiEntityViewCache {
    views: Option<SharedAiEntityViews>,
    stamps: std::collections::HashMap<u32, AiEntityViewStamp>,
}

/// Immutable, RNG-free inputs prepared lazily at the first NPC owner slot.
///
/// The tactical snapshot is a per-tick view. Volatile optical and detectable
/// target geometry is still rebuilt at each NPC slot from live entities; doing
/// the full all-soldier tactical extraction for every owner made large maps
/// quadratic without providing fresher optical inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PreparedNpcOwnerPass {
    world: Option<snapshots::AiWorldView>,
    /// Derived AI views reused across consecutive creation-order owners.
    /// Mutable entity borrows and the small set of non-entity view inputs
    /// invalidate individual entries before the next synchronous Think.
    #[serde(skip)]
    entity_views: PreparedAiEntityViewCache,
}

impl PreparedNpcOwnerPass {
    /// A PC's Human::Hourglass tail refreshes its produced-noise record in
    /// creation order. Later NPC slots must not retain the earlier tactical
    /// snapshot: Original RefreshDetection reads that live record directly.
    pub(super) fn invalidate_after_pc_noise_refresh(&mut self) {
        self.world = None;
    }
}

/// Exact `ubFramePhase` computed by `RHElementActorNPC::Hourglass`.
/// `register_number` is the original creation/register ordering value.
pub(super) fn npc_hourglass_frame_phase(frame: u32, register_number: u32) -> u8 {
    (frame as u8).wrapping_sub((register_number as u8).wrapping_add(100))
}

/// Number of arrows given to Merry Man archers in forest levels.
const MERRY_MAN_ARROWS: u16 = 3;

/// Match `RHElementActorNPC::GetDirectionVector()` for the directed-panic
/// front-facing test. Original builds the facing vector with
/// `SetSector0to15(direction, ASPECT_RATIO)`, which compresses its Y member
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
        // specifically guards Original's ASPECT_RATIO-compressed facing Y.
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
        ActorData, ActorSoldier, AiBrain, ElementData, ElementKind, HumanData, NpcData, Posture,
        SoldierData,
    };

    fn enemy_soldier() -> Entity {
        let mut enemy_ai = crate::ai_enemy::EnemyAi::default();
        enemy_ai.hth_weapon_id = 1;
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai_brain: AiBrain::Enemy(Box::new(enemy_ai)),
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        })
    }

    #[test]
    fn new_no_door_panic_boundary_closes_recursive_reachpoint() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let npc_id = engine.add_entity(enemy_soldier());
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

        engine.begin_panic_no_door_branch(
            &sim,
            &assets,
            npc_id,
            &request,
            &AiContext::default(),
            false,
        );

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
        let npc_id = engine.add_entity(enemy_soldier());
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

        engine.begin_panic_no_door_branch(
            &sim,
            &LevelAssets::default(),
            npc_id,
            &request,
            &AiContext::default(),
            false,
        );

        let ai = engine.get_entity(npc_id).unwrap().ai_controller().unwrap();
        assert_eq!(ai.current_state, crate::ai::AiState::Fleeing);
        assert_eq!(ai.current_substate, crate::ai::Substate::FleeingPanic);
        assert_eq!(ai.view_alert_status, crate::ai::AlertLevel::Red);
        assert_eq!(ai.current_music_alert_status, crate::ai::AlertLevel::Red);
        assert_eq!(ai.lasting_panic_runs, 11);
        assert!(ai.outbox.reentrant.owner_work.is_empty());
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
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

fn sleeping_enemy_candidates_from_fighter_registry(
    fighters: &[crate::ai_enemy::FighterSnapshot],
) -> Vec<crate::ai::SleepingEnemyInfo> {
    fighters
        .iter()
        .filter(|fighter| !fighter.is_friendly && fighter.is_unconscious && !fighter.is_carried)
        .map(|fighter| crate::ai::SleepingEnemyInfo {
            handle: fighter.handle,
            position: fighter.position,
            is_pc: fighter.is_pc,
            is_robin: fighter.is_robin,
            is_vip: fighter.is_vip,
        })
        .collect()
}

#[cfg(test)]
mod sleeping_enemy_candidate_tests {
    use super::*;

    #[test]
    fn live_registry_keeps_only_unconscious_non_carried_enemies_in_order() {
        let fighter =
            |handle, is_friendly, is_unconscious, is_carried| crate::ai_enemy::FighterSnapshot {
                handle,
                is_friendly,
                is_unconscious,
                is_carried,
                is_pc: true,
                is_robin: handle == 14,
                is_vip: handle == 14,
                ..Default::default()
            };
        let registry = vec![
            fighter(11, false, false, false),
            fighter(12, false, true, true),
            fighter(13, true, true, false),
            fighter(14, false, true, false),
            fighter(15, false, true, false),
        ];

        let candidates = sleeping_enemy_candidates_from_fighter_registry(&registry);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.handle)
                .collect::<Vec<_>>(),
            vec![14, 15]
        );
        assert!(candidates[0].is_pc);
        assert!(candidates[0].is_robin);
        assert!(candidates[0].is_vip);
    }
}

/// Snapshot of a potential detectable human at level-load time.
///
/// Used by [`EngineInner::init_one_ai`] to filter which other humans each
/// NPC should start with in its `detectable_lists[Enemy]` array —
/// the "create list of detectable enemies" pass inside the per-NPC
/// init for both enemy and friendly AI.
#[derive(Debug, Clone, Copy)]
pub(super) struct PotentialDetectable {
    id: EntityId,
    is_pc: bool,
    is_soldier: bool,
    camp: Camp,
}

/// Apply the numeric tail of Original `RHElementActorNPC::GetHearVolume`.
///
/// The distance remainder is truncated to `UWORD` before deafness is tested
/// and subtracted. A positive fractional remainder is therefore inaudible;
/// testing the float first can incorrectly dispatch `EVENT_HEAR` with a
/// zero-volume payload.
fn subjective_hear_volume(modified_volume: f32, distance: f32, deafness: u16) -> u16 {
    let remainder = modified_volume - distance;
    if remainder <= 0.0 {
        return 0;
    }
    let truncated = remainder as u16;
    if truncated <= deafness {
        0
    } else {
        truncated - deafness
    }
}

/// Project Original's live Enemy detectable list into the ordered handle list
/// consumed by `RefreshArrowProtection`.
fn seen_last_frame_detectable_handles(detectables: &[Detectable]) -> Vec<u32> {
    detectables
        .iter()
        .filter(|detectable| detectable.seen_last_frame)
        .filter_map(|detectable| detectable.element.map(EntityId::index))
        .collect()
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
pub(super) fn build_potential_detectables(engine: &EngineInner) -> Vec<PotentialDetectable> {
    let mut out = Vec::new();
    for (id, entity) in engine.world.entities.humans() {
        // Original InitOneAI walks the complete engine element array and
        // tests only IsHuman/IsPC/camp; it does not gate this bootstrap list
        // on RHElement::IsActive (RHartificialmalignity.cpp:10121-10138 and
        // RHartificialbonhomie.cpp:1429-1445). Authored rescue PCs commonly
        // begin inactive but must already occur in every applicable Enemy
        // detectable list so activation does not alter list identity/order.
        match entity {
            Entity::Pc(_) => {
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: true,
                    is_soldier: false,
                    // All PCs are Royalists.
                    camp: Camp::Royalists,
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
                // Civilians are tracked in the snapshot so the `IsFriend`
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
pub(super) fn build_detectable_enemies_for(
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
        // check / AddDetectable class filter).
        let pd_is_civilian = !pd.is_pc && !pd.is_soldier;
        if pd_is_civilian {
            continue;
        }
        let is_detectable = if self_is_civilian {
            // Bonhomie considers Royalist soldiers for Lacklandist
            // civilians in its outer loop, but AddDetectable's civilian arm
            // rejects them. Both civilian camps therefore retain PCs only.
            pd.is_pc
        } else {
            // Malignity (enemy soldier) AddDetectable cases:
            // - Royalist (Good) soldier → detects enemy (Lacklandist) soldiers.
            // - Lacklandist (Evil) soldier → detects good (Royalist) soldiers
            //   AND PCs.
            match self_camp {
                Camp::Royalists => pd.is_soldier && pd.camp == Camp::Lacklandists,
                Camp::Lacklandists => pd.is_pc || (pd.is_soldier && pd.camp == Camp::Royalists),
                Camp::Error => false,
            }
        };
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

/// Preserve `InitializePatrol`'s left-to-right C++ `&&` evaluation.
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

/// Preserve `InitializePatrol`'s insertion-loop comparison exactly.
///
/// Original advances past an existing member only while
/// `new_distance > existing_distance`.  Spelling the stopping condition as
/// `new_distance <= existing_distance` is not equivalent for unordered IEEE
/// values: a NaN distance stops the C++ loop immediately and is inserted at
/// that position.
fn patrol_distance_inserts_before(new_distance: f32, existing_distance: f32) -> bool {
    !(new_distance > existing_distance)
}

/// Preserve `HeyFolksLookThere`'s positive, strict range admission.
///
/// Original sends `CALL_LOOKTHERE` only when `SquareNorm() < radius²`.
/// Rewriting this as an early rejection with `distance² >= radius²` is not
/// equivalent for unordered IEEE values: a soldier with a NaN position must
/// not receive the call.
fn look_there_target_is_inside_radius(distance_squared: f32, radius_squared: f32) -> bool {
    distance_squared < radius_squared
}

/// Original `InitializePatrol` uses AI `Position(actor)` for sorting and
/// formation, but its admission ray calls the actor overload of
/// `IsDetecting360Degrees`. That overload builds both endpoints from the
/// actors' literal stored 3-D positions. In particular, a door-passing member
/// must not substitute the committed gate-side AI position here.
#[allow(clippy::too_many_arguments)]
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

/// Preserve `RefreshPatrol`'s missed-member `&&` evaluation order.
///
/// Original calls `IsDetecting360Degrees` before `IsAbleToHelp` and the AI
/// state check. A visible civilian therefore emits the LOS query even though
/// its default `IsAbleToHelp` implementation rejects re-acquisition.
fn missed_patrol_member_reacquired(
    both_active: bool,
    detect_360: impl FnOnce() -> bool,
    is_able_to_help: bool,
    ai_state: crate::ai::AiState,
) -> bool {
    let detected = both_active && detect_360();
    detected && is_able_to_help && ai_state == crate::ai::AiState::Default
}

/// Original `NearbyCiviliansPanic` asks every active outdoor civilian whether
/// it detects the source. Dead and unconscious civilians are not rejected
/// before `IsDetecting360Degrees`, so they can still emit its LOS query.
fn nearby_panic_civilian_reaches_visibility(active: bool, in_building: bool) -> bool {
    active && !in_building
}

/// Original's money-brawl inline panic sweep uses the civilian's 180-degree
/// detector. Keep LOS lazy so actors outside the forward cone do not emit an
/// obstacle query. The shared `NearbyCiviliansPanic()` callback must not use
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

/// Recreate the still-live `EVENT_DONE` Think frame around the deferred tail
/// of `TowerGuardCallAlert`. Original calls `BattleDecisions` before that
/// handler reaches `EndThink`; Rust releases the AI borrow while the alert's
/// recipient Think calls run and resumes the tail afterward.
fn begin_suspended_tower_guard_alert_think(ai: &mut crate::ai::AiController) {
    ai.think_recursion_depth = ai
        .think_recursion_depth
        .checked_add(1)
        .expect("tower-guard alert suspended Think depth overflow");
}

fn end_suspended_tower_guard_alert_think(ai: &mut crate::ai::AiController) {
    assert!(
        ai.end_think_completion_events(),
        "tower-guard alert suspended Think unexpectedly hit the typed recursion fallback"
    );
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn potential_detectables_include_inactive_authored_pcs() {
        let mut engine = EngineInner::new();
        let add_pc = |engine: &mut EngineInner, active| {
            engine.add_entity(Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorPc,
                    active,
                    ..Default::default()
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            }))
        };
        let inactive = add_pc(&mut engine, false);
        let active = add_pc(&mut engine, true);

        let ids = build_potential_detectables(&engine)
            .into_iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![inactive, active]);
    }

    #[test]
    fn patrol_distance_insertion_preserves_cpp_unordered_and_tie_semantics() {
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
    fn look_there_range_admission_preserves_cpp_nan_and_boundary_semantics() {
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
    fn pending_move_condolation_owns_failure_before_engine_completion_surface() {
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));
        let sequence = engine.orders.sequence_manager.launch_element(
            crate::sequence::SequenceElement::new_movement(
                1,
                crate::element::Command::MoveOk,
                Some(owner),
                crate::order::OrderType::RunningUpright,
            ),
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .orders
            .sequence_manager
            .element_impossible(sequence, 0);
        assert!(
            engine
                .orders
                .sequence_manager
                .has_pending_couldnt_reachpoint_condolation(owner)
        );

        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier has AI");
        ai.completion_latch_inside_think = true;
        ai.couldnt_reachpoint = true;

        engine.surface_synchronous_completion_events_for_owner(owner);

        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI");
        assert!(ai.couldnt_reachpoint);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
        assert!(
            engine
                .orders
                .sequence_manager
                .has_pending_couldnt_reachpoint_condolation(owner),
            "the suspended Original callback must remain next in line"
        );
    }

    #[test]
    fn selected_move_preflight_failure_has_condolation_provenance() {
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));
        let sequence = engine.orders.sequence_manager.launch_element(
            crate::sequence::SequenceElement::new_movement(
                1,
                crate::element::Command::MoveOk,
                Some(owner),
                crate::order::OrderType::RunningUpright,
            ),
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier has AI");
        ai.completion_latch_inside_think = true;
        ai.couldnt_reachpoint = true;

        engine.surface_synchronous_completion_events_for_owner(owner);

        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI");
        assert_eq!(ai.outbox.reentrant.self_stimuli.len(), 1);
        assert_eq!(
            ai.outbox.reentrant.self_stimuli[0].stimulus_type,
            crate::ai::StimulusType::EventCouldntReachPoint
        );
        assert_eq!(
            ai.outbox.reentrant.self_stimuli[0].origin,
            crate::ai::SelfStimulusOrigin::Condolation
        );
    }

    #[test]
    fn suspended_look_there_tail_surfaces_engine_deferred_route_rejection() {
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));
        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier has AI");

        // HeyFolksLookThere resumes its caller tail after the typed Think has
        // unwound. The helper closes synchronous flags now, while a failed
        // gate-route build is reported by the immediately following owner
        // drain. Model that exact split boundary.
        ai.finish_suspended_common_handler();
        ai.couldnt_reachpoint = true;
        engine.surface_synchronous_completion_events_for_owner(owner);

        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI");
        assert_eq!(
            ai.outbox.reentrant.self_stimuli,
            [crate::ai::StimulusType::EventCouldntReachPoint]
        );
        assert_eq!(
            ai.outbox.reentrant.self_stimuli[0].origin,
            crate::ai::SelfStimulusOrigin::EngineCompletion
        );

        // A genuine outside-Think operation still has no EndThink delivery
        // boundary, so the same deferred result must be discarded.
        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier retains AI");
        ai.outbox.reentrant.self_stimuli.clear();
        ai.completion_latch_inside_think = false;
        ai.couldnt_reachpoint = true;
        engine.surface_synchronous_completion_events_for_owner(owner);
        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI");
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    }

    #[test]
    fn engine_deferred_completion_preserves_recursive_think_depth_until_success() {
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));

        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier has AI");
        ai.think_recursion_depth = 1;
        ai.completion_latch_inside_think = true;
        ai.outbox
            .actor
            .orders
            .push(crate::order::AiOrderIntent::new(
                crate::order::OrderType::RunningUpright,
                100.0,
                200.0,
            ));
        assert!(ai.end_think_completion_events());
        ai.outbox.actor.orders.clear();
        ai.couldnt_reachpoint = true;

        engine.surface_synchronous_completion_events_for_owner(owner);
        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier retains AI");
        assert_eq!(
            ai.outbox.reentrant.self_stimuli,
            [crate::ai::StimulusType::EventCouldntReachPoint]
        );
        assert_eq!(ai.think_recursion_depth, 1);
        assert_eq!(ai.open_end_think_frames, 1);

        // Model the recursively dispatched failure issuing a successful
        // replacement GoTo. Its StartThink adds one level; authorization
        // then returns through both retained Original frames.
        ai.outbox.reentrant.self_stimuli.clear();
        ai.think_recursion_depth += 1;
        ai.outbox
            .actor
            .orders
            .push(crate::order::AiOrderIntent::new(
                crate::order::OrderType::RunningUpright,
                300.0,
                400.0,
            ));
        assert!(ai.end_think_completion_events());
        ai.outbox.actor.orders.clear();
        ai.resolve_engine_completion_verdict();
        engine.surface_synchronous_completion_events_for_owner(owner);

        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI");
        assert_eq!(ai.think_recursion_depth, 0);
        assert_eq!(ai.open_end_think_frames, 0);
        assert_eq!(ai.engine_deferred_end_think_frames, 0);
    }

    #[test]
    fn set_state_prefix_boundary_retains_enclosing_end_think_for_tail_route_failure() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));

        {
            let ai = engine
                .world
                .entities
                .get_mut(owner)
                .and_then(Entity::ai_controller_mut)
                .expect("test soldier has AI");
            // Model EventView's EndThink retained across an engine-owned
            // GoNear verdict. SetState has detached that caller-tail intent
            // while its pre-callback effects are being settled.
            ai.think_recursion_depth = 1;
            ai.completion_latch_inside_think = true;
            ai.open_end_think_frames = 1;
            ai.engine_deferred_end_think_frames = 1;
        }

        engine.drain_direct_ai_owner_prefix_boundary_mode(&sim, owner, &assets, false, true);
        {
            let ai = engine
                .world
                .entities
                .get(owner)
                .and_then(Entity::ai_controller)
                .expect("test soldier retains AI");
            assert!(ai.completion_latch_inside_think);
            assert_eq!(ai.think_recursion_depth, 1);
            assert_eq!(ai.open_end_think_frames, 1);
        }

        // The restored caller-tail GoNear now fails gate construction. This
        // is the real Original EndThink surface and must recursively deliver
        // EVENT_COULDNT_REACHPOINT instead of discarding it as outside-Think.
        engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier retains AI")
            .couldnt_reachpoint = true;
        engine.surface_synchronous_completion_events_for_owner(owner);
        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI after failure surface");
        assert_eq!(
            ai.outbox.reentrant.self_stimuli,
            [crate::ai::StimulusType::EventCouldntReachPoint]
        );
    }

    #[test]
    fn detached_goto_tail_does_not_turn_an_absent_verdict_into_success() {
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));

        {
            let ai = engine
                .world
                .entities
                .get_mut(owner)
                .and_then(Entity::ai_controller_mut)
                .expect("test soldier has AI");
            // Model EndThink after it retained a GoTo whose caller tail is
            // temporarily held outside the controller by a nested SetState
            // drain. No visible order and no failure is not a verdict.
            ai.think_recursion_depth = 1;
            ai.open_end_think_frames = 1;
            ai.engine_deferred_end_think_frames = 1;
            ai.completion_latch_inside_think = true;
        }

        engine.surface_synchronous_completion_events_for_owner(owner);
        {
            let ai = engine
                .world
                .entities
                .get(owner)
                .and_then(Entity::ai_controller)
                .expect("test soldier retains AI");
            assert_eq!(ai.think_recursion_depth, 1);
            assert_eq!(ai.open_end_think_frames, 1);
            assert_eq!(ai.engine_deferred_end_think_frames, 1);
            assert!(ai.completion_latch_inside_think);
        }

        // Once the engine actually consumes the restored order, a successful
        // authorization closes the retained Original frame.
        engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier retains AI")
            .resolve_engine_completion_verdict();
        engine.surface_synchronous_completion_events_for_owner(owner);
        let ai = engine
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .expect("test soldier retains AI after success");
        assert_eq!(ai.think_recursion_depth, 0);
        assert_eq!(ai.open_end_think_frames, 0);
        assert_eq!(ai.engine_deferred_end_think_frames, 0);
        assert!(!ai.completion_latch_inside_think);
    }

    #[test]
    fn suspended_tower_guard_alert_tail_owns_deferred_route_rejection() {
        let mut engine = EngineInner::new();
        let mut soldier = crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(Entity::Soldier(soldier));
        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier has AI");

        begin_suspended_tower_guard_alert_think(ai);
        ai.go_to(
            crate::ai::Position {
                x: 100.0,
                y: 200.0,
                ..Default::default()
            },
            crate::ai::GotoFlags::RUN,
            &crate::ai::AiContext::default(),
        );
        assert_eq!(ai.think_recursion_depth, 1);
        assert!(ai.completion_latch_inside_think);

        // Path construction is engine-owned and can reject only after the
        // resumed BattleDecisions borrow has ended. The suspended outer
        // Think must still own and surface that result.
        ai.couldnt_reachpoint = true;
        engine.surface_synchronous_completion_events_for_owner(owner);
        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::ai_controller_mut)
            .expect("test soldier retains AI");
        assert_eq!(
            ai.outbox.reentrant.self_stimuli,
            [crate::ai::StimulusType::EventCouldntReachPoint]
        );
        end_suspended_tower_guard_alert_think(ai);
        assert_eq!(ai.think_recursion_depth, 0);

        // Ordinary direct BattleDecisions calls do not gain completion
        // ownership merely because this specific tower-guard tail does.
        ai.outbox.reentrant.self_stimuli.clear();
        ai.go_to(
            crate::ai::Position {
                x: 300.0,
                y: 400.0,
                ..Default::default()
            },
            crate::ai::GotoFlags::RUN,
            &crate::ai::AiContext::default(),
        );
        assert!(!ai.completion_latch_inside_think);
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
        assert_eq!(queries[0].destination, [1022.0, 2074.301, 52.101158]);
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
            element: crate::element::ElementData {
                active: true,
                ..Default::default()
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
                .map(|(index, _)| crate::gate::DoorIndex(index as u32))
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
        level.sectors[0].gate_indices = vec![crate::gate::DoorIndex(0), crate::gate::DoorIndex(1)];
        let mut duplicate = level.sectors[0].clone();
        duplicate.gate_indices = vec![crate::gate::DoorIndex(2), crate::gate::DoorIndex(3)];
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
    fn seen_last_frame_enemy_projection_preserves_detectable_order() {
        let detectable = |element, seen_last_frame| Detectable {
            element,
            detectable_type: DetectableType::Enemy,
            seen_last_frame,
            ..Detectable::default()
        };
        let detectables = vec![
            detectable(Some(EntityId::Pc(PcId(12))), true),
            detectable(Some(EntityId::Soldier(SoldierId(7))), false),
            detectable(None, true),
            detectable(Some(EntityId::Soldier(SoldierId(3))), true),
        ];

        assert_eq!(
            seen_last_frame_detectable_handles(&detectables),
            vec![12, 3]
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
/// ported — log emission is the equivalent.
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
/// `RHActor::IsPassingDoor`.
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
/// element. `RHArtificialIntelligence::Position` reads these fields from the
/// sequence element itself; unlike `ForecastDestinationForIA`, it does not
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
    // ForecastDestinationForIA gates the serialized GetDoor() pointer on
    // IsPassingDoor(), then uses the independent mbPassingDoorDirectly latch
    // for the destination side. A legacy save restores the selected PassDoor
    // element and both serialized actor fields even though Rust's runtime-only
    // ActiveDoorPass choreography is not reconstructed.
    let live_door = entity.position_iface().get_door();
    let door_pass = (is_passing_door && !live_door.is_null()).then_some((
        crate::gate::DoorIndex(live_door.0),
        actor.passing_door_directly,
    ));
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

/// Map the position returned by Original AI `Position(actor)` during
/// SeekArea's nearby-friend scan. A PassDoor sequence reports its committed
/// destination side even while the sprite is still interpolating along the
/// door rail.
fn seek_area_friend_position_map(
    raw_position: MapPoint,
    door_pass: Option<(crate::gate::DoorIndex, bool)>,
    doors: &[crate::gate::Door],
) -> MapPoint {
    let Some((door_index, direct)) = door_pass else {
        return raw_position;
    };
    let door = doors
        .get(usize::from(door_index))
        .unwrap_or_else(|| panic!("SeekArea friend references missing door {}", door_index.0));
    if direct {
        door.point_in
    } else {
        door.point_out
    }
}

/// Resolve one soldier's contribution to Original `SeekArea`'s nearby-NPC
/// scan. Both `Position(...)` calls are owned by their currently selected
/// PassDoor movement elements, not the sprites' runtime door-pass choreography
/// latches.
fn seek_area_friend_contribution(
    sequence_manager: &crate::sequence::SequenceManager,
    friend_id: EntityId,
    owner_id: EntityId,
    owner_raw_position: MapPoint,
    friend_raw_position: MapPoint,
    doors: &[crate::gate::Door],
    friend_seeks_with_help: bool,
) -> Option<bool> {
    let owner_selected_door = selected_pass_door_movement(sequence_manager, owner_id)
        .map(|(door_index, direction)| (door_index, direction != 0));
    let owner_position =
        seek_area_friend_position_map(owner_raw_position, owner_selected_door, doors);
    let selected_door_pass = selected_pass_door_movement(sequence_manager, friend_id)
        .map(|(door_index, direction)| (door_index, direction != 0));
    let friend_position =
        seek_area_friend_position_map(friend_raw_position, selected_door_pass, doors);
    let delta = friend_position - owner_position;
    (delta.x * delta.x + delta.y * delta.y < 500.0 * 500.0).then_some(friend_seeks_with_help)
}

#[cfg(test)]
mod seek_area_friend_position_tests {
    use super::*;
    use crate::element::{Command, EntityIdKind};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceManager};

    #[test]
    fn door_passing_friend_uses_committed_side_for_radius_gate() {
        let owner = MapPoint::new(1155.7197, 1421.6211);
        let raw_friend = MapPoint::new(727.0, 1168.0);
        let doors = [crate::gate::Door {
            point_out: MapPoint::new(718.0, 1179.0),
            point_in: MapPoint::new(735.0, 1156.0),
            ..Default::default()
        }];
        let distance_squared = |point: MapPoint| {
            let delta = point - owner;
            delta.x * delta.x + delta.y * delta.y
        };

        assert!(distance_squared(raw_friend) < 500.0 * 500.0);
        let indirect = seek_area_friend_position_map(
            raw_friend,
            Some((crate::gate::DoorIndex(0), false)),
            &doors,
        );
        assert_eq!(indirect, doors[0].point_out);
        assert!(distance_squared(indirect) >= 500.0 * 500.0);

        let direct = seek_area_friend_position_map(
            raw_friend,
            Some((crate::gate::DoorIndex(0), true)),
            &doors,
        );
        assert_eq!(direct, doors[0].point_in);
        assert_eq!(
            seek_area_friend_position_map(raw_friend, None, &doors),
            raw_friend
        );
    }

    #[test]
    fn selected_pass_door_controls_seek_area_friend_count_and_help() {
        let owner = MapPoint::new(1155.7197, 1421.6211);
        let raw_friend = MapPoint::new(727.0, 1168.0);
        let friend = EntityId::new(2, EntityIdKind::Soldier);
        let doors = [crate::gate::Door {
            point_out: MapPoint::new(718.0, 1179.0),
            point_in: MapPoint::new(735.0, 1156.0),
            ..Default::default()
        }];

        let mut sequences = SequenceManager::new();
        let mut pass = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(friend),
            OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            panic!("PassDoor test element changed kind")
        };
        *gate_id = Some(crate::gate::DoorIndex(0));
        *direction = 0;
        let sequence_id = sequences.launch_element(pass);
        sequences.element_in_progress(sequence_id, 0);

        // Raw friend coordinates are inside the radius, but the selected
        // indirect PassDoor destination is outside. Pre-fix production used
        // the absent runtime ActiveDoorPass latch and returned `Some(true)`.
        assert_eq!(
            seek_area_friend_contribution(
                &sequences,
                friend,
                EntityId::new(1, EntityIdKind::Soldier),
                owner,
                raw_friend,
                &doors,
                true,
            ),
            None
        );

        let SequenceElementData::Movement { direction, .. } = &mut sequences
            .get_element_mut(sequence_id, 0)
            .expect("selected PassDoor exists")
            .data
        else {
            panic!("PassDoor test element changed kind")
        };
        *direction = 1;
        assert_eq!(
            seek_area_friend_contribution(
                &sequences,
                friend,
                EntityId::new(1, EntityIdKind::Soldier),
                owner,
                raw_friend,
                &doors,
                true,
            ),
            Some(true)
        );

        // Without a selected PassDoor, stale/runtime door state is irrelevant
        // and the raw nearby position contributes to both aggregate outputs.
        sequences.element_terminated(sequence_id, 0);
        let contributions = [seek_area_friend_contribution(
            &sequences,
            friend,
            EntityId::new(1, EntityIdKind::Soldier),
            owner,
            raw_friend,
            &doors,
            true,
        )];
        let visible_seeking_friends = contributions.iter().flatten().count();
        let friend_seek_clears_help_flag = contributions.into_iter().flatten().any(|help| help);
        assert_eq!(visible_seeking_friends, 1);
        assert!(friend_seek_clears_help_flag);
    }

    #[test]
    fn selected_pass_door_controls_seek_area_owner_radius_position() {
        let owner = EntityId::new(1, EntityIdKind::Soldier);
        let friend = EntityId::new(2, EntityIdKind::Soldier);
        let raw_owner = MapPoint::new(0.0, 0.0);
        let raw_friend = MapPoint::new(100.0, 0.0);
        let doors = [crate::gate::Door {
            point_out: MapPoint::new(1_000.0, 0.0),
            point_in: MapPoint::new(1_000.0, 0.0),
            ..Default::default()
        }];
        let mut sequences = SequenceManager::new();

        assert_eq!(
            seek_area_friend_contribution(
                &sequences, friend, owner, raw_owner, raw_friend, &doors, false,
            ),
            Some(false),
            "raw owner position keeps the friend inside the 500-unit radius"
        );

        let mut pass = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        );
        let SequenceElementData::Movement {
            gate_id, direction, ..
        } = &mut pass.data
        else {
            panic!("PassDoor test element changed kind")
        };
        *gate_id = Some(crate::gate::DoorIndex(0));
        *direction = 1;
        let sequence_id = sequences.launch_element(pass);
        sequences.element_in_progress(sequence_id, 0);

        assert_eq!(
            seek_area_friend_contribution(
                &sequences, friend, owner, raw_owner, raw_friend, &doors, false,
            ),
            None,
            "Original Position(owner) publishes the committed PassDoor side"
        );
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
#[allow(clippy::too_many_arguments)]
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
) -> AiContext {
    let elem = entity.element_data();
    let original_creation_order =
        context_original_creation_order(elem.index_in_elements_list as u32, entity_views);
    let camp = match entity {
        Entity::Soldier(s) => s.soldier.cached_camp,
        Entity::Civilian(c) => c.civilian.cached_camp,
        _ => crate::element::Camp::default(),
    };
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
    let remaining_arrows = match entity {
        Entity::Soldier(s) => s.npc.number_of_arrows,
        _ => 0,
    };
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
    let self_is_soldier = matches!(entity, Entity::Soldier(_));
    // `self_is_rider` is the cached `SoldierData.rider` flag from the
    // soldier profile, set at level load.  Non-soldier NPCs are never
    // riders.
    let self_is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
    // `self_rank` / `self_pride` are the soldier's profile rank and
    // pride, used by the bored-time picker for longer officer/pride
    // bored intervals.  `ProfileRank::None` for non-soldiers makes the
    // officer check fall through.
    let (self_rank, self_pride) = match entity {
        Entity::Soldier(s) => {
            let rank = s
                .npc
                .ai_brain
                .enemy()
                .map(|e| e.soldier_profile_rank)
                .unwrap_or(crate::profiles::ProfileRank::None);
            let pride = s
                .npc
                .ai_brain
                .enemy()
                .map(|e| e.soldier_profile_pride)
                .unwrap_or(0);
            (rank, pride)
        }
        _ => (crate::profiles::ProfileRank::None, 0),
    };
    // Number of detectables of type Friend — the
    // `return_to_duty_common_stuff` guard uses this to decide whether
    // to clear the stashed detected body.
    let self_detectable_friend_count = entity
        .npc_data()
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
        .npc_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::MissedFriend as usize)
        })
        .map(|lst| lst.len() as u16)
        .unwrap_or(0);
    let self_seen_enemy_handles = entity
        .npc_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::Enemy as usize)
        })
        .into_iter()
        .flatten()
        .filter(|detectable| detectable.seen_now)
        .filter_map(|detectable| detectable.element.map(|target| target.index()))
        .collect();
    // RHElementActor::GetAnimation() reads the actor's current order, not the
    // sprite's background animation. In particular, GetBored can play a
    // WAITING_UPRIGHT_BORED sprite while the authoritative actor order remains
    // WAITING_UPRIGHT; GoTo's close-point shortcut must still recognize that
    // idle order and synchronously advance the patrol waypoint.
    //
    // `installed_order` is Rust's exact current-order pointer mirror. A null
    // pointer is the NonanimationEnd sentinel; sequence selection and the
    // visible sprite are not substitutes for an installed mpOrder.
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
        .npc_data()
        .and_then(|npc| npc.ai_brain.enemy())
        .is_some_and(|enemy| enemy.forced_attentive);
    let self_view_radius = entity
        .npc_data()
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
        .npc_data()
        .map(|npc| npc.stare_point)
        .unwrap_or_else(|| {
            crate::coordinates::GroundPoint::from_map_and_z(elem.position_map(), elem.position().z)
        });
    let self_view_direction = entity
        .npc_data()
        .map(|npc| npc.view_direction)
        .unwrap_or_else(|| {
            let (x, y) = crate::ai_vision::sector_to_forward(elem.direction());
            [x, y]
        });
    let self_real_half_aperture = entity
        .npc_data()
        .map(|npc| npc.real_half_aperture)
        .unwrap_or(crate::ai_vision::NORMAL_HALF_APERTURE);
    let self_eye_status = entity
        .npc_data()
        .map(|npc| npc.eye_status)
        .unwrap_or_default();
    // `RHArtificialIntelligence::Position(mpMe)` uses the committed gate
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
    AiContext {
        difficulty,
        original_creation_order,
        position: self_position,
        self_layer: elem.layer(),
        self_body_position_world: elem.position(),
        frame,
        direction: elem.direction() as u16,
        posture: elem.posture,
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
        self_is_unconscious: entity.human_data().is_some_and(|human| human.unconscious),
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

/// Look up the live metadata for an enemy's `primary_target` from the
/// engine entity table. Returns `(position, posture, current
/// animation, optional carrier position when the target is on
/// another entity's shoulders)`. Used by the per-tick caller to
/// populate [`AiPerTickData::primary_target_position`] and its
/// siblings so [`EnemyAi::reconsider_enemy_approach`] sees the live
/// target's position, posture, and current order.
///
/// Returns `None` when `target_id` is zero (unassigned target) or the
/// target slot is vacant. The caller should leave the tick fields
/// `None`/`false` in that case — `reconsider_enemy_approach` falls
/// back to the stored `seek_position`.
type PrimaryTargetMetadata = (
    crate::ai::Position,
    crate::element::Posture,
    Option<crate::order::OrderType>,
    Option<crate::ai::Position>,
    Option<crate::ai::HumanHandle>,
);

pub(super) struct AiPositionResolution {
    /// Target's own position after the door-first arm, before an optional
    /// carried-PC substitution.
    pub(super) target: crate::ai::Position,
    /// Final `RHArtificialIntelligence::Position(entity)` result.
    pub(super) effective: crate::ai::Position,
    pub(super) carrier: Option<crate::ai::Position>,
    pub(super) carrier_handle: Option<crate::ai::HumanHandle>,
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
        let door = doors.get(gate_id.0 as usize).unwrap_or_else(|| {
            panic!(
                "AI position target {target_id:?} references missing door {}",
                gate_id.0
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
            carrier: None,
            carrier_handle: None,
        };
    }

    let target_position = position_of(target_id);
    let carrier_id = match target {
        Entity::Pc(pc) if pc.element.posture == crate::element::Posture::OnShoulders => {
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
        carrier,
        carrier_handle: carrier_id.map(crate::element::EntityId::index),
    }
}

pub(super) fn lookup_primary_target_metadata(
    engine: &EngineInner,
    target_id: crate::element::EntityId,
) -> Option<PrimaryTargetMetadata> {
    if target_id.index() == 0 {
        return None;
    }
    let target = engine.world.entities.get(target_id)?;
    let elem = target.element_data();
    let resolved = resolve_ai_position_with(
        &engine.world.entities,
        engine.script_domains.interactables.doors.as_slice(),
        &engine.orders.sequence_manager,
        target_id,
        |id| {
            let element = engine
                .world
                .entities
                .get(id)
                .unwrap_or_else(|| panic!("AI metadata position owner {id:?} disappeared"))
                .element_data();
            crate::ai::Position {
                x: element.position_map().x,
                y: element.position_map().y,
                sector: ai_view_position_sector(engine, element),
                level: element.layer(),
            }
        },
    );
    let posture = elem.posture;
    // Orders live on the target's owning `SequenceElement.orders` —
    // look up the current in-progress element for the target actor.
    let animation = engine
        .orders
        .sequence_manager
        .current_order_for_actor(target_id)
        .map(|(_, _, o)| o.order_type);
    Some((
        resolved.target,
        posture,
        animation,
        resolved.carrier,
        resolved.carrier_handle,
    ))
}

/// Build the list of same-camp friend candidates for the target-swap
/// heuristic in `ReconsiderEnemyApproach`.
///
/// Only soldiers currently in one of the approach substates
/// (`ATTACKING_RUNNING_TO_ENEMY`, `ATTACKING_WALKING_TO_ENEMY`,
/// `ATTACKING_CHARGING_ENEMY`) with a live primary target are
/// eligible.
pub(super) fn build_friend_swap_candidates(
    entities: &Entities,
    doors: &[crate::gate::Door],
    sequence_manager: &crate::sequence::SequenceManager,
    me_id: impl Into<crate::element::EntityId>,
    my_camp: crate::element::Camp,
) -> Vec<crate::ai::FriendSwapCandidate> {
    let me_id = me_id.into();
    let mut out = Vec::new();
    for (friend_id, s) in entities.soldiers() {
        if friend_id == me_id {
            continue;
        }
        if s.soldier.cached_camp != my_camp {
            continue;
        }
        let substate = s.npc.ai_substate();
        if !matches!(
            substate,
            crate::ai::Substate::AttackingRunningToEnemy
                | crate::ai::Substate::AttackingWalkingToEnemy
                | crate::ai::Substate::AttackingChargingEnemy
        ) {
            continue;
        }
        let friend_target_handle = match s
            .npc
            .ai_brain
            .base()
            .map(|ai| ai.primary_target)
            .unwrap_or(0)
        {
            0 => continue,
            h => h,
        };
        let Some(friend_target_id) = entities.id_at_legacy_slot(friend_target_handle) else {
            continue;
        };
        let Some(_friend_target_entity) = entities.get(friend_target_id) else {
            continue;
        };
        let resolve_position = |position_owner| {
            resolve_ai_position_with(
                entities,
                doors,
                sequence_manager,
                position_owner,
                |position_id| {
                    let element = entities
                        .get(position_id)
                        .unwrap_or_else(|| {
                            panic!("friend-swap position owner {position_id:?} disappeared")
                        })
                        .element_data();
                    crate::ai::Position {
                        x: element.position_map().x,
                        y: element.position_map().y,
                        sector: element.sector(),
                        level: element.layer(),
                    }
                },
            )
            .effective
        };
        // Original resolves both Position(pFriend) and
        // Position(pFriend->GetPrimaryTarget()) here. Each therefore uses a
        // committed gate endpoint while passing a door; an on-shoulders PC
        // target resolves to its carrier after that door-first arm.
        let friend_pos = resolve_position(friend_id.into());
        let friend_target_pos = resolve_position(friend_target_id);
        out.push(crate::ai::FriendSwapCandidate {
            friend_id: friend_id.into(),
            friend_position: friend_pos,
            friend_primary_target: friend_target_handle,
            friend_primary_target_position: friend_target_pos,
        });
    }
    out
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

    // Original `GetAvengerOnTheRoofWaitPosition` calls AI `Position(...)`
    // for both actors (RHartificialmalignity.cpp:20231-20242). That virtual
    // position commits a selected PassDoor actor to the destination gate
    // endpoint before falling back to its sprite coordinates
    // (RHartificialintelligence.cpp:4307-4346).
    let resolve_position = |id| {
        resolve_ai_position_with(entities, doors, sequence_manager, id, |position_id| {
            let element = entities
                .get(position_id)
                .unwrap_or_else(|| panic!("roof-wait position owner {position_id:?} disappeared"))
                .element_data();
            crate::ai::Position {
                x: element.position_map().x,
                y: element.position_map().y,
                sector: element.sector(),
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

/// Build a `MyExitDoorInfo` snapshot from the AI's stashed
/// `my_door_index`.  Strict semantics: returns `None` when no door has
/// been stashed upstream.  The stash is set by paths that explicitly
/// choose an exit door (MerryMan flee, RunAndAlertSoldiers); a
/// directly-invoked indoor AlertSoldiers without an upstream stash
/// refuses to project gather slots.
pub(super) fn build_my_exit_door_info(
    stashed_index: Option<u32>,
    doors: &[crate::gate::Door],
) -> Option<crate::ai::MyExitDoorInfo> {
    use crate::ai::MyExitDoorInfo;
    let idx = stashed_index?;
    let door = doors.get(idx as usize)?;
    let sector_out = crate::position_interface::SectorHandle::new(u16::from(door.sector_out));
    let position_out = crate::ai::Position {
        x: door.point_out.x,
        y: door.point_out.y,
        sector: sector_out,
        level: door.layer_out,
    };
    Some(MyExitDoorInfo {
        point_out: door.point_out,
        point_mid: door.point_mid,
        layer_out: door.layer_out,
        sector_out,
        position_out,
    })
}

/// Build the per-tick [`SharedAiEntityViews`] map from the live
/// entity store.
///
/// Called by [`EngineInner::build_sim_scratch`] at the start of each
/// AI dispatch pass so the map reflects current entity
/// positions / states. Includes every PC, soldier, civilian, and active
/// object-hierarchy entity. Human views include inactive actors because normal
/// `IsDetecting(human)` ignores activity in its same-building arm; inactive
/// objects remain excluded.
pub(super) fn build_entity_views(engine: &EngineInner) -> AiEntityViewMap {
    build_entity_views_and_stamps(engine).0
}

fn build_entity_views_without_forecast(engine: &EngineInner) -> AiEntityViewMap {
    build_entity_views_and_stamps(engine).0
}

fn entity_views_nets_generation(engine: &EngineInner) -> u64 {
    engine.world.entities.nets().fold(0_u64, |stamp, (id, _)| {
        stamp.rotate_left(7) ^ u64::from(id.index()) ^ engine.world.entities.generation(id)
    })
}

fn pc_in_coma_for_view(engine: &EngineInner, entity: &Entity) -> bool {
    let Entity::Pc(pc) = entity else {
        return false;
    };
    let description_index = pc
        .pc
        .campaign_description_index
        .unwrap_or_else(|| panic!("live PC is missing its required campaign-description identity"));
    engine
        .mission_domain
        .campaign
        .characters
        .get(description_index as usize)
        .unwrap_or_else(|| {
            panic!(
                "live PC campaign-description index {description_index} is outside the campaign character table"
            )
        })
        .status
        .in_coma
}

fn entity_view_stamp(
    engine: &EngineInner,
    entity_id: EntityId,
    entity: &Entity,
    nets_generation: u64,
) -> AiEntityViewStamp {
    let position_dependency_generation = match entity {
        Entity::Pc(pc) if pc.element.posture == crate::element::Posture::OnShoulders => pc
            .human
            .carrier
            .map(|carrier| engine.world.entities.generation(carrier))
            .unwrap_or_else(|| {
                panic!("on-shoulders PC {entity_id:?} has no carrier for AI Position")
            }),
        _ => 0,
    };
    AiEntityViewStamp {
        entity_generation: engine.world.entities.generation(entity_id),
        position_dependency_generation,
        current_animation: engine
            .live_actor_animation(entity_id)
            .unwrap_or(crate::order::OrderType::NonanimationEnd),
        selected_door: matches!(
            entity,
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
        )
        .then(|| selected_pass_door_movement(&engine.orders.sequence_manager, entity_id))
        .flatten(),
        building_sector: engine.entity_building_sector(entity.element_data().sector()),
        in_coma: pc_in_coma_for_view(engine, entity),
        nets_generation,
    }
}

fn build_one_entity_view(
    engine: &EngineInner,
    doors_ref: &[crate::gate::Door],
    nets_by_victim: &mut std::collections::HashMap<u32, Vec<ai_entity_view::NetCoverInfo>>,
    entity_id: EntityId,
    entity: &Entity,
    stamp: AiEntityViewStamp,
) -> ai_entity_view::AiEntityView {
    let mut view = ai_entity_view::entity_view_from_entity(
        entity,
        engine.world.original_creation_order(entity_id),
        stamp.building_sector.is_some(),
        stamp.building_sector,
        Some(&engine.mission_domain.campaign),
        stamp.current_animation,
    );

    if matches!(
        entity,
        Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
    ) {
        view.position = resolve_ai_position_with_selected(
            &engine.world.entities,
            doors_ref,
            entity_id,
            stamp.selected_door,
            |position_id| {
                let position_element = engine
                    .world
                    .entities
                    .get(position_id)
                    .unwrap_or_else(|| {
                        panic!("AI entity-view position owner {position_id:?} disappeared")
                    })
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

    if view.stuck_under_net
        && let Some(nets) = nets_by_victim.remove(&entity_id.index())
    {
        view.covering_nets = nets;
    }

    if matches!(
        entity,
        Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
    ) && let Some(input) = extract_forecast_input(entity, stamp.selected_door.is_some())
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

/// Preserve the `RHSector*` carried by Original's `RHElement::GetPosition`
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
            _ => panic!(
                "AI Position sector {public} at {point:?} has no containing sector and is ambiguous in the exact arena"
            ),
        },
        _ => panic!("AI Position sector {public} at {point:?} is ambiguous in the exact arena"),
    };
    let index = crate::fast_find_grid::SectorIndex::new(index as u32)
        .expect("AI Position exact sector index exceeds the arena range");
    Some(sector.with_arena_index(index))
}

#[cfg(test)]
mod ai_view_position_sector_tests {
    use super::*;
    use crate::coordinates::{MapBBox, MapPoint};
    use crate::fast_find_grid::{GridSector, SectorIndex};
    use crate::gate::Door;
    use crate::sector::{SectorNumber, SectorType};

    fn square_sector(number: i16, layer: u16, min: f32, max: f32) -> GridSector {
        GridSector {
            points: vec![
                MapPoint::new(min, min),
                MapPoint::new(max, min),
                MapPoint::new(max, max),
                MapPoint::new(min, max),
            ],
            bounding_box: MapBBox::from_coords(min, min, max, max),
            sector_type: SectorType::MOTION | SectorType::AREA,
            layer,
            sector_number: SectorNumber::new(number),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        }
    }

    #[test]
    fn entity_view_recovers_duplicate_public_goal_for_exact_gate_route() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        let wrong = engine
            .world
            .fast_grid_mut()
            .add_sector(square_sector(88, 2, 300.0, 350.0), 2);
        let goal = engine
            .world
            .fast_grid_mut()
            .add_sector(square_sector(88, 2, 100.0, 200.0), 2);
        let source = engine
            .world
            .fast_grid_mut()
            .add_sector(square_sector(77, 2, 10.0, 60.0), 2);
        assert_ne!(wrong, goal);

        let _legacy_null_slot =
            engine.add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorPc,
                    ..Default::default()
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            }));
        let target = engine.add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture: crate::element::Posture::Upright,
                ..Default::default()
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
        let metadata_position = lookup_primary_target_metadata(&engine, target)
            .expect("live primary target metadata exists")
            .0;
        assert_eq!(
            metadata_position
                .sector
                .and_then(|sector| sector.arena_index()),
            SectorIndex::new(goal)
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
            engine.add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorPc,
                    ..Default::default()
                },
                actor: Default::default(),
                human: Default::default(),
                pc: Default::default(),
            }));
        let target = engine.add_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture: crate::element::Posture::Upright,
                ..Default::default()
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
        let metadata_position = lookup_primary_target_metadata(&engine, target)
            .expect("compatibility primary target metadata exists")
            .0;
        assert_eq!(
            metadata_position
                .sector
                .and_then(|sector| sector.arena_index()),
            None
        );
    }

    #[test]
    #[should_panic(expected = "ambiguous in the exact arena")]
    fn duplicate_public_noncontaining_position_does_not_guess_an_identity() {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        engine
            .world
            .fast_grid_mut()
            .add_sector(square_sector(88, 2, 250.0, 300.0), 2);
        engine
            .world
            .fast_grid_mut()
            .add_sector(square_sector(88, 2, 350.0, 400.0), 2);
        let mut element = crate::element::ElementData::default();
        element.set_position_map(MapPoint::new(150.0, 150.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(88));
        let _ = ai_view_position_sector(&engine, &element);
    }
}

fn build_nets_by_victim(
    engine: &EngineInner,
) -> std::collections::HashMap<u32, Vec<ai_entity_view::NetCoverInfo>> {
    let mut nets_by_victim: std::collections::HashMap<u32, Vec<ai_entity_view::NetCoverInfo>> =
        std::collections::HashMap::new();
    for (net_id, net) in engine.world.entities.nets() {
        if !net.element.active || net.net.victims.is_empty() {
            continue;
        }
        let net_pos = net.element.position_map();
        let info = ai_entity_view::NetCoverInfo {
            handle: net_id.index(),
            position: crate::ai::Position {
                x: net_pos.x,
                y: net_pos.y,
                sector: net.element.sector(),
                level: net.element.layer(),
            },
            radius: if net.net.crumpled { 10.0 } else { 40.0 },
        };
        for victim in &net.net.victims {
            nets_by_victim.entry(victim.index()).or_default().push(info);
        }
    }
    nets_by_victim
}

fn build_entity_views_and_stamps(
    engine: &EngineInner,
) -> (
    AiEntityViewMap,
    std::collections::HashMap<u32, AiEntityViewStamp>,
) {
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

    // Pre-scan nets for `compute_nets_covering_me` reverse index:
    // victim entity-id → list of covering nets.  Per-victim loop:
    // iterate every net entity, include those whose `victims` contains
    // the probed human.  Doing it once up-front amortises the scan
    // across every stuck-victim view.
    //
    // Net radius: 10 when crumpled, else 40.
    let mut nets_by_victim = build_nets_by_victim(engine);

    let nets_generation = entity_views_nets_generation(engine);
    let mut map = AiEntityViewMap::with_capacity(engine.world.entities.len());
    let mut stamps = std::collections::HashMap::with_capacity(engine.world.entities.len());
    for (entity_id, entity) in engine.world.entities.occupied() {
        if !entity_has_ai_view(entity) {
            continue;
        }
        let stamp = entity_view_stamp(engine, entity_id, entity, nets_generation);
        let view = build_one_entity_view(
            engine,
            doors_ref,
            &mut nets_by_victim,
            entity_id,
            entity,
            stamp,
        );

        // AI handle == entity slot index (see `FighterSnapshot.handle =
        // target_id.index()` elsewhere, and `self.world.entities.get_mut(target as
        // usize)` for `CrossNpcAction` handlers).
        map.insert(entity_id.index(), view);
        stamps.insert(entity_id.index(), stamp);
    }
    (map, stamps)
}

fn entity_has_ai_view(entity: &Entity) -> bool {
    match entity {
        Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) => true,
        _ => entity.object_data().is_some() && entity.element_data().active,
    }
}

fn refresh_prepared_entity_views(
    engine: &EngineInner,
    cache: &mut PreparedAiEntityViewCache,
) -> usize {
    let _detail =
        super::tick::entity_system_detail_guard(super::tick::EntitySystemDetail::BuildEntityViews);
    if cache.views.is_none() {
        let (entities, stamps) = build_entity_views_and_stamps(engine);
        cache.views = Some(engine.share_ai_entity_views(entities));
        cache.stamps = stamps;
        let rebuilt = cache.stamps.len();
        return rebuilt;
    }

    let doors_ref = engine
        .scripts
        .mission
        .as_ref()
        .map(|_| engine.script_domains.interactables.doors.as_slice())
        .unwrap_or(&[]);
    let nets_generation = entity_views_nets_generation(engine);
    let mut nets_by_victim = build_nets_by_victim(engine);
    let mut live_slots = vec![false; engine.world.entities.len()];
    let shared = cache
        .views
        .as_mut()
        .expect("prepared AI entity-view cache disappeared");
    let views = std::sync::Arc::get_mut(shared).unwrap_or_else(|| {
        panic!("prepared AI entity views escaped their synchronous owner dispatch")
    });

    let mut rebuilt = 0;
    for (entity_id, entity) in engine.world.entities.occupied() {
        if !entity_has_ai_view(entity) {
            continue;
        }
        let index = entity_id.index();
        live_slots[index as usize] = true;
        let stamp = entity_view_stamp(engine, entity_id, entity, nets_generation);
        if cache.stamps.get(&index) == Some(&stamp) {
            continue;
        }
        let view = build_one_entity_view(
            engine,
            doors_ref,
            &mut nets_by_victim,
            entity_id,
            entity,
            stamp,
        );
        views.entities.insert(index, view);
        cache.stamps.insert(index, stamp);
        rebuilt += 1;
    }
    views
        .entities
        .retain(|index, _| live_slots.get(*index as usize).copied().unwrap_or(false));
    cache
        .stamps
        .retain(|index, _| live_slots.get(*index as usize).copied().unwrap_or(false));
    views.building_authorizations = engine.building_authorizations_for_ai_views();
    rebuilt
}

#[cfg(test)]
mod prepared_entity_view_cache_tests {
    use super::*;
    use crate::coordinates::MapPoint;
    use crate::element::{
        ElementBonus, ElementData, ElementProjectile, ObjectData, ProjectileData,
    };
    use crate::element_kinds::ObjectType;

    fn active_bonus(x: f32) -> Entity {
        let mut element = ElementData {
            active: true,
            ..Default::default()
        };
        element.set_position_map(MapPoint::new(x, 0.0));
        Entity::Bonus(ElementBonus {
            element,
            object: ObjectData::default(),
        })
    }

    fn active_coin_projectile(x: f32) -> Entity {
        let mut element = ElementData {
            active: true,
            ..Default::default()
        };
        element.set_position_map(MapPoint::new(x, 0.0));
        Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: ObjectType::Coin,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        })
    }

    #[test]
    fn unchanged_views_are_reused_and_mutable_slot_access_invalidates_only_that_slot() {
        let mut engine = EngineInner::new();
        let first = engine.add_entity(active_bonus(10.0));
        let second = engine.add_entity(active_bonus(20.0));
        let mut cache = PreparedAiEntityViewCache::default();

        assert_eq!(refresh_prepared_entity_views(&engine, &mut cache), 2);
        assert_eq!(refresh_prepared_entity_views(&engine, &mut cache), 0);

        engine
            .world
            .entities
            .get_mut(first)
            .expect("first bonus")
            .element_data_mut()
            .set_position_map(MapPoint::new(30.0, 0.0));
        assert_eq!(refresh_prepared_entity_views(&engine, &mut cache), 1);
        let views = cache.views.as_ref().expect("cached views");
        assert_eq!(views[&first.index()].position.x, 30.0);
        assert_eq!(views[&second.index()].position.x, 20.0);
    }

    #[test]
    fn active_projectile_coin_is_available_to_ai_object_handle_lookups() {
        let mut engine = EngineInner::new();
        let coin = engine.add_entity(active_coin_projectile(42.0));
        let mut cache = PreparedAiEntityViewCache::default();

        assert_eq!(refresh_prepared_entity_views(&engine, &mut cache), 1);
        let views = cache.views.as_ref().expect("cached views");
        assert_eq!(views[&coin.index()].position.x, 42.0);
        assert_eq!(views[&coin.index()].object_type, ObjectType::Coin);
        assert_eq!(
            views[&coin.index()].entity_id(coin.index()),
            Some(coin),
            "AI object handles must preserve the projectile-derived entity identity"
        );

        engine
            .world
            .entities
            .get_mut(coin)
            .expect("coin projectile")
            .element_data_mut()
            .active = false;
        assert_eq!(refresh_prepared_entity_views(&engine, &mut cache), 0);
        assert!(
            !cache
                .views
                .as_ref()
                .expect("cached views")
                .contains_key(&coin.index())
        );
    }
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
    /// a `GoTo(door_in)` order queued.
    /// Orchestrate a building-wide enemy alert.
    ///
    /// Walks the building's occupant list, splits it into royalists /
    /// lacklandists / civilians, panics the civilians, and — if both
    /// camps are present — stages the outnumbered side to flee the
    /// building while the stronger side pursues
    /// (`init_battle_before_door` follow-on).
    ///
    /// `send_before_door_to_fight` is ported as
    /// [`EngineInner::send_before_door_to_fight`], and the
    /// `init_battle_before_door` orchestration — pick nearest door,
    /// compute defender/attacker positions, fan out
    /// `send_before_door_to_fight` per occupant — is ported as
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
        let occupant_ids = house.occupant_ids.clone();

        // Split occupants into royalists / lacklandists / civilians,
        // skipping dead and unconscious.  PCs count as royalists.
        let mut royalist_ids: Vec<EntityId> = Vec::new();
        let mut lacklandist_ids: Vec<EntityId> = Vec::new();
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
                    match s.soldier.cached_camp {
                        crate::element::Camp::Royalists => royalist_ids.push(eid),
                        crate::element::Camp::Lacklandists => lacklandist_ids.push(eid),
                        _ => {}
                    }
                }
                Entity::Civilian(c) => {
                    if c.npc.life_points <= 0 || c.human.unconscious {
                        continue;
                    }
                    civilian_ids.push(eid);
                }
                Entity::Pc(pc) if pc.pc.life_points > 0 && !pc.human.unconscious => {
                    royalist_ids.push(eid);
                }
                _ => {}
            }
        }

        if building_exit_wait_owner_debug_enabled() {
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
                describe(&occupant_ids),
                describe(&royalist_ids),
                describe(&lacklandist_ids),
                describe(&civilian_ids),
            );
            for &eid in &occupant_ids {
                let Some(entity) = self.world.entities.get(eid) else {
                    continue;
                };
                let detail = match entity {
                    Entity::Soldier(s) => format!(
                        "soldier lp={} unconscious={} camp={:?} posture={:?}",
                        s.npc.life_points,
                        s.human.unconscious,
                        s.soldier.cached_camp,
                        entity.element_data().posture
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

        // No battle unless both camps present.
        if royalist_ids.is_empty() || lacklandist_ids.is_empty() {
            return;
        }

        // Every live civilian panics.
        let panic_runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
        for civ_id in civilian_ids {
            self.process_building_civilian_panic(sim, assets, civ_id, panic_runs);
        }

        // Outnumbered side flees; the stronger side pursues.
        let (fleeing, pursuing) = if royalist_ids.len() > lacklandist_ids.len() {
            (lacklandist_ids, royalist_ids)
        } else {
            (royalist_ids, lacklandist_ids)
        };

        self.init_battle_before_door(sim, assets, &door_indices, &fleeing, &pursuing);

        tracing::debug!(
            source = source.index(),
            building = building_sector_num,
            fleeing = fleeing.len(),
            pursuing = pursuing.len(),
            "EnemyInHouseAlert: civilians panicked, door-battle dispatched"
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
        let scratch = self.build_sim_scratch(sim, assets);
        let mut ctx = {
            let Some(entity) = self.world.entities.get(civ_id) else {
                return;
            };
            let entity_sector = entity.element_data().sector();
            let building_sector = self.entity_building_sector(entity_sector);
            let Some(entity) = self.world.entities.get(civ_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                self.control.frame_counter,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &assets.hiking_waypoint_sectors,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(civ_id, &mut ctx);

        if let Some(Entity::Civilian(c)) = self.world.entities.get_mut(civ_id)
            && let Some(friendly_ai) = c.npc.ai_brain.friendly_mut()
        {
            let was_already_fleeing = matches!(
                friendly_ai.base.current_substate,
                crate::ai::Substate::FleeingPanic | crate::ai::Substate::FleeingRunToDoor
            );
            friendly_ai.base.lasting_panic_runs = runs;
            friendly_ai.base.directed_panic = false;
            friendly_ai.base.current_state = crate::ai::AiState::Fleeing;
            friendly_ai.base.current_substate = crate::ai::Substate::FleeingPanic;
            friendly_ai.base.outbox.actor.begin_panic = Some(crate::ai::PanicRequest {
                center: None,
                runs,
                alert: crate::ai::AlertLevel::Red,
                is_new_panic: !was_already_fleeing,
            });
        }

        // Drain the PanicRequest so a door gets picked and GoTo fires.
        self.process_pending_begin_panic_for(sim, assets, civ_id, &ctx);
        self.refresh_selected_default_wait_identity(civ_id, &mut ctx);
        self.process_pending_panic_seek_fallback_for(sim, assets, civ_id, &ctx);
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
    /// directly to its own `IsDetecting180Degrees` implementation.
    fn brawl_nearby_civilians_panic_exact(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source: EntityId,
    ) {
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
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
                // `IsDetecting180Degrees` checks both actors' raw active
                // flags. The target/source check remains inside the shared
                // detector so its gate ordering stays source-exact.
                if !civilian.element.active {
                    continue;
                }
                let building_sector = self.entity_building_sector(civilian.element.sector());
                build_ai_context_from_entity(
                    self.world
                        .entities
                        .get(npc_id)
                        .expect("civilian disappeared"),
                    self.control.frame_counter,
                    building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
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
            self.dispatch_think_with_drain_without_forecast(
                sim,
                npc_id,
                &stimulus,
                &ctx,
                &AiPerTickData::stub(),
                assets,
            );
        }
    }

    fn nearby_civilians_panic_generic(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        source: EntityId,
    ) {
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let view_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS as f32
        };
        // `nearby_civilians_panic` builds an aspect-ratio-stretched
        // axis-aligned box (radius, radius * ASPECT_RATIO) around
        // self, then walks every NPC asking:
        // The shared callback calls IsDetecting360Degrees. The separate
        // money-brawl completion sweep calls IsDetecting180Degrees. Both use
        // the civilian's upright eye point, the source actor's detection
        // point, the civilian's live view radius, and opaque 3D LOS.
        let radius_y = view_radius * crate::position_interface::ASPECT_RATIO;

        let (source_map, source_ground, source_detection_point) = {
            let Some(entity) = self.world.entities.get(source) else {
                tracing::trace!(target: "parity_nearby_panic", "source missing");
                return;
            };
            // Source must be IsActiveAndOutsideBuilding for
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
                // `GetPositionGround()` is the cached world-space X/Y pair,
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
                // IsDetecting360Degrees(actor) stretched-Y 3D distance
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

            // Build per-civilian AiContext and dispatch EVENT_PANIC.
            let ctx = {
                let Some(entity) = self.world.entities.get(npc_id) else {
                    continue;
                };
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    None,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };

            let stimulus = crate::ai::Stimulus::with_position(
                crate::ai::StimulusType::EventPanic,
                panic_center,
            );
            // Civilian EventPanic: FriendlyAi — no combat tick data
            // consumed, stub is correct.
            let tick_data = AiPerTickData::stub();
            // `NearbyCiviliansPanic` directly calls `pNPC->Think(stimulus)`.
            // Close that recipient's complete owner-local Think boundary:
            // EVENT_PANIC chooses a door and queues GoTo, whose movement
            // element and synchronous path request must exist before the
            // caller resumes. A raw dispatch plus manual PanicRequest drain
            // left the GoTo stranded in the civilian outbox until its next
            // owner slot.
            self.dispatch_think_with_drain_without_forecast(
                sim, npc_id, &stimulus, &ctx, &tick_data, assets,
            );
        }
    }

    /// Re-issue an in-flight patrol `GoTo` so a freshly-changed
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
        let scratch = self.build_sim_scratch(sim, assets);
        // Re-check the gate (state may have changed between the
        // native pushing the deferred command and us draining it).
        let (has_path, substate) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller() else {
                return;
            };
            (ai.has_patrol_path, ai.current_substate)
        };
        if !has_path
            || !matches!(
                substate,
                crate::ai::Substate::DefaultGotoRoute | crate::ai::Substate::DefaultEnroute
            )
        {
            return;
        }

        // Look up the current waypoint position from the level's
        // hiking paths.  Bail if the AI has no patrol path or the
        // waypoint index is out of range — both indicate a desync
        // that the relaunch can't repair on its own.
        let waypoint_position = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller() else {
                return;
            };
            let Some(path) = ai.patrol_path.as_ref() else {
                return;
            };
            let Some(wp) = path.current_waypoint(&assets.hiking_paths) else {
                return;
            };
            crate::ai::Position {
                x: wp.x as f32,
                y: wp.y as f32,
                sector: assets.hiking_waypoint_sector(
                    usize::from(path.hiking_path_index),
                    usize::from(path.current_waypoint_index),
                    wp.sector,
                ),
                level: wp.level,
            }
        };

        // Build the per-tick AiContext for `go_to` (mirrors how the
        // panic / patrol-coordination paths build it).
        let mut ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let entity_sector = entity.element_data().sector();
            let building_sector = self.entity_building_sector(entity_sector);
            build_ai_context_from_entity(
                entity,
                self.control.frame_counter,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &assets.hiking_waypoint_sectors,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);

        // Compute `WillStopAtNextWaypoint` and call `go_to`.
        let Some(entity) = self.world.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };
        let will_stop = ai.will_stop_at_next_waypoint_debug(
            sim,
            &assets.hiking_paths,
            &ctx,
            crate::ai::WillStopCaller::SetPathWalkingFlags,
        );
        let mut flags = ai.default_path_walking_flags;
        if !will_stop {
            flags |= crate::ai::GotoFlags::DONT_STOP;
        }
        ai.go_to(waypoint_position, flags, &ctx);

        // SetPathWalkingFlags calls GoTo directly inside the script native.
        // Promote that exact owner's intent now instead of leaving it for the
        // next frame's global pending-order pass. The enclosing script driver
        // subsequently drains the resulting deferred InstructOwner action
        // with the still-active VM stack, so the replacement transition is
        // constructed from this call frame's position just like Original.
        self.launch_pending_orders_for_npc(sim, assets, npc_id);
        let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
    }

    /// Drain a queued [`PanicRequest`] on a single NPC.
    ///
    /// Called right after any `FriendlyAi::think` that could have
    /// pushed a panic request (the civilian EVENT_PANIC /
    /// EVENT_VIEW-from-swordfighting-soldier handlers).  The `panic`
    /// door-search + GoTo fall back:
    ///
    ///  * Walk `ai_global.door_seek_infos` for a `Building` door in a
    ///    *different* building from the actor, authorised for the
    ///    actor, and — when `directed` — pointing *away* from the
    ///    panic center.  Apply +500 sector-change / +300 layer-change
    ///    malus to the `MaxNorm` distance and pick the minimum.
    ///  * If found → `Substate::FleeingRunToDoor`, reset
    ///    `lasting_panic_runs`, issue a running `GoTo(door_in)` via
    ///    the AI base's `go_to` helper.
    ///  * If not found → stay in `Substate::FleeingPanic`, bump
    ///    `lasting_panic_runs` to `runs + 1`, and fire a self
    ///    `EventReachPoint` so the `think_expected_event_common_stuff`
    ///    panic-run branch picks a random escape vector next tick.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_begin_panic_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
        ctx: &crate::ai::AiContext,
    ) {
        let think_debug = self.debug_think_stimulus_matches(npc_id);
        if think_debug {
            eprintln!(
                "THINK_STIMULUS phase=before_panic_launch frame={} owner={} creation_order={} rng_cursor={:?}",
                self.control.frame_counter,
                npc_id.index(),
                self.world.original_creation_order(npc_id),
                self.control.rng.original_replay_cursor(),
            );
        }
        // Peel the request off the AI base.
        let Some(entity) = self.world.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };
        let Some(request) = ai.outbox.actor.begin_panic.take() else {
            return;
        };

        // Resolve the actor's current building for the
        // "not this building" filter used by `GetNearestDoor`.
        let my_building = ctx.in_building.then_some(ctx.building_sector).flatten();
        // `GetNearestDoor` mixes three different views of "where I am"
        // (`original-code/RHartificialintelligence.cpp:5494-5560`):
        //
        //  * `ptMe = mpMe->GetPositionMap()` — the RAW element position, used
        //    only to build `vMeToDoor` for the flee-direction dot product
        //    (`:5511`, `:5524`).
        //  * `Position( mpMe )` — the AI position, which SNAPS to the gate's
        //    destination endpoint while a door pass is committed.  It is the
        //    other side of that dot product and the origin of the scored
        //    distance vector (`:5524`, `:5533`).
        //  * `mpMe->GetSector()` / `mpMe->GetLayer()` — the LIVE element
        //    sector and layer, which during a door pass still name the side
        //    the actor physically stands on.  They decide the +500 / +300
        //    malus (`:5536`, `:5541`).
        //
        // Reading `ctx.position` for all three collapses the distinction and
        // mis-scores every door while the actor straddles a gate.
        let (raw_map_position, my_sector, my_layer) = {
            let elem = self.expect_entity(npc_id, "door-seek owner").element_data();
            (elem.position_map(), elem.sector(), elem.layer())
        };
        let actor_auth = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("panic requester {npc_id:?} disappeared"))
            .actor_auth_info();

        // Pre-compute the set of house sector indices that contain a
        // PC (the `dangerous_house` set).  Snapshot it here so the
        // `pick_door` closure doesn't need to borrow `self.world.entities`
        // (which is re-borrowed mutably after door selection).
        let dangerous_house_sectors: std::collections::HashSet<u32> =
            if ctx.camp == crate::element::Camp::Lacklandists {
                self.ai
                    .global
                    .houses
                    .iter()
                    .filter(|h| {
                        h.occupant_ids.iter().any(|&eid| {
                            matches!(
                                self.world.entities.get(eid),
                                Some(crate::element::Entity::Pc(_))
                            )
                        })
                    })
                    .map(|h| h.sector_index)
                    .collect()
            } else {
                std::collections::HashSet::new()
            };
        let authorized_building_doors: std::collections::BTreeSet<crate::gate::DoorIndex> = self
            .script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
            .filter_map(|(index, door)| {
                (door.door_type == crate::gate::DoorType::Building
                    && door.is_actor_authorized(
                        true,
                        &actor_auth,
                        self.building_sector_is_authorized(door.sector_in),
                        false,
                    ))
                .then_some(crate::gate::DoorIndex(index as u32))
            })
            .collect();

        // Pick the best door.  `directed` gates the dot-product
        // filter: when a panic center is known, first try to find a
        // door in the "away" half-plane; if none exists, fall back to
        // an undirected lookup (clearing `directed_panic`).
        let pick_door = |door_seek_infos: &[crate::ai::DoorSeekInfo],
                         directed: bool|
         -> Option<(crate::ai::Position, u32)> {
            let mut best: Option<(crate::ai::Position, u32)> = None;
            for door in door_seek_infos {
                if !matches!(door.door_type, crate::gate::DoorType::Building) {
                    continue;
                }
                if !authorized_building_doors.contains(&door.door_index) {
                    continue;
                }
                if my_building == crate::position_interface::SectorHandle::new(door.sector_in) {
                    continue;
                }
                // Flee-direction test: `vMeToDoor` is measured from the RAW
                // map position, the flee vector from the AI position
                // (`original-code/RHartificialintelligence.cpp:5511`, `:5524`).
                if directed && let Some(center) = request.center {
                    let dx_door = door.point_out.x - raw_map_position.x;
                    let dy_door = door.point_out.y - raw_map_position.y;
                    let dx_flee = center.x - ctx.position.x;
                    let dy_flee = center.y - ctx.position.y;
                    if dx_door * dx_flee + dy_door * dy_flee >= 0.0 {
                        continue;
                    }
                }
                // Scored distance: `( pGate->GetPositionOut() - Position( mpMe ) ).MaxNorm()`
                // (`original-code/RHartificialintelligence.cpp:5533-5534`).
                let dx_score = door.point_out.x - ctx.position.x;
                let dy_score = door.point_out.y - ctx.position.y;
                let mut distance = dx_score.abs().max(dy_score.abs()) as u32;
                if Some(door.sector_out) != my_sector.map(u16::from) {
                    distance = distance.saturating_add(500);
                }
                if door.layer_out != my_layer {
                    distance = distance.saturating_add(300);
                }
                if best.map(|(_, d)| distance < d).unwrap_or(true) {
                    // `dangerous_house` check.  A fleeing Lacklandist
                    // never runs into a building that already contains
                    // a PC; the gate is camp-gated so Royalist
                    // civilians (and all other camps) skip it.
                    if !dangerous_house_sectors.contains(&(door.sector_in as u32)) {
                        best = Some((door.position_in, distance));
                    }
                }
            }
            best
        };

        let directed_initial = request.center.is_some();
        let mut best = pick_door(&self.ai.global.door_seek_infos, directed_initial);
        // Directed → undirected door fallback.  If no door satisfies
        // the away-half-plane filter, retry with the filter dropped
        // and clear the directed-panic flag on the controller.
        let mut directed_after_door_pick = directed_initial;
        if best.is_none() && directed_initial {
            best = pick_door(&self.ai.global.door_seek_infos, false);
            directed_after_door_pick = false;
        }

        // Snapshot whether the entity is a civilian so we can pick
        // the right Say() remark after we re-borrow the AI base.
        let is_civilian = self.expect_entity(npc_id, "door-seek owner").is_civilian();

        {
            let entity = self.expect_entity_mut(npc_id, "door-seek owner");
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };

            // Sync `directed_panic` with the door-pick outcome
            // (`directed_panic = false` on the fallback path).
            ai.directed_panic = directed_after_door_pick;
            ai.break_macro();
            ai.set_transient_emoticon(crate::ai::EmoticonType::XMark, 0, ctx.frame);
        }

        if let Some((door_in, _)) = best {
            // Door-found arm.
            if is_civilian {
                self.world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| panic!("panic owner {} lost AI", npc_id.index()))
                    .say(crate::ai::Remark::CivPanic);
                self.drain_ai_owner_work_for(sim, assets, npc_id);
            }
            self.set_typed_npc_state(
                npc_id,
                crate::ai::AiState::Fleeing,
                crate::ai::Substate::FleeingRunToDoor,
                "Panic door entry",
            );
            self.drain_ai_owner_work_for(sim, assets, npc_id);
            {
                let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                    panic!(
                        "panic owner {} disappeared before state tail",
                        npc_id.index()
                    )
                });
                let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                    panic!("panic owner {} lost AI before state tail", npc_id.index())
                });
                ai.set_alert_status(request.alert);
                ai.lasting_panic_runs = 0;
                ai.go_to(door_in, crate::ai::GotoFlags::RUN, ctx);
            }

            // RHArtificialIntelligence::Panic observes GoTo's path result
            // immediately and may retry without the directed-door filter in
            // the same call. Resolve this owner's queued move before reading
            // `couldnt_reachpoint`.
            self.launch_pending_orders_for_npc(sim, assets, npc_id);
            self.drain_pending_move_requests_for_owner(sim, npc_id);
            let couldnt_reachpoint = self
                .world
                .entities
                .get(npc_id)
                .and_then(Entity::ai_controller)
                .unwrap_or_else(|| panic!("panic owner {} lost AI after GoTo", npc_id.index()))
                .couldnt_reachpoint;
            if couldnt_reachpoint {
                self.world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!("panic owner {} lost AI after failed GoTo", npc_id.index())
                    })
                    .couldnt_reachpoint = false;
                if directed_after_door_pick
                    && let Some((retry_door, _)) = pick_door(&self.ai.global.door_seek_infos, false)
                {
                    {
                        let Some(entity) = self.world.entities.get_mut(npc_id) else {
                            return;
                        };
                        let Some(ai) = entity.ai_controller_mut() else {
                            return;
                        };
                        ai.directed_panic = false;
                        ai.go_to(retry_door, crate::ai::GotoFlags::RUN, ctx);
                    }
                    self.launch_pending_orders_for_npc(sim, assets, npc_id);
                    self.drain_pending_move_requests_for_owner(sim, npc_id);
                    let retry_failed = self
                        .world
                        .entities
                        .get(npc_id)
                        .and_then(Entity::ai_controller)
                        .unwrap_or_else(|| {
                            panic!("panic owner {} lost AI after retry GoTo", npc_id.index())
                        })
                        .couldnt_reachpoint;
                    if !retry_failed {
                        return;
                    }
                    self.world
                        .entities
                        .get_mut(npc_id)
                        .and_then(Entity::ai_controller_mut)
                        .unwrap_or_else(|| {
                            panic!("panic owner {} lost AI after failed retry", npc_id.index())
                        })
                        .couldnt_reachpoint = false;
                    self.begin_panic_no_door_branch(
                        sim,
                        assets,
                        npc_id,
                        &request,
                        ctx,
                        is_civilian,
                    );
                    return;
                }
                self.begin_panic_no_door_branch(sim, assets, npc_id, &request, ctx, is_civilian);
            }
            return;
        }

        self.begin_panic_no_door_branch(sim, assets, npc_id, &request, ctx, is_civilian);
        if think_debug {
            eprintln!(
                "THINK_STIMULUS phase=after_panic_launch frame={} owner={} creation_order={} rng_cursor={:?}",
                self.control.frame_counter,
                npc_id.index(),
                self.world.original_creation_order(npc_id),
                self.control.rng.original_replay_cursor(),
            );
        }
    }

    /// Drain a queued `pending_panic_seek_fallback` on a single NPC.
    ///
    /// `FLEEING_PANIC` / `EventCouldntReachPoint` fallback: the
    /// panic-run GoTo was blocked, so pick the nearest seek point
    /// (with a +1000 sector-change and +5000 fleeing-toward-source
    /// penalty applied by
    /// [`crate::ai::AiController::nearest_seek_point_to_flee`]) and
    /// GoTo it, with `RUN | DONT_STOP` mid-panic-run and plain `RUN`
    /// on the last segment.  If no seek point is in range, re-fire
    /// the self `EventReachPoint` for the emergency case
    /// fall-through.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_panic_seek_fallback_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
        ctx: &crate::ai::AiContext,
    ) {
        let Some(entity) = self.world.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };
        if !ai.outbox.actor.panic_seek_fallback {
            return;
        }
        ai.outbox.actor.panic_seek_fallback = false;

        let anchor = ai.nearest_seek_point_to_flee(
            &self.ai.global.seek_points,
            ctx.position,
            ctx.position.sector,
        );

        let Some(entity) = self.world.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };

        match anchor {
            Some(idx) => {
                let dest = self.ai.global.seek_points[idx].position;
                // The blocked movement order has already sent its
                // condolence callback before Original enters
                // EVENT_COULDNT_REACHPOINT.  `GetAnimation()` at this nested
                // GoTo therefore observes the sequence manager's live order
                // (usually RHNONANIMATION_END), not the actor's movement
                // latch, which Rust clears later in the owner drain.
                let mut goto_ctx = ctx.clone();
                goto_ctx.self_animation = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(npc_id)
                    .map(|(_, _, order)| order.order_type)
                    .unwrap_or(crate::order::OrderType::NonanimationEnd);
                let Some(entity) = self.world.entities.get_mut(npc_id) else {
                    return;
                };
                let Some(ai) = entity.ai_controller_mut() else {
                    return;
                };
                let mut flags = crate::ai::GotoFlags::RUN;
                if ai.lasting_panic_runs > 0 {
                    flags |= crate::ai::GotoFlags::DONT_STOP;
                }
                ai.go_to_with_live_animation(dest, flags, &goto_ctx);

                // Original GoTo builds cross-sector gate routes and translates
                // the first movement before returning to this handler. Mirror
                // that owner-local translation now so AppendMoveToSequence's
                // synchronous no-gate-route failure is visible to the emergency
                // retry below. `drain_pending_move_requests_for_owner` does not
                // run the pathfinder: same-area A*-requiring work is only queued
                // and remains deferred to the normal ProcessPathRequests phase.
                self.launch_pending_orders_for_npc(sim, assets, npc_id);
                let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
                let ai = self
                    .world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "panic seek fallback owner {} disappeared after GoTo",
                            npc_id.index()
                        )
                    });
                if ai.couldnt_reachpoint {
                    // Emergency-case retry — decrement runs and
                    // self-fire `EventReachPoint` so the common-stuff
                    // state machine tries a new random direction before
                    // the enclosing Think returns.
                    ai.couldnt_reachpoint = false;
                    ai.lasting_panic_runs = ai.lasting_panic_runs.saturating_sub(1);
                    ai.fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
                }
            }
            None => {
                // Emergency case — no seek point available, re-fire
                // reach-point so the common-stuff handler picks a
                // fresh random direction.
                ai.fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
            }
        }
    }

    /// No-door branch of `panic`.  Split out so the door-found
    /// branch can fall through on a post-GoTo unreachable-point
    /// error.
    fn begin_panic_no_door_branch(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
        request: &crate::ai::PanicRequest,
        ctx: &crate::ai::AiContext,
        is_civilian: bool,
    ) {
        // If directed, OR in the "panic center is in front of me"
        // dot-product test so a center that has flipped in front
        // during a prior run still counts as a new panic.
        let mut is_new_panic = request.is_new_panic;
        if request.center.is_some() && !is_new_panic {
            let ai = self
                .world
                .entities
                .get(npc_id)
                .and_then(Entity::ai_controller)
                .unwrap_or_else(|| panic!("panic owner {} has no AI", npc_id.index()));
            if directed_panic_center_is_in_front(
                ctx.direction as i16,
                ctx.position.x,
                ctx.position.y,
                ai.panic_center_x,
                ai.panic_center_y,
            ) {
                is_new_panic = true;
            }
        }

        if is_new_panic {
            // New panic — full side-effect set.
            self.set_typed_npc_state(
                npc_id,
                crate::ai::AiState::Fleeing,
                crate::ai::Substate::FleeingPanic,
                "Panic run entry",
            );
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("panic owner {} has no AI", npc_id.index()))
                .say(if is_civilian {
                    crate::ai::Remark::CivPanic
                } else {
                    crate::ai::Remark::Panic
                });
            self.drain_ai_owner_work_for(sim, assets, npc_id);
            let deferred_self_stimuli = {
                let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                    panic!("panic owner {} disappeared after speech", npc_id.index())
                });
                let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                    panic!("panic owner {} lost AI after speech", npc_id.index())
                });
                ai.set_alert_status(request.alert);
                ai.lasting_panic_runs = request.runs.saturating_add(1);
                ai.first_try = true;

                // A pre-existing Rust self-stimulus is deferred work from an
                // enclosing boundary.  It is not part of Original Panic's
                // direct recursive Think call and must not be pulled into it.
                let deferred = std::mem::take(&mut ai.outbox.reentrant.self_stimuli);
                ai.fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
                deferred
            };

            // `RHArtificialIntelligence::Panic` calls
            // `Think(EVENT_REACHPOINT)` directly here.  This is a recursive
            // owner-local boundary, not a deferred event: in particular, a
            // retained sibling stimulus must not run first and replace the
            // freshly installed `FLEEING_PANIC` substate.  Close the generated
            // Think (and its two direction/distance RNG draws) before Panic
            // returns to its caller.
            self.drain_self_stimuli_for_npc_without_forecast(sim, npc_id, assets);
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "panic owner {} lost AI after recursive Think",
                        npc_id.index()
                    )
                })
                .outbox
                .reentrant
                .self_stimuli
                .extend(deferred_self_stimuli);
        } else {
            // Not new: upgrade-only bump of `lasting_panic_runs`
            // (`if lasting_panic_runs < runs`).  No state change, no
            // `say()`, no self-fire.
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("panic owner {} has no AI", npc_id.index()));
            if ai.lasting_panic_runs < request.runs {
                ai.lasting_panic_runs = request.runs;
            }
        }
    }

    /// Enter a virtual Enemy/Friendly `SetState` call after releasing the
    /// engine's prior controller borrow. Required callers must not degrade a
    /// missing owner or mismatched brain into a silent no-op.
    pub(super) fn set_typed_npc_state(
        &mut self,
        npc_id: EntityId,
        state: crate::ai::AiState,
        substate: crate::ai::Substate,
        context: &'static str,
    ) {
        match self.world.entities.get_mut(npc_id) {
            Some(Entity::Soldier(s)) => s
                .npc
                .ai_brain
                .enemy_mut()
                .unwrap_or_else(|| panic!("{context} owner {} requires Enemy AI", npc_id.index()))
                .set_state(state, substate),
            Some(Entity::Civilian(c)) => c
                .npc
                .ai_brain
                .friendly_mut()
                .unwrap_or_else(|| {
                    panic!("{context} owner {} requires Friendly AI", npc_id.index())
                })
                .set_state(state, substate),
            Some(other) => panic!(
                "{context} owner {} has invalid entity kind {:?}",
                npc_id.index(),
                other.element_data().kind
            ),
            None => panic!("{context} owner {} disappeared", npc_id.index()),
        }
    }

    /// Enter the pre-filter half of typed `StartThink(NO_EVENT)`.
    pub(super) fn start_script_ai_native_think_pre_filter(&mut self, npc_id: EntityId) {
        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::NoEvent);
        match self.world.entities.get_mut(npc_id) {
            Some(Entity::Soldier(s)) => s
                .npc
                .ai_brain
                .enemy_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "SetAIState StartThink owner {} requires Enemy AI",
                        npc_id.index()
                    )
                })
                .start_think_pre_filter(&stimulus),
            Some(Entity::Civilian(c)) => c
                .npc
                .ai_brain
                .friendly_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "SetAIState StartThink owner {} requires Friendly AI",
                        npc_id.index()
                    )
                })
                .start_think_pre_filter(&stimulus),
            Some(other) => panic!(
                "SetAIState StartThink owner {} has invalid entity kind {:?}",
                npc_id.index(),
                other.element_data().kind
            ),
            None => panic!("SetAIState StartThink owner {} disappeared", npc_id.index()),
        }
    }

    /// Run the post-filter half of typed `StartThink(NO_EVENT)` and return
    /// its normal Think admission decision. SetAIState deliberately ignores
    /// this bool, but the lock/freeze/special-state side effects still occur.
    pub(super) fn start_script_ai_native_think_post_filter(&mut self, npc_id: EntityId) -> bool {
        let (self_is_dead, self_is_unconscious) = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| {
                (
                    entity.is_dead(),
                    entity.human_data().is_some_and(|human| human.unconscious),
                )
            })
            .unwrap_or_else(|| {
                panic!(
                    "SetAIState post-filter StartThink owner {} disappeared",
                    npc_id.index()
                )
            });
        let static_ai_frozen = self.ai.global.freeze;
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "SetAIState post-filter StartThink owner {} lost its typed AI",
                    npc_id.index()
                )
            })
            .start_no_event_post_filter(static_ai_frozen, self_is_dead, self_is_unconscious)
    }

    /// Close typed `EndThink` after SeekArea/Panic and their recursively
    /// produced owner work have stabilized.
    pub(super) fn end_script_ai_native_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        let normal_depth_complete = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "SetAIState EndThink owner {} lost its typed AI",
                    npc_id.index()
                )
            })
            .end_think_completion_events();
        if normal_depth_complete {
            return;
        }
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let entity =
            self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("SetAIState EndThink owner {} disappeared", npc_id.index())
            });
        let mut ctx = build_ai_context_from_entity(
            entity,
            self.control.frame_counter,
            self.entity_building_sector(entity.element_data().sector()),
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &assets.hiking_waypoint_sectors,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        let enemy_tick = matches!(self.world.entities.get(npc_id), Some(Entity::Soldier(_)))
            .then(|| self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets));
        let stimulus_depth = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .map(|ai| ai.think_recursion_depth)
            .unwrap_or(0);
        assert!(
            stimulus_depth > 0,
            "SetAIState EndThink owner {} has no matching StartThink",
            npc_id.index()
        );
        let global = &mut self.ai.global;
        match self.world.entities.get_mut(npc_id) {
            Some(Entity::Soldier(s)) => s
                .npc
                .ai_brain
                .enemy_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "SetAIState EndThink owner {} requires Enemy AI",
                        npc_id.index()
                    )
                })
                .end_think(
                    sim,
                    global,
                    &ctx,
                    enemy_tick.as_ref().unwrap_or_else(|| {
                        panic!(
                            "SetAIState EndThink owner {} lost its Enemy tick context",
                            npc_id.index()
                        )
                    }),
                    None,
                ),
            Some(Entity::Civilian(c)) => c
                .npc
                .ai_brain
                .friendly_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "SetAIState EndThink owner {} requires Friendly AI",
                        npc_id.index()
                    )
                })
                .end_think(sim, global, &ctx),
            Some(other) => panic!(
                "SetAIState EndThink owner {} has invalid entity kind {:?}",
                npc_id.index(),
                other.element_data().kind
            ),
            None => panic!("SetAIState EndThink owner {} disappeared", npc_id.index()),
        }
    }

    /// Drain a pending script-driven `SeekArea` request.  Consumes
    /// `AiController::outbox.actor.script_seek_area` set by
    /// `script_set_ai_state` when a script fires
    /// `SetAIState(actor, STATE_SEEKING)`.  Dispatches into
    /// `EnemyAi::seek_area` (soldier-only — `seek_area` is defined
    /// only on the soldier subtype).
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_script_seek_area_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
        ctx: &crate::ai::AiContext,
        tick: &crate::ai::AiPerTickData,
    ) {
        let request = {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "accepted SetAIState SEEKING owner {} disappeared before SeekArea",
                    npc_id.index()
                )
            });
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "accepted SetAIState SEEKING owner {} lost its AI before SeekArea",
                    npc_id.index()
                )
            });
            ai.outbox.actor.script_seek_area.take().unwrap_or_else(|| {
                panic!(
                    "accepted SetAIState SEEKING owner {} lost its required SeekArea request",
                    npc_id.index()
                )
            })
        };

        let Some(entity) = self.world.entities.get_mut(npc_id) else {
            panic!(
                "accepted SetAIState SEEKING owner {} disappeared before typed SeekArea",
                npc_id.index()
            );
        };
        let Entity::Soldier(s) = entity else {
            panic!(
                "accepted SetAIState SEEKING owner {} is not a soldier",
                npc_id.index()
            );
        };
        let enemy_ai = s.npc.ai_brain.enemy_mut().unwrap_or_else(|| {
            panic!(
                "accepted SetAIState SEEKING owner {} requires Enemy AI",
                npc_id.index()
            )
        });
        if crate::ai_enemy::EnemyAi::seek_area_phase6_caller_debug_enabled()
            && crate::ai_enemy::EnemyAi::seek_area_phase6_caller_debug_matches(
                ctx.frame,
                ctx.original_creation_order,
            )
        {
            eprintln!(
                "SEEKAREA_CALLER {{\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"caller\":\"script_set_ai_state\",\"stimulus\":\"no_event\"}}",
                ctx.frame,
                npc_id.index(),
                ctx.original_creation_order
                    .expect("phase6 caller diagnostic matched an owner without creation order"),
            );
        }
        enemy_ai.seek_area(
            sim,
            request.center,
            request.radius,
            crate::ai_enemy::SeekFlags::empty(),
            crate::ai_enemy::UNDEFINED_DIRECTION,
            &mut self.ai.global,
            ctx,
            tick,
        );
        // SeekArea's typed SetState callback is inside the StartThink /
        // EndThink pair and must finish before its later GoTo/order tail is
        // exposed to the enclosing native barrier.
        self.drain_ai_owner_work_for(sim, assets, npc_id);
    }
}
