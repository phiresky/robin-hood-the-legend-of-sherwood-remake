//! Enemy AI initialization and ticking.
//!
//! The per-tick `tick_enemy_ai` orchestrator is split across submodules
//! by phase grouping:
//!  - [`snapshots`] — phases P1..P2d: read-only views built once per tick.
//!  - [`detection`] — phases P2a, P3, P3b: per-NPC visibility passes.
//!  - [`post_detection`] — phases P4..P6d: alert dispatch, pursuit, drains.

mod detection;
#[cfg(test)]
pub(crate) use detection::set_heard_callback_observer;
mod post_detection;
mod snapshots;

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

#[derive(Debug)]
struct RefreshViewLifecycleDebugConfig {
    enabled: bool,
    from_frame: u32,
    through_frame: u32,
    creation_order: Option<u32>,
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

/// Immutable, RNG-free inputs prepared once before the live actor-owner walk.
/// Volatile entity/AI views are deliberately absent and rebuilt at each NPC
/// slot after earlier owners have closed their recursive work.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct PreparedNpcOwnerPass;

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

/// Build a snapshot of every live human in the engine.  Called once at
/// the start of [`EngineInner::init_ai`] and handed to every per-NPC init
/// pass.
pub(super) fn build_potential_detectables(engine: &EngineInner) -> Vec<PotentialDetectable> {
    let mut out = Vec::new();
    for (id, entity) in engine.world.entities.humans() {
        if !entity.element_data().active {
            continue;
        }
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
                point_out: MapPoint::new(10.0, 20.0),
                layer_out: 1,
                ..Default::default()
            },
            crate::gate::Door {
                door_type: crate::gate::DoorType::LiftHigh,
                sector_in: crate::sector::SectorNumber::new(42),
                sector_out: crate::sector::SectorNumber::new(8),
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
        layer: elem.layer(),
        direction: elem.direction() as u16,
        forecasted_movement_z: forecasted_z,
        door_pass,
        passing_door_directly: actor.passing_door_directly,
    })
}

/// Map position returned by Original AI `Position(friend)` during SeekArea's
/// nearby-friend scan. A PassDoor sequence reports its committed destination
/// side even while the sprite is still interpolating along the door rail.
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
/// scan. The friend-side `Position(...)` call is owned by the currently
/// selected PassDoor movement element, not the sprite's runtime door-pass
/// choreography latch. The owner position is supplied explicitly because the
/// caller observes it at the surrounding Think boundary.
fn seek_area_friend_contribution(
    sequence_manager: &crate::sequence::SequenceManager,
    friend_id: EntityId,
    owner_position: MapPoint,
    friend_raw_position: MapPoint,
    doors: &[crate::gate::Door],
    friend_seeks_with_help: bool,
) -> Option<bool> {
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
            seek_area_friend_contribution(&sequences, friend, owner, raw_friend, &doors, true,),
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
            seek_area_friend_contribution(&sequences, friend, owner, raw_friend, &doors, true,),
            Some(true)
        );

        // Without a selected PassDoor, stale/runtime door state is irrelevant
        // and the raw nearby position contributes to both aggregate outputs.
        sequences.element_terminated(sequence_id, 0);
        let contributions = [seek_area_friend_contribution(
            &sequences, friend, owner, raw_friend, &doors, true,
        )];
        let visible_seeking_friends = contributions.iter().flatten().count();
        let friend_seek_clears_help_flag = contributions.into_iter().flatten().any(|help| help);
        assert_eq!(visible_seeking_friends, 1);
        assert!(friend_seek_clears_help_flag);
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
        self_detectable_friend_count,
        self_detectable_missed_friend_count,
        self_seen_enemy_handles,
        self_forced_attentive,
        self_animation_reached_action_done,
        self_animation,
        self_selected_element_is_default_wait: None,
        self_selected_element_priority: None,
        antagonist: None,
        entity_views: entity_views.clone(),
        sight_obstacles: sight_obstacles.clone(),
        view_radius_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        fast_grid: fast_grid.clone(),
        hiking_paths: hiking_paths.clone(),
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
    mut position_of: impl FnMut(crate::element::EntityId) -> crate::ai::Position,
) -> AiPositionResolution {
    let target = entities
        .get(target_id)
        .unwrap_or_else(|| panic!("AI position target {target_id:?} disappeared"));
    if target.actor_data().is_some()
        && let Some((gate_id, direction)) = selected_pass_door_movement(sequence_manager, target_id)
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
                sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_in)),
                level: door.layer_in,
            }
        } else {
            crate::ai::Position {
                x: door.point_out.x,
                y: door.point_out.y,
                sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_out)),
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
    entities: &crate::entities::Entities,
    sequence_manager: &crate::sequence::SequenceManager,
    doors: &[crate::gate::Door],
    target_id: crate::element::EntityId,
) -> Option<PrimaryTargetMetadata> {
    if target_id.index() == 0 {
        return None;
    }
    let target = entities.get(target_id)?;
    let elem = target.element_data();
    let resolved = resolve_ai_position_with(entities, doors, sequence_manager, target_id, |id| {
        let element = entities
            .get(id)
            .unwrap_or_else(|| panic!("AI metadata position owner {id:?} disappeared"))
            .element_data();
        crate::ai::Position {
            x: element.position_map().x,
            y: element.position_map().y,
            sector: element.sector(),
            level: element.layer(),
        }
    });
    let posture = elem.posture;
    // Orders live on the target's owning `SequenceElement.orders` —
    // look up the current in-progress element for the target actor.
    let animation = sequence_manager
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
    let me_sector: u16 = me_position.sector?.into();
    let target_sector: u16 = target_position.sector?.into();
    if me_sector == target_sector {
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
        sector: crate::position_interface::SectorHandle::new(wait.sector),
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
/// positions / states.  Includes every PC, soldier, civilian, and
/// pickup-style bonus entity. Human views include inactive actors because
/// normal `IsDetecting(human)` ignores activity in its same-building arm;
/// inactive bonuses and projectile entities remain excluded.
pub(super) fn build_entity_views(engine: &EngineInner) -> AiEntityViewMap {
    build_entity_views_inner(engine)
}

fn build_entity_views_without_forecast(engine: &EngineInner) -> AiEntityViewMap {
    build_entity_views_inner(engine)
}

fn build_entity_views_inner(engine: &EngineInner) -> AiEntityViewMap {
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
    let mut nets_by_victim: std::collections::HashMap<u32, Vec<ai_entity_view::NetCoverInfo>> =
        std::collections::HashMap::new();
    for (net_id, net) in engine.world.entities.nets() {
        if !net.element.active {
            continue;
        }
        if net.net.victims.is_empty() {
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

    let mut map = AiEntityViewMap::with_capacity(engine.world.entities.len());
    for (entity_id, entity) in engine.world.entities.occupied() {
        let elem = entity.element_data();
        match entity {
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) => {}
            Entity::Bonus(_) if elem.active => {}
            _ => continue,
        }
        // Resolve building sector (if any) through the same helper
        // used by the existing AiContext building logic.
        let building_sector = engine.entity_building_sector(elem.sector());
        let mut view = ai_entity_view::entity_view_from_entity(
            entity,
            engine.world.original_creation_order(entity_id),
            building_sector.is_some(),
            building_sector,
            Some(&engine.mission_domain.campaign),
            engine
                .live_actor_animation(entity_id)
                .unwrap_or(crate::order::OrderType::NonanimationEnd),
        );

        if matches!(
            entity,
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
        ) {
            view.position = resolve_ai_position_with(
                &engine.world.entities,
                doors_ref,
                &engine.orders.sequence_manager,
                entity_id,
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
                        sector: position_element.sector(),
                        level: position_element.layer(),
                    }
                },
            )
            .effective;
        }

        // Attach pre-scanned covering nets for stuck victims, consumed
        // by `RunToFreeNetVictim`.
        if view.stuck_under_net
            && let Some(nets) = nets_by_victim.remove(&entity_id.index())
        {
            view.covering_nets = nets;
        }

        // Prepare the destination alternatives without drawing RNG. Original
        // calls ForecastDestinationForIA only from specific AI statements;
        // eagerly resolving every human here made unrelated building transit
        // consume BuildingExitGate draws once per scratch rebuild.
        if matches!(
            entity,
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
        ) && let Some(input) = extract_forecast_input(
            entity,
            selected_actor_is_passing_door(&engine.orders.sequence_manager, entity_id),
        ) {
            view.forecasted_destination = crate::ai::prepare_forecast_destination_for_ia(
                &input,
                doors_ref,
                &engine.world.fast_grid.level.sectors,
                &engine.world.fast_grid.level.sector_number_map,
            );
        }

        // AI handle == entity slot index (see `FighterSnapshot.handle =
        // target_id.index()` elsewhere, and `self.world.entities.get_mut(target as
        // usize)` for `CrossNpcAction` handlers).
        map.insert(entity_id.index(), view);
    }
    map
}

impl EngineInner {
    pub(super) fn debug_building_exit_wait_event_view(
        &self,
        owner: EntityId,
        queue_index: usize,
        stimulus: &crate::ai::Stimulus,
    ) {
        if !building_exit_wait_owner_debug_enabled()
            || stimulus.stimulus_type != crate::ai::StimulusType::EventView
        {
            return;
        }
        let (target_handle, target_creation_order) = match stimulus.info {
            crate::ai::StimulusInfo::Human(handle) => (
                Some(handle),
                self.entity_id_for_index(handle)
                    .map(|target| self.world.original_creation_order(target)),
            ),
            _ => (None, None),
        };
        eprintln!(
            "BEXITWAIT {{\"event\":\"queued_event_view\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"queue_index\":{queue_index},\"target_handle\":{target_handle:?},\"target_creation_order\":{target_creation_order:?}}}",
            self.control.frame_counter,
            owner,
            self.world.original_creation_order(owner),
        );
    }

    pub(super) fn debug_building_exit_wait_pc_route(
        &self,
        owner: EntityId,
        source_sector: crate::position_interface::SectorHandle,
        goal_sector: crate::position_interface::SectorHandle,
    ) {
        if !building_exit_wait_owner_debug_enabled() {
            return;
        }
        eprintln!(
            "BEXITWAIT {{\"event\":\"pc_door_fight_route\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"source_sector\":{},\"goal_sector\":{}}}",
            self.control.frame_counter,
            owner,
            self.world.original_creation_order(owner),
            source_sector.get(),
            goal_sector.get(),
        );
    }

    pub(super) fn debug_refresh_view_lifecycle(
        &self,
        stage: &str,
        npc_id: EntityId,
        derived_tail_order_type: Option<crate::order::OrderType>,
    ) {
        let config = refresh_view_lifecycle_debug_config();
        if !config.enabled
            || self.control.frame_counter < config.from_frame
            || self.control.frame_counter > config.through_frame
        {
            return;
        }
        let creation_order = self.world.original_creation_order(npc_id);
        if config
            .creation_order
            .is_some_and(|expected| creation_order != expected)
        {
            return;
        }
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} disappeared at stage {stage}",
                npc_id.index()
            )
        });
        let Some(npc) = entity.npc_data() else {
            return;
        };
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} is not an actor at stage {stage}",
                npc_id.index()
            )
        });
        let human = entity.human_data().unwrap_or_else(|| {
            panic!(
                "RVLIFE owner {} is not human at stage {stage}",
                npc_id.index()
            )
        });
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let installed_order = actor
            .installed_order
            .map(|order| order.order_type as u32)
            .map_or(-1_i64, i64::from);
        let derived_tail_order = derived_tail_order_type
            .map(|order| order as u32)
            .map_or(-1_i64, i64::from);
        eprintln!(
            "RVLIFE {{\"engine\":\"rust\",\"seq\":{sequence},\"stage\":{stage:?},\"frame\":{},\"owner_slot\":{},\"creation_order\":{creation_order},\"eye_status\":{},\"alpha_start\":{},\"radius_goal\":{},\"radius_step\":{},\"radius\":{},\"active\":{},\"unconscious\":{},\"tied\":{},\"dead\":{},\"frozen_all\":{},\"installed_order\":{installed_order},\"derived_tail_order\":{derived_tail_order},\"motion_state\":{},\"execution_frozen\":{}}}",
            self.control.frame_counter,
            npc_id.index(),
            npc.eye_status as u8,
            npc.view_alpha_start,
            npc.view_radius_goal,
            npc.view_radius_step,
            npc.view_radius,
            entity.element_data().active,
            human.unconscious,
            entity.element_data().posture == crate::element::Posture::Tied,
            entity.is_dead(),
            self.actors_frozen(),
            actor.continuation.motion_state as u8,
            actor.execution_frozen,
        );
    }

    /// Refresh the Original actor's selected `mpWaitSequenceElement` identity
    /// on a context immediately before an AI call that may project deferred
    /// Halt effects. The legacy ownership decoder proves that only Wait and
    /// historical Freeze elements may occupy that pointer.
    pub(super) fn refresh_selected_default_wait_identity(
        &self,
        entity_id: EntityId,
        ctx: &mut crate::ai::AiContext,
    ) {
        let selected = self
            .orders
            .sequence_manager
            .current_element_for_actor(entity_id)
            .and_then(|(sequence_id, element_index)| {
                self.orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)
            });
        ctx.self_selected_element_is_default_wait = Some(selected.is_some_and(|element| {
            matches!(
                element.command,
                crate::element::Command::Wait | crate::element::Command::Freeze
            )
        }));
        ctx.self_selected_element_priority = Some(selected.map(|element| element.priority));
    }

    /// Resolve an AI `HumanHandle` back through the original sparse element
    /// table without inventing an entity kind.  AI still stores these handles
    /// as raw slots, so a target can be a PC, soldier, or civilian.
    pub(super) fn expect_human_id_for_ai_handle(
        &self,
        handle: crate::ai::HumanHandle,
        context: &str,
    ) -> EntityId {
        let id = self.expect_entity_id_for_index(handle, context);
        assert!(
            self.world
                .entities
                .get(id)
                .is_some_and(crate::element::Entity::is_human),
            "{context}: entity in raw slot {handle} is not human"
        );
        id
    }

    /// Position visible at one Original legacy owner boundary. Rust movement
    /// is globally batched, so later slots are projected back to the preserved
    /// pre-movement oracle. Callback-spawned slots are absent from that oracle
    /// and correctly retain their current (never-moved) position.
    pub(super) fn position_at_owner_boundary(
        &self,
        target: EntityId,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_actor_complete: bool,
    ) -> MapPoint {
        self.boundary_position(
            target,
            owner,
            positions_before_movement,
            owner_actor_complete,
        )
        .map
    }

    /// The same boundary choice, keeping both stored coordinate spaces. Direct
    /// actor geometry (`ComputeEyesPoint` / `ComputeDetectionPoint`) reads the
    /// 3D position, which is not recoverable from the map projection without
    /// rounding.
    pub(super) fn boundary_position(
        &self,
        target: EntityId,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_actor_complete: bool,
    ) -> crate::entities::BoundaryPosition {
        let current = crate::entities::BoundaryPosition::of(
            self.world
                .entities
                .get(target)
                .unwrap_or_else(|| {
                    panic!(
                        "owner {} requires position for missing entity {}",
                        owner.index(),
                        target.index()
                    )
                })
                .element_data(),
        );
        let target_has_not_moved = target.index() > owner.index()
            || (!owner_actor_complete && target.index() == owner.index());
        if target_has_not_moved {
            positions_before_movement
                .get(target)
                .copied()
                .flatten()
                .unwrap_or(current)
        } else {
            current
        }
    }

    fn build_ai_sight_obstacles(
        &self,
        assets: &LevelAssets,
    ) -> crate::sight_obstacle::SharedSightObstacles {
        crate::sight_obstacle::SharedSightObstacles {
            static_obstacles: assets.static_sight_obstacles.clone(),
            dynamic_obstacles: std::sync::Arc::new(self.world.dynamic_sight_obstacles.clone()),
            static_active: std::sync::Arc::new(self.world.static_sight_obstacle_active.clone()),
        }
    }

    pub(crate) fn build_sim_scratch(
        &self,
        _sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> SimScratch {
        let scratch = SimScratch {
            ai_entity_views: self.share_ai_entity_views(build_entity_views(self)),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        };
        scratch
    }

    pub(crate) fn build_owner_context_scratch_without_forecast(
        &self,
        assets: &LevelAssets,
    ) -> SimScratch {
        SimScratch {
            ai_entity_views: self.share_ai_entity_views(build_entity_views_without_forecast(self)),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        }
    }

    pub(crate) fn build_owner_context_scratch_at_slot_without_forecast(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        owner_actor_complete: bool,
    ) -> SimScratch {
        let mut views = build_entity_views_without_forecast(self);
        for (target, _) in self.world.entities.occupied() {
            let boundary = self.boundary_position(
                target,
                owner,
                positions_before_movement,
                owner_actor_complete,
            );
            // The initial view builder already applies
            // `RHArtificialIntelligence::Position(actor)`'s committed
            // gate-side override. Creation-slot projection is for live map
            // positions and must not replace that AI-specific value.
            // Direct geometry is different: ComputeDetectionPoint starts from
            // literal GetPosition even during a door pass, so always stamp its
            // map/world pair with the owner-boundary values.
            let passing_door = self
                .world
                .entities
                .get(target)
                .and_then(Entity::actor_data)
                .is_some_and(|actor| actor.active_door_pass.is_some());
            if let Some(view) = views.get_mut(&target.index()) {
                view.detection_position = boundary.map;
                view.detection_position_world = boundary.world;
                if !passing_door {
                    view.position.x = boundary.map.x;
                    view.position.y = boundary.map.y;
                }
            }
        }
        SimScratch {
            ai_entity_views: self.share_ai_entity_views(views),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        }
    }

    fn share_ai_entity_views(&self, entities: AiEntityViewMap) -> SharedAiEntityViews {
        let building_authorizations = self
            .script_domains
            .interactables
            .doors
            .iter()
            .filter(|door| {
                matches!(
                    door.door_type,
                    crate::gate::DoorType::Building | crate::gate::DoorType::BuildingTrap
                )
            })
            .map(|door| {
                (
                    door.sector_in,
                    self.building_sector_is_authorized(door.sector_in),
                )
            })
            .collect();
        std::sync::Arc::new(AiEntityViews {
            entities,
            building_authorizations,
        })
    }

    /// Build a per-NPC [`AiPerTickData`] snapshot on demand, outside
    /// the main detection pass.
    ///
    /// The detection pass (see the builder at `engine/ai.rs:4319`)
    /// assembles a full-fidelity `AiPerTickData` with camp soldiers,
    /// nearby fighters, battle points, multiplicity, etc. — but it
    /// only runs once per frame per NPC.  Off-detection dispatch sites
    /// (timer events, reach-point events, panic, patrol, cross-NPC
    /// actions, civilian EventView...) previously called
    /// `AiPerTickData::stub()`, losing every field that matters for
    /// `battle_decisions` and swordfight tactics.  The symptom: a
    /// soldier with a valid `primary_target` but empty
    /// `enemy_sq_distances` bails to `return_to_duty`, producing the
    /// Reactiontime/Default ping-pong.
    ///
    /// This builder fills in everything that can be cheaply computed
    /// from the live entity store without re-running the detection
    /// loop: same-camp soldier snapshots for alert coordination,
    /// primary target metadata (position, posture, animation,
    /// carrier, destination forecast, table-swordfight jump line),
    /// `primary_target_is_pc`, friend-swap candidates for
    /// `ReconsiderEnemyApproach`, the avenger-on-the-roof wait
    /// position, and a single-target seed for
    /// `enemy_sq_distances` / `min_sq_enemy_distance` so
    /// `battle_decisions` doesn't see an empty list when a valid
    /// `primary_target` exists. Fields that truly require the full detection
    /// scan (`unconscious_enemies`, `nearby_sleeping_enemies`, final visible
    /// enemy distances/latches, ...) remain empty; RefreshDetection overlays
    /// those scan products when it uses this builder for a queued stimulus.
    ///
    /// Returns a stub for non-enemy-soldier entities (civilians, PCs,
    /// beggar/animal NPCs); their AI paths don't consult the combat
    /// tick fields, so the stub is adequate.
    pub(super) fn build_npc_tick_data(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target_mode(sim, npc_id, scratch, assets, None, true)
    }

    pub(super) fn build_npc_tick_data_for_target(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
        target_override: Option<crate::element::EntityId>,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target_mode(
            sim,
            npc_id,
            scratch,
            assets,
            target_override,
            true,
        )
    }

    pub(super) fn build_npc_tick_data_without_forecasts(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
    ) -> crate::ai::AiPerTickData {
        match self.world.entities.get(npc_id) {
            Some(Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some() => {}
            Some(Entity::Soldier(_)) => panic!(
                "owner-local tick context owner {} requires Enemy AI",
                npc_id.index()
            ),
            Some(other) => panic!(
                "owner-local tick context owner {} has invalid entity kind {:?}",
                npc_id.index(),
                other.element_data().kind
            ),
            None => panic!(
                "owner-local tick context owner {} disappeared",
                npc_id.index()
            ),
        }
        self.build_npc_tick_data_for_target_mode(sim, npc_id, scratch, assets, None, false)
    }

    /// Build the typed live value consumed by Friendly AI. The narrow type
    /// has no stub/default fields for future handlers to read accidentally.
    pub(super) fn build_friendly_tick_data_without_forecasts(
        &self,
        npc_id: crate::element::EntityId,
    ) -> crate::ai_friendly::FriendlyPerTickData {
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "owner-local friendly tick context owner {} disappeared",
                npc_id.index()
            )
        });
        let Entity::Civilian(civilian) = entity else {
            panic!(
                "owner-local friendly tick context owner {} is not a Civilian",
                npc_id.index()
            )
        };
        let ai = civilian.npc.ai_brain.friendly().unwrap_or_else(|| {
            panic!(
                "owner-local friendly tick context owner {} requires Friendly AI",
                npc_id.index()
            )
        });

        if let Some(chief_id) = ai.base.patrol_chief {
            let chief = self.world.entities.get(chief_id).unwrap_or_else(|| {
                panic!(
                    "owner-local friendly tick context owner {} has stale patrol chief {}",
                    npc_id.index(),
                    chief_id.index()
                )
            });
            let chief_ai = chief.ai_controller().unwrap_or_else(|| {
                panic!(
                    "owner-local friendly tick context patrol chief {} has no AI",
                    chief_id.index()
                )
            });
            let point = chief.element_data().position_map();
            crate::ai_friendly::FriendlyPerTickData::with_patrol_chief(
                crate::ai::Position {
                    x: point.x,
                    y: point.y,
                    sector: chief.element_data().sector(),
                    level: chief.element_data().layer(),
                },
                chief_ai.current_state,
            )
        } else {
            crate::ai_friendly::FriendlyPerTickData::without_patrol_chief()
        }
    }

    fn build_npc_tick_data_for_target_mode(
        &self,
        _sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
        target_override: Option<crate::element::EntityId>,
        build_forecasts: bool,
    ) -> crate::ai::AiPerTickData {
        use crate::ai::AiPerTickData;

        // Pull the minimum we need from the NPC: its position, camp,
        // primary target handle, and the `couldnt_reachpoint` flag
        // (drives avenger-on-roof computation).
        let Some(entity) = self.world.entities.get(npc_id) else {
            return AiPerTickData::stub();
        };
        let Entity::Soldier(soldier) = entity else {
            // Non-enemy NPC — civilians use FriendlyAi which doesn't
            // consume combat tick fields.  Return stub.
            return AiPerTickData::stub();
        };
        let Some(ai) = soldier.npc.ai_brain.base() else {
            return AiPerTickData::stub();
        };
        let primary_target_handle = target_override
            .map(|id| id.index())
            .unwrap_or(ai.primary_target);
        let target_id = target_override.or_else(|| {
            (primary_target_handle != 0)
                .then(|| self.entity_id_for_index(primary_target_handle))
                .flatten()
        });
        let my_camp = soldier.soldier.cached_camp;
        let me_handle = ai.me;
        let me_pos = soldier.element.position_map();
        let me_layer = soldier.element.layer();
        let couldnt_reachpoint = soldier
            .npc
            .ai_brain
            .enemy()
            .map(|e| e.base.couldnt_reachpoint)
            .unwrap_or(false);

        let mut tick = AiPerTickData::stub();
        tick.owner_live_position = Some(crate::ai::Position {
            x: me_pos.x,
            y: me_pos.y,
            sector: soldier.element.sector(),
            level: me_layer,
        });
        let enemy_idx = DetectableType::Enemy as usize;
        tick.seen_last_frame_enemies =
            seen_last_frame_detectable_handles(&soldier.npc.detectable_lists[enemy_idx]);
        tick.primary_target_snapshot_handle = primary_target_handle;
        tick.profile_manager = Some(assets.profile_manager.clone());
        // `SeekArea` scans the live global NPC register at the call site.
        // Despite the old local name "visible friends", the Original applies
        // no visibility, camp, layer, posture, or AI-state filter here: every
        // other soldier with alert status above green and raw map-space
        // distance below 500 contributes to the point-count multiplier.
        // Build this for every Think boundary, not only RefreshDetection,
        // because timer/report callbacks also enter SeekArea synchronously.
        let doors = self.script_domains.interactables.doors.as_slice();
        let frame = self.frame_counter();
        // Keep the diagnostic entirely absent from the disabled path. In
        // particular, do not resolve Original identity for an observation
        // that production does not otherwise need here.
        let seek_area_debug_enabled = seek_area_owner_position_debug_enabled();
        let creation_order = if seek_area_debug_enabled {
            self.world.original_creation_order(npc_id)
        } else {
            0
        };
        let seek_area_debug = seek_area_debug_enabled
            && seek_area_owner_position_debug_matches(frame, creation_order);
        if seek_area_debug {
            let owner_selected_door =
                selected_pass_door_movement(&self.orders.sequence_manager, npc_id);
            let owner_effective_position = seek_area_friend_position_map(
                me_pos,
                owner_selected_door.map(|(door_index, direction)| (door_index, direction != 0)),
                doors,
            );
            eprintln!(
                "SEEKAREA {{\"event\":\"owner_position\",\"frame\":{},\"owner\":{:?},\"owner_creation_order\":{},\"raw\":[{},{}],\"effective\":[{},{}],\"selected_door\":{:?}}}",
                frame,
                npc_id,
                creation_order,
                me_pos.x,
                me_pos.y,
                owner_effective_position.x,
                owner_effective_position.y,
                owner_selected_door,
            );
        }
        for (other_id, other) in self.world.entities.soldiers() {
            if other_id == npc_id {
                continue;
            }
            let Some(other_ai) = other.npc.ai_brain.enemy() else {
                continue;
            };
            // Original `RHElementActorNPC::GetAlertStatus()` reads
            // `mViewParameters.ubAlertStatus`, not the independently tracked
            // music alert. Forced-attentive and music-only transitions can
            // deliberately make those values differ.
            let alert_status = other_ai.base.view_alert_status;
            let friend_raw_position = other.element.position_map();
            if alert_status == crate::ai::AlertLevel::Green {
                if seek_area_debug {
                    let friend_selected_door = selected_pass_door_movement(
                        &self.orders.sequence_manager,
                        EntityId::Soldier(other_id),
                    );
                    let friend_effective_position = seek_area_friend_position_map(
                        friend_raw_position,
                        friend_selected_door
                            .map(|(door_index, direction)| (door_index, direction != 0)),
                        doors,
                    );
                    eprintln!(
                        "SEEKAREA {{\"event\":\"friend_contribution\",\"frame\":{},\"owner_creation_order\":{},\"friend\":{:?},\"friend_creation_order\":{},\"alert\":{:?},\"raw\":[{},{}],\"effective\":[{},{}],\"selected_door\":{:?},\"contributes\":false,\"reason\":\"green\"}}",
                        frame,
                        creation_order,
                        other_id,
                        self.world
                            .original_creation_order(EntityId::Soldier(other_id)),
                        alert_status,
                        friend_raw_position.x,
                        friend_raw_position.y,
                        friend_effective_position.x,
                        friend_effective_position.y,
                        friend_selected_door,
                    );
                }
                continue;
            }
            let friend_seeks_with_help = other_ai.base.current_substate.is_seek_area()
                && other_ai
                    .seek_flags
                    .contains(crate::ai_enemy::SeekFlags::LOOK_FOR_HELP_AFTER);
            let contribution = seek_area_friend_contribution(
                &self.orders.sequence_manager,
                EntityId::Soldier(other_id),
                me_pos,
                friend_raw_position,
                doors,
                friend_seeks_with_help,
            );
            if seek_area_debug {
                let friend_selected_door = selected_pass_door_movement(
                    &self.orders.sequence_manager,
                    EntityId::Soldier(other_id),
                );
                let friend_effective_position = seek_area_friend_position_map(
                    friend_raw_position,
                    friend_selected_door
                        .map(|(door_index, direction)| (door_index, direction != 0)),
                    doors,
                );
                eprintln!(
                    "SEEKAREA {{\"event\":\"friend_contribution\",\"frame\":{},\"owner_creation_order\":{},\"friend\":{:?},\"friend_creation_order\":{},\"alert\":{:?},\"raw\":[{},{}],\"effective\":[{},{}],\"selected_door\":{:?},\"contributes\":{},\"clears_help\":{}}}",
                    frame,
                    creation_order,
                    other_id,
                    self.world
                        .original_creation_order(EntityId::Soldier(other_id)),
                    alert_status,
                    friend_raw_position.x,
                    friend_raw_position.y,
                    friend_effective_position.x,
                    friend_effective_position.y,
                    friend_selected_door,
                    contribution.is_some(),
                    contribution.unwrap_or(false),
                );
            }
            let Some(clears_help) = contribution else {
                continue;
            };
            tick.visible_seeking_friends += 1;
            if clears_help {
                tick.friend_seek_clears_help_flag = true;
            }
        }
        if seek_area_debug {
            eprintln!(
                "SEEKAREA {{\"event\":\"friend_summary\",\"frame\":{},\"owner_creation_order\":{},\"visible_friends\":{},\"clears_help\":{}}}",
                frame,
                creation_order,
                tick.visible_seeking_friends,
                tick.friend_seek_clears_help_flag,
            );
        }
        tick.camp_soldiers =
            self.build_camp_soldier_tick_infos(npc_id, my_camp, scratch, build_forecasts);
        tick.alert_soldier_candidates = self.build_alert_soldier_candidates(npc_id);
        if build_forecasts
            && let Some(enemy_ai) = soldier.npc.ai_brain.enemy()
            && enemy_ai.missed_pc != 0
            && let Some(missed_id) = self.entity_id_for_index(enemy_ai.missed_pc)
            && let Some(missed_entity) = self.world.entities.get(missed_id)
            && let Some(input) = extract_forecast_input(
                missed_entity,
                selected_actor_is_passing_door(&self.orders.sequence_manager, missed_id),
            )
        {
            let doors = self.script_domains.interactables.doors.as_slice();
            tick.missed_pc_forecast = Some(crate::ai::prepare_forecast_destination_for_ia(
                &input,
                doors,
                &self.world.fast_grid.level.sectors,
                &self.world.fast_grid.level.sector_number_map,
            ));
            tick.missed_pc_is_pc = matches!(missed_entity, Entity::Pc(_));
        }
        // `fill_list_with_all_near_fighters` walks the global fighter
        // registry on every call.  Populate `nearby_fighters` here so
        // off-detection dispatch sites (timer events, reach-point
        // events, panic, patrol, cross-NPC actions, pending-stimuli
        // drain, …) see the same fighter view that the in-detection
        // builder produces.  Without this, AI predicates that consume
        // `tick.nearby_fighters` (rider charge target lookup,
        // PhalanxIsEncercledByEnemies, NumberOfNearbyArchersWhoNeedProtection,
        // ReconsiderPhalanx geometry, IsAnyFriendInThisPolygon)
        // observe an empty list outside swordfight substates.
        tick.nearby_fighters =
            self.build_nearby_fighters_for(npc_id, assets, &scratch.ai_sight_obstacles);
        tick.fighter_registry = self.build_fighter_snapshots_for(npc_id, assets, None);
        tick.reconsider_swordfight_observation_fighters = tick
            .fighter_registry
            .iter()
            .map(|fighter| {
                let id = self.entity_id_for_index(fighter.handle).unwrap_or_else(|| {
                    panic!(
                        "NPC {} observation fighter {} disappeared from the registry",
                        npc_id.index(),
                        fighter.handle
                    )
                });
                let raw_world_position = self
                    .world
                    .entities
                    .get(id)
                    .unwrap_or_else(|| {
                        panic!(
                            "NPC {} observation fighter {} has no live entity",
                            npc_id.index(),
                            fighter.handle
                        )
                    })
                    .element_data()
                    .position();
                crate::ai::ReconsiderSwordfightObservationFighter {
                    handle: fighter.handle,
                    raw_world_position,
                    is_friendly: fighter.is_friendly,
                    is_able_to_fight: fighter.is_able_to_fight,
                    is_soldier: fighter.is_soldier,
                    primary_target: fighter.primary_target,
                    current_substate: fighter.current_substate,
                }
            })
            .collect();
        if reconsider_observation_debug_enabled() {
            let creation_order = self.world.original_creation_order(npc_id);
            if reconsider_observation_debug_matches(frame, creation_order, me_handle) {
                eprintln!(
                    "RECONSIDER {{\"event\":\"snapshot_begin\",\"frame\":{},\"owner\":{},\"owner_creation_order\":{},\"owner_state\":{:?},\"owner_substate\":{:?},\"registry_len\":{}}}",
                    frame,
                    me_handle,
                    creation_order,
                    soldier.npc.ai_state(),
                    soldier.npc.ai_substate(),
                    tick.reconsider_swordfight_observation_fighters.len(),
                );
                for (ordinal, fighter) in tick
                    .reconsider_swordfight_observation_fighters
                    .iter()
                    .enumerate()
                {
                    let fighter_id =
                        self.entity_id_for_index(fighter.handle).unwrap_or_else(|| {
                            panic!(
                                "RECONSIDER owner {npc_id:?} cannot resolve fighter {}",
                                fighter.handle
                            )
                        });
                    let entity = self.world.entities.get(fighter_id).unwrap_or_else(|| {
                        panic!("RECONSIDER owner {npc_id:?} cannot read fighter {fighter_id:?}")
                    });
                    let current_sequence = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(fighter_id)
                        .and_then(|(sequence_id, element_index)| {
                            self.orders
                                .sequence_manager
                                .get_element(sequence_id, element_index)
                                .map(|element| {
                                    (sequence_id, element_index, element.command, element.state)
                                })
                        });
                    let eligibility = match entity {
                        Entity::Soldier(other) => Some((
                            other.npc.life_points <= 0,
                            other.human.unconscious,
                            other.element.posture == crate::element::Posture::Tied,
                            other.human.carrier.is_some(),
                            other.element.active,
                            other.npc.ai_state(),
                            other.npc.ai_substate(),
                        )),
                        _ => None,
                    };
                    eprintln!(
                        "RECONSIDER {{\"event\":\"snapshot_fighter\",\"frame\":{},\"owner_creation_order\":{},\"ordinal\":{},\"fighter\":{},\"fighter_creation_order\":{},\"friendly\":{},\"able\":{},\"raw\":[{},{},{}],\"soldier_eligibility\":{:?},\"current_sequence\":{:?}}}",
                        frame,
                        creation_order,
                        ordinal,
                        fighter.handle,
                        self.world.original_creation_order(fighter_id),
                        fighter.is_friendly,
                        fighter.is_able_to_fight,
                        fighter.raw_world_position.x,
                        fighter.raw_world_position.y,
                        fighter.raw_world_position.z,
                        eligibility,
                        current_sequence,
                    );
                }
            }
        }
        // KillNearbySleepingEnemies walks the live opposing-camp fighter
        // registry synchronously at the BattleDecisions boundary. Populate
        // this from the same live registry for every Think, including timer
        // events that do not own a RefreshDetection VIEW/OUTOFVIEW aggregate.
        // Restricting this field to the detection aggregate leaves the final
        // fallback blind whenever no optical stimulus was queued.
        tick.nearby_sleeping_enemies =
            sleeping_enemy_candidates_from_fighter_registry(&tick.fighter_registry);
        tick.reconsider_swordfight_enemies = tick
            .fighter_registry
            .iter()
            .filter(|fighter| !fighter.is_friendly)
            .cloned()
            .collect();
        tick.reconsider_swordfight_friends = self.build_reconsider_swordfight_friends_for(npc_id);

        // Phalanx right-chain "them" snapshots — consumed by
        // `PhalanxReinitializeThemList` so the leftmost member can
        // union each neighbour's enemies via the recursion.  Always
        // populated; empty when this NPC has no right neighbour.
        tick.phalanx_member_them_lists = self.build_phalanx_member_them_lists(npc_id);

        // Patrol-chief data is live per-dispatch context, not specific to
        // CALL_PATROL_COORDINATE. ReturnToDuty can synchronously enter
        // DefaultGotoChief and immediately receive EventReachPoint; that
        // handler faces this position on the same Think stack. Leaving the
        // general builder's stub origin here made every such member face
        // sector 15 even though the preceding GoNear used the real chief.
        if let Some(chief_id) = ai.patrol_chief {
            let chief = self.world.entities.get(chief_id).unwrap_or_else(|| {
                panic!(
                    "enemy tick context owner {} has stale patrol chief {}",
                    npc_id.index(),
                    chief_id.index()
                )
            });
            let chief_ai = chief.ai_controller().unwrap_or_else(|| {
                panic!(
                    "enemy tick context patrol chief {} has no AI",
                    chief_id.index()
                )
            });
            let chief_point = chief.element_data().position_map();
            tick.patrol_chief_position = crate::ai::Position {
                x: chief_point.x,
                y: chief_point.y,
                sector: chief.element_data().sector(),
                level: chief.element_data().layer(),
            };
            tick.patrol_chief_state = chief_ai.current_state;
        }

        // Avenger-on-roof wait positions — only computed when the AI
        // set the `couldnt_reachpoint` flag.  Decision arms re-pick
        // their target from the personal enemy list (even from a null
        // pre-think target), so compute one wait position per
        // candidate handle plus the current target; consumers resolve
        // their own live handle at use time.
        if couldnt_reachpoint {
            assert!(
                self.scripts.mission.is_some(),
                "AI roof recovery requires an installed mission script"
            );
            let doors_slice = self.script_domains.interactables.doors.as_slice();
            let mut candidates: Vec<crate::ai::HumanHandle> = Vec::new();
            if let Some(enemy_ai) = soldier.npc.ai_brain.enemy() {
                candidates.extend(enemy_ai.list_them.iter().copied());
            }
            if primary_target_handle != 0 {
                candidates.push(primary_target_handle);
            }
            candidates.retain(|&h| h != 0);
            candidates.dedup();
            for handle in candidates {
                if tick
                    .avenger_on_roof_wait_positions
                    .iter()
                    .any(|(h, _)| *h == handle)
                {
                    continue;
                }
                let Some(candidate_id) = self.entity_id_for_index(handle) else {
                    continue;
                };
                if let Some(wait) = precompute_avenger_on_roof_wait_position(
                    &self.world.entities,
                    doors_slice,
                    &self.orders.sequence_manager,
                    npc_id,
                    candidate_id,
                    &|sector| self.building_sector_is_authorized(sector),
                    &|sector| self.get_sector_lift_type(sector),
                ) {
                    tick.avenger_on_roof_wait_positions.push((handle, wait));
                }
            }
        }

        let Some(target_id) = target_id else {
            // No target selected — primary-target fields stay None,
            // enemy_sq_distances stays empty.  Friend-swap still
            // scans the other soldiers; the helper handles the
            // empty-target case.
            tick.friend_swap_candidates = build_friend_swap_candidates(
                &self.world.entities,
                doors,
                &self.orders.sequence_manager,
                npc_id,
                my_camp,
            );
            return tick;
        };

        // Primary target metadata (position, posture, animation,
        // carrier) from the live entity store.
        let target_meta = lookup_primary_target_metadata(
            &self.world.entities,
            &self.orders.sequence_manager,
            self.script_domains.interactables.doors.as_slice(),
            target_id,
        );

        if let Some((pos, posture, anim, carrier_pos, carrier_handle)) = target_meta {
            tick.primary_target_position = Some(pos);
            let target = self
                .world
                .entities
                .get(target_id)
                .unwrap_or_else(|| panic!("resolved primary target {target_id:?} disappeared"));
            let element = target.element_data();
            tick.primary_target_live_position = Some(crate::ai::Position {
                x: element.position_map().x,
                y: element.position_map().y,
                sector: element.sector(),
                level: element.layer(),
            });
            tick.primary_target_posture = Some(posture);
            tick.primary_target_animation = anim;
            tick.primary_target_carrier_position = carrier_pos;
            tick.primary_target_carrier_handle = carrier_handle;

            // Seed enemy_sq_distances from the primary target so
            // `battle_decisions` sees a non-empty list when the
            // soldier has a valid target — same rationale as the
            // timer-dispatch seed at line 5916.
            let dx = pos.x - me_pos.x;
            let dy = (pos.y - me_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let sq = (dx * dx + dy * dy) as i32;
            tick.enemy_sq_distances.push((target_id.index(), sq));
            tick.min_sq_enemy_distance = sq;
        }

        // primary_target_is_pc: look up the target's entity variant.
        tick.primary_target_is_pc =
            matches!(self.world.entities.get(target_id), Some(Entity::Pc(_)));
        if build_forecasts
            && let Some(target_entity) = self.world.entities.get(target_id)
            && let Some(input) = extract_forecast_input(
                target_entity,
                selected_actor_is_passing_door(&self.orders.sequence_manager, target_id),
            )
        {
            let doors = self.script_domains.interactables.doors.as_slice();
            tick.primary_target_forecast = Some(crate::ai::prepare_forecast_destination_for_ia(
                &input,
                doors,
                &self.world.fast_grid.level.sectors,
                &self.world.fast_grid.level.sector_number_map,
            ));
        }
        tick.primary_target_jump_line = crate::engine::melee::is_table_swordfight_needed(
            &self.world.entities,
            &self.world.fast_grid,
            &assets.profile_manager,
            npc_id,
            target_id,
        );
        tick.primary_target_multiplicity = self
            .ai
            .global
            .primary_target_multiplicity_scratch
            .iter()
            .map(|(&target, &count)| (target, count))
            .collect();

        if let Some(enemy_ai) = soldier.npc.ai_brain.enemy() {
            let my_company = enemy_ai.company_number;
            let my_pride = enemy_ai.soldier_profile_pride;
            tick.us_battle_points = 100 + my_pride as u32;

            let self_to_target_sq = tick.primary_target_position.map(|target_pos| {
                let dx = target_pos.x - me_pos.x;
                let dy =
                    (target_pos.y - me_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                dx * dx + dy * dy
            });

            for friend in &tick.nearby_fighters {
                if !friend.is_friendly || friend.handle == me_handle || !friend.is_able_to_fight {
                    continue;
                }

                if friend.is_pc {
                    tick.us_battle_points += 100;
                    if my_company > 0 {
                        tick.friends_lower_company = tick.friends_lower_company.saturating_add(1);
                    }
                    continue;
                }

                if !matches!(
                    friend.ai_state,
                    crate::ai::AiState::Default
                        | crate::ai::AiState::Wondering
                        | crate::ai::AiState::Seeking
                        | crate::ai::AiState::Attacking
                ) {
                    continue;
                }

                let friend_company = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == friend.handle)
                    .map(|cs| cs.company_number)
                    .unwrap_or(u16::MAX);
                if my_company > friend_company
                    && (ai.current_substate == crate::ai::Substate::AttackingReactiontime
                        || friend.ai_state == crate::ai::AiState::Attacking)
                {
                    tick.friends_lower_company = tick.friends_lower_company.saturating_add(1);
                }

                if my_pride > friend.soldier_profile_pride {
                    tick.soldiers_lower_pride = true;
                }
                tick.us_battle_points += 100 + friend.soldier_profile_pride as u32;

                if friend.rank == crate::profiles::ProfileRank::Soldier {
                    tick.simple_soldiers_near = true;
                }
                if friend.rank == crate::profiles::ProfileRank::Officer {
                    tick.has_officer_nearby = true;
                }

                if friend.ai_state == crate::ai::AiState::Attacking && friend.primary_target != 0 {
                    if crate::ai_enemy::is_any_swordfight_substate(friend.current_substate) {
                        tick.friends_nearer_to_enemy =
                            tick.friends_nearer_to_enemy.saturating_add(1);
                    } else if let Some(self_sq) = self_to_target_sq {
                        let Some(target_pos) = tick.primary_target_position else {
                            continue;
                        };
                        let dx = friend.position.x - target_pos.x;
                        // Original deliberately compares two differently
                        // shaped distances here: `SquareDistance(primary)`
                        // stretches the owner's Y delta, while the friend's
                        // raw `RHposition` delta uses an unstretched
                        // `SquareNorm()`.
                        let dy = friend.position.y - target_pos.y;
                        if dx * dx + dy * dy < self_sq {
                            tick.friends_nearer_to_enemy =
                                tick.friends_nearer_to_enemy.saturating_add(1);
                        }
                    }
                }
            }

            for &(attacker, target) in &self.ai.global.same_frame_target_claims {
                if attacker == me_handle || target == 0 {
                    continue;
                }
                let attacker_id = EntityId::Soldier(SoldierId(attacker));
                let Some(Entity::Soldier(s)) = self.world.entities.get(attacker_id) else {
                    continue;
                };
                if s.soldier.cached_camp != my_camp
                    || !s.element.active
                    || s.human.unconscious
                    || s.npc.life_points <= 0
                {
                    continue;
                }
                if target == primary_target_handle {
                    tick.friends_nearer_to_enemy = tick.friends_nearer_to_enemy.saturating_add(1);
                }
            }
        }

        // Friend-swap candidates for ReconsiderEnemyApproach.
        tick.friend_swap_candidates = build_friend_swap_candidates(
            &self.world.entities,
            doors,
            &self.orders.sequence_manager,
            npc_id,
            my_camp,
        );

        // Stashed-exit-door snapshot for the AlertSoldiers indoor
        // branch and the merry-man flee path.  Always populated
        // whenever the AI has stashed a door (irrespective of
        // in-building status), so paths that reach the door's
        // point_out through a sequence of substates still see the
        // cached geometry.  No fallback when no door is stashed.
        let stashed = soldier.npc.ai_brain.base().and_then(|ai| ai.my_door_index);
        if stashed.is_some() {
            assert!(
                self.scripts.mission.is_some(),
                "stashed AI exit-door state requires an installed mission script"
            );
            let doors_slice = self.script_domains.interactables.doors.as_slice();
            tick.my_exit_door = build_my_exit_door_info(stashed, doors_slice);
        }

        tick
    }

    /// Rank-soldier NPCs of *every* camp, in NPC registry order.
    ///
    /// `CommandSoldiersToAttack` walks the whole NPC array and gates each
    /// entry on rank, body state and the candidate's own 360° detection of
    /// the officer — no camp test anywhere. Feeding it the same-camp
    /// snapshot dropped the opposing camp's soldiers from both the alert
    /// broadcast and the observable detection-call stream.
    fn build_alert_soldier_candidates(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai_enemy::AlertSoldierCandidate> {
        let mut candidates = Vec::new();
        for other_id in self.world.entities.npc_ids() {
            if other_id == npc_id {
                continue;
            }
            let Some(entity) = self.world.entities.get(other_id) else {
                continue;
            };
            let crate::element::Entity::Soldier(s) = entity else {
                continue;
            };
            let Some(enemy_ai) = s.npc.ai_brain.enemy() else {
                continue;
            };
            if enemy_ai.soldier_profile_rank != crate::profiles::ProfileRank::Soldier
                || !crate::element::Human::is_able_to_fight(s)
            {
                continue;
            }
            let position = s.element.position_map();
            candidates.push(crate::ai_enemy::AlertSoldierCandidate {
                handle: other_id.index(),
                position: crate::ai::Position {
                    x: position.x,
                    y: position.y,
                    sector: s.element.sector(),
                    level: s.element.layer(),
                },
                elevation: s.element.sprite.position_iface.get_elevation(),
                is_rider: s.soldier.rider,
                view_radius: s.npc.view_radius,
                in_building: self.entity_data_in_building_sector(&s.element),
            });
        }
        candidates
    }

    fn build_camp_soldier_tick_infos(
        &self,
        npc_id: crate::element::EntityId,
        my_camp: crate::element::Camp,
        _scratch: &SimScratch,
        forecast_destinations: bool,
    ) -> Vec<crate::ai_enemy::CampSoldierInfo> {
        let mut camp_soldiers =
            Vec::with_capacity(self.world.entities.soldiers().count().saturating_sub(1));
        for (other_id, s) in self.world.entities.soldiers() {
            if other_id == npc_id {
                continue;
            }
            // GetNumberOfSoldiers(camp) includes unconscious and inactive
            // soldiers. Individual Original consumers apply their own gates:
            // CreateListOfSoldiersYouCanAlert retains everyone of the allowed
            // rank, while GetNearestFighter rejects dead, unconscious, and
            // inactive candidates.
            if s.soldier.cached_camp != my_camp {
                continue;
            }
            let able_to_fight = crate::element::Human::is_able_to_fight(s);
            let alive_and_conscious = s.npc.life_points > 0 && !s.human.unconscious;
            let Some(enemy_ai) = s.npc.ai_brain.enemy() else {
                continue;
            };
            let in_building = self.entity_data_in_building_sector(&s.element);
            let forecast_destination = if forecast_destinations {
                // Missing scripts are a recoverable developer-data load path;
                // `init_ai` warns once before these per-NPC snapshots are built.
                let doors = self
                    .scripts
                    .mission
                    .as_ref()
                    .map(|_| self.script_domains.interactables.doors.as_slice())
                    .unwrap_or(&[]);
                let pos_now = s.element.position_map();
                let live_door = s.element.sprite.position_iface.get_door();
                let door_pass = (selected_actor_is_passing_door(
                    &self.orders.sequence_manager,
                    EntityId::Soldier(other_id),
                ) && !live_door.is_null())
                .then_some((
                    crate::gate::DoorIndex(live_door.0),
                    s.actor.passing_door_directly,
                ));
                let input = crate::ai::ForecastInput {
                    position_map_x: pos_now.x,
                    position_map_y: pos_now.y,
                    sector: s.element.sector().map(u16::from).unwrap_or(0),
                    layer: s.element.layer(),
                    direction: s.element.direction() as u16,
                    forecasted_movement_z: s
                        .element
                        .sprite
                        .position_iface
                        .get_forecasted_movement()
                        .z,
                    door_pass,
                    passing_door_directly: s.actor.passing_door_directly,
                };
                Some(crate::ai::prepare_forecast_destination_for_ia(
                    &input,
                    doors,
                    &self.world.fast_grid.level.sectors,
                    &self.world.fast_grid.level.sector_number_map,
                ))
            } else {
                None
            };
            let position = s.element.position_map();
            // Snapshot the soldier's `DETECTABLE_BODY` list — handles of
            // corpses they have not yet reacted to.  Snapshotting the
            // data here lets AI predicates run off `tick.camp_soldiers`
            // alone instead of poking at the live detectable list.
            let detectable_body_idx = crate::element::DetectableType::Body as usize;
            let detectable_bodies = s
                .npc
                .detectable_lists
                .get(detectable_body_idx)
                .map(|list| {
                    let mut bodies = Vec::with_capacity(list.len());
                    bodies.extend(list.iter().filter_map(|d| d.element.map(|e| e.index())));
                    bodies
                })
                .unwrap_or_default();
            let cs_position = crate::ai::Position {
                x: position.x,
                y: position.y,
                sector: s.element.sector(),
                level: s.element.layer(),
            };
            let eye_blind = s.npc.eye_status.is_blind();
            camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
                handle: other_id.index(),
                active: s.element.active,
                position: cs_position,
                position_world: s.element.position(),
                direction: s.element.direction() as u16,
                rank: enemy_ai.soldier_profile_rank,
                ai_state: s.npc.ai_state(),
                ai_substate: s.npc.ai_substate(),
                is_able_to_fight: able_to_fight,
                is_dead: s.npc.life_points <= 0,
                primary_target: enemy_ai.base.primary_target,
                pride: enemy_ai.soldier_profile_pride,
                is_able_to_help: crate::ai_enemy::soldier_is_able_to_help_state(
                    alive_and_conscious,
                    s.npc.ai_state(),
                    s.npc.ai_substate(),
                ),
                script_locked: enemy_ai.base.script_locked,
                ai_lock_frozen: enemy_ai
                    .base
                    .locks_flag_field
                    .contains(crate::ai::AiLockFlags::FREEZE),
                layer: s.element.layer(),
                report_type: enemy_ai.base.my_reconnaissance_report.report_type,
                report_seek_position: enemy_ai.base.my_reconnaissance_report.seek_position,
                report_seen_bodies: enemy_ai.base.my_reconnaissance_report.seen_bodies.clone(),
                report_charly: enemy_ai.base.my_reconnaissance_report.charly,
                alert_soldiers_point: enemy_ai.base.alert_soldiers_point,
                patrol_chief: enemy_ai.base.patrol_chief,
                antagonist: enemy_ai.base.antagonist,
                detected_body: enemy_ai.base.detected_body,
                duty_flag: enemy_ai.soldier_profile_duty,
                is_tower_guard: enemy_ai.tower_guard,
                company_number: enemy_ai.company_number,
                in_building,
                forecast_destination,
                detectable_bodies,
                seek_position: enemy_ai.base.seek_position,
                current_task_priority: enemy_ai.current_task_priority,
                minimal_task_priority: enemy_ai.minimal_task_priority,
                view_direction: s.npc.view_direction,
                view_radius: s.npc.view_radius,
                real_half_aperture: s.npc.real_half_aperture,
                eye_blind,
            });
        }
        camp_soldiers
    }

    /// Build a `nearby_fighters` snapshot list for one enemy NPC.
    ///
    /// Walks the entity store directly, the same scan-the-global-fighter-
    /// registry approach used by swordfight reconsideration. Filters
    /// non-self entries to the same 500-unit Chebyshev radius the
    /// detection-pass builder applies.
    ///
    /// Returns an empty Vec for non-enemy soldiers — civilians and
    /// PCs don't consume `nearby_fighters`.
    pub(super) fn build_nearby_fighters_for(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        _sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        self.build_fighter_snapshots_for(npc_id, assets, Some(500.0))
    }

    #[cfg(test)]
    pub(super) fn build_full_fighter_registry_for_test(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        self.build_fighter_snapshots_for(npc_id, assets, None)
    }

    fn build_fighter_snapshots_for(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        max_distance: Option<f32>,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        use crate::ai::Position;
        use crate::ai_enemy::FighterSnapshot;
        use crate::element::Posture;

        let Some(Entity::Soldier(soldier)) = self.world.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(enemy_ai) = soldier.npc.ai_brain.enemy() else {
            return Vec::new();
        };
        let doors = self.script_domains.interactables.doors.as_slice();
        let fighter_position = |id: crate::element::EntityId| {
            resolve_ai_position_with(
                &self.world.entities,
                doors,
                &self.orders.sequence_manager,
                id,
                |position_id| {
                    let element = self
                        .world
                        .entities
                        .get(position_id)
                        .unwrap_or_else(|| {
                            panic!("fighter snapshot owner {position_id:?} disappeared")
                        })
                        .element_data();
                    Position {
                        x: element.position_map().x,
                        y: element.position_map().y,
                        sector: element.sector(),
                        level: element.layer(),
                    }
                },
            )
            .effective
        };
        let me_position = fighter_position(npc_id);
        let me_pos_pt = crate::coordinates::MapPoint::new(me_position.x, me_position.y);
        let me_elevation = soldier.element.position().z;
        let my_camp = soldier.soldier.cached_camp;
        let me_handle = enemy_ai.base.me;

        // Build a friendly soldier snapshot for `handle` (which may be self).
        // The original fighter registry retains inactive and out-of-order
        // soldiers, and `FillListWithAllNearFighters` inserts self before it
        // applies `IsAbleToFight` to the remaining registry entries.
        let build_soldier = |handle: u32, require_able: bool| -> Option<FighterSnapshot> {
            let s = self.world.entities.get_soldier(SoldierId(handle))?;
            let is_able_to_fight = s.is_able_to_fight();
            if require_able && !is_able_to_fight {
                return None;
            }
            let position = fighter_position(EntityId::Soldier(SoldierId(handle)));
            let enemy_ai_other = s
                .npc
                .ai_brain
                .enemy()
                .unwrap_or_else(|| panic!("active soldier {handle} has no EnemyAi brain"));
            let soldier_profile = assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier {handle} requires missing soldier profile {}",
                        u32::from(s.soldier.soldier_profile_index)
                    )
                });
            let has_formation = soldier_profile.formation;
            let fighting_ability = {
                let base = soldier_profile.fighting;
                if s.soldier.cached_camp == Camp::Lacklandists {
                    let diff = self.control.sim_config.difficulty;
                    diff.modify_capacity(
                        base,
                        crate::player_profile::difficulty_params::EASY_ENEMY_FIGHTING,
                        crate::player_profile::difficulty_params::HARD_ENEMY_FIGHTING,
                        100,
                    )
                } else {
                    base
                }
            };
            let bow_profile = if soldier_profile.shooting_weapon_id == 0 {
                None
            } else {
                Some(
                    assets
                        .profile_manager
                        .get_bow(soldier_profile.shooting_weapon_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "soldier {handle} requires missing bow profile {}",
                                soldier_profile.shooting_weapon_id
                            )
                        }),
                )
            };
            let is_archer_unit = snapshots::is_archer_from_bow(bow_profile);
            let bow_max_range = bow_profile
                .map(|bow| {
                    if bow.has_long_shoot {
                        bow.long_shoot.range
                    } else {
                        bow.normal_shoot.range
                    }
                })
                .unwrap_or(0);
            let hth_id = enemy_ai_other.hth_weapon_id;
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_id)
                .unwrap_or_else(|| {
                    panic!("soldier {handle} requires missing HtH weapon profile {hth_id}")
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            let weapon_is_shield = hth_profile.shield;
            let has_shield_anim = s
                .element
                .sprite
                .has_animation(crate::order::OrderType::WaitingShield);
            let is_shield_bearer = weapon_is_shield && has_shield_anim;
            let in_recovery = self.actor_is_in_sword_recovery(EntityId::Soldier(SoldierId(handle)));
            let seek_position = Position {
                x: enemy_ai_other.base.seek_position.x,
                y: enemy_ai_other.base.seek_position.y,
                sector: enemy_ai_other.base.seek_position.sector,
                level: s.element.layer(),
            };
            let opponent_handles: Vec<u32> =
                s.human.opponents.iter().map(|id| id.index()).collect();
            let number_of_opponents = opponent_handles.len().min(u16::MAX as usize) as u16;
            let is_friendly = s.soldier.cached_camp == my_camp;
            Some(FighterSnapshot {
                handle,
                position: Position {
                    x: position.x,
                    y: position.y,
                    // `Position(entity)` in Original copies the complete
                    // RHposition, including its authoritative sector
                    // pointer.  Combat helpers later copy this position
                    // when deriving destinations (notably the archer
                    // cover point behind a stationary shield bearer), so
                    // discarding the sector here turns an otherwise valid
                    // same-sector GoTo into EVENT_COULDNT_REACHPOINT.
                    sector: position.sector,
                    level: position.level,
                },
                direction: s.element.direction() as u16,
                is_friendly,
                is_swordfighting: !s.human.opponents.is_empty(),
                is_able_to_fight,
                is_tied: s.element.posture == Posture::Tied,
                is_unconscious: s.human.unconscious,
                is_dead: s.npc.life_points <= 0,
                is_carried: s.human.carrier.is_some(),
                is_pc: false,
                is_soldier: true,
                rank: enemy_ai_other.soldier_profile_rank,
                primary_target: enemy_ai_other.base.primary_target,
                principal_opponent: s.human.opponents.first().map(|id| id.index()).unwrap_or(0),
                number_of_opponents,
                opponent_handles,
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                has_formation,
                is_shield_bearer,
                is_archer_unit,
                is_tower_guard: enemy_ai_other.tower_guard,
                is_vip: soldier_profile.vip,
                soldier_profile_pride: enemy_ai_other.soldier_profile_pride,
                is_robin: false,
                left_combat_neighbour: enemy_ai_other.left_combat_neighbour,
                right_combat_neighbour: enemy_ai_other.right_combat_neighbour,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: s.actor.action_state.is_sword(),
                elevation: s.element.position().z,
                seek_position,
                current_substate: s.npc.ai_substate() as u32,
                archer_behind_me: enemy_ai_other.archer_behind_me,
                ai_state: s.npc.ai_state(),
                shield_bearer_before_me: enemy_ai_other.shield_bearer_before_me,
                hth_weapon_id: hth_id,
                action_state: s.actor.action_state,
                shield_bearer_direction: enemy_ai_other.shield_bearer_direction,
                shield_bearer_seek_position: seek_position,
                bow_max_range,
            })
        };

        // Build a PC snapshot for `handle`. PCs belong to the Royalist camp,
        // so they are enemies for Lackland soldiers and friends for friendly
        // forest soldiers.
        let build_pc = |handle: u32, require_able: bool| -> Option<FighterSnapshot> {
            let pc = self.world.entities.get_pc(PcId(handle))?;
            let is_dead = pc.pc.life_points <= 0;
            let is_unconscious = pc.human.unconscious;
            let is_able_to_fight = pc.element.active
                && !is_dead
                && !is_unconscious
                && !matches!(pc.element.posture, Posture::Tree | Posture::Spy);
            if require_able && !is_able_to_fight {
                return None;
            }
            let is_carried = pc.human.carrier.is_some();
            let position = fighter_position(EntityId::Pc(PcId(handle)));
            let character = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {handle} requires missing character profile {}",
                        u32::from(pc.pc.profile_index)
                    )
                });
            let hth_id = character.hth_weapon_id;
            let fighting_ability = character.fighting;
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_id)
                .unwrap_or_else(|| {
                    panic!("PC {handle} requires missing HtH weapon profile {hth_id}")
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            let in_recovery =
                !is_able_to_fight || self.actor_is_in_sword_recovery(EntityId::Pc(PcId(handle)));
            let opponent_handles: Vec<u32> =
                pc.human.opponents.iter().map(|id| id.index()).collect();
            let number_of_opponents = opponent_handles.len().min(u16::MAX as usize) as u16;
            let live_position = pc.element.position_map();
            let pc_seek_position = Position {
                x: live_position.x,
                y: live_position.y,
                sector: pc.element.sector(),
                level: pc.element.layer(),
            };
            Some(FighterSnapshot {
                handle,
                position,
                direction: pc.element.direction() as u16,
                is_friendly: my_camp == Camp::Royalists,
                is_swordfighting: !pc.human.opponents.is_empty(),
                is_able_to_fight,
                is_tied: pc.element.posture == Posture::Tied,
                is_unconscious,
                is_dead,
                is_carried,
                is_pc: true,
                is_soldier: false,
                rank: crate::profiles::ProfileRank::None,
                primary_target: pc.pc.melee_target.map(|id| id.index()).unwrap_or(0),
                principal_opponent: pc.human.opponents.first().map(|id| id.index()).unwrap_or(0),
                number_of_opponents,
                opponent_handles,
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                has_formation: false,
                is_shield_bearer: false,
                is_archer_unit: false,
                is_tower_guard: false,
                is_vip: character.vip,
                soldier_profile_pride: 0,
                is_robin: pc.pc.robin,
                left_combat_neighbour: 0,
                right_combat_neighbour: 0,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: pc.actor.action_state.is_sword(),
                elevation: pc.element.sprite.position_iface.get_elevation(),
                seek_position: pc_seek_position,
                current_substate: 0,
                archer_behind_me: 0,
                ai_state: crate::ai::AiState::default(),
                shield_bearer_before_me: 0,
                hth_weapon_id: hth_id,
                action_state: pc.actor.action_state,
                shield_bearer_direction: 0,
                shield_bearer_seek_position: pc_seek_position,
                bow_max_range: 0,
            })
        };

        let mut out: Vec<FighterSnapshot> = Vec::with_capacity(1 + self.world.pc_ids.len() + 4);

        // Self entry first — no radius filter (the AI is at distance 0).
        out.push(build_soldier(me_handle, false).unwrap_or_else(|| {
            panic!("enemy AI self {me_handle} is absent from the fighter registry")
        }));

        // Walk the registration order so each camp's fighter order matches
        // Original's append-only registry even when PCs and soldiers are
        // interleaved. Friendly scans still put `mpMe` first, as
        // `FillListWithAllNearFighters` does explicitly.
        for id in self.world.fighter_registry_order() {
            if id == npc_id {
                continue;
            }
            let Some(entity) = self.world.entities.get(id) else {
                continue;
            };
            let (position, elevation, snapshot) = match entity {
                Entity::Soldier(soldier) => (
                    fighter_position(id),
                    soldier.element.position().z,
                    // Radius-limited snapshots model
                    // FillListWithAllNearFighters and therefore exclude
                    // unable fighters. The complete registry is the backing
                    // store for already-held Original pointers, which remain
                    // dereferenceable while their owner decides how to prune
                    // them.
                    build_soldier(id.index(), max_distance.is_some()),
                ),
                Entity::Pc(pc) => (
                    fighter_position(id),
                    pc.element.sprite.position_iface.get_elevation(),
                    build_pc(id.index(), max_distance.is_some()),
                ),
                _ => continue,
            };
            // `MaxNormDistance` subtracts full world positions before
            // stretching Y, so the elevation enters twice: once as the
            // projection offset baked into map Y and once as its own
            // component. Comparing raw map coordinates instead pushed
            // fighters standing a layer above or below out of every
            // consideration radius built on this snapshot.
            let world = crate::coordinates::GroundPoint::from_map_and_z(
                crate::coordinates::MapPoint::new(position.x, position.y),
                elevation,
            );
            let me_world = crate::coordinates::GroundPoint::from_map_and_z(me_pos_pt, me_elevation);
            let dx = world.x - me_world.x;
            let dy = (world.y - me_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = elevation - me_elevation;
            if max_distance.is_some_and(|radius| dx.abs().max(dy.abs()).max(dz.abs()) > radius) {
                continue;
            }
            if let Some(snapshot) = snapshot {
                out.push(snapshot);
            }
        }

        out
    }

    /// Build the friendly half of Original `ReconsiderSwordfight`'s
    /// per-call camp-fighter scan.
    ///
    /// This cannot reuse `build_nearby_fighters_for`: that shared cache uses
    /// projected map positions and filters non-self soldiers through
    /// `IsAbleToFight`, while `ReconsiderSwordfight` walks every fighter in
    /// `marrayFighters[myCamp]`, tests only `IsSwordfighting`, and computes
    /// `MaxNormDistance` from full 3D world positions.
    fn build_reconsider_swordfight_friends_for(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai::ReconsiderSwordfightFriend> {
        use crate::ai::ReconsiderSwordfightFriend;

        let Some(Entity::Soldier(me)) = self.world.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(me_ai) = me.npc.ai_brain.enemy() else {
            return Vec::new();
        };
        let me_world = me.element.position();
        let my_camp = me.soldier.cached_camp;
        let radius = crate::parameters_ai::MAX_SWORDFIGHT_CONSIDERATION_RADIUS as u16;
        let mut out = Vec::new();

        // Walk the registration order, matching the Original's append-only
        // camp fighter arrays (including the PC/soldier interleaving
        // established during level creation).
        for id in self.world.fighter_registry_order() {
            let Some(entity) = self.world.entities.get(id) else {
                continue;
            };
            let (handle, world, opponents, same_camp) = match entity {
                Entity::Soldier(friend) => (
                    id.index(),
                    friend.element.position(),
                    &friend.human.opponents,
                    friend.soldier.cached_camp == my_camp,
                ),
                Entity::Pc(friend) => (
                    id.index(),
                    friend.element.position(),
                    &friend.human.opponents,
                    my_camp == Camp::Royalists,
                ),
                _ => continue,
            };
            if handle == me_ai.base.me || !same_camp || opponents.is_empty() {
                continue;
            }

            let dx = (world.x - me_world.x).abs();
            let dy =
                ((world.y - me_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO).abs();
            let dz = (world.z - me_world.z).abs();
            // Original explicitly casts the FLOAT result to UWORD before
            // comparing it with MAX_SWORDFIGHT_CONSIDERATION_RADIUS.
            let max_norm_distance = dx.max(dy).max(dz) as u16;
            if max_norm_distance >= radius {
                continue;
            }
            out.push(ReconsiderSwordfightFriend {
                handle,
                max_norm_distance,
                number_of_opponents: opponents.len().min(u16::MAX as usize) as u16,
            });
        }
        out
    }

    /// Snapshot every right-chain phalanx member (including self) with
    /// their live viewer state, persistent `list_them`, and current
    /// detectable-enemy list. `PhalanxReinitializeThemList` replays the
    /// original recursion over this data without borrowing sibling AI
    /// brains while one member is mutable.
    pub(super) fn build_phalanx_member_them_lists(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai::PhalanxMemberThemList> {
        use crate::ai::{PhalanxEnemySnapshot, PhalanxMemberThemList, Position};
        use crate::element::Human;
        let Some(Entity::Soldier(soldier)) = self.world.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(enemy_ai) = soldier.npc.ai_brain.enemy() else {
            return Vec::new();
        };

        let snapshot_enemy = |handle: u32, member_camp: Camp| -> PhalanxEnemySnapshot {
            let entity_id = self.entity_id_for_index(handle).unwrap_or_else(|| {
                panic!("phalanx member references missing enemy handle {handle}")
            });
            let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
                panic!("phalanx enemy handle {handle} resolved to a vacant entity slot")
            });
            let human = entity
                .human_data()
                .unwrap_or_else(|| panic!("phalanx enemy handle {handle} is not a human entity"));
            let element = entity.element_data();
            let map = element.position_map();
            let able_to_fight = match entity {
                Entity::Pc(pc) => pc.is_able_to_fight(),
                Entity::Soldier(soldier) => soldier.is_able_to_fight(),
                Entity::Civilian(civilian) => civilian.is_able_to_fight(),
                _ => unreachable!("human_data returned Some for a non-human entity"),
            };
            PhalanxEnemySnapshot {
                handle,
                position: Position {
                    x: map.x,
                    y: map.y,
                    sector: element.sector(),
                    level: element.layer(),
                },
                direction: element.direction() as u16,
                posture: element.posture,
                elevation: entity.position_iface().get_elevation(),
                is_rider: entity.soldier_data().is_some_and(|data| data.rider),
                active: element.active,
                able_to_fight,
                dead: entity.is_dead(),
                unconscious: human.unconscious,
                friend: entity.camp() == member_camp,
                in_building: self.entity_data_in_building_sector(element),
                obstacle: element.obstacle_index(),
            }
        };

        let mut out: Vec<PhalanxMemberThemList> = Vec::new();
        let mut current = enemy_ai.base.me;
        // Cap at 16 like the consumer's right-chain walk; phalanxes are
        // small and the cap guards against any cycle in cached neighbour
        // links.
        for _ in 0..16 {
            if current == 0 {
                break;
            }
            let Some(s) = self.world.entities.get_soldier(SoldierId(current)) else {
                break;
            };
            let Some(neighbour_ai) = s.npc.ai_brain.enemy() else {
                break;
            };
            let pos = s.element.position_map();
            let member_camp = s.soldier.cached_camp;
            let current_them_list = neighbour_ai
                .list_them
                .iter()
                .map(|&handle| snapshot_enemy(handle, member_camp))
                .collect();
            let enemy_list = s
                .npc
                .detectable_lists
                .get(crate::element::DetectableType::Enemy as usize)
                .unwrap_or_else(|| panic!("phalanx member {current} has no detectable-enemy list"));
            let detectable_enemies = enemy_list
                .iter()
                .map(|detectable| {
                    let entity_id = detectable.element.unwrap_or_else(|| {
                        panic!("phalanx member {current} has a null detectable enemy")
                    });
                    snapshot_enemy(entity_id.index(), member_camp)
                })
                .collect();
            out.push(PhalanxMemberThemList {
                handle: current,
                entity: self.entity_id_for_index(current).unwrap_or_else(|| {
                    panic!("phalanx member handle {current} is absent from the entity table")
                }),
                current_them_list,
                detectable_enemies,
                position: Position {
                    x: pos.x,
                    y: pos.y,
                    sector: s.element.sector(),
                    level: s.element.layer(),
                },
                direction: s.element.direction() as u16,
                posture: s.element.posture,
                elevation: s.element.sprite.position_iface.get_elevation(),
                is_rider: s.soldier.rider,
                active: s.element.active,
                in_building: self.entity_data_in_building_sector(&s.element),
                view_radius: s.npc.view_radius,
                view_direction: s.npc.view_direction,
                real_half_aperture: s.npc.real_half_aperture,
                sq_view_radius: (s.npc.view_radius as f32) * (s.npc.view_radius as f32),
            });
            let next = neighbour_ai.right_combat_neighbour;
            if next == 0 || next == current {
                break;
            }
            current = next;
        }
        out
    }
}

impl EngineInner {
    // ─── AI initialization ──────────────────────────────────────

    /// Initialize AI for all NPCs and reset global AI state.
    ///
    /// Called from `initialize()` after level loading, and again after
    /// deserialization when re-initialization is requested.
    pub(crate) fn init_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
    ) {
        // Script loading is intentionally recoverable so incomplete developer
        // data can still reach the renderer.  In that mode AI starts without
        // door-derived views, houses, or rally points; make the degraded state
        // explicit rather than silently manufacturing valid-looking caches.
        if self.scripts.mission.is_none() {
            tracing::warn!(
                "Initializing AI without a mission script; door-derived AI state will be unavailable"
            );
        }

        // Reset global AI state
        // think-method recursion depth = 0
        self.ai.global.there_are_royalist_soldiers = false;
        self.ai.global.there_are_lacklandist_soldiers = false;
        self.ai.global.overall_alert_status = crate::ai::AlertLevel::Green;
        self.ai.global.overall_villain_alert_status = crate::ai::AlertLevel::Green;
        self.ai.global.init_green_yellow_red_alert_soldiers();

        // golden_eye_mode is set from CliArgs after initialize() returns

        // Build the houses list and door rally points.  Collects every
        // building sector, attaches its doors, records occupants, and
        // anchors a rally point outside each door at
        // `AI_DOOR_RALLY_POINT_DISTANCE`.  Must run before the NPC init
        // loop below, because `InitOneAI` reads `leave_house_number`
        // off the AI controller which is assigned here.
        self.initialize_buildings();

        // Beam hiking-path waypoints that sit just outside a building
        // door into the building's interior.  Mutates the shared
        // `hiking_paths` arc in place through `Arc::make_mut` so
        // subsequent NPC clones see the beamed paths.
        {
            let paths = std::sync::Arc::make_mut(&mut assets.hiking_paths);
            for path in paths.iter_mut() {
                for wp in path.waypoints.iter_mut() {
                    for door in &self.ai.global.door_seek_infos {
                        if door.door_type != crate::gate::DoorType::Building {
                            continue;
                        }
                        // Chebyshev distance <= 5.
                        let dx = (wp.x as f32 - door.point_out.x).abs();
                        let dy = (wp.y as f32 - door.point_out.y).abs();
                        if dx.max(dy) <= 5.0 {
                            wp.x = door.position_in.x as i16;
                            wp.y = door.position_in.y as i16;
                            wp.sector = door.position_in.sector.map(u16::from).unwrap_or(0);
                            wp.level = door.position_in.level;
                            break;
                        }
                    }
                }
            }
        }

        // Teleport standalone seek points (those used by AI
        // investigators) that sit just outside a building door to
        // the door's inside position — same rule as the waypoint
        // beaming above.  Already implemented as
        // `AiGlobalState::teleport_seek_points_inside_doors`.
        self.ai.global.teleport_seek_points_inside_doors();

        // Initialize each NPC's AI.
        let npc_ids: Vec<EntityId> = self.world.entities.npc_ids().collect();
        let hiking_paths = assets.hiking_paths.clone();
        // Populate the handle → entity view map so the per-NPC
        // init_ctx hands each AI a usable map (even though init
        // mostly just reads self position).
        let scratch = self.build_sim_scratch(sim, assets);
        // For "get soldier from all by id" in the AI tick: copy the
        // level's soldier load-order array onto AiGlobalState so
        // AiContext can resolve script-baked friend IDs.
        self.ai.global.all_soldier_handles = std::sync::Arc::new(
            assets
                .entities
                .soldier_entity_ids
                .iter()
                .map(|eid| eid.index())
                .collect(),
        );
        let entity_views = scratch.ai_entity_views.clone();
        let sight_obstacles = scratch.ai_sight_obstacles.clone();
        let all_soldier_handles = self.ai.global.all_soldier_handles.clone();
        let ambiance = self.world.weather.ambiance;

        // Snapshot of every live human in the engine; every per-NPC
        // init pass reuses the same list to build its detectable enemy
        // array.  Equivalent to iterating the engine's element list
        // inside each per-NPC init.
        let potential_detectables = build_potential_detectables(self);
        let ambush_points_count = self.ai.global.ambush_points.len();

        let all_soldier_entity_ids = assets.entities.soldier_entity_ids.clone();
        let soldier_subordinate_ids = assets.entities.soldier_subordinate_ids.clone();
        let fast_grid = self.world.fast_grid.clone();
        for &npc_id in &npc_ids {
            self.init_one_ai(
                sim,
                npc_id,
                &hiking_paths,
                &potential_detectables,
                ambush_points_count,
                &entity_views,
                &sight_obstacles,
                &fast_grid,
                ambiance,
                &all_soldier_handles,
                &all_soldier_entity_ids,
                &soldier_subordinate_ids,
            );

            // InitState may finish by calling the actor's Wait method for an
            // authored sleeping, sitting, dead, unconscious, or special
            // pose. Priority-WAIT launch is a synchronous call chain in the
            // Original: Wait -> LaunchSequenceElement ->
            // NextSequenceElementsGo -> Go -> Instruct. Finish that chain
            // before InitOneAI returns to the next NPC. Leaving the
            // instruction queued lets Actor::Hourglass execute a lazy
            // fallback first and restart the authored animation one frame
            // late.
            self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                .unwrap_or_else(|error| {
                    panic!(
                        "NPC {npc_id:?} initialization failed synchronous sequence dispatch: {error:?}"
                    )
                });

            // Original InitOneAI invokes virtual SetState inline, after all
            // actor scripts have been initialized. Close the same owner's
            // callback/effect boundary before the next NPC initializes so a
            // state callback cannot leak to the first Hourglass tick or
            // observe later owners' initialized state.
            self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
        }

        // Lift each ambush point's 2D position into 3D (eye height
        // = 32 units above the ground) and assign a sequential ID.
        // The 3D anchor feeds the sight-polygon query that decides
        // whether an NPC on the ambush point can be seen; the ID is
        // how AI scripts reference the point.
        let ambush_points_3d: Vec<_> = self
            .ai
            .global
            .ambush_points
            .iter()
            .map(|ap| {
                self.position_to_point_3d(
                    assets,
                    ap.position.sector,
                    ap.position.level,
                    ap.position.x,
                    ap.position.y,
                )
            })
            .collect();
        for (idx, (ap, mut point_3d)) in self
            .ai
            .global
            .ambush_points
            .iter_mut()
            .zip(ambush_points_3d)
            .enumerate()
        {
            point_3d.z += 32.0;
            ap.position_3d = point_3d;
            ap.id = idx as u16;
        }

        tracing::info!("AI initialized for {} NPCs", npc_ids.len(),);
    }

    /// Per-NPC initialization pass — runs the per-NPC init for both
    /// enemy and friendly AI.
    ///
    /// Runs every entity-level side effect that must happen once at
    /// level load:
    ///
    /// 1. `InitializeDirectionOffsetVeryOld` — seed `direction_old`
    ///    from the current body direction so the vision pipeline has a
    ///    stable starting value.
    /// 2. `InitViewRadius` — clamp `view_radius` / `view_radius_base`
    ///    / `view_radius_goal` to the engine's standard view radius
    ///    for this level (day/night dependent).
    /// 3. Give Merry Man archers in forest levels their starting bow
    ///    ammo (`MERRY_MAN_ARROWS`).
    /// 4. Build the per-NPC "detectable enemies" list from a snapshot
    ///    of live humans.
    /// 5. Stuck-in-obstacle correction (Malignity only): if the NPC
    ///    starts inside a motion obstacle, push its move box out to
    ///    an authorized position and rewrite its map position.
    /// 6. `StoreInitialPositionParameters` — freeze the NPC's current
    ///    position / sector / level / facing as the "initial" values
    ///    that the AI returns to after idle wanders.
    /// 7. Initialize this NPC's patrol path from `path_id`, then run
    ///    `TestIfPathIsFine` against the fast-find grid; clear the
    ///    patrol if any segment intersects an obstacle.
    /// 8. Seed `old_life_points` / `initial_life_points` on enemy AIs
    ///    for the "still has his initial HP" check.  Difficulty-based
    ///    life-point scaling is already applied at entity-spawn time
    ///    in `engine::level_loading::spawn_soldier`, so we just
    ///    snapshot the current value here.
    /// 9. Fill this enemy's `ambush_point_status` vector with
    ///    `Far` × `ambush_points_count` so `RefreshAmbushPoints`
    ///    has a slot per global ambush point.
    /// 10. Dispatch to the subclass's `init_one_ai` for the
    ///     initial-action / state-transition / return-to-duty logic.
    #[allow(clippy::too_many_arguments)]
    fn init_one_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
        potential_detectables: &[PotentialDetectable],
        ambush_points_count: usize,
        entity_views: &SharedAiEntityViews,
        sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
        fast_grid: &std::sync::Arc<crate::fast_find_grid::FastFindGrid>,
        ambiance: crate::engine::types::Ambiance,
        all_soldier_handles: &std::sync::Arc<Vec<u32>>,
        all_soldier_entity_ids: &[EntityId],
        soldier_subordinate_ids: &[Vec<u16>],
    ) {
        // -- Phase 1: Peek at the entity to classify (enemy / friendly,
        //    camp) and read the fields we need for the obstacle fix. --
        let (is_enemy, is_friendly, self_camp, pos_map, layer, move_box_opt) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let (is_enemy, is_friendly, self_camp) = match entity {
                Entity::Soldier(s) => (
                    s.npc.ai_brain.enemy().is_some(),
                    false,
                    s.soldier.cached_camp,
                ),
                Entity::Civilian(c) => (
                    false,
                    c.npc.ai_brain.friendly().is_some(),
                    c.civilian.cached_camp,
                ),
                _ => return,
            };
            let elem = entity.element_data();
            let move_box = entity
                .actor_data()
                .map(|_| *entity.position_iface().get_move_box());
            (
                is_enemy,
                is_friendly,
                self_camp,
                elem.position_map(),
                elem.layer(),
                move_box,
            )
        };
        if !(is_enemy || is_friendly) {
            return;
        }

        // -- Phase 2: Stuck-in-obstacle correction (enemy only). --
        // If the NPC's move-box overlaps the playable area, attempt to
        // push it to an authorized position via `find_authorized_position`.
        if is_enemy && let Some(move_box) = move_box_opt {
            let mut abs_box = move_box.translated(pos_map);
            if !self.world.fast_grid.is_position_authorized(&abs_box, layer)
                && self
                    .world
                    .fast_grid
                    .find_authorized_position(&mut abs_box, layer)
            {
                let new_center = abs_box.center();
                if let Some(entity) = self.world.entities.get_mut(npc_id)
                    && entity.actor_data().is_some()
                {
                    let new_center_map = new_center;
                    let pi = entity.position_iface_mut();
                    pi.set_map_position(new_center_map);
                    entity.element_data_mut().set_position_map(new_center_map);
                }
            }
        }

        // -- Phase 3: Build the detectable-enemy list for this NPC. --
        let detectables =
            build_detectable_enemies_for(self_camp, is_friendly, npc_id, potential_detectables);

        // -- Phase 4: Re-read entity (post-fix) and mutate all the
        //    per-NPC state fields in one shot. --
        let standard_view_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS
        };
        let is_forest_level = self.world.weather.is_forest_level;

        // `entity_building_sector` needs a `&self` borrow; compute it
        // up-front while we don't hold a mutable entity borrow.
        let building_sector = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            self.entity_building_sector(entity.element_data().sector())
        };

        // Determine whether this NPC is a Merry-Man archer (Royalist
        // soldier, forest level, archer flag set by the level loader).
        let is_merry_man_archer = if is_enemy {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let is_archer = entity.enemy_ai().map(|e| e.is_archer()).unwrap_or(false);
            let is_rider = entity.soldier_data().map(|s| s.rider).unwrap_or(false);
            self_camp == Camp::Royalists && is_forest_level && is_archer && !is_rider
        } else {
            false
        };

        // Grab the (possibly corrected) map position / direction /
        // sector / layer before the write-back borrow.
        let (pos_map_final, direction_final, sector_final, layer_final, current_lp) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let elem = entity.element_data();
            let lp = entity.npc_data().map(|n| n.life_points).unwrap_or(0);
            (
                elem.position_map(),
                elem.direction(),
                elem.sector(),
                elem.layer(),
                lp,
            )
        };

        // Write-back block: mutate every field this init pass owns.
        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(npc) = entity.npc_data_mut() {
                // `initialize_direction_offset_very_old`: seed from current body dir.
                npc.direction_old = direction_final;

                // `init_view_radius`: real radius + goal = standard view
                // radius.  We also seed `view_radius_base` so subsequent
                // alert/drunk modifiers scale off the correct baseline.
                npc.view_radius = standard_view_radius;
                npc.view_radius_base = standard_view_radius;
                npc.view_radius_goal = standard_view_radius;

                if is_merry_man_archer {
                    // Seed the bow ammo for forest-level Merry Man archers.
                    npc.number_of_arrows = MERRY_MAN_ARROWS;
                }

                // Detectable enemies list (`DetectableType::Enemy`
                // slot).  Other slots (Body/Object/Friend/...) are
                // populated later by runtime events.
                let enemy_idx = DetectableType::Enemy as usize;
                npc.detectable_lists[enemy_idx] = detectables;

                // `store_initial_position_parameters`: snapshot current
                // position, sector, level, and facing into the
                // initial-position fields.
                npc.initial_position_x = pos_map_final.x;
                npc.initial_position_y = pos_map_final.y;
                npc.initial_position_sector = sector_final;
                npc.initial_position_level = layer_final;
                let dir_vec = crate::shadow_polygon::sector_to_direction(direction_final);
                npc.initial_view_direction.x = dir_vec[0];
                npc.initial_view_direction.y = dir_vec[1];
            }

            // Switch on the NPC's camp to set the static camp-present
            // flags (`there_are_royalist_soldiers` /
            // `there_are_lacklandist_soldiers`).  Reading
            // `npcs_can_be_enemies()` later gates mixed-camp soldier
            // hostility on both flags being true.  The life-point
            // Easy/Hard scaling on the Lacklandist arm is already
            // applied at spawn time in `level_loading::spawn_soldier`.
            if is_enemy {
                match self_camp {
                    Camp::Royalists => self.ai.global.there_are_royalist_soldiers = true,
                    Camp::Lacklandists => self.ai.global.there_are_lacklandist_soldiers = true,
                    _ => {}
                }
            }

            // Enemy-specific state.
            if is_enemy && let Some(enemy) = entity.enemy_ai_mut() {
                // `old_life_points` = `initial_life_points` =
                // `get_life_points()`.  The level loader already applied
                // difficulty scaling to `cached_max_life_points` at
                // `engine::level_loading::spawn_soldier`, so the current
                // life points are already correct.
                let clamped = current_lp.clamp(0, 255) as u8;
                enemy.old_life_points = clamped;
                enemy.initial_life_points = clamped;

                // Reset the ambush-point-status array and insert
                // `AMBUSH_POINT_FAR` for every point in the global
                // ambush array.
                enemy.ambush_point_array_reset = true;
                enemy.ambush_point_status.clear();
                enemy
                    .ambush_point_status
                    .resize(ambush_points_count, crate::ai_enemy::AmbushPointStatus::Far);
            }
        }

        // -- Phase 5: Patrol path init + TestIfPathIsFine. --
        // Initialize the path from path_id, then test it; on failure,
        // assert in debug and silently clear in release.
        let patrol_path_opt = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            entity
                .ai_controller()
                .and_then(|ai| ai.path_id)
                .and_then(|pid| crate::ai::PatrolPath::new(pid, hiking_paths))
        };

        let patrol_path_ok = if let Some(ref patrol) = patrol_path_opt {
            // Grab the actual hiking-path waypoints + the NPC's move
            // box and run the obstacle check.
            let waypoints = hiking_paths
                .get(usize::from(patrol.hiking_path_index))
                .map(|p| p.waypoints.as_slice())
                .unwrap_or(&[]);
            let move_box = move_box_opt.unwrap_or_default();
            let ok = test_hiking_path_fine(&self.world.fast_grid, waypoints, &move_box);
            if !ok {
                tracing::warn!(
                    npc = npc_id.index(),
                    path_id = patrol.hiking_path_index.get(),
                    waypoints = waypoints.len(),
                    move_box = ?move_box,
                    "BUG: patrol path rejected by TestIfPathIsFine — debug asserts this \
                     never fails; in release the path is cleared and the NPC silently \
                     stops patrolling"
                );
            }
            ok
        } else {
            false
        };

        // -- Phase 6: Build the init ctx and commit patrol/path state. --
        let mut init_ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                0,
                building_sector,
                is_forest_level,
                ambiance,
                standard_view_radius,
                entity_views,
                sight_obstacles,
                fast_grid,
                hiking_paths,
                all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut init_ctx);

        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(ai) = entity.ai_controller_mut() {
                ai.initial_position = crate::ai::Position {
                    x: pos_map_final.x,
                    y: pos_map_final.y,
                    sector: sector_final,
                    level: layer_final,
                };
                // `StoreInitialPositionParameters` stores a direction vector
                // with `SetSector0to15(GetDirection())` (default aspect 1),
                // while return-to-post later calls `FaceTo(vector)`, which
                // bins it with `GetSector0to15(ASPECT_RATIO)`. Those two
                // operations deliberately do not round-trip diagonal body
                // sectors (for example body sector 2 becomes view sector 1).
                // Keep the controller's discrete cache equal to that later
                // FaceTo result; storing the body sector directly made an NPC
                // believe it was already facing its authored post direction.
                let initial_view_vector =
                    crate::shadow_polygon::sector_to_direction(direction_final);
                ai.initial_view_direction = crate::position_interface::vector_to_sector_0_to_15(
                    initial_view_vector[0] * crate::position_interface::ASPECT_RATIO,
                    initial_view_vector[1],
                ) as u16;
                if patrol_path_opt.is_some() && patrol_path_ok {
                    ai.patrol_path = patrol_path_opt;
                    ai.has_patrol_path = true;
                } else {
                    ai.detach_patrol_path(None, false);
                    ai.has_patrol_path = false;
                }

                // -- TransformPatrolIDsToRealPatrol --
                // Runs exactly once at AI init from the enemy AI's
                // `init_ai` before the first `initialize_patrol()`.
                // The raw mission subordinate IDs live on LevelAssets,
                // not on the serialized AI controller; runtime patrol
                // rebuilds use `theoretical_patrol`.
                if let Some(soldier_load_index) =
                    all_soldier_entity_ids.iter().position(|&eid| eid == npc_id)
                    && let Some(patrol_ids) = soldier_subordinate_ids.get(soldier_load_index)
                    && !patrol_ids.is_empty()
                {
                    ai.patrol.clear();
                    ai.missed_patrol_members.clear();
                    ai.theoretical_patrol.clear();
                    for &id in patrol_ids {
                        if let Some(&eid) = all_soldier_entity_ids.get(id as usize) {
                            ai.theoretical_patrol.push(eid);
                        } else {
                            tracing::warn!(
                                "NPC {} patrol ID {} out of range (max {})",
                                npc_id.index(),
                                id,
                                all_soldier_entity_ids.len()
                            );
                        }
                    }
                }
            }
        }

        // Original `RHArtificialMalignity::InitOneAI` transforms a patrol
        // chief's authored soldier IDs and immediately calls
        // `InitializePatrol` before evaluating the chief's initial state.
        // That synchronous pass also writes `patrol_chief` onto admitted
        // minions.  Some of those minions occur later in the NPC creation
        // order, so their own InitOneAI must already see the chief and return
        // to formation instead of independently starting their authored
        // hiking path.
        //
        // Runtime patrol refresh remains owner-ticked, but mission bootstrap
        // cannot defer this first pass: doing so changes ReturnToDuty,
        // consumes extra macro RNG draws, and starts the minion on the wrong
        // route for its first simulation frame.
        let theoretical_patrol = self
            .world
            .entities
            .get(npc_id)
            .and_then(|entity| entity.ai_controller())
            .map(|ai| ai.theoretical_patrol.clone())
            .unwrap_or_default();
        if !theoretical_patrol.is_empty() {
            let chief_view = entity_views.get(&npc_id.index()).unwrap_or_else(|| {
                panic!(
                    "patrol chief {} is absent from the AI initialization view map",
                    npc_id.index()
                )
            });
            let chief_position = chief_view.position;
            let chief_ground_z = chief_view.elevation;
            let obstacles = sight_obstacles.list();
            let mut patrol = Vec::new();
            let mut missed = Vec::new();

            for &member in &theoretical_patrol {
                let member_view = entity_views.get(&member.index()).unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} references missing authored member {}",
                        npc_id.index(),
                        member.index()
                    )
                });
                let admitted = member_view.active
                    && !member_view.is_dead
                    && member_view.ai_state == crate::ai::AiState::Default
                    && (member_view.is_civilian() || member_view.is_able_to_fight)
                    && crate::ai_enemy::soldier_detects_target_360(
                        chief_position,
                        chief_ground_z,
                        chief_view.is_rider,
                        standard_view_radius,
                        chief_view.in_building,
                        member_view.position,
                        member_view.elevation,
                        member_view.posture,
                        member_view.is_rider,
                        member_view.direction as i16,
                        member_view.in_building,
                        obstacles,
                    );
                if admitted {
                    patrol.push(member);
                } else if !member_view.is_dead {
                    missed.push(member);
                }
            }

            // `InitializePatrol` inserts by increasing 3-D square distance;
            // ties insert before the existing member.
            let patrol_distance = |member: EntityId| {
                let view = entity_views.get(&member.index()).unwrap_or_else(|| {
                    panic!(
                        "patrol member {} disappeared from the AI initialization view map",
                        member.index()
                    )
                });
                let dx = view.position.x - chief_position.x;
                let dy_world =
                    (view.position.y + view.elevation) - (chief_position.y + chief_ground_z);
                let dy = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
                let dz = view.elevation - chief_ground_z;
                dx * dx + dy * dy + dz * dz
            };
            let mut sorted_patrol = Vec::with_capacity(patrol.len());
            for member in patrol {
                let distance = patrol_distance(member);
                let insert_at = sorted_patrol
                    .iter()
                    .position(|&existing| distance <= patrol_distance(existing))
                    .unwrap_or(sorted_patrol.len());
                sorted_patrol.insert(insert_at, member);
            }

            // Arrange each pair left/right relative to the chief.
            for i in (1..sorted_patrol.len()).step_by(2) {
                let even = &entity_views[&sorted_patrol[i - 1].index()].position;
                let odd = &entity_views[&sorted_patrol[i].index()].position;
                let ex = even.x - chief_position.x;
                let ey = even.y - chief_position.y;
                let ox = odd.x - chief_position.x;
                let oy = odd.y - chief_position.y;
                if ex * oy - ey * ox < 0.0 {
                    sorted_patrol.swap(i - 1, i);
                }
            }

            {
                let chief = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} disappeared during AI initialization",
                        npc_id.index()
                    )
                });
                let ai = chief.ai_controller_mut().unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} has no AI controller during initialization",
                        npc_id.index()
                    )
                });
                ai.patrol = sorted_patrol.clone();
                ai.missed_patrol_members = missed;
                ai.needs_patrol_reinit = false;
            }
            for member in sorted_patrol {
                let entity = self.world.entities.get_mut(member).unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} admitted missing member {}",
                        npc_id.index(),
                        member.index()
                    )
                });
                let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                    panic!(
                        "patrol member {} has no AI controller during initialization",
                        member.index()
                    )
                });
                ai.patrol_chief = Some(npc_id);
            }
        }

        // -- Phase 7: Dispatch to the subclass for state transitions. --
        // The InitState / ReturnToDuty / beggar-lock tail.  The subclass
        // commits the AI-side state transition via
        // `AiController::init_state` and returns the entity-side side
        // effects — posture / action state / eye status / life-point /
        // concussion writes that the AI layer can't reach on its own.
        let init_fx: crate::ai::InitStateSideEffects = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            match &mut entity.npc_data_mut().map(|n| &mut n.ai_brain) {
                Some(crate::element::AiBrain::Enemy(e)) => {
                    // Init runs during `InitOneAI` before any detection
                    // or target selection — `primary_target` is 0 and
                    // no battle context exists yet, so the centralized
                    // `build_npc_tick_data` would return `stub()`
                    // anyway.  Skip the round-trip and pass stub
                    // directly.
                    let tick = AiPerTickData::stub();
                    e.init_one_ai(sim, &init_ctx, &tick)
                }
                Some(crate::element::AiBrain::Friendly(f)) => f.init_one_ai(sim, &init_ctx),
                _ => return,
            }
        };
        if let Some(enemy) = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(|entity| entity.enemy_ai_mut())
        {
            // Both InitializePatrol calls made by Original InitOneAI have
            // completed synchronously above.  Do not repeat the bootstrap
            // pass on the first owner tick.
            enemy.base.needs_patrol_reinit = false;
        }

        // -- Phase 8: Apply entity-side side effects from `init_state`. --
        // Posture, action state, eye status, life points, and
        // concussion all live on the entity, not the AI brain.  The
        // subclass dispatch already committed the AI-side state
        // transitions; here we flush the entity half.
        if init_fx.set_posture.is_some()
            || init_fx.set_action_state.is_some()
            || init_fx.set_eye_status.is_some()
            || init_fx.zero_life_points
            || init_fx.concussion_max_and_unconscious
        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };

            // Posture: write to `ElementData::posture`.  Matches the
            // existing `pending_posture` drain path which deliberately
            // skips `PositionInterface::set_posture` — the move-box
            // recomputation is deferred and every other posture write
            // in the codebase (melee knock-out paths,
            // ability.CarryingCorpse, …) follows the same pattern.
            if let Some(posture) = init_fx.set_posture {
                entity.set_posture(posture);
            }

            // Action state: write on `ActorData`.  `set_states(...,
            // action_state)` + `wait()` collapse to a direct
            // `action_state = X` at init time, since the entity has no
            // active animation to interrupt.
            if let Some(action_state) = init_fx.set_action_state
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.action_state = action_state;
            }

            // Eye status: use the existing `ai_vision::set_view_status`
            // helper so `view_transition` is flipped alongside the raw
            // field.  Equivalent to `close_eyes` (which just calls
            // `set_view_status(EYES_CLOSED)`).
            if let Some(status) = init_fx.set_eye_status
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, status);
            }

            // Zero life points + killed-by-accident.  Bundled because
            // they are always written together at init
            // (`init_with_zero_life_points` followed by
            // `set_killed_by_accident(true)`).
            if init_fx.zero_life_points {
                if let Some(npc) = entity.npc_data_mut() {
                    npc.life_points = 0;
                }
                if let Some(human) = entity.human_data_mut() {
                    human.killed_by_accident = true;
                }
            }

            // Max concussion + unconscious.  Init-time bypasses
            // the full `combat::set_concussion` state machine
            // because none of its gates (script lock, tied,
            // carried) apply on a freshly-spawned NPC.
            if init_fx.concussion_max_and_unconscious
                && let Some(human) = entity.human_data_mut()
            {
                human.concussion_of_the_brain = crate::combat::CONCUSSION_MAX;
                human.unconscious = true;
            }
        }

        if init_fx.launch_wait {
            // RHArtificialIntelligence::InitState calls mpMe->Wait() only
            // after SetStates for authored sleeping/sitting/dead/etc. poses.
            // This must be a new launch, not ensure_wait_element: mission
            // script initialization may already have installed an Upright
            // wait whose translated orders are stale for the new posture.
            self.actor_wait(npc_id);
        }
    }

    /// Populate `AiGlobalState::houses` and `door_rally_points` from
    /// the currently-loaded level, and assign `leave_house_number` to
    /// each NPC occupant.
    ///
    /// The building-loop portion of AI init.  Each building sector
    /// becomes one [`House`]: its doors are looked up via the door
    /// table (doors whose `sector_in` matches the building), its
    /// occupants are found by scanning entities currently in that
    /// sector, and one [`DoorRallyPoint`] is anchored at every door's
    /// `point_out`.
    ///
    /// Runtime occupant tracking is wired at the `execute_pass_door`
    /// Enter / Leave branches in `engine::door_pass`: the same hook
    /// that updates canonical `BuildingState` occupants also updates
    /// `House::occupant_ids`.
    pub(super) fn initialize_buildings(&mut self) {
        use crate::ai::{AI_DOOR_RALLY_POINT_DISTANCE, DoorRallyPoint, House, Position};

        self.ai.global.houses.clear();
        self.ai.global.door_rally_points.clear();

        // Index canonical doors by their `sector_in` (building interior side).
        // BTreeMap (not HashMap) so the `for (sector_in, …) in
        // doors_by_building` iteration below assigns `leave_house_number`
        // in a stable, sector-ordered sequence — replay/lockstep multi-
        // player need deterministic AI state.
        let mut doors_by_building: std::collections::BTreeMap<
            crate::sector::SectorNumber,
            Vec<u32>,
        > = std::collections::BTreeMap::new();
        let mut rally_points: Vec<DoorRallyPoint> = Vec::new();

        // Include every building's doors — not just those occupied
        // at init time — so the runtime enter/leave hooks have a
        // pre-existing `House` to update when an NPC walks into a
        // previously-empty building.  The reference's restriction to
        // NPC-populated buildings (the houses list being built from
        // starting sectors) is an artifact of *how* it initializes,
        // not a semantic invariant; live occupant tracking supersedes
        // it.  Trap doors (`BuildingTrap`) remain excluded — those
        // sectors aren't regular building interiors and shouldn't
        // carry rally points.
        // A missing script is the explicitly warned degraded-load path from
        // `init_ai`; houses intentionally remain empty in that mode.
        if self.scripts.mission.is_some() {
            for (idx, door) in self.script_domains.interactables.doors.iter().enumerate() {
                if !matches!(door.door_type, crate::gate::DoorType::Building) {
                    continue;
                }
                doors_by_building
                    .entry(door.sector_in)
                    .or_default()
                    .push(idx as u32);

                // Rally point: use the door's `point_out` directly
                // (the sectorised "outside" position).
                rally_points.push(DoorRallyPoint {
                    position: Position {
                        x: door.point_out.x,
                        y: door.point_out.y,
                        sector: crate::position_interface::SectorHandle::new(u16::from(
                            door.sector_out,
                        )),
                        level: door.layer_out,
                    },
                    door_index: crate::gate::DoorIndex(idx as u32),
                    radius: AI_DOOR_RALLY_POINT_DISTANCE,
                });
            }
        }

        // Collect occupants per building from the current entity set.
        // An entity is "in building X" if its sector is X *and* that
        // sector is flagged `is_building()`.  We skip entities without
        // actor data (objects, FX) since only actors can be NPCs.
        let mut occupants_by_building: std::collections::HashMap<
            crate::sector::SectorNumber,
            Vec<EntityId>,
        > = std::collections::HashMap::new();

        for (entity_id, entity) in self.world.entities.actors() {
            let elem = entity.element_data();
            let sector_raw = match elem.sector() {
                Some(s) => crate::sector::SectorNumber::new(u16::from(s) as i16),
                None => continue,
            };
            // Only record occupants of sectors we know are buildings
            // (have at least one building door pointing at them).
            if !doors_by_building.contains_key(&sector_raw) {
                continue;
            }
            occupants_by_building
                .entry(sector_raw)
                .or_default()
                .push(entity_id.into());
        }

        // Build the houses list from the collected door/occupant maps.
        for (sector_in, door_indices) in doors_by_building {
            let occupant_ids = occupants_by_building.remove(&sector_in).unwrap_or_default();

            // Distribute sequential `leave_house_number` to each
            // occupant — used by the departure scheduler to stagger
            // NPCs exiting during alerts.
            for (n, &eid) in occupant_ids.iter().enumerate() {
                if let Some(entity) = self.world.entities.get_mut(eid)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    ai.leave_house_number = n as u16;
                }
            }

            // Look up the building index on the grid sector.  `None`
            // for script-synthesised or otherwise proto-unlinked
            // building sectors — rare but non-fatal.
            let building_index = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&sector_in)
                .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                .and_then(|gs| gs.building_index);

            // Read `arrow_reserve` from the engine-owned building domain
            // (populated from the GUYS/CAVE tenant chunk at level
            // load).  `max_occupants` still has no proto source — we
            // leave it at the `0xFFFF` default (unlimited) matching
            // `BuildingData::default()`.
            let arrow_reserve = building_index
                .and_then(|bi| {
                    self.script_domains
                        .buildings
                        .arrow_reserves
                        .get(usize::from(bi))
                        .copied()
                })
                .unwrap_or(false);

            self.ai.global.houses.push(House {
                sector_index: u32::from(u16::from(sector_in)),
                building_index,
                door_indices,
                occupant_ids,
                arrow_reserve,
            });
        }

        self.ai.global.door_rally_points = rally_points;

        tracing::info!(
            houses = self.ai.global.houses.len(),
            rally_points = self.ai.global.door_rally_points.len(),
            "Initialized AI building data"
        );
    }

    /// Recompute overall villain alert status from soldier NPCs, updating
    /// global counters and triggering combat/alert music transitions.
    ///
    /// Ports the per-NPC work of `change_alert_status` into a
    /// single-shot sweep that runs once per frame. The per-NPC
    /// `set_alert_status` already writes `current_music_alert_status`
    /// but doesn't touch the global counters or call
    /// `set_music_mode`; this method fills that gap.
    ///
    /// Call once per frame before the sound `hourglass` so a transition
    /// to yellow/red promptly bumps the music pool weight.
    pub(crate) fn update_overall_villain_alert(
        &mut self,
        profiles: &crate::profiles::ProfileManager,
    ) {
        let mut yellow = 0u16;
        let mut red = 0u16;
        let mut green = 0u16;
        // Per-call `ALERT_INSTANT_MUSIC_CHANGE` flag is staged on each
        // AiController by `set_alert_status_with_flags`; OR it across
        // soldiers here and clear after consumption.  Non-soldier flags
        // are ignored to match the soldier-only gate.
        let mut any_instant_change = false;
        for (_, soldier) in self.world.entities.soldiers_mut() {
            let Some(ai) = soldier.npc.ai_brain.base_mut() else {
                continue;
            };
            match ai.current_music_alert_status {
                crate::ai::AlertLevel::Green => green += 1,
                crate::ai::AlertLevel::Yellow => yellow += 1,
                crate::ai::AlertLevel::Red => red += 1,
            }
            if ai.outbox.music.instant_change {
                any_instant_change = true;
                ai.outbox.music.instant_change = false;
            }
        }
        self.ai.global.green_alert_soldiers = green;
        self.ai.global.yellow_alert_soldiers = yellow;
        self.ai.global.red_alert_soldiers = red;

        let new_overall = self.ai.global.overall_villain_alert();
        if new_overall == self.ai.global.overall_villain_alert_status {
            return;
        }
        let prev = self.ai.global.overall_villain_alert_status;
        self.ai.global.overall_villain_alert_status = new_overall;
        self.ai.global.overall_alert_status = new_overall;

        // Only call `set_music_mode` when not in Sherwood.  Sherwood
        // has its own ambient track and shouldn't hear combat/alert
        // cues even if a soldier briefly goes yellow.
        let is_sherwood = Some(&self.mission_domain.campaign)
            .and_then(|c| c.current_mission_idx)
            .and_then(|idx| Some(&self.mission_domain.campaign).and_then(|c| c.missions.get(idx)))
            .is_some_and(|m| {
                m.profile(profiles).location == crate::profiles::MissionLocation::Sherwood
            });

        if !is_sherwood {
            use crate::sound::MusicMode;
            // On the Green arm, forest levels keep the alert track
            // instead of dropping to quiet so the woodland ambient
            // layer keeps playing under any residual yellow soldiers.
            let mode = match new_overall {
                crate::ai::AlertLevel::Green => {
                    if self.world.weather.is_forest_level {
                        MusicMode::Alert
                    } else {
                        MusicMode::Quiet
                    }
                }
                crate::ai::AlertLevel::Yellow => MusicMode::Alert,
                crate::ai::AlertLevel::Red => MusicMode::Fight,
            };
            // `set_alert_status` calls `force_music_mode` when the
            // caller passes `ALERT_INSTANT_MUSIC_CHANGE`.  Known
            // shipped call sites are all Green-target (two AI sites
            // and the NPC death path).  The flag is now staged per-NPC
            // on `AiController::pending_instant_music_change` by
            // `set_alert_status_with_flags`; the sweep above OR'd it
            // across soldiers into `any_instant_change`, so any
            // transition direction passing the flag forces immediately.
            let cmd = if any_instant_change {
                super::SoundCommand::ForceMusicMode(mode)
            } else {
                super::SoundCommand::SetMusicMode(mode)
            };
            self.feedback.pending_side_effects.sounds.push(cmd);
        }

        tracing::debug!(
            "Overall villain alert {:?} → {:?} (green={green} yellow={yellow} red={red})",
            prev,
            new_overall,
        );
    }

    // ─── Turn order processing ──────────────────────────────────

    /// Process pending turn orders from NPC order queues.
    ///
    /// `face_direction` / `face_position` produce `Turning` orders that
    /// `process_pending_ai_orders` routes to `actor.order_queue`.
    /// These become `Turn` sequence elements that complete in one
    /// frame and fire `EventDone`.  We replicate that here: set the
    /// entity's direction toward the target position, then dispatch
    /// `EventDone` so the AI state machine continues.
    /// Drain animation-type orders (Pointing, RaisingShield, LoweringShield,
    /// Menacing, etc.) from NPC order queues and start them as `active_ai_anim`.
    /// Like `process_turn_orders` but for multi-frame animations that
    /// need EventDone when the sprite animation completes.
    pub(super) fn process_animation_orders(&mut self) {
        // Legacy entry point — left as a no-op now that the animation
        // driver reads the front order directly via
        // `current_order_for_actor`.  Animations booked onto sequence
        // elements are picked up automatically; there is no longer a
        // separate drain-and-rebook step.
    }

    // ─── EventReachPoint dispatch ───────────────────────────────

    /// Dispatch `EventReachPoint` stimulus to NPCs whose movement just
    /// completed.
    ///
    /// `send_condolation_card` calls `think(EVENT_REACHPOINT)` when a
    /// MOVE sequence element reaches the terminated state.  Originally
    /// this fires through the owner-local pending-condolation drain after
    /// movement terminates the selected element. This helper remains for
    /// non-movement callers that explicitly synthesize the same stimulus.
    ///
    /// Any new orders produced by the AI (e.g. "walk to next waypoint")
    /// will be drained on the next frame by `process_pending_ai_orders`.
    pub(super) fn dispatch_reach_point_events(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entities: &[EntityId],
    ) {
        let scratch = self.build_sim_scratch(sim, assets);
        let current_frame = self.control.frame_counter;

        for &entity_id in entities {
            // Build ctx in a read-only scope so we can then call
            // `dispatch_filtered_stimulus`, which needs `&mut self`.
            let in_uninterruptible_command = self.is_very_very_busy(entity_id);
            let ctx = {
                let Some(entity) = self.world.entities.get(entity_id) else {
                    continue;
                };
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    current_frame,
                    None,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                ctx
            };
            let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventReachPoint);
            // Centralized builder: assembles primary target metadata,
            // friend-swap candidates, avenger-on-roof wait position,
            // and a seeded enemy_sq_distances.  Non-enemy-NPC entities
            // get a stub.
            let tick_data = self.build_npc_tick_data(sim, entity_id, &scratch, assets);

            self.dispatch_think_with_drain(sim, entity_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    // ─── EventGaloppLoopEnd dispatch ────────────────────────────

    #[cfg(test)]
    pub(super) fn set_galopp_dispatch_observer(
        observer: Option<Box<dyn FnMut(&EngineInner, EntityId)>>,
    ) {
        GALOPP_DISPATCH_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
    }

    /// Dispatch `EventGaloppLoopEnd` to riders with `RHMOVE_RIDER_CHARGE`
    /// flag that reached an intermediate waypoint during movement.
    ///
    /// When a rider's running animation reaches half/end frame with
    /// the `RIDER_CHARGE` move flag, `think(EVENT_GALOPP_LOOP_END)`
    /// fires so the AI can call `maybe_make_rider_attack()` to check
    /// if it's close enough to begin the actual charge pass.
    pub(super) fn dispatch_galopp_loop_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let scratch = self.build_sim_scratch(sim, assets);
        let current_frame = self.control.frame_counter;
        let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
            panic!("rider {entity_id:?} disappeared before its synchronous GALOPP Execute callback")
        });
        let soldier = entity.soldier_data().unwrap_or_else(|| {
            panic!("GALOPP Execute callback owner {entity_id:?} is not a soldier")
        });
        assert!(
            soldier.rider,
            "GALOPP Execute callback owner {entity_id:?} is not a rider"
        );
        let ctx = build_ai_context_from_entity(
            entity,
            current_frame,
            None,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );

        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventGaloppLoopEnd);
        // EventGaloppLoopEnd fires on enemy riders mid-charge towards their
        // primary target. Think and every order/script callback it creates
        // close here, before Actor::Hourglass can complete this movement or
        // the mutable legacy walk can advance to the next owner.
        let tick_data = self.build_npc_tick_data(sim, entity_id, &scratch, assets);
        self.dispatch_think_with_drain(sim, entity_id, &stimulus, &ctx, &tick_data, assets);
        #[cfg(test)]
        GALOPP_DISPATCH_OBSERVER.with(|observer| {
            if let Some(observer) = observer.borrow_mut().as_mut() {
                observer(self, entity_id);
            }
        });
    }

    /// Map a PC's currently-executing animation (`OrderType`) to the
    /// noise volume they produce, via a per-animation switch in
    /// `refresh_produced_noise`.
    ///
    /// `refresh_produced_noise` runs from `hourglass()` each frame and
    /// reads `get_animation()` — the currently-running animation — to
    /// set `currently_produced_noise.volume`.  We reproduce the same
    /// lookup here from the PC's active `OrderType` (peeked from
    /// `actor.order_queue`, the equivalent of the sequence slot that
    /// `get_animation` reads).
    ///
    /// Material selects the walk/run/drop volume (wood = loud, grass =
    /// quiet, water = noisiest, light-shadow = silent).  The jump,
    /// sword-fight and breath volumes are material-independent.
    ///
    /// Returns the volume and whether Original reaches the final
    /// `mboxHearMyNoiseBox` rebuild. The inactive/building and breath arms
    /// return early and deliberately preserve the previous box.
    fn pc_noise_volume(
        order_type: crate::order::OrderType,
        material: crate::element::GameMaterial,
        in_building: bool,
        active: bool,
        prev_volume: u16,
    ) -> (u16, bool) {
        use crate::element::GameMaterial as Material;
        use crate::order::OrderType as OT;

        // When the actor is inside a building or inactive, the volume
        // is forced to 0.  Hearing then becomes impossible because the
        // current noise is silent, but Original returns before updating the
        // persistent hear-my-noise box.
        if in_building || !active {
            return (0, false);
        }

        // Walk/run/drop volumes per material.
        let (walk, run, drop) = match material {
            Material::Ground => (20, 70, 50),
            Material::Wood => (40, 150, 100),
            Material::Stone => (20, 75, 50),
            Material::Grass => (40, 150, 100), // GRASS_DRY
            Material::Leaves => (10, 50, 30),  // GRASS_FRESH
            Material::Water => (200, 400, 300),
            Material::Bush => (40, 150, 100),
            Material::Ice => (20, 75, 50),
            // LightShadow has no assignment in either the walk or run
            // switch, so `volume` keeps whatever it was on the prior
            // frame.  Substitute `prev_volume` for the walk and run
            // slots.  `drop` (Rolling / CarryingCorpse) has no
            // counterpart in `refresh_produced_noise`, so keep the
            // pre-existing 50 fallback.
            Material::LightShadow => (prev_volume, prev_volume, 50),
            _ => (20, 70, 50), // default = ground
        };

        // NOISE_VOLUME_* constants.
        const BREATH: u16 = 15;
        const SWORDFIGHT: u16 = 200;
        const JUMP_UP: u16 = 50;
        const JUMP_LONG: u16 = 50;
        const JUMP_DOWN: u16 = 80;

        match order_type {
            // ── BREATH: idle, bow aim, sitting, freezing ──
            OT::WaitingUprightBored
            | OT::WaitingUprightBoredRandom
            | OT::WaitingUpright
            | OT::WaitingCrouched
            | OT::TransitionEquipBow
            | OT::TransitionUnequipBow
            | OT::TransitionLoadingBow
            | OT::TransitionUnloadBow
            | OT::TransitionRaisingBow
            | OT::TransitionLoweringBow
            | OT::AimingWithBow
            | OT::AimingWithBowUp
            | OT::ShootingWithBow
            | OT::ShootingWithBowUp
            | OT::Freezing
            | OT::WaitingFreeLift
            | OT::Sitting
            | OT::TransitionWaitingUprightSitting
            | OT::TransitionSittingWaitingUpright => (BREATH, false),

            // ── WALK (material-dependent) ──
            OT::WalkingUpright
            | OT::TransitionWaitingUprightBoredWaitingUpright
            | OT::TransitionWaitingUprightWaitingUprightBored
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::WalkingStairs
            | OT::TransitionCrouchingUp
            | OT::TransitionCrouchingDown
            | OT::TransitionWaitingUprightClimbingWallUp
            | OT::ClimbingWallUp
            | OT::ClimbingWallDown
            | OT::TransitionClimbingWallUpWaitingCrouchedCrenel
            | OT::TransitionWaitingCrouchedClimbingWallDownCrenel
            | OT::TransitionClimbingWallUpWaitingCrouched
            | OT::TransitionClimbingWallDownWaitingUpright
            | OT::TransitionWaitingCrouchedClimbingWallDown
            | OT::TransitionWaitingUprightClimbingLadderUp
            | OT::ClimbingLadderUp
            | OT::TransitionClimbingLadderUpWaitingCrouched
            | OT::TransitionWaitingCrouchedClimbingLadderDown
            | OT::ClimbingLadderDown
            | OT::TransitionClimbingLadderDownWaitingUpright
            | OT::StandingUp
            | OT::Turning
            | OT::TransitionWalkingUprightWaitingUpright
            | OT::PassingDoor
            | OT::WalkingWithSword
            | OT::TransitionWaitingCrouchedWalkingCrouched
            | OT::WalkingCrouched
            | OT::TransitionWalkingCrouchedWaitingCrouched
            | OT::TransitionWalkingUprightWalkingCrouched
            | OT::TransitionWalkingCrouchedWalkingUpright => (walk, true),

            // ── RUN (material-dependent) ──
            OT::RunningUpright
            | OT::TransitionWalkingUprightRunningUpright
            | OT::TransitionRunningUprightWalkingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightRunningUpright
            | OT::TransitionRunningUprightWalkingCrouched
            | OT::TransitionWalkingCrouchedRunningUpright
            | OT::RunningStairs
            | OT::ClimbingLadderUpFast
            | OT::ClimbingLadderDownFast
            | OT::RunningWithSword => (run, true),

            // ── JUMP land transitions ──
            OT::TransitionJumpingUpWaitingCrouched => (JUMP_UP, true),
            OT::TransitionJumpingLongWaitingUpright
            | OT::TransitionJumpingLongSwordWaitingSword => (JUMP_LONG, true),
            OT::TransitionJumpingDownWaitingCrouched => (JUMP_DOWN, true),

            // ── SWORDFIGHT ──
            OT::StrikingRightSmalltalk
            | OT::StrikingLeftSmalltalk
            | OT::ParryingRightSmalltalk
            | OT::ParryingLeftSmalltalk
            | OT::StrikingLowRightSmalltalk
            | OT::StrikingLowLeftSmalltalk
            | OT::ParryingLowRightSmalltalk
            | OT::ParryingLowLeftSmalltalk
            | OT::StrikingStraightSword
            | OT::StrikingStraightStrongSword
            | OT::StrikingRightSword
            | OT::StrikingLeftSword
            | OT::StrikingRoundRightSword
            | OT::StrikingRoundLeftSword
            | OT::StrikingSemiroundRightSword
            | OT::StrikingSemiroundLeftSword
            | OT::ExecutingSword
            | OT::TransitionWaitingSwordParryingSword
            | OT::ParryingSword
            | OT::TransitionParryingSwordWaitingSword
            | OT::ParryingLowSword
            | OT::Provoking
            | OT::StrikingDownSword => (SWORDFIGHT, true),

            // ── DROP (material-dependent) ──
            OT::Rolling | OT::TransitionCarryingCorpseWaitingUpright => (drop, true),

            // Everything else (injuries, death, bow injuries, menacing,
            // beggar, climbing shoulders, drinking, etc.) — silent.
            _ => (0, true),
        }
    }

    /// Refresh the produced-noise state at one PC's live human-Hourglass
    /// boundary. Original `RefreshProducedNoise` follows the base Actor slice,
    /// so only NPC slots after this PC may observe the new volume this frame.
    pub(super) fn refresh_pc_produced_noise_for(&mut self, pc_id: EntityId) {
        let order_type = self
            .orders
            .sequence_manager
            .current_order_for_actor(pc_id)
            .map(|(_, _, order)| order.order_type)
            .unwrap_or(crate::order::OrderType::Invalid);
        self.refresh_pc_produced_noise_for_with_order(pc_id, order_type);
    }

    /// Refresh produced noise using the `mpOrder` animation visible to the
    /// Original Human::Hourglass tail.
    ///
    /// Actor completion may already have instructed a different sequence
    /// element by this boundary. In that case the sequence manager's current
    /// order is newer than Original's latched `mpOrder`; the fused owner walk
    /// supplies the correctly stale animation explicitly.
    pub(super) fn refresh_pc_produced_noise_for_with_order(
        &mut self,
        pc_id: EntityId,
        order_type: crate::order::OrderType,
    ) {
        let (material, in_building, active, previous, noise) = {
            let entity = self.world.entities.get(pc_id).unwrap_or_else(|| {
                panic!(
                    "PC produced-noise owner {} disappeared from its legacy slot",
                    pc_id.index()
                )
            });
            let Entity::Pc(pc) = entity else {
                panic!("produced-noise owner {} is not a PC actor", pc_id.index());
            };
            let position = pc.element.position_map();
            let noise = crate::ai::Noise {
                origin: crate::ai::Position {
                    x: position.x,
                    y: position.y,
                    sector: pc.element.sector(),
                    level: pc.element.layer(),
                },
                noise_type: if pc.human.opponents.is_empty() {
                    crate::ai::NoiseType::TapTapTap
                } else {
                    crate::ai::NoiseType::ZingZing
                },
                volume: 0,
                elevation: pc.element.sprite.position_iface.get_elevation() as u16,
                element_id: u16::try_from(pc_id.index()).unwrap_or_else(|_| {
                    panic!(
                        "PC produced-noise owner {} exceeds noise element-id range",
                        pc_id.index()
                    )
                }),
            };
            (
                pc.element.sprite.position_iface.get_material(),
                self.entity_building_sector(pc.element.sector()).is_some(),
                pc.element.active,
                pc.actor.last_noise_volume,
                noise,
            )
        };
        let (volume, refresh_hear_box) =
            Self::pc_noise_volume(order_type, material, in_building, active, previous);
        let Entity::Pc(pc) = self.world.entities.get_mut(pc_id).unwrap_or_else(|| {
            panic!(
                "PC produced-noise owner {} disappeared before write-back",
                pc_id.index()
            )
        }) else {
            panic!(
                "produced-noise owner {} changed kind before write-back",
                pc_id.index()
            );
        };
        pc.actor.last_noise_volume = volume;
        if refresh_hear_box {
            let half_x = volume as f32 + 100.0;
            let half_y = volume as f32 * crate::position_interface::ASPECT_RATIO + 100.0;
            pc.actor.hear_noise_box = crate::coordinates::MapBBox::from_coords(
                noise.origin.x - half_x,
                noise.origin.y - half_y,
                noise.origin.x + half_x,
                noise.origin.y + half_y,
            );
        }
        pc.actor.produced_noise = Some(crate::ai::Noise { volume, ..noise });
    }

    /// Rebuild every PC's non-serialized produced-noise fields after Original
    /// save pointer fixup.
    ///
    /// `RHEngine::Serialize` walks every human here, but
    /// `RHElementActorHuman::RefreshProducedNoise` immediately returns for
    /// NPCs. The remaining PC walk must use stable Original creation order.
    pub(crate) fn refresh_legacy_loaded_produced_noise(&mut self) {
        let pc_ids = self
            .world
            .entities
            .occupied()
            .filter_map(|(id, entity)| entity.is_pc().then_some(id))
            .collect::<Vec<_>>();
        for pc_id in pc_ids {
            self.refresh_pc_produced_noise_for(pc_id);
        }
    }

    /// Complete remarks which were active in an Original save.
    ///
    /// Original clears the remark latch and invokes
    /// `InformAIOnFinishedRemark` inline during local-AI deserialization.
    /// Ordinary state adoption has already installed the cleared latch; this
    /// method reproduces only the synchronous MYTALK callback, in serialized
    /// element order, before the later global RNG reseed.
    pub(crate) fn complete_legacy_loaded_remarks(
        &mut self,
        completions: &[(EntityId, u16)],
        assets: &LevelAssets,
    ) {
        self.with_simulation_context(|engine, sim| {
            for &(owner, raw_flags) in completions {
                let (current_remark, current_flags) = engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "preflighted loaded-remark owner {} disappeared",
                            owner.index()
                        )
                    })
                    .ai_controller()
                    .map(|ai| (ai.current_remark, ai.current_remark_flags))
                    .unwrap_or_else(|| {
                        panic!(
                            "preflighted loaded-remark owner {} lost its AI",
                            owner.index()
                        )
                    });
                assert_eq!(
                    current_remark,
                    crate::ai::Remark::TheSoundOfSilence,
                    "loaded-remark owner {} must be cleared before its callback",
                    owner.index()
                );
                assert_eq!(
                    current_flags,
                    0,
                    "loaded-remark owner {} flags must be cleared before its callback",
                    owner.index()
                );

                let Some(stimulus_type) = Self::speech_finished_stimulus(
                    crate::ai::SpeechFlags::from_bits_truncate(raw_flags),
                ) else {
                    continue;
                };
                let scratch = engine.build_sim_scratch(sim, assets);
                let in_uninterruptible_command = engine.is_very_very_busy(owner);
                let entity =
                    engine.world.entities.get(owner).unwrap_or_else(|| {
                        panic!("loaded-remark owner {} disappeared", owner.index())
                    });
                let building_sector = engine.entity_building_sector(entity.element_data().sector());
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    engine.control.frame_counter,
                    building_sector,
                    engine.world.weather.is_forest_level,
                    engine.world.weather.ambiance,
                    engine.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &engine.world.fast_grid,
                    &assets.hiking_paths,
                    &engine.ai.global.all_soldier_handles,
                    engine.control.sim_config.difficulty,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                let tick_data = engine.build_npc_tick_data(sim, owner, &scratch, assets);
                let stimulus = crate::ai::Stimulus::new(stimulus_type);
                engine.dispatch_think_with_drain(sim, owner, &stimulus, &ctx, &tick_data, assets);
            }
        });
    }

    /// The wide "indoors" test: the building-sector flag OR the
    /// door-transit branch — true during the few frames an actor is on a
    /// door whose inside-sector is a building but whose current sector
    /// pointer has not yet been swapped.
    ///
    /// This is the predicate that governs whether the view polygon is
    /// deactivated, so only the detection refresh may use it. Every other
    /// indoor test in the AI resolves the sector alone; use
    /// [`Self::entity_data_in_building_sector`] there.
    pub(super) fn entity_data_inside_building(&self, elem: &crate::element::ElementData) -> bool {
        self.entity_data_in_building_sector(elem) || elem.is_in_door_transit()
    }

    /// The narrow "indoors" test: the entity's current sector is a
    /// building. An actor still on a door rail is outdoors by this
    /// measure, which is what the 180°/360° detection short-circuits, the
    /// them/us-list membership scans, the outdoor question gate, and the
    /// stuck-on-ladder counter all ask for.
    pub(super) fn entity_data_in_building_sector(
        &self,
        elem: &crate::element::ElementData,
    ) -> bool {
        self.entity_building_sector(elem.sector()).is_some()
    }

    /// Consume one NPC's deferred `inform_my_friends` edge at that NPC's
    /// creation-order Hourglass boundary.
    ///
    /// Original `RHElementActorNPC::Hourglass` clears the flag and calls
    /// `MyDearFriendsPleasePleaseDetectMe` immediately before that same NPC's
    /// `RefreshView` / `RefreshDetection` (`RHelementactornpc.cpp:3534-3546`).
    pub(super) fn tick_inform_my_friends_for_npc(&mut self, npc_id: EntityId) {
        if self.actors_frozen() {
            return;
        }

        let should_broadcast = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::npc_data_mut)
            .is_some_and(|npc| {
                let pending = npc.inform_my_friends;
                npc.inform_my_friends = false;
                pending
            });
        if should_broadcast {
            self.broadcast_body_detectable(npc_id);
        }
    }

    /// Dispatch this NPC's natural-wakeup `EVENT_FITAGAIN` synchronously at
    /// its base-human → NPC Hourglass boundary.
    ///
    /// `tick_concussion_healing` runs the globally batched stand-in for
    /// `RHElementActorHuman::Hourglass` and queues the event. The original
    /// calls `Think(EVENT_FITAGAIN)` inline before `mbInformMyFriends`,
    /// `RefreshView`, and `RefreshDetection` (human.cpp:335-390;
    /// npc.cpp:3528-3544). Drain the existing FIFO prefix through that wake
    /// event here; never pluck it ahead of older stimuli. The suffix remains
    /// queued for `RefreshDetection`'s ordinary drain.
    pub(super) fn dispatch_pending_fit_again_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) -> bool {
        let (prefix_through_wake, mut suffix) = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return false;
            };
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "NPC {} is missing its required AI controller while dispatching wakeup",
                    npc_id.index()
                )
            });
            let mut queued = std::mem::take(&mut ai.outbox.detection.stimuli);
            let Some(wake_index) = queued
                .iter()
                .position(|stimulus| stimulus.stimulus_type == StimulusType::EventFitAgain)
            else {
                ai.outbox.detection.stimuli = queued;
                return false;
            };
            let suffix = queued.split_off(wake_index + 1);
            assert!(
                !suffix
                    .iter()
                    .any(|stimulus| stimulus.stimulus_type == StimulusType::EventFitAgain),
                "NPC {} queued more than one EVENT_FITAGAIN before its Hourglass slot",
                npc_id.index()
            );
            (queued, suffix)
        };

        {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "NPC {} disappeared before its wakeup stimulus prefix",
                    npc_id.index()
                )
            });
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "NPC {} lost its AI controller before its wakeup stimulus prefix",
                    npc_id.index()
                )
            });
            ai.outbox.detection.stimuli = prefix_through_wake;
        }
        self.tick_enemy_ai_drain_pending_stimuli_for_npc(sim, npc_id, assets, None, None);

        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC {} disappeared after synchronous EVENT_FITAGAIN",
                npc_id.index()
            )
        });
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!(
                "NPC {} lost its AI controller after synchronous EVENT_FITAGAIN",
                npc_id.index()
            )
        });
        suffix.append(&mut ai.outbox.detection.stimuli);
        ai.outbox.detection.stimuli = suffix;
        true
    }

    /// Iterates every NPC except the body itself and registers the
    /// body under DETECTABLE_BODY.
    #[tracing::instrument(level = "trace", skip_all, fields(body = body_id.index()))]
    pub(super) fn broadcast_body_detectable(&mut self, body_id: EntityId) {
        use crate::element::DetectableType;

        // Snapshot the body's position + `knocked_out_in_money_fight`
        // flag for the per-friend radius check below.
        let (body_pos, body_knocked_out_in_money_fight, body_is_soldier) = {
            let entity = self.expect_entity(body_id, "broadcast_body_detectable body");
            let is_soldier = matches!(entity, Entity::Soldier(_));
            let pos = entity.element_data().position_map();
            let ko = entity
                .npc_data()
                .and_then(|n| n.ai_brain.base())
                .map(|b| b.knocked_out_in_money_fight)
                .unwrap_or(false);
            (pos, ko, is_soldier)
        };

        // Append to every other NPC's Body detectable list (skip duplicates).
        // The NPC list holds both soldiers and civilians, so civilian
        // NPCs must receive the body detectable too — otherwise
        // `get_worst_detected_type` never climbs past DETECTABLE_FRIEND
        // for civilians, dropping their emoticon / alert reactions to
        // nearby bodies.
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        let det_idx = DetectableType::Body as usize;
        for friend_id in npc_ids {
            if friend_id == body_id {
                continue;
            }
            let Some(entity) = self.world.entities.get_mut(friend_id) else {
                continue;
            };
            let friend_pos = entity.element_data().position_map();
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };

            // If this body was knocked out during a money fight, only
            // register the body with friends beyond
            // `AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS` (Chebyshev
            // distance).  Close-by money-fight participants
            // deliberately ignore the downed fighter.
            let add_detectable = if body_knocked_out_in_money_fight {
                let dx = (body_pos.x - friend_pos.x).abs();
                let dy = (body_pos.y - friend_pos.y).abs();
                dx.max(dy) > crate::parameters_ai::AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS as f32
            } else {
                true
            };

            if add_detectable && det_idx < npc.detectable_lists.len() {
                let already = npc.detectable_lists[det_idx]
                    .iter()
                    .any(|d| d.element == Some(body_id));
                if !already {
                    npc.detectable_lists[det_idx].push(crate::element::Detectable {
                        element: Some(body_id),
                        detectable_type: DetectableType::Body,
                        ..Default::default()
                    });
                }
            }

            // Also remove the body from the friend's
            // money-fight-enemies list when both are soldiers.  Runs
            // unconditionally of the radius check.  Civilians have no
            // `EnemyAi`, so `enemy_mut()` is None and this arm is a
            // natural no-op for them — only soldiers track money-fight
            // enemies.
            if body_is_soldier && let Some(enemy_ai) = npc.ai_brain.enemy_mut() {
                enemy_ai
                    .money_fight_enemies
                    .retain(|h| *h != body_id.index());
            }
        }
    }

    /// Remove `beggar_id` from every NPC's `DETECTABLE_BEGGAR` list.
    /// Once any seek-area soldier has claimed the PC-beggar (queued it into
    /// `beggars_to_control`), this sweeps the beggar out of every
    /// soldier's and civilian's BEGGAR list so no other soldier fires
    /// a duplicate `EVENT_SEES_BEGGAR` on subsequent frames.
    ///
    /// Modelled on `engine/nets.rs:delete_body_detectable_for_all_npc`
    /// but hardcoded to `DetectableType::Beggar`.
    #[tracing::instrument(level = "trace", skip_all, fields(beggar = beggar_id.index()))]
    pub(super) fn delete_beggar_detectable_for_all_npc(&mut self, beggar_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Beggar as usize;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for friend_id in npc_ids {
            let Some(entity) = self.world.entities.get_mut(friend_id) else {
                continue;
            };
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if det_idx < npc.detectable_lists.len() {
                npc.detectable_lists[det_idx].retain(|d| d.element != Some(beggar_id));
            }
        }
    }

    /// Original `RestoreDetectableObjects`, executed inline by the waking
    /// soldier before resurrection fan-out and any SetState callback.
    pub(super) fn restore_detectable_objects_for_npc(
        &mut self,
        npc_id: EntityId,
        knocked_out_in_money_fight: bool,
    ) {
        use crate::element::DetectableType;
        use crate::element_kinds::ObjectType;

        let mut to_add = Vec::new();
        for (entity_id, entity) in self.world.entities.objects() {
            if !entity.is_active() {
                continue;
            }
            let object = entity.object_data().unwrap_or_else(|| {
                panic!(
                    "object slot {} lost object data during recovery for NPC {}",
                    entity_id.index(),
                    npc_id.index()
                )
            });
            if matches!(object.object_type, ObjectType::Ale)
                || matches!(object.object_type, ObjectType::Coin) && !knocked_out_in_money_fight
            {
                to_add.push(EntityId::from(entity_id));
            }
        }

        let npc = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::npc_data_mut)
            .unwrap_or_else(|| {
                panic!(
                    "recovery owner {} vanished before RestoreDetectableObjects",
                    npc_id.index()
                )
            });
        let objects = npc
            .detectable_lists
            .get_mut(DetectableType::Object as usize)
            .unwrap_or_else(|| {
                panic!(
                    "recovery owner {} has no DETECTABLE_OBJECT list",
                    npc_id.index()
                )
            });
        for element in to_add {
            if !objects
                .iter()
                .any(|detectable| detectable.element == Some(element))
            {
                objects.push(crate::element::Detectable {
                    element: Some(element),
                    detectable_type: DetectableType::Object,
                    ..Default::default()
                });
            }
        }
    }

    /// Apply the resurrection fan-out and eye-status writes produced by
    /// `EVENT_FITAGAIN`. The caller invokes this immediately after the
    /// synchronous Think drain, before returning to Human/Actor Hourglass.
    pub(super) fn tick_ai_pending_resurrection_and_eyes_for_npc(&mut self, npc_id: EntityId) {
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC {} disappeared while applying synchronous recovery state",
                npc_id.index()
            )
        });
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!(
                "NPC {} is missing its required AI controller while applying recovery state",
                npc_id.index()
            )
        });
        let inform_resurrection = ai.outbox.recovery.inform_resurrection;
        ai.outbox.recovery.inform_resurrection = false;
        let eye_status = ai.outbox.recovery.set_eye_status.take();

        if inform_resurrection {
            self.broadcast_resurrection(npc_id);
        }
        if let Some(status) = eye_status {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "NPC {} disappeared while applying its pending eye status",
                    npc_id.index()
                )
            });
            let npc = entity.npc_data_mut().unwrap_or_else(|| {
                panic!(
                    "entity {} lost its NPC data while applying its pending eye status",
                    npc_id.index()
                )
            });
            crate::ai_vision::set_view_status(npc, status);
        }
    }

    /// Remove `resurrected_id` from every other NPC's
    /// `DETECTABLE_BODY` list.  The per-NPC body of
    /// `inform_on_resurrection` — the engine-side fan-out triggered by
    /// `inform_everyone_on_my_resurrection`.
    #[tracing::instrument(level = "trace", skip_all, fields(resurrected = resurrected_id.index()))]
    pub(super) fn broadcast_resurrection(&mut self, resurrected_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Body as usize;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for friend_id in npc_ids {
            if friend_id == resurrected_id {
                continue;
            }
            let Some(entity) = self.world.entities.get_mut(friend_id) else {
                continue;
            };
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if det_idx < npc.detectable_lists.len() {
                npc.detectable_lists[det_idx].retain(|d| d.element != Some(resurrected_id));
            }
        }
    }

    /// Per-frame view parameter refresh for every NPC.
    ///
    /// This test-facing wrapper preserves the focused EYES_FOLLOW oracle;
    /// production coordinates the extracted per-NPC helper directly with
    /// that NPC's `RefreshDetection` slot.
    #[cfg(test)]
    pub(super) fn refresh_npc_views(
        &mut self,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        if self.actors_frozen() {
            return;
        }

        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.refresh_npc_view_for_npc(npc_id, positions_before_movement);
        }
    }

    /// Refresh one NPC's view immediately before its own creation-ordered
    /// `RefreshDetection` call.
    pub(super) fn refresh_npc_view_for_npc(
        &mut self,
        npc_id: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        if self.actors_frozen() {
            return;
        }

        // ── Phase 1: read-only — gather context ──
        let ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let Some(npc) = entity.npc_data() else {
                return;
            };

            let edata = entity.element_data();
            let own_world = entity.position_iface().get_position();
            let pos = crate::coordinates::GroundPoint::new(own_world.x, own_world.y);

            // RHElement::IsActiveAndOutsideBuilding is deliberately narrower
            // than IsInsideBuilding: it only inspects the current sector's
            // BUILDING flag. A sprite door pointer makes IsInsideBuilding
            // true while traversing an outdoor approach rail, but RefreshView
            // must still follow the actor's turning body on that rail.
            let is_active_and_outside_building =
                edata.active && self.entity_building_sector(edata.sector()).is_none();

            let animation = self
                .orders
                .sequence_manager
                .current_order_for_actor(npc_id)
                .map(|(_, _, o)| o.order_type);

            let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);

            let follow_target_position = npc.follow_target.and_then(|target_id| {
                self.world.entities.get(target_id).map(|target| {
                    // Original provenance:
                    // - RHEngine::PerformHourglass walks marrayElements in
                    //   creation order (RHengine.cpp:3715-3724,7909-7944).
                    // - RHElementActorNPC::Hourglass delegates to the base
                    //   human Hourglass before RefreshView
                    //   (RHelementactornpc.cpp:3528-3544).
                    // - EYES_FOLLOW reads pMobileTarget->GetPositionGround
                    //   inside RefreshView (RHelementactornpc.cpp:1012-1018).
                    // Thus a later-created target has not moved yet, while
                    // an earlier-created target has. EntityId::index is the
                    // append-only legacy creation slot in this port.
                    let boundary = if target_id.index() > npc_id.index() {
                        positions_before_movement
                            .get(target_id)
                            .copied()
                            .flatten()
                            .unwrap_or_else(|| {
                                panic!(
                                    "NPC {npc_id:?} follows later-created target {target_id:?}, \
                                         but the required pre-movement position snapshot is missing"
                                )
                            })
                    } else {
                        crate::entities::BoundaryPosition::of(target.element_data())
                    };
                    // The recorded boundary carries the target's own 3D
                    // position, so jump/flying Z survives and nothing has to
                    // be re-derived from the plane.
                    crate::coordinates::GroundPoint::new(boundary.world.x, boundary.world.y)
                })
            });

            let blood_alcohol = entity
                .enemy_ai()
                .map(|enemy| enemy.base.blood_alcohol)
                .unwrap_or(0);

            ai_vision::RefreshViewContext {
                body_direction: edata.direction(),
                posture: edata.posture,
                animation,
                is_unconscious,
                is_tied: edata.posture == crate::element::Posture::Tied,
                is_dead: entity.is_dead(),
                is_active_and_outside_building,
                is_rider: matches!(entity, Entity::Soldier(s) if s.soldier.rider),
                blood_alcohol,
                own_position: pos,
                follow_target_position,
            }
        };
        // shared borrow dropped ──

        // ── Phase 2: mutable — apply RefreshView ──
        self.debug_refresh_view_lifecycle("refresh_view_before", npc_id, None);
        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(npc) = entity.npc_data_mut() {
                let span = tracing::trace_span!("refresh_npc_view", npc = npc_id.index());
                let _guard = span.enter();
                ai_vision::refresh_view(npc, &ctx);
            }
        }
        self.debug_refresh_view_lifecycle("refresh_view_after", npc_id, None);
    }

    // ─── Owner-local NPC speech ─────────────────────────────────

    fn speech_finished_stimulus(flags: crate::ai::SpeechFlags) -> Option<StimulusType> {
        use crate::ai::SpeechFlags;
        if flags.contains(SpeechFlags::MYTALK_1) {
            Some(StimulusType::EventMyTalk1)
        } else if flags.contains(SpeechFlags::MYTALK_2) {
            Some(StimulusType::EventMyTalk2)
        } else if flags.contains(SpeechFlags::MYTALK_3) {
            Some(StimulusType::EventMyTalk3)
        } else if flags.contains(SpeechFlags::MYTALK_0) {
            Some(StimulusType::EventMyTalk0)
        } else {
            None
        }
    }

    fn reject_npc_speech_attempt(
        &mut self,
        owner: EntityId,
        flags: crate::ai::SpeechFlags,
        reason: u16,
    ) -> NpcSpeechSettlement {
        let ai = self
            .world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} disappeared during rejection",
                    owner.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} lost its AI during rejection",
                    owner.index()
                )
            });
        ai.cached_frame = self.control.frame_counter;
        ai.register_log_line(crate::ai::LogLineType::SpeakImpossible, reason);
        tracing::trace!(
            target: "robin_engine::engine::speech",
            frame = self.control.frame_counter,
            owner = owner.index(),
            reason,
            "Say rejected"
        );
        let invoke_finished_callback = if let Some(stimulus) = Self::speech_finished_stimulus(flags)
        {
            ai.outbox.reentrant.self_stimuli.insert(0, stimulus);
            true
        } else {
            false
        };
        NpcSpeechSettlement {
            invoke_finished_callback,
            category_rejection: None,
        }
    }

    /// Settle one queued Say invocation at the current AI owner's return
    /// barrier.
    ///
    /// Ordering follows `RHArtificialIntelligence::Say`
    /// (`original-code/RHartificialintelligence.cpp:5846-6178`): blip,
    /// script forbid, recent-remark forbid, house, CYCLE_3 advance,
    /// active-speech arbitration, active remark assignment, speech-profile
    /// category dispatch, screen remark, then automatic forbidding.
    pub(super) fn settle_npc_speech_attempt(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        attempt: crate::ai::AiSpeechAttempt,
    ) -> NpcSpeechSettlement {
        use crate::ai::{Remark, RemarkTargetFlags, SpeechFlags};
        use crate::sound::ExclamationGroup;

        let flags = SpeechFlags::from_bits_truncate(attempt.flags);
        #[derive(Clone, Copy)]
        enum OwnerProfile {
            Soldier(crate::profiles::SoldierProfileIdx),
            Civilian(crate::profiles::CivilianProfileIdx),
        }

        let (
            owner_profile,
            blipped,
            sector,
            in_door_transit,
            position,
            frame_profile_name,
            script_forbidden,
            active_remark,
        ) = {
            let entity = self
                .world
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("queued speech owner {} is missing", owner.index()));
            let owner_profile = match entity {
                Entity::Soldier(s) => OwnerProfile::Soldier(s.soldier.soldier_profile_index),
                Entity::Civilian(c) => OwnerProfile::Civilian(c.civilian.civilian_profile_index),
                other => panic!(
                    "queued NPC speech owner {} has invalid entity kind {:?}",
                    owner.index(),
                    other.element_data().kind
                ),
            };
            let ai = entity.ai_controller().unwrap_or_else(|| {
                panic!("queued speech owner {} has no AI controller", owner.index())
            });
            (
                owner_profile,
                entity.element_data().blipped,
                entity.element_data().sector(),
                entity.element_data().is_in_door_transit(),
                entity.element_data().position_map(),
                entity.element_data().sprite.frame_profile_name.clone(),
                ai.forbidden_remark_ids.contains(&(attempt.remark as u32)),
                ai.current_remark,
            )
        };
        let is_soldier = matches!(owner_profile, OwnerProfile::Soldier(_));
        let mut resolved_profile: Option<(bool, u32)> = None;
        let resolve_profile = |cached: &mut Option<(bool, u32)>| {
            if cached.is_none() {
                *cached = Some(match owner_profile {
                    OwnerProfile::Soldier(profile_index) => {
                        let profile = assets
                            .profile_manager
                            .get_soldier(profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "speech owner {} requires missing soldier profile {} after early gates",
                                    owner.index(),
                                    profile_index
                                )
                            });
                        (profile.vip, profile.exclamation_id)
                    }
                    OwnerProfile::Civilian(profile_index) => {
                        let profile = assets
                            .profile_manager
                            .civilians
                            .get(usize::from(profile_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "speech owner {} requires missing civilian profile {} after early gates",
                                    owner.index(),
                                    profile_index
                                )
                            });
                        (
                            profile.civilian_type == crate::profiles::CivilianType::Vip,
                            profile.exclamation_id,
                        )
                    }
                });
            }
            cached.clone().expect("speech profile cache was populated")
        };

        {
            let ai = self
                .world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| {
                    panic!(
                        "speech owner {} disappeared before Speak log",
                        owner.index()
                    )
                })
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!("speech owner {} lost AI before Speak log", owner.index())
                });
            ai.cached_frame = self.control.frame_counter;
            ai.register_log_line(crate::ai::LogLineType::Speak, attempt.remark as u16);
        }
        tracing::trace!(
            target: "robin_engine::engine::speech",
            frame = self.control.frame_counter,
            owner = owner.index(),
            remark = ?attempt.remark,
            flags = attempt.flags,
            "Say attempt"
        );

        if blipped {
            return self.reject_npc_speech_attempt(owner, flags, 0);
        }
        if script_forbidden {
            return self.reject_npc_speech_attempt(owner, flags, 1);
        }

        if !flags.contains(SpeechFlags::ALWAYS) {
            let frame = self.control.frame_counter;
            let owner_creation_order = self.world.original_creation_order(owner);
            // Original scans lazily in list order. It deletes expired entries
            // only as encountered and returns on the first live match, leaving
            // every later entry (including expired ones) untouched.
            let mut forbidden = false;
            let mut index = 0;
            while index < self.ai.global.forbidden_remarks.len() {
                if self.ai.global.forbidden_remarks[index].forbidden_till_frame < frame {
                    self.ai.global.forbidden_remarks.remove(index);
                    continue;
                }
                let entry = &self.ai.global.forbidden_remarks[index];
                if entry.remark == attempt.remark {
                    let scope = RemarkTargetFlags::from_bits_truncate(entry.flags);
                    if scope.contains(RemarkTargetFlags::THIS_TYPE)
                        && entry.bad_guy == is_soldier
                        && entry.speech_id == resolve_profile(&mut resolved_profile).1
                    {
                        forbidden = true;
                    } else if scope.contains(RemarkTargetFlags::THIS_GUY)
                        && u32::from(entry.guy_index) == owner_creation_order
                    {
                        forbidden = true;
                    } else if is_soldier && scope.contains(RemarkTargetFlags::VILLAINS) {
                        forbidden = true;
                    } else if !is_soldier && scope.contains(RemarkTargetFlags::CIVILIANS) {
                        forbidden = true;
                    }
                }
                if forbidden {
                    break;
                }
                index += 1;
            }
            if forbidden {
                return self.reject_npc_speech_attempt(owner, flags, 2);
            }
        }

        if !flags.contains(SpeechFlags::HOUSE)
            && (self.entity_building_sector(sector).is_some() || in_door_transit)
        {
            return self.reject_npc_speech_attempt(owner, flags, 3);
        }

        // This is deliberately before the already-speaking gate, exactly as
        // in Original Say. Rejected overlapping attempts still consume one
        // shared CYCLE_3 slot.
        let variant = if flags.contains(SpeechFlags::CYCLE_3_VARIANTS) {
            self.ai.global.current_speech_variant = (self.ai.global.current_speech_variant + 1) % 3;
            self.ai.global.current_speech_variant as i32
        } else {
            -1
        };

        if active_remark != Remark::TheSoundOfSilence {
            if flags.contains(SpeechFlags::EMERGENCY) {
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::StopExclamation { actor_id: owner });
                // StopExclamation removes the old pending/playing line without
                // calling SoundIsFinished, so its MYTALK callback is discarded.
                self.cancel_exclamation_callbacks(owner.index());
            } else {
                return self.reject_npc_speech_attempt(owner, flags, 4);
            }
        }

        {
            let ai = self
                .world
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| {
                    panic!("speech owner {} disappeared before latch", owner.index())
                })
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("speech owner {} lost AI before latch", owner.index()));
            ai.current_remark = attempt.remark;
            ai.current_remark_flags = attempt.flags;
        }

        let (is_vip, speech_id) = resolve_profile(&mut resolved_profile);

        // Original skips the entire category/sound branch for speech ID zero,
        // but still leaves current_remark latched and performs the display and
        // auto-forbid tail. With no SoundIsFinished callback this can remain
        // active indefinitely.
        if speech_id != 0 {
            let raw = attempt.remark as u32;
            let first_vip = Remark::FIRST_VIP as u32;
            let first_civilian = Remark::FIRST_CIVILIAN as u32;
            let prefix = if flags.contains(SpeechFlags::SCRIPT) {
                "Script error"
            } else {
                "AI error"
            };
            let resolved = if raw >= first_vip {
                if !is_vip {
                    tracing::warn!(
                        target: "ai_speech_mismatch",
                        "{}: VIP remark [{}] for non-VIP NPC {} at ({},{})",
                        prefix,
                        attempt.remark.speech(),
                        owner.index(),
                        position.x as u16,
                        position.y as u16
                    );
                    None
                } else {
                    Some((ExclamationGroup::Vip, raw.wrapping_sub(first_vip) as u16))
                }
            } else if raw >= first_civilian {
                if is_soldier || is_vip {
                    if is_soldier {
                        tracing::warn!(
                            target: "ai_speech_mismatch",
                            "{}: civilian remark [{}] for soldier {} at ({},{})",
                            prefix,
                            attempt.remark.speech(),
                            owner.index(),
                            position.x as u16,
                            position.y as u16
                        );
                    }
                    None
                } else {
                    Some((
                        ExclamationGroup::Civilian,
                        raw.wrapping_sub(first_civilian) as u16,
                    ))
                }
            } else if !is_soldier || is_vip {
                if !is_soldier {
                    tracing::warn!(
                        target: "ai_speech_mismatch",
                        "{}: soldier remark [{}] for civilian {} at ({},{})",
                        prefix,
                        attempt.remark.speech(),
                        owner.index(),
                        position.x as u16,
                        position.y as u16
                    );
                }
                None
            } else {
                // Original's ordinary soldier bank uses EXCLAMATION_CIVILIAN.
                Some((ExclamationGroup::Civilian, raw as u16))
            };

            let Some((group, exclamation_id)) = resolved else {
                let reason = if raw >= first_vip {
                    if is_soldier { 5 } else { 6 }
                } else if raw >= first_civilian {
                    if is_soldier { 7 } else { 8 }
                } else if !is_soldier {
                    9
                } else {
                    10
                };
                let log_before_callback = !matches!(reason, 8 | 9);
                let ai = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "speech owner {} disappeared after category rejection",
                            owner.index()
                        )
                    })
                    .ai_controller_mut()
                    .unwrap_or_else(|| {
                        panic!(
                            "speech owner {} lost AI after category rejection",
                            owner.index()
                        )
                    });
                if log_before_callback {
                    ai.register_log_line(crate::ai::LogLineType::SpeakImpossible, reason);
                }
                let invoke_finished_callback =
                    if let Some(stimulus) = Self::speech_finished_stimulus(flags) {
                        ai.outbox.reentrant.self_stimuli.insert(0, stimulus);
                        true
                    } else {
                        false
                    };
                return NpcSpeechSettlement {
                    invoke_finished_callback,
                    category_rejection: Some(CategorySpeechRejectionFinalization {
                        reason_after_callback: (!log_before_callback).then_some(reason),
                    }),
                };
            };

            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::Exclamation {
                    group,
                    profile_id: speech_id,
                    exclamation_id,
                    variant,
                    position,
                    actor_id: Some(owner),
                });
            self.feedback
                .sound_sim
                .pending_exclamations
                .push(crate::sound::PendingExclamation {
                    actor_id: owner.index(),
                    group,
                    profile_id: speech_id,
                    exclamation_id,
                    variant,
                });
        }

        self.ai.global.screen_remarks.push(crate::ai::ScreenRemark {
            timer: 100,
            prefix: frame_profile_name,
            remark: attempt.remark,
        });
        Self::auto_forbid_remark(
            &mut self.ai.global.forbidden_remarks,
            attempt.remark,
            speech_id,
            self.world.original_creation_order(owner) as u16,
            is_soldier,
            self.control.frame_counter,
        );
        NpcSpeechSettlement::default()
    }

    /// Finish the unconditional tail of a category-rejected Original `Say`.
    /// Reasons 8/9 log only after `InformAIOnFinishedRemark`; every category
    /// rejection clears the latch after that callback returns, overwriting any
    /// recursively started emergency line.
    pub(super) fn finalize_category_speech_rejection(
        &mut self,
        owner: EntityId,
        finalization: CategorySpeechRejectionFinalization,
    ) {
        use crate::ai::Remark;

        let ai = self
            .world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} disappeared during category-rejection tail",
                    owner.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| {
                panic!(
                    "speech owner {} lost AI during category-rejection tail",
                    owner.index()
                )
            });
        if let Some(reason) = finalization.reason_after_callback {
            ai.register_log_line(crate::ai::LogLineType::SpeakImpossible, reason);
        }
        ai.current_remark = Remark::TheSoundOfSilence;
        ai.current_remark_flags = 0;
    }

    /// Deliver deterministic SoundIsFinished callbacks at the first mutation
    /// of the `PerformHourglass` deferred-effects phase where matured
    /// exclamations are collected.
    ///
    /// `RHElementActorNPC::SoundIsFinished`
    /// (`original-code/RHelementactornpc.cpp:6473-6511`) converts the
    /// currently active remark through the owner's category and clears it only
    /// when the callback's exact exclamation ID matches. A stale/mismatched
    /// completion is logged and deliberately retains the active line.
    pub(super) fn settle_npc_speech_completions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        use crate::ai::{Remark, SpeechFlags};

        let completions = std::mem::take(&mut self.feedback.sound_sim.finished_exclamations);
        for (actor_slot, completed_id) in completions {
            let actor_id = self
                .world
                .entities
                .id_at_legacy_slot(actor_slot)
                .unwrap_or_else(|| {
                    panic!(
                        "speech completion references missing legacy actor slot {} (id {})",
                        actor_slot, completed_id
                    )
                });
            let (active, expected_id, flags, is_pc) = {
                let entity = self.world.entities.get(actor_id).unwrap_or_else(|| {
                    panic!(
                        "speech completion owner {} vanished after slot resolution",
                        actor_id.index()
                    )
                });
                if entity.is_pc() {
                    (Remark::TheSoundOfSilence, 0, 0, true)
                } else {
                    let ai = entity.ai_controller().unwrap_or_else(|| {
                        panic!(
                            "speech completion owner {} is neither PC nor an NPC with AI",
                            actor_id.index()
                        )
                    });
                    let active = ai.current_remark;
                    let raw = active as u32;
                    let expected = match entity {
                        Entity::Soldier(s) => {
                            let profile = assets
                                .profile_manager
                                .get_soldier(s.soldier.soldier_profile_index)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "speech completion owner {} requires missing soldier profile {}",
                                        actor_id.index(),
                                        s.soldier.soldier_profile_index
                                    )
                                });
                            if profile.vip {
                                raw.wrapping_sub(Remark::FIRST_VIP as u32)
                            } else {
                                raw
                            }
                        }
                        Entity::Civilian(c) => {
                            let profile = assets
                                .profile_manager
                                .civilians
                                .get(usize::from(c.civilian.civilian_profile_index))
                                .unwrap_or_else(|| {
                                    panic!(
                                        "speech completion owner {} requires missing civilian profile {}",
                                        actor_id.index(),
                                        c.civilian.civilian_profile_index
                                    )
                                });
                            if profile.civilian_type == crate::profiles::CivilianType::Vip {
                                raw.wrapping_sub(Remark::FIRST_VIP as u32)
                            } else {
                                raw.wrapping_sub(Remark::FIRST_CIVILIAN as u32)
                            }
                        }
                        other => panic!(
                            "speech completion owner {} has invalid entity kind {:?}",
                            actor_id.index(),
                            other.element_data().kind
                        ),
                    };
                    (active, expected, ai.current_remark_flags, false)
                }
            };
            if is_pc {
                continue;
            }
            if active == Remark::TheSoundOfSilence || expected_id != completed_id {
                tracing::warn!(
                    actor = actor_id.index(),
                    ?active,
                    expected_id,
                    completed_id,
                    "stale or mismatched NPC speech completion retained active speech"
                );
                continue;
            }

            let ai = self
                .world
                .entities
                .get_mut(actor_id)
                .unwrap_or_else(|| {
                    panic!("speech completion owner {} disappeared", actor_id.index())
                })
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("speech completion owner {} lost AI", actor_id.index()));
            ai.current_remark = Remark::TheSoundOfSilence;
            ai.current_remark_flags = 0;
            ai.register_log_line(crate::ai::LogLineType::SpeakFinished, 0);
            if let Some(stimulus) =
                Self::speech_finished_stimulus(SpeechFlags::from_bits_truncate(flags))
            {
                ai.outbox.reentrant.self_stimuli.push(stimulus);
            }
            self.drain_direct_ai_owner_boundary(sim, actor_id, assets);
        }
    }

    /// Per-tick decay + eviction of the screen-remark HUD overlay list.
    /// The timer half of `display_screen_remarks`: each entry's timer
    /// is decremented and entries whose timer reaches zero are
    /// dropped.  Without this the list grows unbounded for the
    /// lifetime of the mission (one entry per accepted remark).  The
    /// rendering half lives in `hud_text::render_screen_remarks`.
    pub(super) fn tick_screen_remarks(&mut self) {
        self.ai.global.screen_remarks.retain_mut(|r| {
            r.timer = r.timer.saturating_sub(1);
            r.timer > 0
        });
    }

    /// Auto-forbid a remark after speaking, with per-remark duration and scope.
    fn auto_forbid_remark(
        forbidden_remarks: &mut Vec<crate::ai::ForbiddenRemark>,
        remark: crate::ai::Remark,
        speech_id: u32,
        guy_index: u16,
        is_soldier: bool,
        current_frame: u32,
    ) {
        use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags};
        use crate::parameters_ai::{
            AI_DRUNKEN_REMARK_FORBIDDEN_TIME, AI_REMARK_FORBIDDEN_TIME,
            AI_SHORT_REMARK_FORBIDDEN_TIME,
        };

        let push = |list: &mut Vec<ForbiddenRemark>, frames: i32, scope: RemarkTargetFlags| {
            list.push(ForbiddenRemark {
                remark,
                flags: scope.bits(),
                speech_id,
                guy_index,
                bad_guy: is_soldier,
                forbidden_till_frame: current_frame + frames as u32,
            });
        };

        match remark {
            // Never forbid — one-shot dialogue remarks.
            // These are used inside scripted conversations where a
            // second line in the same window must still play; forbidding
            // them would break multi-line officer/charly/beggar dialogs
            // and civ/vip wounded/dies pairs.
            Remark::Dies
            | Remark::Strangled
            | Remark::CivWounded
            | Remark::CivDies
            | Remark::VipWounded
            | Remark::VipDies
            | Remark::BadExcuse
            | Remark::CivBeggarBegging
            | Remark::CivBeggarGivesInfo
            | Remark::CivBeggarWantsMore
            | Remark::CivBeggarGivesLastInfo
            | Remark::CivBeggarThanx
            | Remark::OfficerStopsPatrol
            | Remark::OfficerStartsPatrol
            | Remark::OfficerAsksWhatsup
            | Remark::OfficerAsksWhere
            | Remark::OfficerEndsConversation
            | Remark::OfficerCallsSoldier
            | Remark::OfficerSendsOutSoldier
            | Remark::OfficerCallsGroup
            | Remark::OfficerSendsOutGroup
            | Remark::OfficerSendsOutGroupForCharly
            | Remark::OfficerRebukesCharly
            | Remark::OfficerRebukesCharlyEnd
            | Remark::OfficerGivesAttackOrder
            | Remark::OfficerSeesBrawl
            | Remark::OfficerEndsBrawl
            | Remark::GiveOrReceiveOrder
            | Remark::CallsOfficer
            | Remark::TellsOfficerBody
            | Remark::TellsOfficerEnemy
            | Remark::TellsOfficerOther
            | Remark::TellsOfficerCharlyAway
            | Remark::TellsOfficerWhere
            | Remark::AwaitsOrders
            | Remark::TellsOfficerNothing
            | Remark::CharlyDefendsHimself
            | Remark::MissesCharly
            | Remark::DidntFindCharly
            | Remark::FoundCharly
            | Remark::SendsCharlyToOfficer => {}

            // Short forbidden time.
            Remark::Wounded => {
                push(
                    forbidden_remarks,
                    AI_SHORT_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
            }

            // Civilian sees body/dead body: ALL_NPC scope.
            Remark::CivSeesBody | Remark::CivSeesDeadBody => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::ALL_NPC,
                );
            }

            // Drunken: double forbid — type + personal.
            Remark::Drunken => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
                push(
                    forbidden_remarks,
                    AI_DRUNKEN_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_GUY,
                );
            }

            // Standard THIS_TYPE list from Original Say's switch.
            Remark::AwakensSleeperr
            | Remark::HuntsEnemy
            | Remark::StartsCombat
            | Remark::ProvokesCombat
            | Remark::GoodStrikeCombat
            | Remark::CombatInsult
            | Remark::Warcry
            | Remark::KilledAdversary
            | Remark::Cassos
            | Remark::WaspSting
            | Remark::UnderNet
            | Remark::SeesFriendUnderNet
            | Remark::Arrow
            | Remark::TiedUp
            | Remark::SeesObject
            | Remark::AleYes
            | Remark::AleNo
            | Remark::HitByApple
            | Remark::ChasesChild
            | Remark::CaughtChild
            | Remark::GoldYes
            | Remark::GoldNo
            | Remark::GoldBrawl
            | Remark::SearchingSoldierGold
            | Remark::SearchingSoldierNothing
            | Remark::EndsSearch
            | Remark::Panic
            | Remark::ControlsBeggar
            | Remark::MenacesPcInComa
            | Remark::CryAlert
            | Remark::ShieldBearerCovers
            | Remark::ProudDontFight
            | Remark::ProudFinallyFight
            | Remark::OfficerComplains
            | Remark::OutOfAmmunition
            | Remark::AdmiresObjectScript
            | Remark::MissesObjectScript
            | Remark::CivCallsSoldier
            | Remark::ShieldBearersLineFormation
            | Remark::ArchersBehindShieldBearers
            | Remark::CivDenunciates
            | Remark::CivAdmiresRobin
            | Remark::CivPanic
            | Remark::CivThanx
            | Remark::CivCries
            | Remark::CivBeerYes
            | Remark::CivBeerNo
            | Remark::CivSeesSoldiersUnderNet
            | Remark::CivUnderNet
            | Remark::CivApple
            | Remark::CivWasps
            | Remark::CivWhistling
            | Remark::CivSeesBrawl
            | Remark::CivGoldYes
            | Remark::CivGoldNo
            | Remark::CivBeggarIdentifiesHimself
            | Remark::CivChildCaughtBySoldier
            | Remark::CivChildChasedBySoldier
            | Remark::VipProudDontFight
            | Remark::VipProudFinallyFight
            | Remark::VipStartsCombat
            | Remark::VipGoodStrikeCombat
            | Remark::VipWarcry
            | Remark::VipVictory
            | Remark::VipSpeaksToHimself
            | Remark::VipAleNo
            | Remark::VipNetNo
            | Remark::VipAppleNo
            | Remark::VipWaspsNo
            | Remark::VipGoldNo
            | Remark::HearsNoise
            | Remark::SeesEnemy
            | Remark::SeesBody
            | Remark::BahIlBougePus
            | Remark::SpecialAction => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
            }
            Remark::NumberOfRemarks | Remark::TheSoundOfSilence => {
                panic!("invalid automatic-forbid remark {remark:?}")
            }
        }
    }

    #[cfg(test)]
    pub(super) fn tick_enemy_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // This detection-only test seam predates the production owner walk.
        // Preserve its contract that all PCs have refreshed their noise before
        // the first NPC is evaluated.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.refresh_pc_produced_noise_for(pc_id);
        }
        self.tick_enemy_ai_inner(sim, assets, None);
    }

    /// Production NPC coordinator for the pre-detection portion of
    /// `RHElementActorNPC::Hourglass`.
    ///
    /// Each NPC consumes only its own body/recovery work and refreshes its
    /// own view immediately before its creation-ordered `RefreshDetection`.
    /// The direct `tick_enemy_ai` entry point remains detection-only for
    /// focused tests that construct already-refreshed vision state.
    #[cfg(test)]
    pub(super) fn tick_enemy_ai_with_creation_ordered_prelude(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        self.tick_enemy_ai_inner(sim, assets, Some(positions_before_movement));
    }

    /// Prepare the shared, RNG-free portion of the fused owner pass.
    pub(super) fn prepare_npc_owner_pass(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
    ) -> PreparedNpcOwnerPass {
        self.ai.global.same_frame_target_claims.clear();
        if !self.ai.global.primary_target_multiplicity_initialized {
            let initial = self.tick_enemy_ai_build_primary_target_multiplicity();
            self.ai.global.primary_target_multiplicity_scratch = initial
                .into_iter()
                .map(|(target, count)| (target.index(), count))
                .collect();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }
        PreparedNpcOwnerPass
    }

    /// Run one NPC's complete post-human envelope using live inputs sampled at
    /// this legacy slot. No later owner's view or forecast is constructed.
    pub(super) fn tick_npc_owner_pass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        _prepared: PreparedNpcOwnerPass,
        npc_id: EntityId,
    ) {
        self.debug_refresh_view_lifecycle("npc_tail_enter", npc_id, None);
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "NPC owner {} disappeared before its fused legacy-slot envelope",
                npc_id.index()
            )
        });
        assert!(
            entity.npc_data().is_some(),
            "fused NPC owner {} has no NPC data",
            npc_id.index()
        );
        // FrozenAll is volatile script state. Sample it at the consuming NPC
        // slot rather than caching it before earlier owners run callbacks.
        if self.actors_frozen() {
            self.debug_refresh_view_lifecycle("npc_tail_frozen_skip", npc_id, None);
            self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
            return;
        }

        let world =
            self.tick_enemy_ai_build_world_view(assets, Some((npc_id, positions_before_movement)));
        self.tick_enemy_ai_refresh_detection(
            sim,
            assets,
            &world,
            Some(positions_before_movement),
            Some(npc_id),
            false,
        );
    }

    pub(super) fn finish_npc_owner_pass(&mut self) {
        self.ai.global.same_frame_target_claims.clear();
    }

    pub(super) fn tick_enemy_ai_blip_detection_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        self.tick_enemy_ai_blip_detection(sim, assets, owner)
    }

    #[cfg(test)]
    fn tick_enemy_ai_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: Option<&EntitySlots<Option<crate::entities::BoundaryPosition>>>,
    ) {
        if self.actors_frozen() {
            // Frozen-all skips patrol/view/detection/ambush/deafness but the
            // original still enters each NPC's busy/ladder/speech/lock gate,
            // where all three deadlines are extended before returning.
            if positions_before_movement.is_some() {
                let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
                for npc_id in npc_ids {
                    self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
                }
            }
            return;
        }
        self.ai.global.same_frame_target_claims.clear();
        if !self.ai.global.primary_target_multiplicity_initialized {
            let initial = self.tick_enemy_ai_build_primary_target_multiplicity();
            self.ai.global.primary_target_multiplicity_scratch = initial
                .into_iter()
                .map(|(target, count)| (target.index(), count))
                .collect();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }

        // ── 1. Build one immutable per-tick AI world view. ────────
        // Snapshot construction does not dispatch behavior. The phase calls
        // below remain in the original soldier/NPC Hourglass order.
        let world = self.tick_enemy_ai_build_world_view(assets, None);

        // ── 2a. Listen/object blip work. ────────────────────────
        // NPC-owned SeesBlip remains inside its creation-ordered
        // RefreshDetection slot below.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.tick_enemy_ai_blip_detection(sim, assets, pc_id);
        }

        // ── 3. Creation-ordered per-NPC prelude + RefreshDetection. ───
        // Production first consumes the current NPC's inform/recovery outbox
        // and refreshes its view. Acoustic detection + synchronous EVENT_HEAR,
        // Enemy detection, volatile target rebuild, non-Enemy detectable
        // buckets, and the resulting FIFO Think dispatches then all finish for
        // that NPC before the next creation slot starts.
        self.tick_enemy_ai_refresh_detection(
            sim,
            assets,
            &world,
            positions_before_movement,
            None,
            positions_before_movement.is_some(),
        );

        // Focused detection tests retain the legacy fallback drains for
        // stimuli injected outside the production owner coordinator. Every
        // production Think now drains its own effects/stimuli synchronously;
        // re-running 6c/6d globally here would re-batch owner work.
        #[cfg(test)]
        if positions_before_movement.is_none() {
            self.tick_enemy_ai_drain_swordfight_requests(sim, assets);
            self.tick_enemy_ai_drain_pending_stimuli(sim, assets);
        }
        self.ai.global.same_frame_target_claims.clear();

        // Sword strikes are launched by `engine::melee::tick_enemy_sword_attacks`.
        // Keep this AI pass to target selection, pursuit, and swordfight
        // requests; applying direct damage here would bypass the
        // wait-timer + interaction sequence timing.
    }

    /// Per-NPC drain for all `pending_*` flags on [`AiController`] that
    /// mutate engine state (launch sequences / orders, toggle attentive
    /// mode, fire cross-NPC stimuli, etc.).  Extracted from the global
    /// post-Think drain loop so the same body can also run synchronously
    /// right after each [`Self::dispatch_filtered_stimulus`] call via
    /// [`Self::dispatch_think_with_drain`] — matching `think()`
    /// semantics where handler side effects (`launch_sequence`,
    /// `set_attentive_mode`, `face`, …) are immediate.
    #[cfg(test)]
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn drain_pending_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_pending_for_npc_mode(sim, npc_id, assets, false, false);
    }

    pub(super) fn drain_pending_for_npc_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        // Direct engine-owned AI calls also enter this drain. Close the
        // SetState callback boundary before consuming halt/effect/order work.
        self.drain_ai_owner_work_for_mode(
            sim,
            assets,
            npc_id,
            owner_local_no_forecast,
            defer_turn_instruction,
        );
        self.drain_patrol_direction_broadcast_for(sim, npc_id, assets);

        // Direct SetDirection calls made before StopAll must update the goal
        // before the halt/transition barrier. They do not create a standalone
        // Turn element; the subsequently selected action performs the turn.
        let direction_goal = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.outbox.actor.take_direction_goal()
        };
        if let Some(direction_goal) = direction_goal
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.position_iface_mut().set_direction(
                crate::position_interface::Direction::from_raw(direction_goal as i32),
            );
        }

        // Drain pending_halt FIRST so the actor's in-progress sequence
        // (typically a Move element while running toward the target) is
        // torn down before any subsequent intent (e.g.
        // `pending_enter_swordfight`) launches a new sequence.
        // `begin_swordfight` / `break_macro` callers call
        // `stop_all() → halt() → stop(PREFERENCE)` inline before
        // `launch_sequence_element(EnterSwordfight)`.
        //
        // Without this ordering, `enter_swordfight`'s
        // `pathfinder.cancel_requests_for` (a no-op post-refactor) and
        // local `clear_path` leave the orphaned Move sequence in
        // InProgress state.  An in-flight path response then
        // `try_dispatch_move_path`s onto the actor a few ticks later,
        // restoring `active_movement` and re-driving the run animation
        // — the visual "stuck in running pose" symptom.
        let halt_count = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.outbox.actor.take_halt_count()
        };
        if halt_count != 0 {
            // `StopAll` calls `Stop(PREFERENCE)` synchronously before the
            // handler continues into SetState/SetAttentiveMode and other
            // replacement work. Deliver the halt condolence at that same
            // barrier: actor-base cleanup must clear a selected movement's
            // cached goal before a newly launched attentive transition can
            // become the selected element. `from_halt` suppresses the NPC
            // EventDone/Impossible callbacks while retaining that base
            // selected-element cleanup.
            for _ in 0..halt_count {
                self.halt_actor(npc_id);
                self.dispatch_condolations_for_npc(sim, npc_id, assets);
            }
        }

        // The halt application above is a real same-frame barrier: only now
        // take the prefixes that the original `go_to` path launches next.
        let preemption = {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!("pending-drain NPC {} has no AI controller", npc_id.index())
            });
            ai.outbox.actor.take_movement_prefixes()
        };

        // Take exactly the channels read at the first post-Think barrier.
        // Later barrier groups remain live so re-entrant sequence work can
        // still enqueue effects that this pass observes at their Original
        // application point.
        let (effects, finish_lost_enemy_overview) = {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!("pending-drain NPC {} has no AI controller", npc_id.index())
            });
            (
                ai.outbox.actor.take_core(),
                ai.outbox.actor.take_lost_enemy_overview_after_quit(),
            )
        };
        assert!(
            effects.enter_swordfight_jump_line.is_none()
                || matches!(
                    effects.enter_swordfight,
                    Some(crate::ai::EnterSwordfightRequest::Engage(_))
                ),
            "pending-drain owner {} queued a swordfight jump line without an engagement",
            npc_id.index()
        );

        // EndSwordfight launches an explicit QUIT_SWORDFIGHT element.  Do
        // not tear down the relationship directly here: the command owns
        // both that teardown and the visible lowering-sword transition, and
        // LaunchSequenceElement arbitrates it synchronously in the Original.
        if effects.quit_swordfight {
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::QuitSwordfight,
                Some(npc_id),
            ));
            // LaunchSequenceElement reaches Instruct synchronously. If the
            // quit replaces a selected command, its SendCondolationCard
            // callback therefore re-enters Think before EndSwordfight
            // returns to its caller.
            self.dispatch_condolations_for_npc(sim, npc_id, assets);
        }

        // Process stop_menace — the explicit `STOP_MENACE` element
        // prepend in `go_to`.  Launching a `Command::StopMenace`
        // element here lets the per-element dispatch in `tick.rs`
        // queue `TRANSITION_MENACING_WAITING_SWORD` then
        // `TRANSITION_LOWERING_SWORD` before the move that
        // `launch_pending_orders_for_npc` is about to launch starts.
        if preemption.stop_menace {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::StopMenace,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process lower_shield — the explicit `LOWER_SHIELD` element
        // prepend in `go_to`.  Launching a `Command::LowerShield`
        // element here lets `dispatch_lower_shield` queue the
        // `LoweringShield` order so the shield arm completes before
        // `launch_pending_orders_for_npc` runs the move.
        if preemption.lower_shield {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::LowerShield,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process pending `stop()` on a different entity — the
        // `primary_target.stop()` call inside `begin_swordfight`.  The
        // default `stop()` uses `Normal` priority.  Drained before
        // `enter_swordfight` so the target's in-flight Move element is
        // torn down before the engine-side ENTER_SWORDFIGHT sequence
        // runs.
        if let Some(target_handle) = effects.stop_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI stop_target");
            // Original BeginSwordfight queries these members at this exact
            // point in the live entity walk. Do not use the AI tick snapshot:
            // an earlier-created target may have completed a movement-start
            // transition since that snapshot was built.
            let should_stop = self
                .get_entity(target_id)
                .and_then(|entity| {
                    Some((
                        entity.human_data()?.opponents.is_empty(),
                        entity.actor_data()?.action_state.is_moving(),
                    ))
                })
                .unwrap_or_else(|| {
                    panic!(
                        "AI stop_target {} did not resolve to a human actor",
                        target_id.index()
                    )
                });
            if should_stop.0 && should_stop.1 {
                self.stop_owner(target_id, crate::sequence::SequencePriority::Normal);
            }
        }

        // The near-enemy EventView path calls
        // `SetState(ATTACKING, REACTIONTIME)` before BattleDecisions reaches
        // BeginSwordfight. SetState synchronously registers
        // ENTER_ATTENTIVE_MODE; BeginSwordfight registers
        // ENTER_SWORDFIGHT afterward, so the attentive lean-forward element
        // is authoritative and the fight waits behind it. Rust batches both
        // effects in one outbox; preserve that authored order instead of
        // draining the core swordfight channel first.
        let attentive_request = {
            let mut take = None;
            if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
                && let Some(base) = s.npc.ai_brain.base_mut()
            {
                take = base.outbox.actor.set_attentive_mode.take();
            }
            take
        };
        if let Some(request) = attentive_request {
            self.set_soldier_attentive_mode(npc_id, request.target, request.fast_officer_variant);
            if request.forget_after
                && let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(npc_id)
                && let Some(enemy) = soldier.npc.ai_brain.enemy_mut()
            {
                // Original SetState's SetAttentiveMode call has now launched
                // its transition and written will-be-attentive. The special
                // event handler's following ForgetAttentiveMode call wins the
                // final flag state without clearing forced-attentive.
                enemy.attentive = false;
                enemy.will_be_attentive = false;
            }
        }

        // Process enter_swordfight.  Two shapes:
        //   * Engage(target) — engagement against a specific opponent.
        //     Original `BeginSwordfight` launches ENTER_SWORDFIGHT; it
        //     does not call `EnterSwordFight` directly from Think.  Keep
        //     relationship and animation changes behind that owner boundary.
        //   * Direct(target) — EVENT_GOTHIT's direct EnterSwordFight call.
        //     This immediately updates both opponent lists and, when the
        //     attacker is not already swordfighting, authors the reciprocal
        //     ENTER_SWORDFIGHT element with the attacker as owner.
        //   * Rebalance(target) — ReconsiderSwordfight's direct
        //     `RHElementActorHuman::EnterSwordFight` call. This updates the
        //     relationship without authoring a recursive command/EventDone.
        //   * RaiseSword — sword pose without engagement. `go_to`'s
        //     `GOTO_SWORD` arm, `AttackingApproachToObserve`, and
        //     menace-effect-of-hit need a sword pose held without an
        //     active fight.
        if let Some(request) = effects.enter_swordfight {
            match request {
                crate::ai::EnterSwordfightRequest::RaiseSword => {
                    let mut elem = crate::sequence::SequenceElement::new_generic(
                        1,
                        crate::element::Command::EnterSwordfight,
                        Some(npc_id),
                    );
                    // Original AI explicitly stores null in RHFIELD_OPPONENT
                    // for the raise-sword-only form.  Preserve that
                    // distinction from a malformed element which omitted the
                    // required property altogether.
                    elem.set_property(
                        crate::sequence::Field::Opponent,
                        crate::sequence::FieldValue::Integer(0),
                    );
                    elem.set_property(
                        crate::sequence::Field::JumplineDestination,
                        crate::sequence::FieldValue::Integer(0),
                    );
                    self.launch_element(elem);
                }
                crate::ai::EnterSwordfightRequest::Engage(target_handle) => {
                    let target_id = self
                        .expect_human_id_for_ai_handle(target_handle, "AI enter_swordfight target");
                    let mut elem = crate::sequence::SequenceElement::new_generic(
                        1,
                        crate::element::Command::EnterSwordfight,
                        Some(npc_id),
                    );
                    elem.set_property(
                        crate::sequence::Field::Opponent,
                        crate::sequence::FieldValue::Element(target_id),
                    );
                    if let Some(jump_line) = effects
                        .enter_swordfight_jump_line
                        .and_then(crate::jump_line::JumpLineIndex::new)
                    {
                        elem.set_property(
                            crate::sequence::Field::JumplineDestination,
                            crate::sequence::FieldValue::LineId(jump_line),
                        );
                    } else {
                        elem.set_property(
                            crate::sequence::Field::JumplineDestination,
                            crate::sequence::FieldValue::Integer(0),
                        );
                    }
                    // `RHArtificialMalignity::BeginSwordfight` performs
                    // `StopAll()` before registering ENTER_SWORDFIGHT.
                    // `Stop(PREFERENCE)` does not itself run the selected
                    // movement's condolence callback, so the sprite retains
                    // its last movement goal while the sword transition takes
                    // ownership.
                    self.launch_element(elem);
                }
                crate::ai::EnterSwordfightRequest::Direct(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle,
                        "AI direct swordfight target",
                    );
                    self.direct_enter_swordfight(sim, assets, npc_id, target_id);
                }
                crate::ai::EnterSwordfightRequest::Rebalance(target_handle) => {
                    let target_id = self.expect_human_id_for_ai_handle(
                        target_handle,
                        "AI reconsider swordfight target",
                    );
                    if self.direct_enter_swordfight(sim, assets, npc_id, target_id) {
                        let Entity::Soldier(soldier) = self
                            .world
                            .entities
                            .get_mut(npc_id)
                            .expect("successful AI swordfight rebalance owner disappeared")
                        else {
                            panic!("successful AI swordfight rebalance owner is not a soldier");
                        };
                        soldier
                            .npc
                            .ai_brain
                            .enemy_mut()
                            .expect("successful AI swordfight rebalance owner lost enemy AI")
                            .base
                            .primary_target = target_handle;
                    }
                }
            }
        }

        // Process set_as_new_principal_opponent.
        if let Some(opponent_handle) = effects.set_principal {
            let opponent_id =
                self.expect_human_id_for_ai_handle(opponent_handle, "AI principal opponent");
            self.set_as_new_principal_opponent(assets, npc_id, opponent_id);
        }

        // Process friend primary-target swaps.  The reference calls
        // `friend.set_primary_target(primary_target)` directly on the
        // other soldier when the swap heuristic fires — for every
        // improving friend in the pass, not just the last one — so we
        // apply the whole queue here after the owner's AI tick ran.
        for (friend_id, new_target) in effects.friend_primary_target_swaps {
            let friend = self.world.entities.get_mut(friend_id).unwrap_or_else(|| {
                panic!(
                    "pending-drain NPC {} primary-target friend {} disappeared",
                    npc_id.index(),
                    friend_id.index()
                )
            });
            let Entity::Soldier(friend) = friend else {
                panic!(
                    "pending-drain NPC {} primary-target friend {} is not a soldier",
                    npc_id.index(),
                    friend_id.index()
                );
            };
            friend
                .npc
                .ai_brain
                .base_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "pending-drain NPC {} primary-target friend {} has no AI",
                        npc_id.index(),
                        friend_id.index()
                    )
                })
                .primary_target = new_target;
        }

        // Process pending bow shot.
        if let Some(target_handle) = effects.shoot_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI bow target");
            self.shoot_bow_at(assets, npc_id, target_id);
        }

        // Process pending focus / focus_point / unfocus — the
        // `focus(primary_target)` / `focus(position&)` / `focus(NULL)`
        // calls.  Each explicit channel "consumes" the primary_target
        // edge by stamping `last_synced_focus_target = primary_target`,
        // so `refresh_npc_views` sees no edge and does not auto-revert
        // the explicit focus state next tick.  This is what makes
        // patterns like rider-charge passing (`focus(NULL)` while
        // `primary_target` stays set) and `battle_decisions` entry
        // honour the synchronous ordering even though the channel
        // itself is deferred.
        let mut focus_channel_fired = false;
        if let Some(target_handle) = effects.focus {
            // Original `RHElementActorNPC::Focus` accepts an arbitrary
            // `RHElement*`. Object-handling AI legitimately focuses bonuses
            // such as ale bottles, so preserve the element kind while still
            // treating a missing raw slot as corrupted state.
            let target_id = self.expect_entity_id_for_index(target_handle, "AI focus target");
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            crate::ai_vision::focus_entity(npc, target_id);
            focus_channel_fired = true;
        }

        if let Some(point) = effects.focus_point {
            // Original `Focus(RHposition&)` first calls
            // `PositionToPoint3D(posTarget, false)` and stores that point's
            // world X/Y in `starePoint`.
            let point_3d =
                self.position_to_point_3d(assets, point.sector, point.level, point.x, point.y);
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            crate::ai_vision::focus_point(
                npc,
                crate::coordinates::GroundPoint::new(point_3d.x, point_3d.y),
            );
            focus_channel_fired = true;
        }

        if effects.unfocus {
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            crate::ai_vision::unfocus(npc);
            focus_channel_fired = true;
        }

        if focus_channel_fired {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!("pending-drain owner {} lost AI after focus", npc_id.index())
                });
            ai.last_synced_focus_target = (ai.primary_target != 0).then_some(ai.primary_target);
        }

        // Process pending SlowlyOpenEyes — `slowly_open_eyes` sets
        // `view_radius = 5`, points `view_radius_goal` at the engine's
        // standard view radius, switches `eye_status` to
        // `ViewconeGrow`, and marks `view_transition`.  The
        // `ViewconeGrow` branch of `refresh_view` then ramps the cone
        // back open at 8 units/frame.
        if effects.slowly_open_eyes {
            let standard = self.ai.standard_view_polygon_radius;
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            npc.view_transition = true;
            npc.view_radius = 5;
            npc.view_radius_base = 5;
            npc.view_radius_goal = standard;
            npc.eye_status = crate::element::EyeStatus::ViewconeGrow;
        }

        // Process pending set_direction_instantly.
        if let Some(dir) = effects.set_direction_instantly
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.position_iface_mut().set_direction_instantly(
                crate::position_interface::Direction::from_raw(dir as i32),
            );
        }

        // Preserve the authored boundary on either side of SetState's
        // attentive-mode call. Face normally launches before attentive mode;
        // Face authored after SetState is held until that transition has
        // launched, matching the two distinct C++ statement orders.
        let orders_after_attentive = {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "pending-drain owner {} lost AI before Face split",
                        npc_id.index()
                    )
                });
            let orders = std::mem::take(&mut ai.outbox.actor.orders);
            let (before, after) = orders
                .into_iter()
                .partition(|intent| !intent.after_attentive_mode);
            ai.outbox.actor.orders = before;
            after
        };
        self.launch_pending_orders_for_npc_mode_after_halt(
            sim,
            assets,
            npc_id,
            defer_turn_instruction,
            halt_count != 0,
        );
        // Original GoTo constructs and launches its movement sequence inline
        // inside the AI call. Promote this owner's queued intent now so path
        // topology and any construction-time RNG are observed at this exact
        // Think boundary. The returned sequence actions remain registered for
        // the later SequenceManager::Hourglass instruction phase.
        let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);

        if !orders_after_attentive.is_empty() {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "pending-drain owner {} lost AI after attentive mode",
                        npc_id.index()
                    )
                });
            ai.outbox.actor.orders.extend(orders_after_attentive);
            // EnterAttentiveMode is registered but remains Todo until the
            // sequence-manager instruction phase. Register following Turns
            // behind it as well so that phase arbitrates the two elements in
            // authored FIFO order instead of eagerly instructing the Turn
            // past the still-Todo attentive barrier.
            self.launch_pending_orders_for_npc_mode_after_halt(sim, assets, npc_id, true, true);
        }

        // Process pending `SetGuardedPC` — `set_guarded_pc`.  The AI
        // wrote its own `guarded_pc` field already; here we flip the
        // reciprocal `pc.guard` on the old and new target PCs.
        let guard_delta = if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
            && let Some(base) = s.npc.ai_brain.base_mut()
        {
            base.outbox.actor.set_guarded_pc.take()
        } else {
            None
        };
        if let Some(guard_delta) = guard_delta {
            // Clear `pc.guard` on the old target
            // (`guarded_pc.set_guard(NULL)`).
            if let Some(old_pc) = guard_delta.old {
                let old_pc_id = EntityId::Pc(old_pc);
                match self.world.entities.get_mut(old_pc_id) {
                    Some(Entity::Pc(pc)) => pc.pc.guard = None,
                    Some(entity) => tracing::warn!(
                        npc = ?npc_id,
                        target = ?old_pc_id,
                        actual_kind = ?entity.kind(),
                        "guarded-PC clear target has the wrong entity kind"
                    ),
                    None => tracing::warn!(
                        npc = ?npc_id,
                        target = ?old_pc_id,
                        "guarded-PC clear target does not exist"
                    ),
                }
            }
            // Set `pc.guard` on the new target
            // (`guarded_pc.set_guard(self)`).  Asserts `is_in_coma()`
            // on the PC; the only caller already gates on the coma
            // check in the `AttackingApproachingSleepingEnemy`
            // handler, so skip the redundant debug_assert here.
            if let Some(new_pc) = guard_delta.new {
                let new_pc_id = EntityId::Pc(new_pc);
                match self.world.entities.get_mut(new_pc_id) {
                    Some(Entity::Pc(pc)) => pc.pc.guard = Some(npc_id),
                    Some(entity) => tracing::warn!(
                        npc = ?npc_id,
                        target = ?new_pc_id,
                        actual_kind = ?entity.kind(),
                        "guarded-PC set target has the wrong entity kind"
                    ),
                    None => tracing::warn!(
                        npc = ?npc_id,
                        target = ?new_pc_id,
                        "guarded-PC set target does not exist"
                    ),
                }
            }
        }

        // Process pending entity deactivation (merry man leaving map).
        // Equivalent to `set_active(false)`.
        if effects.deactivate
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.element_data_mut().active = false;
            tracing::debug!(
                npc = npc_id.index(),
                "Deactivated entity (merry man left map)"
            );
        }

        // Process pending `set_reported_to_officer(flag)` — the
        // `charly.set_reported_to_officer(false)` call inside
        // `missed_charly_alert`.  Writes the other NPC's
        // `EnemyAi::reported_to_officer` flag.
        let reported_updates = if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            std::mem::take(&mut ai.outbox.actor.set_reported_to_officer)
        } else {
            Vec::new()
        };
        for (target_handle, value) in reported_updates {
            let target_id = EntityId::Soldier(SoldierId(target_handle));
            let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                continue;
            };
            if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                enemy_ai.reported_to_officer = value;
            }
        }

        // Process pending bow-ammo refill — the
        // `set_ammo_amount(BOW, MAX_NPC_ARROWS)` call inside
        // `fleeing_run_for_arrow_reserves`.
        {
            let refill = if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
                && let Some(ai) = s.npc.ai_brain.base_mut()
            {
                let r = ai.outbox.actor.refill_bow_ammo;
                ai.outbox.actor.refill_bow_ammo = false;
                r
            } else {
                false
            };
            if refill && let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id) {
                s.npc.number_of_arrows = crate::parameters_ai::MAX_NPC_ARROWS as u16;
            }
        }

        // Process the ordered archery-reservation release — the
        // `set_my_archery_sector(NULL)` call queued from
        // `EnemyAi::set_state` when the soldier leaves an archer-wait
        // substate.  Decrement the owner counter on the current
        // archery sector and clear the index.  The companion
        // typed effect carries the prior shooting
        // point's `(sector, point)` so we can also run the
        // `set_my_shooting_point(NULL)` `set_owner(NULL)` write here —
        // the AI layer already cleared its own `my_shooting_point`
        // field synchronously in `set_state`.
        {
            let release = if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
                && let Some(enemy) = s.npc.ai_brain.enemy_mut()
            {
                let effect = enemy.base.outbox.actor.take_archery_reservation_release();
                let sector = if effect.release_sector {
                    enemy.my_archery_sector.take()
                } else {
                    None
                };
                (sector, effect.shooting_point)
            } else {
                (None, None)
            };
            if let (_, Some(point)) = release
                && let Some(sector) = self
                    .ai
                    .global
                    .archery_sectors
                    .get_mut(point.sector_index as usize)
                && let Some(pt) = sector.points.get_mut(usize::from(point.point_index))
            {
                pt.owner = None;
            }
            if let (Some(idx), _) = release
                && let Some(sector) = self.ai.global.archery_sectors.get_mut(idx as usize)
            {
                sector.decrement_owner_counter();
            }
        }

        // Process pending UnalertAllNearCharlySeekers — walks all
        // soldier NPCs in the same camp and for each candidate that
        //   - is alive / active / not the seeker / not the charly,
        //   - passes the rank/antagonist guard
        //     `(seeker_rank == OFFICER || cs != antagonist)`,
        //   - and detects either charly or self within 180°,
        // dispatches `CALL_CHARLY_IS_BACK` carrying charly's handle.
        // The pending field's payload selects either self or an
        // explicit Charly handle.
        let unalert = if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            let u = ai.outbox.actor.unalert_near_charly_seekers;
            ai.outbox.actor.unalert_near_charly_seekers = None;
            u
        } else {
            None
        };
        if let Some(target_charly) = unalert {
            let (my_rank, my_antagonist) = match self.get_entity(npc_id) {
                Some(Entity::Soldier(s)) => (
                    s.npc
                        .ai_brain
                        .enemy()
                        .map(|e| e.soldier_profile_rank)
                        .unwrap_or(crate::profiles::ProfileRank::None),
                    s.npc.ai_brain.base().map(|b| b.antagonist).unwrap_or(0),
                ),
                _ => (crate::profiles::ProfileRank::None, 0),
            };
            let charly_handle = match target_charly {
                crate::ai::CharlySeekerTarget::SelfNpc => npc_id.index(),
                crate::ai::CharlySeekerTarget::Npc(handle) => handle,
            };
            if self
                .world
                .entities
                .id_at_legacy_slot(charly_handle)
                .and_then(|charly_id| self.world.entities.get(charly_id))
                .is_some()
            {
                let charly_is_self = charly_handle == npc_id.index();
                for other_id in self.world.entities.npc_ids().collect::<Vec<_>>() {
                    if other_id == npc_id {
                        continue;
                    }
                    if other_id.index() == charly_handle {
                        continue;
                    }
                    // Rank/antagonist guard:
                    //   `rank == Officer || other != antagonist`.
                    if my_rank != crate::profiles::ProfileRank::Officer
                        && other_id.index() == my_antagonist
                    {
                        continue;
                    }
                    let eligible = {
                        let Some(Entity::Soldier(os)) = self.world.entities.get(other_id) else {
                            continue;
                        };
                        // `IsDetecting180Degrees` checks only the raw active
                        // flag for the viewer and target. Dead or unconscious
                        // soldiers are not filtered by the outer Original
                        // `UnalertAllNearCharlySeekers` walk.
                        os.element.active
                    };
                    if !eligible {
                        continue;
                    }
                    // Original evaluates the full visibility predicate in
                    // this exact short-circuit order. Besides the cone gate,
                    // each surviving arm samples ComputeViewRadius and runs
                    // opaque LOS, both of which are observable and may affect
                    // whether CALL_CHARLY_IS_BACK is delivered.
                    let scratch = self.build_sim_scratch(sim, assets);
                    let other_ctx = {
                        let Some(entity) = self.world.entities.get(other_id) else {
                            continue;
                        };
                        let building_sector =
                            self.entity_building_sector(entity.element_data().sector());
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
                            &self.ai.global.all_soldier_handles,
                            self.control.sim_config.difficulty,
                        )
                    };
                    other_ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
                    let detects_charly = self
                        .world
                        .entities
                        .get(other_id)
                        .and_then(Entity::enemy_ai)
                        .unwrap_or_else(|| {
                            panic!(
                                "UnalertAllNearCharlySeekers candidate {} lost EnemyAi",
                                other_id.index()
                            )
                        })
                        .is_detecting_180_degrees(charly_handle, &other_ctx);
                    let detects_me_branch = !detects_charly
                        && !charly_is_self
                        && self
                            .world
                            .entities
                            .get(other_id)
                            .and_then(Entity::enemy_ai)
                            .unwrap_or_else(|| {
                                panic!(
                                    "UnalertAllNearCharlySeekers candidate {} lost EnemyAi",
                                    other_id.index()
                                )
                            })
                            .is_detecting_180_degrees(npc_id.index(), &other_ctx);
                    other_ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);
                    if !(detects_charly || detects_me_branch) {
                        continue;
                    }
                    let stimulus = crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::CallCharlyIsBack,
                        charly_handle,
                    );
                    // The preceding drain work and earlier Charly recipients
                    // may have synchronously changed entity state. The
                    // recipient context above was built at this exact Think
                    // boundary and is also the one used for dispatch.
                    let tick_data = self.build_npc_tick_data(sim, other_id, &scratch, assets);
                    self.dispatch_filtered_stimulus(
                        sim, assets, other_id, &stimulus, &other_ctx, &tick_data,
                    );
                }
            }
        }

        // Process pending launch commands — create and launch
        // sequence elements for commands the AI wants to execute.
        for cmd in effects.launch_commands {
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(npc_id));
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(elem);
            // AI helpers call LaunchSequenceElement, which registers an
            // ordinary owned command with SequenceManager. It does not run
            // the actor's Instruct/arbitration inline at the AI call site.
            self.launch_sequence(sequence);
        }

        // Sequence commands the AI wants to launch on *another*
        // entity (e.g. soldier forcing a beggar to stand up).
        // Equivalent to a `launch_sequence_element(cmd,
        // other_actor)` call as used by the enemy beggar-identify
        // cascade.
        for (target_handle, cmd) in effects.launch_on_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI command target");
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(target_id));
            self.launch_element(elem);
        }

        // Cross-actor speech authored directly after a target command. The
        // beggar-identification path, for example, launches SHOW_FACE and
        // then synchronously calls Say on that civilian. Queue through the
        // target AI's ordinary owner-work path so all Say gates, sound
        // requests, and automatic forbids remain authoritative.
        for (target_handle, remark) in effects.say_on_target {
            let target_id = self.expect_human_id_for_ai_handle(target_handle, "AI speech target");
            let target = self.world.entities.get_mut(target_id).unwrap_or_else(|| {
                panic!(
                    "AI speech target {} disappeared before Say",
                    target_id.index()
                )
            });
            target
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "AI speech target {} has no AI controller",
                        target_id.index()
                    )
                })
                .say(remark);
            self.drain_ai_owner_work_for(sim, assets, target_id);
        }

        // Full sequences the AI wants to launch verbatim — the
        // `launch_sequence(SEQ_INFO, sequence)` calls inside AI
        // handlers (e.g. the officer's turn/gather/point alert
        // sequence). `RHSequence::Launch` calls
        // `RegisterSequenceElementToGo` while the AI handler is still on the
        // stack, and `ExecutedImmediately` dispatches engine commands inline.
        // Close that exact boundary for each sequence: batching the drain
        // until after the NPC tail lets a Timer/LockUser successor escape the
        // owner's legacy Hourglass slot.
        for seq in effects.launch_sequences {
            self.launch_sequence(seq);
            self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                .unwrap_or_else(|error| {
                    panic!(
                        "AI owner {} failed to drain a synchronously launched sequence: {error:?}",
                        npc_id.index()
                    )
                });
        }

        if effects.raise_shield_immediately {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("instant shield owner {npc_id:?} disappeared"));
            entity.set_posture(crate::element::Posture::Upright);
            entity
                .actor_data_mut()
                .expect("instant shield owner is not an actor")
                .action_state = crate::element::ActionState::HoldingShield;
            self.refresh_retained_shield_obstacle(assets, npc_id);
        }

        if effects.refresh_shield {
            self.refresh_retained_shield_obstacle(assets, npc_id);
        }

        // Process pending LookSidewards — build a one- or two-element
        // sequence of LookLeft / LookRight / LeanOut commands and
        // launch it.
        if let Some(dir) = effects.look_sidewards {
            use crate::ai::LookDirection;
            use crate::element::Command;
            let cmds: &[Command] = match dir {
                LookDirection::Left => &[Command::LookLeft],
                LookDirection::Right => &[Command::LookRight],
                LookDirection::LeftRight => &[Command::LookLeft, Command::LookRight],
                LookDirection::RightLeft => &[Command::LookRight, Command::LookLeft],
                LookDirection::Down => &[Command::LeanOut],
            };
            tracing::trace!(
                npc = npc_id.index(),
                ?dir,
                ?cmds,
                "launching look-sidewards sequence"
            );
            // `look_sidewards` calls `focus(NULL)` before allocating
            // the sequence so the soldier's gaze drops its lock for
            // the head-turn animation.  Centralise it here instead of
            // patching every caller.
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            crate::ai_vision::unfocus(npc);
            let mut seq = crate::sequence::Sequence::new();
            for (i, cmd) in cmds.iter().enumerate() {
                let elem =
                    crate::sequence::SequenceElement::new((i as u16) + 1, *cmd, Some(npc_id));
                seq.append_element(elem);
            }
            self.launch_sequence(seq);
        }

        // Process pending "strip beggar from every NPC" requests:
        //   delete_detectable_for_all_npc(stimulus.human, BEGGAR);
        // Fired from the `EventSeesBeggar` handler in `ai_enemy.rs`
        // once a seek-area soldier has claimed the PC-beggar via
        // `beggars_to_control`, so every other soldier's BEGGAR list
        // drops the PC and stops firing duplicate `EventSeesBeggar`
        // stimuli on subsequent frames.
        let delete_beggar_requests: Vec<EntityId> = {
            match self.world.entities.get_mut(npc_id) {
                Some(Entity::Soldier(s)) => std::mem::take(
                    &mut s
                        .npc
                        .ai_brain
                        .base_mut()
                        .unwrap_or_else(|| {
                            panic!("pending-drain soldier {} lost its AI", npc_id.index())
                        })
                        .outbox
                        .actor
                        .delete_beggar_for_all_npc,
                ),
                Some(Entity::Civilian(_)) => Vec::new(),
                Some(_) => panic!("pending-drain owner {} is not an NPC", npc_id.index()),
                None => panic!("pending-drain NPC {} disappeared", npc_id.index()),
            }
        };
        for beggar_id in delete_beggar_requests {
            self.delete_beggar_detectable_for_all_npc(beggar_id);
        }

        // Process pending detectable modifications.
        if !effects.add_detectables.is_empty()
            || !effects.append_detectables.is_empty()
            || !effects.delete_detectables.is_empty()
            || !effects.delete_detectable_entities.is_empty()
        {
            let debug_consider_report = crate::ai::consider_report_debug_matches(
                self.control.frame_counter,
                npc_id.index(),
            );
            if debug_consider_report {
                eprintln!(
                    "CONSIDERREPORT {{\"stage\":\"drain_start\",\"frame\":{},\"owner\":{},\"pending_entity_deletes\":{:?}}}",
                    self.control.frame_counter,
                    npc_id.index(),
                    effects.delete_detectable_entities,
                );
            }
            // Resolve target classification for each ENEMY-arm push
            // so the `add_detectable` filter can run.  Resolved
            // up-front to avoid borrowing `self.world.entities` mutably
            // while we read target metadata from it.
            use crate::element::DetectableType;
            let enemy_target_info: Vec<Option<(bool, bool, crate::element_kinds::Camp, bool)>> =
                effects
                    .add_detectables
                    .iter()
                    .map(|(eid, dt)| {
                        if *dt != DetectableType::Enemy {
                            return None;
                        }
                        let target = self.get_entity(*eid).unwrap_or_else(|| {
                            panic!(
                                "pending-drain owner {} detectable target {} disappeared",
                                npc_id.index(),
                                eid.index()
                            )
                        });
                        Some((
                            target.is_pc(),
                            target.is_soldier(),
                            target.camp(),
                            target.is_human(),
                        ))
                    })
                    .collect();

            let (npc_camp, npc_is_soldier) = {
                let owner = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("pending-drain owner {} disappeared", npc_id.index())
                });
                (owner.camp(), owner.is_soldier())
            };
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            // Delete all detectables of specified types.
            for det_type in &effects.delete_detectables {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                npc.detectable_lists[idx].clear();
            }
            // Per-entity deletes: `delete_detectable(entity, type)`
            // drops a single (element, type) entry, leaving
            // siblings of the same type alone.
            for (entity_id, det_type) in &effects.delete_detectable_entities {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                npc.detectable_lists[idx].retain(|d| d.element != Some(*entity_id));
            }
            if debug_consider_report {
                let body_ids = npc.detectable_lists[DetectableType::Body as usize]
                    .iter()
                    .map(|detectable| detectable.element.map(EntityId::index))
                    .collect::<Vec<_>>();
                eprintln!(
                    "CONSIDERREPORT {{\"stage\":\"drain_after_delete\",\"frame\":{},\"owner\":{},\"body_ids\":{:?}}}",
                    self.control.frame_counter,
                    npc_id.index(),
                    body_ids,
                );
            }
            // Add new detectables.
            for ((entity_id, det_type), tgt) in
                effects.add_detectables.iter().zip(enemy_target_info.iter())
            {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                // ENEMY-arm filter — drop pushes that fail the
                // per-NPC camp/rank arm so a Royalist soldier
                // never tracks a PC and a Lacklandist civilian
                // never tracks a Royalist soldier.
                if *det_type == DetectableType::Enemy {
                    let Some((tgt_pc, tgt_soldier, tgt_camp, tgt_human)) = *tgt else {
                        continue;
                    };
                    if !tgt_human {
                        continue;
                    }
                    if !crate::ai_detectable_filter::should_add_enemy_detectable(
                        npc_camp,
                        npc_is_soldier,
                        tgt_pc,
                        tgt_soldier,
                        tgt_camp,
                    ) {
                        continue;
                    }
                }
                append_detectable(&mut npc.detectable_lists[idx], *entity_id, *det_type, false);
            }
            // Preserve the release-build behavior of selected direct
            // AddDetectable calls. Original's uniqueness check is an assert,
            // so it disappears in the shipped build and the append still
            // occurs when the entry is already present.
            for (entity_id, det_type) in &effects.append_detectables {
                let idx = *det_type as usize;
                assert!(
                    idx < npc.detectable_lists.len(),
                    "pending-drain owner {} has no {:?} detectable list",
                    npc_id.index(),
                    det_type
                );
                append_detectable(&mut npc.detectable_lists[idx], *entity_id, *det_type, true);
            }
        }

        // Process pending `ForgetAllNearbyCoins` request — the first
        // half of `forget_all_nearby_coins`: walk the
        // `DETECTABLE_OBJECT` list and drop every coin entry whose
        // referenced element is within Chebyshev 500 of `pos`.  The
        // second half (`other_seen_money.clear()`) is performed
        // synchronously on the AI side in
        // `EnemyAi::forget_all_nearby_coins`.
        let forget_pos = {
            if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id) {
                s.npc
                    .ai_brain
                    .base_mut()
                    .unwrap_or_else(|| {
                        panic!("pending-drain soldier {} lost its AI", npc_id.index())
                    })
                    .outbox
                    .actor
                    .forget_nearby_coins
                    .take()
            } else {
                None
            }
        };
        if let Some(pos) = forget_pos {
            use crate::element::DetectableType;
            use crate::element_kinds::ObjectType;
            const NEARBY_COIN_DISTANCE: f32 = 500.0;
            let det_idx = DetectableType::Object as usize;
            // Snapshot the candidate element ids first so we can read
            // `entities` immutably while iterating, then mutate the
            // detectable list in a second pass.
            let mut to_remove: Vec<crate::element::EntityId> = Vec::new();
            if let Some(Entity::Soldier(s)) = self.world.entities.get(npc_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                for det in &s.npc.detectable_lists[det_idx] {
                    let Some(elem_id) = det.element else {
                        continue;
                    };
                    let Some(elem) = self.world.entities.get(elem_id) else {
                        continue;
                    };
                    let Some(obj) = elem.object_data() else {
                        continue;
                    };
                    if obj.object_type != ObjectType::Coin {
                        continue;
                    }
                    let elem_pos = elem.element_data().position_map();
                    let dx = (elem_pos.x - pos.x).abs();
                    let dy = (elem_pos.y - pos.y).abs();
                    if dx.max(dy) < NEARBY_COIN_DISTANCE {
                        to_remove.push(elem_id);
                    }
                }
            }
            if !to_remove.is_empty()
                && let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                s.npc.detectable_lists[det_idx]
                    .retain(|d| d.element.is_none_or(|id| !to_remove.contains(&id)));
            }
        }

        // Process pending SetPosture request.  Like the
        // `set_posture(Sitting/Leisure)` calls in the reference.
        // The move-box recomputation in
        // `PositionInterface::set_posture` is skipped here because
        // the engine stores posture on the element-data struct and
        // the move box is reshaped lazily elsewhere — this matches
        // every other posture write in the codebase (e.g.
        // `abilities.rs` `CarryingCorpse`, `melee.rs` knock-out
        // paths).
        if let Some(p) = effects.posture
            && let Some(entity) = self.world.entities.get_mut(npc_id)
        {
            entity.set_posture(p);
        }

        // Process pending BlinkEnemy(NULL) request — clear the
        // seen_now / seen_last_frame flags on every enemy detectable
        // so the next detection pass treats anyone still in the cone
        // as a "first-seen" edge and re-issues EVENT_VIEW.
        let blink_all = {
            let entity = self
                .world
                .entities
                .get_mut(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let ai = entity
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("pending-drain NPC {} lost its AI", npc_id.index()));
            std::mem::take(&mut ai.outbox.actor.blink_all_enemies)
        };
        if blink_all {
            // BlinkEnemy is defined on RHElementActorNPC, not the soldier
            // subclass. ScriptGoOn therefore reaches this path for both
            // soldiers and civilians.
            let npc = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .unwrap_or_else(|| panic!("pending-drain owner {} lost NPC data", npc_id.index()));
            let idx = crate::element::DetectableType::Enemy as usize;
            let list = npc.detectable_lists.get_mut(idx).unwrap_or_else(|| {
                panic!(
                    "pending-drain owner {} has no enemy detectable list",
                    npc_id.index()
                )
            });
            for det in list.iter_mut() {
                det.seen_now = false;
                det.seen_last_frame = false;
            }
        }
        // Process pending `EnemyInHouseAlert` request.
        //
        // Orchestrator walks the building's occupant list, sorts by
        // camp, dispatches `panic()` to civilians, and calls
        // `init_battle_before_door` on the outnumbered side.  Both
        // the panic side-effect and the door-battle orchestration
        // (`init_battle_before_door` + `send_before_door_to_fight`
        // in `engine/soldier_helpers.rs`) are wired below.
        let in_house_alert = {
            if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id) {
                let ai = s.npc.ai_brain.base_mut().unwrap_or_else(|| {
                    panic!("pending-drain soldier {} lost its AI", npc_id.index())
                });
                let v = ai.outbox.actor.enemy_in_house_alert;
                ai.outbox.actor.enemy_in_house_alert = false;
                v
            } else {
                false
            }
        };
        if in_house_alert {
            self.dispatch_enemy_in_house_alert(sim, npc_id, assets);
        }

        // Drain any pending panic request from the enemy AI — the
        // analogue of the civilian-side drain that runs inside
        // `nearby_civilians_panic`.  Without this, an EnemyAi that
        // pushes a `PanicRequest` (e.g. from the fleeing arm of
        // `think_alerting_event(sim, EVENT_VIEW)` outdoors) stays wedged
        // in `FleeingPanic` with no door picked.
        let has_begin_panic = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| ai.outbox.actor.begin_panic.is_some());
        if has_begin_panic {
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            self.process_pending_begin_panic_for(sim, assets, npc_id, &ctx);
        }

        let has_panic_seek_fallback = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| ai.outbox.actor.panic_seek_fallback);
        if has_panic_seek_fallback {
            // BeginPanic above can mutate the owner and world; do not reuse
            // its context snapshot at this later synchronous boundary.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            self.process_pending_panic_seek_fallback_for(sim, assets, npc_id, &ctx);
        }

        // Drain any pending script-driven SeekArea request.  Matches
        // the immediate `start_think(NO_EVENT); seek_area(sim, ...);
        // end_think(sim, )` block inside `set_ai_state(STATE_SEEKING)`.
        //
        // Only pay the surrounding battle-context cost when the
        // request exists. Keep the cheap pre-check here so the common
        // drain pass does not rebuild full per-NPC tick data for
        // every soldier just to discover
        // `pending_script_seek_area == None`.
        let has_script_seek = self
            .world
            .entities
            .get(npc_id)
            .and_then(|entity| entity.ai_controller())
            .is_some_and(|ai| ai.outbox.actor.script_seek_area.is_some());
        if has_script_seek {
            // The panic boundaries above may have synchronously changed the
            // world. Script SeekArea gets a fresh owner snapshot and tick
            // data at its own Original call boundary.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("pending-drain NPC {} disappeared", npc_id.index()));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            let tick_for_seek =
                self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets);
            self.process_pending_script_seek_area_for(sim, assets, npc_id, &ctx, &tick_for_seek);
        }

        if finish_lost_enemy_overview {
            // EndSwordfight's explicit sequence launch above has now
            // interrupted the old command and delivered its nested
            // condolence. Resume the outer EVENT_OUTOFVIEW handler at the
            // following GetBattleOverview statement with a fresh live view.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("lost-enemy overview owner {npc_id:?} disappeared"));
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
            let tick = self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets);
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::npc_data_mut)
                .and_then(|npc| npc.ai_brain.enemy_mut())
                .unwrap_or_else(|| panic!("lost-enemy overview owner {npc_id:?} has no enemy AI"))
                .get_battle_overview(0, &ctx, &tick);
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
        }
    }

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
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let view_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS as f32
        };
        // `nearby_civilians_panic` builds an aspect-ratio-stretched
        // axis-aligned box (radius, radius * ASPECT_RATIO) around
        // self, then walks every NPC asking:
        //   is_civilian() && box.is_inside(p) && p->is_detecting_360_degrees(self)
        // The second-stage detection uses the civilian's upright eye
        // point, the source actor's detection point, the civilian's
        // live view radius, and opaque 3D LOS.
        let radius_y = view_radius * crate::position_interface::ASPECT_RATIO;

        let (source_map, source_ground, source_detection_point) = {
            let Some(entity) = self.world.entities.get(source) else {
                tracing::trace!(target: "parity_nearby_panic", "source missing");
                return;
            };
            // Source must be IsActiveAndOutsideBuilding for
            // IsDetecting360Degrees to ever return true.
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
                // IsDetecting360Degrees tests only active/outside-building
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
                sector: crate::position_interface::SectorHandle::new(wp.sector),
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
        let will_stop = ai.will_stop_at_next_waypoint(sim, &assets.hiking_paths);
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
        let my_layer = ctx.position.level;
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
                let dx_door = door.point_out.x - ctx.position.x;
                let dy_door = door.point_out.y - ctx.position.y;
                if directed && let Some(center) = request.center {
                    let dx_flee = center.x - ctx.position.x;
                    let dy_flee = center.y - ctx.position.y;
                    if dx_door * dx_flee + dy_door * dy_flee >= 0.0 {
                        continue;
                    }
                }
                let mut distance = dx_door.abs().max(dy_door.abs()) as u32;
                if Some(door.sector_out) != ctx.position.sector.map(u16::from) {
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
                ai.go_to(dest, flags, &goto_ctx);

                // Original GoTo constructs the route before returning to the
                // EventCouldntReachPoint handler. The emergency retry below
                // therefore observes a failed seek-point route immediately.
                // Rust queues movement construction behind the controller
                // borrow, so close just that owner-local path boundary here.
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

    // ─── Patrol coordination ───────────────────────────────────

    /// Close `CMD_PATROL_DIRECTION` at the macro owner's synchronous engine
    /// boundary. Original iterates the live patrol immediately and each
    /// waiting member may `FaceTo` before the macro advances.
    pub(super) fn drain_patrol_direction_broadcast_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
    ) {
        let (direction, members) = {
            let Some(entity) = self.world.entities.get_mut(owner) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            let Some(direction) = ai.outbox.patrol.direction_broadcast.take() else {
                return;
            };
            (direction, ai.patrol.clone())
        };
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        for member in members {
            let in_uninterruptible_command = self.is_very_very_busy(member);
            let building_sector = self
                .world
                .entities
                .get(member)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| {
                    panic!(
                        "patrol direction owner {} references missing member {}",
                        owner.index(),
                        member.index()
                    )
                });
            let entity = self.world.entities.get_mut(member).unwrap_or_else(|| {
                panic!(
                    "patrol direction owner {} lost member {}",
                    owner.index(),
                    member.index()
                )
            });
            let mut ctx = build_ai_context_from_entity(
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            entity
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "patrol direction member {} lost AI for owner {}",
                        member.index(),
                        owner.index()
                    )
                })
                .set_instructed_patrol_direction(direction, &ctx);
            // GetInstructedPatrolDirection invokes FaceTo synchronously, but
            // FaceTo only registers its Turn element with RHSequenceManager.
            // A patrol direction broadcast can run from the chief's actor
            // slot, after RHSequenceManager::Hourglass has already run for
            // this frame. Close the member's AI side effects now while
            // leaving the registered Turn uninstructed until the next
            // sequence-manager pass.
            self.drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
                sim, member, assets,
            );
        }
    }

    /// Per-frame patrol coordination tick.
    ///
    /// The chief-side patrol management of the base AI class:
    /// 1. **`initialize_patrol`** — build active patrol from
    ///    theoretical members (check state, sort by distance,
    ///    pair-swap) on the `needs_patrol_reinit` one-shot flag.
    /// 2. **`refresh_patrol`** — every frame record chief history,
    ///    every 8th frame compute formation positions and dispatch
    ///    `CALL_PATROL_COORDINATE` to each minion.
    ///
    /// `transform_patrol_ids_to_real_patrol` is no longer part of
    /// this tick — it lives in `EngineInner::init_one_ai`, invoked
    /// once at AI bootstrap.
    pub(super) fn tick_patrol_coordination_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};

        if self.actors_frozen() {
            return;
        }
        let scratch = self.build_owner_context_scratch_at_slot_without_forecast(
            assets,
            owner,
            positions_before_movement,
            false,
        );

        let frame = self.control.frame_counter;
        let all_npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        let npc_ids = [owner];

        // ── Phase 2: Snapshot NPC states ──
        // Needed for patrol initialization and missed-member checks.
        #[derive(Clone, Copy)]
        struct NpcSnap {
            position: Position,
            detection_position_world: crate::coordinates::WorldPoint3D,
            direction: u16,
            ground_z: f32,
            posture: crate::element::Posture,
            is_rider: bool,
            in_building: bool,
            ai_state: AiState,
            is_alive: bool,
            is_active: bool,
            real_view_radius: u16,
            move_box: crate::coordinates::MoveBox,
            // Missed-member reacquisition calls virtual `IsAbleToHelp`.
            // Civilians inherit the Human default (`false`); soldiers use
            // their state/substate-aware override.
            is_able_to_help: bool,
            // Patrol admit gate (`initialize_patrol`):
            // `is_civilian() || is_able_to_fight()`.
            is_civilian: bool,
            is_able_to_fight: bool,
        }
        let mut snaps: std::collections::HashMap<EntityId, NpcSnap> =
            std::collections::HashMap::new();
        for &npc_id in &all_npc_ids {
            let Some(entity) = self.world.entities.get(npc_id) else {
                continue;
            };
            // Every position read in `RefreshPatrol` goes through the AI
            // `Position(actor)` helper. In particular, a member whose current
            // sequence command is PassDoor reports the committed gate side,
            // not its interpolating sprite position. The owner-slot view also
            // preserves creation-order map positions for ordinary actors.
            let view = scratch
                .ai_entity_views
                .get(&npc_id.index())
                .unwrap_or_else(|| {
                    panic!(
                        "patrol owner {} is missing AI position view for NPC {}",
                        owner.index(),
                        npc_id.index()
                    )
                });
            let position = view.position;
            let detection_position_world = view.detection_position_world;
            let dir = entity.element_data().direction();
            let npc = entity.npc_data().unwrap_or_else(|| {
                panic!(
                    "patrol owner {} found NPC slot {} without NPC data",
                    owner.index(),
                    npc_id.index()
                )
            });
            let ai_state = npc.ai_state();
            // IsDetecting360Degrees uses the post-RefreshView real radius,
            // which is the growing/goal radius already multiplied by the
            // long-range, stare/follow, rider and drunkenness factors. Using
            // the pre-factor base radius loses every member a staring chief
            // can still feel.
            let real_view_radius = npc.view_radius;
            let move_box = *entity.position_iface().get_move_box();
            let is_civilian = entity.is_civilian();
            let is_able_to_help = match entity {
                crate::element::Entity::Soldier(soldier) => {
                    crate::ai_enemy::soldier_is_able_to_help_state(
                        !entity.is_dead() && !soldier.human.unconscious,
                        ai_state,
                        npc.ai_substate(),
                    )
                }
                _ => false,
            };
            let is_able_to_fight = match entity {
                crate::element::Entity::Soldier(s) => {
                    use crate::element::Human as _;
                    s.is_able_to_fight()
                }
                crate::element::Entity::Pc(pc) => {
                    use crate::element::Human as _;
                    pc.is_able_to_fight()
                }
                // Civilians, props, etc.: the default
                // `is_able_to_fight()` is `false` — but civilians flow
                // through the `is_civilian()` arm of the patrol gate
                // instead.
                _ => false,
            };

            snaps.insert(
                npc_id,
                NpcSnap {
                    position: Position {
                        x: position.x,
                        y: position.y,
                        sector: position.sector,
                        level: position.level,
                    },
                    detection_position_world,
                    direction: dir as u16,
                    ground_z: entity.element_data().position().z,
                    posture: entity.element_data().posture,
                    is_rider: entity.soldier_data().is_some_and(|soldier| soldier.rider),
                    in_building: self.entity_data_in_building_sector(entity.element_data()),
                    ai_state,
                    is_alive: !entity.is_dead(),
                    is_active: entity.is_active(),
                    real_view_radius,
                    move_box,
                    is_able_to_help,
                    is_civilian,
                    is_able_to_fight,
                },
            );
        }

        // ── Phase 3: Initialize patrols + compute formation positions ──
        struct PatrolCmd {
            minion: EntityId,
            target: Position,
            direction: u16,
        }
        let mut patrol_cmds: Vec<PatrolCmd> = Vec::new();
        let mut chief_assigns: Vec<(EntityId, EntityId)> = Vec::new(); // (minion, chief)

        for &npc_id in &npc_ids {
            // `refresh_patrol`: chiefs in {Flying, OnLadder, OnWall}
            // or mid-{PassDoor, Fall} sequence command skip the
            // entire tick — formation targets would trail an unusable
            // position and the 16-pixel side offset would still get
            // dispatched.  Check before acquiring the entity/ai borrow
            // so the engine-level helper can read `self`.
            if self.is_very_very_busy(npc_id) {
                continue;
            }
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                continue;
            };
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "patrol owner {} has no required AI controller",
                    npc_id.index()
                )
            });

            // ── Initialize patrol on the one-shot reinit trigger ──
            // `initialize_patrol()` is called explicitly from
            // `init_one_ai`, `return_to_duty`, the `CMD_PATROL_START`
            // macro opcode, and the `Substate::DefaultGotoRoute`
            // EVENT_REACHPOINT handler — all of which set
            // `needs_patrol_reinit` on the chief.  Switching on the
            // flag (instead of "both lists empty") prevents a chief
            // whose minions all died/were promoted out from silently
            // re-initialising every tick — such chiefs stay in the
            // `patrol_size == 0 && missed == 0` early-return.  When
            // the flag fires we clear `patrol` and
            // `missed_patrol_members` before re-populating from
            // `theoretical_patrol`.
            if ai.needs_patrol_reinit {
                ai.needs_patrol_reinit = false;
                ai.patrol.clear();
                ai.missed_patrol_members.clear();
                let theoretical = ai.theoretical_patrol.clone();
                let chief_snap = snaps.get(&npc_id).copied().unwrap_or_else(|| {
                    panic!(
                        "patrol owner {} is missing its owner-boundary position snapshot",
                        npc_id.index()
                    )
                });
                let chief_pos = chief_snap.position;
                let obstacles_owned = scratch.ai_sight_obstacles.clone();
                let obstacles = obstacles_owned.list();

                for &member in &theoretical {
                    if member == npc_id {
                        continue;
                    }
                    if let Some(snap) = snaps.get(&member) {
                        // `initialize_patrol`: admit only if
                        // `is_detecting_360_degrees(member) &&
                        // ai_state == Default && (is_civilian() ||
                        // is_able_to_fight())`.  Members failing the
                        // gate but still alive flow into the missed
                        // list for later re-acquisition.
                        // `IsDetecting360Degrees(RHElementActorHuman*)`
                        // is the first operand in Original's admission chain.
                        // It uses the chief's upright eye point and the
                        // member's posture-dependent detection point for both
                        // its 3-D distance and opaque-obstacle ray. Preserve
                        // that call before the state/fighting predicates so
                        // rejected active members still produce the same LOS.
                        let admit = patrol_member_admitted(
                            chief_snap.is_active && snap.is_active,
                            || {
                                patrol_member_visible_from_raw_world(
                                    chief_snap.detection_position_world,
                                    chief_snap.is_rider,
                                    chief_snap.real_view_radius,
                                    chief_snap.in_building,
                                    snap.detection_position_world,
                                    snap.posture,
                                    snap.is_rider,
                                    snap.direction as i16,
                                    snap.in_building,
                                    obstacles,
                                )
                            },
                            snap.ai_state,
                            snap.is_civilian,
                            snap.is_able_to_fight,
                        );
                        if admit {
                            ai.patrol.push(member);
                            chief_assigns.push((member, npc_id));
                        } else if snap.is_alive {
                            ai.missed_patrol_members.push(member);
                        }
                    }
                }

                // `InitializePatrol` orders members with
                // `SquareDistance`: subtract the full 3-D ground
                // positions, stretch world Y by the inverse isometric
                // aspect ratio, then take the squared norm.  Map Y is
                // `world_y - z`, so reconstruct world Y before taking
                // the delta.
                //
                // Preserve the Original's insertion semantics too:
                // it advances only while `new_distance > old_distance`,
                // so a tie is inserted before existing entries.
                let snap_ref = &snaps;
                let patrol_distance = |member: EntityId| {
                    let snap = snap_ref.get(&member).unwrap_or_else(|| {
                        panic!(
                            "patrol member {} is missing its owner-boundary snapshot",
                            member.index()
                        )
                    });
                    let dx = snap.position.x - chief_pos.x;
                    let dy_world =
                        (snap.position.y + snap.ground_z) - (chief_pos.y + chief_snap.ground_z);
                    let dy = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = snap.ground_z - chief_snap.ground_z;
                    dx * dx + dy * dy + dz * dz
                };
                let mut sorted_patrol = Vec::with_capacity(ai.patrol.len());
                for member in std::mem::take(&mut ai.patrol) {
                    let distance = patrol_distance(member);
                    let insert_at = sorted_patrol
                        .iter()
                        .position(|&existing| distance <= patrol_distance(existing))
                        .unwrap_or(sorted_patrol.len());
                    sorted_patrol.insert(insert_at, member);
                }
                ai.patrol = sorted_patrol;

                // Arrange left/right pairs: for each pair, ensure
                // even-index member is to the left of the odd-index
                // one (relative to chief).  Uses a 2D determinant.
                let patrol_size = ai.patrol.len();
                for i in (1..patrol_size).step_by(2) {
                    let even_h = ai.patrol[i - 1];
                    let odd_h = ai.patrol[i];
                    if let (Some(even_s), Some(odd_s)) =
                        (snap_ref.get(&even_h), snap_ref.get(&odd_h))
                    {
                        let ex = even_s.position.x - chief_pos.x;
                        let ey = even_s.position.y - chief_pos.y;
                        let ox = odd_s.position.x - chief_pos.x;
                        let oy = odd_s.position.y - chief_pos.y;
                        // 2D determinant: if even is on the wrong side, swap
                        if ex * oy - ey * ox < 0.0 {
                            ai.patrol.swap(i - 1, i);
                        }
                    }
                }
            }

            // ── Refresh patrol positions ──
            let patrol_size = ai.patrol.len();
            if patrol_size == 0 && ai.missed_patrol_members.is_empty() {
                continue;
            }
            if ai.patrol_stopped {
                continue;
            }
            if ai.current_state != AiState::Default {
                continue;
            }
            if ai.current_substate == Substate::DefaultPatrolChiefReturnToPatrol {
                continue;
            }

            // Must have a patrol path to track history
            let Some(ref mut path) = ai.patrol_path else {
                continue;
            };

            // Record history entry every frame
            if let Some(snap) = snaps.get(&npc_id) {
                path.add_history_entry(snap.position, snap.direction as u8);
            }

            // Every 8th frame: compute positions and coordinate minions
            if (frame & 7) != 0 {
                continue;
            }

            {
                // The Original calls ComputePatrolPositions even with zero
                // active members.  Its post-loop cleanup then discards every
                // history entry except the newest one before missed members
                // are considered for re-acquisition below.
                // Expand the chief's move box by 3 on each side
                // before feeding it to
                // `is_straight_movement_autorized` for the 3-step
                // side-offset fallback.
                let chief_box = match snaps.get(&npc_id).map(|s| s.move_box) {
                    Some(b) if b.is_somewhere() => crate::coordinates::MoveBox::from_coords(
                        b.x_min() - 3.0,
                        b.y_min() - 3.0,
                        b.x_max() + 3.0,
                        b.y_max() + 3.0,
                    ),
                    _ => crate::coordinates::MoveBox::new(),
                };
                let positions = path.compute_patrol_positions(
                    patrol_size,
                    Some(&self.world.fast_grid),
                    &chief_box,
                );
                let patrol_members = ai.patrol.clone();

                for (i, &member) in patrol_members.iter().enumerate() {
                    if let Some(&(ref pos, dir)) = positions.get(i) {
                        // Only coordinate if member is far enough from target (MaxNorm > 3)
                        if let Some(member_snap) = snaps.get(&member) {
                            let dx = (member_snap.position.x - pos.x).abs();
                            let dy = (member_snap.position.y - pos.y).abs();
                            if dx.max(dy) > 3.0 {
                                patrol_cmds.push(PatrolCmd {
                                    minion: member,
                                    target: *pos,
                                    direction: dir,
                                });
                            }
                        }
                    }
                }
            }

            // Check missed patrol members for re-acquisition.
            // `is_detecting_360_degrees`: isometric squared distance
            // check (Y stretched by INVERSE_ASPECT_RATIO) plus the
            // `FastFindGrid::is_reachable(OPAQUE)` LOS gate — a
            // separated minion behind a wall must NOT re-join even
            // within view radius.
            let chief_snap = snaps.get(&npc_id).copied();
            let missed = ai.missed_patrol_members.clone();
            let mut reacquired = Vec::new();
            let obstacles_owned = scratch.ai_sight_obstacles.clone();
            let obstacles = obstacles_owned.list();
            for (i, &member) in missed.iter().enumerate() {
                if let (Some(chief_s), Some(member_s)) = (chief_snap, snaps.get(&member))
                    && missed_patrol_member_reacquired(
                        chief_s.is_active && member_s.is_active,
                        || {
                            patrol_member_visible_from_raw_world(
                                chief_s.detection_position_world,
                                chief_s.is_rider,
                                chief_s.real_view_radius,
                                chief_s.in_building,
                                member_s.detection_position_world,
                                member_s.posture,
                                member_s.is_rider,
                                member_s.direction as i16,
                                member_s.in_building,
                                obstacles,
                            )
                        },
                        member_s.is_able_to_help,
                        member_s.ai_state,
                    )
                {
                    reacquired.push(i);
                    ai.patrol.push(member);
                    chief_assigns.push((member, npc_id));
                }
            }
            for &i in reacquired.iter().rev() {
                ai.missed_patrol_members.remove(i);
            }
        }

        // ── Phase 4: Set patrol_chief on minions ──
        for (minion, chief) in chief_assigns {
            if let Some(entity) = self.world.entities.get_mut(minion)
                && let Some(ai) = entity.ai_controller_mut()
            {
                ai.patrol_chief = Some(chief);
            }
        }

        // ── Phase 5: Build per-minion patrol tick data map ──
        // Build a map of minion → (chief_position, chief_state) for use
        // in the coordinate dispatch below.
        let mut patrol_tick_map: std::collections::HashMap<
            EntityId,
            (crate::ai::Position, crate::ai::AiState),
        > = std::collections::HashMap::new();
        for cmd in &patrol_cmds {
            let minion_id = cmd.minion;
            let Some(entity) = self.world.entities.get(minion_id) else {
                continue;
            };
            let Some(ai) = entity.ai_controller() else {
                continue;
            };
            if let Some(chief_id) = ai.patrol_chief
                && let Some(cs) = snaps.get(&chief_id)
            {
                patrol_tick_map.insert(minion_id, (cs.position, cs.ai_state));
            }
        }

        // ── Phase 6: Dispatch CALL_PATROL_COORDINATE to minions ──
        let patrol_frame = self.control.frame_counter;
        for cmd in patrol_cmds {
            let minion_id = cmd.minion;
            let ctx = {
                let Some(entity) = self.world.entities.get_mut(minion_id) else {
                    continue;
                };
                let ctx = build_ai_context_from_entity(
                    entity,
                    patrol_frame,
                    None,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                );

                ctx
            };

            // Build tick data with patrol chief info.  Use the
            // centralized builder so combat-path fields stay
            // populated — patrol minions can be alerted mid-patrol
            // and dispatched into battle decisions without losing
            // their primary target snapshot.
            let mut tick_data = self.build_npc_tick_data(sim, minion_id, &scratch, assets);
            if let Some(&(chief_pos, chief_state)) = patrol_tick_map.get(&cmd.minion) {
                tick_data.patrol_chief_position = chief_pos;
                tick_data.patrol_chief_state = chief_state;
            }

            // Dispatch CALL_PATROL_COORDINATE through the script filter.
            let stimulus = Stimulus::with_position(StimulusType::CallPatrolCoordinate, cmd.target);
            self.debug_patrol_turn_lifecycle("before_coordinate_think", minion_id);
            self.dispatch_think_with_drain_mode(
                sim, minion_id, &stimulus, &ctx, &tick_data, assets, true, true,
            );
            // `CoordinatePatrol` constructs its Move element inline in the
            // original, making `GetCommand()` report MOVE_OK immediately.
            // Owner instruction still belongs to the sequence-manager phase
            // later this hourglass, so promote the request to an element but
            // deliberately leave its deferred InstructOwner action queued.
            self.drain_pending_move_requests_for_owner(sim, minion_id);
            self.debug_patrol_turn_lifecycle("after_coordinate_think", minion_id);

            // Original applies the instructed direction only after the
            // member's synchronous CALL_PATROL_COORDINATE Think returns.
            let in_uninterruptible_command = self.is_very_very_busy(minion_id);
            let building_sector = self
                .world
                .entities
                .get(minion_id)
                .and_then(|entity| self.entity_building_sector(entity.element_data().sector()));
            let entity = self.world.entities.get_mut(minion_id).unwrap_or_else(|| {
                panic!(
                    "patrol chief {} lost member {} after coordinate Think",
                    owner.index(),
                    minion_id.index()
                )
            });
            let mut live_ctx = build_ai_context_from_entity(
                entity,
                patrol_frame,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            live_ctx.in_uninterruptible_command = in_uninterruptible_command;
            entity
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "patrol member {} lost AI after coordinate Think from chief {}",
                        minion_id.index(),
                        owner.index()
                    )
                })
                .set_instructed_patrol_direction(cmd.direction, &live_ctx);
            self.debug_patrol_turn_lifecycle("after_instructed_direction_emit", minion_id);
            // GetInstructedPatrolDirection may synchronously FaceTo when the
            // member is still waiting. Close its AI/callback work before the
            // chief advances, but leave owner instruction to the later
            // SequenceManager hourglass just like the Original.
            self.drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
                sim, minion_id, assets,
            );
            self.debug_patrol_turn_lifecycle("after_instructed_direction_drain", minion_id);
        }
    }

    /// Execute the complete Original `ClearPatrol` call made by the
    /// `RemoveAllSubordinates` script native.
    ///
    /// `RHArtificialIntelligence::ClearPatrol` clears each member's chief
    /// pointer and calls `ForceReturnToDuty` directly before clearing the
    /// chief's lists. `ForceReturnToDuty` is an inline virtual
    /// `ReturnToDuty()` call, not `Think(EVENT_RETURN_TO_DUTY)`: in
    /// particular, it bypasses `StartThink`'s script-lock refusal. Keep the
    /// direct duty transition, movement construction, and recursive callbacks
    /// inside this engine-owned script barrier while leaving ordinary owner
    /// instruction to the subsequent `RHSequenceManager::Hourglass`.
    pub(crate) fn script_remove_all_subordinates(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        chief: EntityId,
    ) {
        let members = self
            .world
            .entities
            .get(chief)
            .and_then(Entity::ai_controller)
            .unwrap_or_else(|| {
                panic!(
                    "RemoveAllSubordinates chief {} is not an NPC",
                    chief.index()
                )
            })
            .theoretical_patrol
            .clone();

        for member in members.iter().copied() {
            let should_return = {
                let ai = self
                    .world
                    .entities
                    .get_mut(member)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "RemoveAllSubordinates chief {} references missing NPC member {}",
                            chief.index(),
                            member.index()
                        )
                    });
                ai.patrol_chief = None;
                ai.current_state == crate::ai::AiState::Default
            };
            if !should_return {
                continue;
            }
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let mut ctx = {
                let entity = self.world.entities.get(member).unwrap_or_else(|| {
                    panic!(
                        "RemoveAllSubordinates member {} vanished before ForceReturnToDuty",
                        member.index()
                    )
                });
                let building_sector = self.entity_building_sector(entity.element_data().sector());
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
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(member, &mut ctx);
            let tick_data = self.build_npc_tick_data(sim, member, &scratch, assets);
            {
                let entity = self.world.entities.get_mut(member).unwrap_or_else(|| {
                    panic!(
                        "RemoveAllSubordinates member {} vanished before ForceReturnToDuty",
                        member.index()
                    )
                });
                let npc = entity.npc_data_mut().unwrap_or_else(|| {
                    panic!(
                        "RemoveAllSubordinates member {} has no NPC data",
                        member.index()
                    )
                });
                match &mut npc.ai_brain {
                    crate::element::AiBrain::Enemy(ai) => {
                        ai.return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx, &tick_data)
                    }
                    crate::element::AiBrain::Friendly(ai) => {
                        ai.return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx)
                    }
                    crate::element::AiBrain::None => panic!(
                        "RemoveAllSubordinates member {} has no AI brain",
                        member.index()
                    ),
                }
            }
            self.drain_direct_ai_owner_boundary_without_forecast(sim, member, assets);
            self.drain_pending_move_requests_for_owner(sim, member);
        }

        self.world
            .entities
            .get_mut(chief)
            .and_then(Entity::ai_controller_mut)
            .expect("validated RemoveAllSubordinates chief vanished")
            .clear_patrol();
    }

    // ─── One-shot noise broadcast ──────────────────────────────────

    pub(crate) fn one_shot_noise_listener_ids(&self) -> Vec<EntityId> {
        let mut npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        // `RHEngine::GetNPC(i)` follows the Original registration array.
        // Rust's typed arena order is not authoritative after save adoption,
        // where static entities may be reused under restored creation ranks.
        npc_ids.sort_by_key(|&npc_id| self.world.original_creation_order(npc_id));
        npc_ids
    }

    pub(crate) fn one_shot_noise(
        &self,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: u16,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) -> crate::ai::Noise {
        use crate::ai::{Noise, NoiseType, Position};

        let element_id = match noise_type {
            NoiseType::TapTapTap | NoiseType::ZingZing | NoiseType::Aaargh | NoiseType::Heeelp => {
                source_entity.map(|id| id.index() as u16).unwrap_or(0)
            }
            _ => 0,
        };

        // RHnoise keeps the complete RHposition supplied by the source,
        // including its motion-sector pointer. Delayed reactions later feed
        // that position through PositionToPoint3D, so dropping the sector
        // also drops authored elevation. Only inherit it when the supplied
        // source still describes this exact noise origin.
        let origin_sector = source_entity
            .and_then(|id| self.world.entities.get(id))
            .filter(|entity| {
                entity.element_data().position_map() == origin
                    && entity.element_data().layer() == origin_layer
            })
            .and_then(|entity| entity.element_data().sector());

        Noise {
            origin: Position {
                x: origin.x,
                y: origin.y,
                sector: origin_sector,
                level: origin_layer,
            },
            noise_type,
            volume,
            elevation,
            element_id,
        }
    }

    /// Compute one listener's live subjective copy of a one-shot noise.
    ///
    /// This deliberately mutates deafness at the listener slot. Original
    /// `Noise` calls `GetHearVolume` immediately before that listener's
    /// synchronous `Think`, so earlier listeners may alter world state before
    /// this method is called for the next registration-array entry.
    pub(crate) fn subjective_one_shot_noise_for(
        &mut self,
        npc_id: EntityId,
        noise: crate::ai::Noise,
    ) -> Option<crate::ai::Noise> {
        const HEARING_FACTOR: f32 = 1.0;

        let (npc_pos, npc_elev) = {
            let entity = self.world.entities.get(npc_id)?;
            let include = match entity {
                Entity::Civilian(_) => true,
                Entity::Soldier(s) => {
                    s.soldier.cached_camp == crate::element_kinds::Camp::Lacklandists
                }
                _ => false,
            };
            if !include {
                return None;
            }

            // Do not pre-filter inactive or unconscious NPCs. Original runs
            // GetHearVolume for every registered civilian/Lacklandist and
            // leaves refusal to StartThink, after the deafness read.
            let elem = entity.element_data();
            (elem.position_map(), elem.position().z)
        };

        let source_elev = noise.elevation as f32;
        let modified_volume = noise.volume as f32 * HEARING_FACTOR;
        let dx = npc_pos.x - noise.origin.x;
        let dy_world = npc_pos.y + npc_elev - noise.origin.y - source_elev;
        let dz = npc_elev - source_elev;

        // Original compares the full 3D points before range and deafness
        // work. A wounded or trapped source therefore cannot hear its own
        // AAARGH/HEEELP broadcast.
        if dx == 0.0 && dy_world == 0.0 && dz == 0.0 {
            return None;
        }

        let dy_stretched = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
        if dx.abs().max(dy_stretched.abs()).max(dz.abs()) > modified_volume {
            return None;
        }

        let distance = (dx * dx + dy_stretched * dy_stretched + dz * dz).sqrt();
        // GetHearVolume returns before GetDeafness when the Euclidean
        // remainder is non-positive, even if the earlier max-norm range test
        // admitted the source.
        if modified_volume - distance <= 0.0 {
            return None;
        }

        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(npc_pos.x, npc_pos.y, npc_elev);
        let frame = self.control.frame_counter;
        let deafness = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::npc_data_mut)
            .unwrap_or_else(|| {
                panic!(
                    "one-shot noise listener {} lost its required NPC state",
                    npc_id.index()
                )
            })
            .get_deafness(frame, cover_volume);

        let subjective = subjective_hear_volume(modified_volume, distance, deafness);
        (subjective != 0).then_some(crate::ai::Noise {
            volume: subjective,
            ..noise
        })
    }

    fn display_one_shot_noise(&mut self, noise: crate::ai::Noise) {
        // Original AddNoiseToDisplay runs only after every listener's Think.
        self.feedback
            .pending_side_effects
            .displayed_noises
            .push(noise);
    }

    /// Broadcast a one-shot noise event to all nearby NPCs.
    ///
    /// Called by projectile impacts, trap activations, scripted bridges,
    /// etc.  Filters to `is_civilian() || camp == Lacklandists` —
    /// royalist soldiers (player-controlled) do not receive broadcast
    /// noise stimuli.  Per-NPC subjective volume follows the
    /// `get_hear_volume` formula (volume×hearing_factor − iso-stretched
    /// distance − deafness).
    pub fn broadcast_noise(
        &mut self,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: u16,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) {
        use crate::ai::{Stimulus, StimulusType};

        let noise = self.one_shot_noise(
            noise_type,
            origin,
            origin_layer,
            volume,
            elevation,
            source_entity,
        );
        for npc_id in self.one_shot_noise_listener_ids() {
            let Some(subjective_noise) = self.subjective_one_shot_noise_for(npc_id, noise) else {
                continue;
            };
            let stimulus = Stimulus::with_noise(StimulusType::EventHear, subjective_noise);
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "one-shot noise listener {} lost its required AI controller",
                        npc_id.index()
                    )
                })
                .outbox
                .detection
                .stimuli
                .push(stimulus);
        }
        self.display_one_shot_noise(noise);
    }

    /// Broadcast a one-shot noise and synchronously run each listener's new
    /// `EVENT_HEAR`, in Original NPC registration order.
    ///
    /// Original `RHElementActorNPC::Noise` calls `Think` inside the broadcast
    /// loop. Script natives and other in-frame callbacks therefore observe
    /// the listeners' RNG draws, state transitions, and launched sequences
    /// before returning. The ordinary queued form remains available to owner
    /// phases that already provide their own synchronous drain boundary.
    pub(super) fn broadcast_noise_synchronously(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: u16,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) {
        use crate::ai::{Stimulus, StimulusType};

        let noise = self.one_shot_noise(
            noise_type,
            origin,
            origin_layer,
            volume,
            elevation,
            source_entity,
        );

        for npc_id in self.one_shot_noise_listener_ids() {
            let Some(subjective_noise) = self.subjective_one_shot_noise_for(npc_id, noise) else {
                continue;
            };
            let stimulus = Stimulus::with_noise(StimulusType::EventHear, subjective_noise);

            // Listener N sees every mutation produced by listener N-1. Only
            // the new EVENT_HEAR is dispatched: Original calls Think directly
            // and does not consume unrelated deferred stimuli here.
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let ctx = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!(
                        "one-shot noise listener {} disappeared before synchronous Think",
                        npc_id.index()
                    )
                });
                let building_sector = self.entity_building_sector(entity.element_data().sector());
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
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
            self.dispatch_think_with_drain_mode(
                sim, npc_id, &stimulus, &ctx, &tick_data, assets, false, false,
            );
        }
        self.display_one_shot_noise(noise);
    }

    // ── Cross-NPC action processing (phalanx coordination) ──────────
    //
    // After all AI think() calls, drain each NPC's pending cross-NPC
    // actions and apply them to the target NPCs. This covers:
    // - InstructGatherPosition + CALL_INSTRUCTION delivery
    // - BreakPhalanx propagation
    // - SendStimulus (e.g. CALL_COORDINATE to archers)
    // - SetLeft/RightCombatNeighbour for phalanx linking

    fn apply_update_left_combat_neighbour(&mut self, target: u32, old_left: u32, new_left: u32) {
        if old_left != 0
            && let Some(Entity::Soldier(s)) = self
                .world
                .entities
                .get_mut(EntityId::Soldier(SoldierId(old_left)))
            && let Some(ai) = s.npc.ai_brain.enemy_mut()
        {
            ai.right_combat_neighbour = 0;
        }
        if let Some(Entity::Soldier(s)) = self
            .world
            .entities
            .get_mut(EntityId::Soldier(SoldierId(target)))
            && let Some(ai) = s.npc.ai_brain.enemy_mut()
        {
            ai.left_combat_neighbour = new_left;
        }
        if new_left != 0 {
            let new_lefts_old_right = self
                .world
                .entities
                .get(EntityId::Soldier(SoldierId(new_left)))
                .and_then(|e| match e {
                    Entity::Soldier(s) => s.npc.ai_brain.enemy(),
                    _ => None,
                })
                .map(|ai| ai.right_combat_neighbour)
                .unwrap_or(0);
            if new_lefts_old_right != 0
                && let Some(Entity::Soldier(s)) = self
                    .world
                    .entities
                    .get_mut(EntityId::Soldier(SoldierId(new_lefts_old_right)))
                && let Some(ai) = s.npc.ai_brain.enemy_mut()
            {
                ai.left_combat_neighbour = 0;
            }
            if let Some(Entity::Soldier(s)) = self
                .world
                .entities
                .get_mut(EntityId::Soldier(SoldierId(new_left)))
                && let Some(ai) = s.npc.ai_brain.enemy_mut()
            {
                ai.right_combat_neighbour = target;
            }
        }
    }

    fn apply_update_right_combat_neighbour(&mut self, target: u32, old_right: u32, new_right: u32) {
        if old_right != 0
            && let Some(Entity::Soldier(s)) = self
                .world
                .entities
                .get_mut(EntityId::Soldier(SoldierId(old_right)))
            && let Some(ai) = s.npc.ai_brain.enemy_mut()
        {
            ai.left_combat_neighbour = 0;
        }
        if let Some(Entity::Soldier(s)) = self
            .world
            .entities
            .get_mut(EntityId::Soldier(SoldierId(target)))
            && let Some(ai) = s.npc.ai_brain.enemy_mut()
        {
            ai.right_combat_neighbour = new_right;
        }
        if new_right != 0 {
            let new_rights_old_left = self
                .world
                .entities
                .get(EntityId::Soldier(SoldierId(new_right)))
                .and_then(|e| match e {
                    Entity::Soldier(s) => s.npc.ai_brain.enemy(),
                    _ => None,
                })
                .map(|ai| ai.left_combat_neighbour)
                .unwrap_or(0);
            if new_rights_old_left != 0
                && let Some(Entity::Soldier(s)) = self
                    .world
                    .entities
                    .get_mut(EntityId::Soldier(SoldierId(new_rights_old_left)))
                && let Some(ai) = s.npc.ai_brain.enemy_mut()
            {
                ai.right_combat_neighbour = 0;
            }
            if let Some(Entity::Soldier(s)) = self
                .world
                .entities
                .get_mut(EntityId::Soldier(SoldierId(new_right)))
                && let Some(ai) = s.npc.ai_brain.enemy_mut()
            {
                ai.left_combat_neighbour = target;
            }
        }
    }

    pub(super) fn process_pending_cross_npc_actions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // Close any direct Think calls left by a global owner-work/self-
        // stimulus fixed point before collecting genuinely deferred actions.
        // Iterate live owner slots in their stable order (PA-013).
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.process_synchronous_reentrant_actions_for(sim, npc_id, assets);
        }
        // Collect all pending actions first to avoid borrow issues.
        // Both enemy (soldier) and friendly (civilian) AIs can push
        // cross-NPC actions — e.g. civilians send `CALL_ALERT` /
        // `CALL_REPORT` to soldiers via `AiController` on their base.
        let mut all_actions: Vec<crate::ai::CrossNpcAction> = Vec::new();
        for (_, entity) in self.world.entities.npcs_mut() {
            if let Some(ai) = entity.ai_controller_mut() {
                all_actions.extend(ai.take_pending_cross_npc_actions());
            }
        }

        if all_actions.is_empty() {
            // No cross-NPC actions to process, but still deliver any
            // self-stimuli queued last tick (EventDone from
            // `SendCondolationCard`, MYTALK callbacks, etc.).  This
            // drain used to live at the tail of this function, which
            // meant it was skipped entirely on ticks with no cross-NPC
            // actions — the common case — stranding queued stimuli
            // forever and hanging states like
            // `DefaultOnPostLookingSidewards` that wait on `EventDone`
            // to exit.
            self.drain_pending_self_stimuli(sim, assets);
            return;
        }

        // Building the full AI view resolves prepared building-exit forecasts
        // and therefore consumes authoritative RNG.  The original only builds
        // target context while actually delivering a cross-NPC action, so do
        // not speculate before the empty fast path above.
        let scratch = self.build_sim_scratch(sim, assets);
        let frame = self.control.frame_counter;

        for action in all_actions {
            match action {
                crate::ai::CrossNpcAction::RequestAlert { caller, target, .. } => {
                    panic!(
                        "result-bearing CALL_ALERT {caller}->{target} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::RequestThinkResult { caller, target, .. } => {
                    panic!(
                        "result-bearing Think request {caller}->{target} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::RequestPatrolDispatch { caller, chief, .. } => {
                    panic!(
                        "result-bearing patrol dispatch {caller}->{chief} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::FinalizeAlertSoldiers { caller, .. } => {
                    panic!(
                        "AlertSoldiers finalization for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::ResumeTowerGuardBattleDecisions { caller } => {
                    panic!(
                        "tower-guard battle continuation for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::ResumeAfterLookThere { caller, .. } => {
                    panic!(
                        "HeyFolksLookThere resume for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::BroadcastLookThere { caller, .. } => {
                    panic!(
                        "HeyFolksLookThere broadcast for caller {caller} escaped its owner boundary"
                    )
                }
                crate::ai::CrossNpcAction::RelayStimulusToPatrolMembers {
                    stimulus_type,
                    members,
                    ..
                } => {
                    panic!(
                        "whole-patrol {stimulus_type:?} broadcast to {} members escaped its owner boundary",
                        members.len()
                    )
                }
                crate::ai::CrossNpcAction::InstructGatherPosition {
                    target,
                    position,
                    direction,
                    call_instruction,
                    ..
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    tracing::trace!(
                        target: "robin_engine::ai_enemy::phalanx",
                        instructed = target,
                        frame = self.control.frame_counter,
                        ?position,
                        direction,
                        call_instruction,
                        "InstructGatherPosition"
                    );
                    if call_instruction && !self.soldier_stands_in_phalanx(target_id) {
                        continue;
                    }
                    let ctx = {
                        let Some(entity @ Entity::Soldier(_)) =
                            self.world.entities.get_mut(target_id)
                        else {
                            continue;
                        };
                        let ctx = build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
                            self.world.weather.is_forest_level,
                            self.world.weather.ambiance,
                            self.ai.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.world.fast_grid,
                            &assets.hiking_paths,
                            &self.ai.global.all_soldier_handles,
                            self.control.sim_config.difficulty,
                        );
                        let Entity::Soldier(s) = entity else {
                            unreachable!()
                        };
                        if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                            enemy_ai.gather_position = position;
                            enemy_ai.gather_direction = direction;
                            enemy_ai.gather_position_instructed = true;
                        }
                        ctx
                    };
                    if !call_instruction {
                        continue;
                    }
                    // CrossNpcAction::InstructGatherPosition: target
                    // is an enemy soldier.  Build rich tick data so a
                    // subsequent think()-triggered BattleDecisions
                    // sees the target snapshot.
                    let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
                    let stimulus = crate::ai::Stimulus::new(StimulusType::CallInstruction);
                    self.dispatch_filtered_stimulus(
                        sim, assets, target_id, &stimulus, &ctx, &tick_data,
                    );
                }

                crate::ai::CrossNpcAction::BreakPhalanx {
                    target,
                    refresh_them_list,
                } => {
                    self.process_synchronous_break_phalanx(sim, target, refresh_them_list, assets);
                }

                crate::ai::CrossNpcAction::SendStimulus {
                    target,
                    stimulus_type,
                    info,
                    fallback_to_sender,
                    to_whole_patrol,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
                    stimulus.info = info;
                    stimulus.to_whole_patrol = to_whole_patrol;

                    let ctx = {
                        let Some(entity @ Entity::Soldier(_)) = self.world.entities.get(target_id)
                        else {
                            // Target missing → try fallback directly below.
                            if let Some(sender) = fallback_to_sender {
                                let sender_id = EntityId::Soldier(SoldierId(sender));
                                if let Some(entity @ Entity::Soldier(_)) =
                                    self.world.entities.get(sender_id)
                                {
                                    let ctx = build_ai_context_from_entity(
                                        entity,
                                        frame,
                                        None,
                                        self.world.weather.is_forest_level,
                                        self.world.weather.ambiance,
                                        self.ai.standard_view_polygon_radius,
                                        &scratch.ai_entity_views,
                                        &scratch.ai_sight_obstacles,
                                        &self.world.fast_grid,
                                        &assets.hiking_paths,
                                        &self.ai.global.all_soldier_handles,
                                        self.control.sim_config.difficulty,
                                    );
                                    let fallback_tick =
                                        self.build_npc_tick_data(sim, sender_id, &scratch, assets);
                                    self.dispatch_filtered_stimulus(
                                        sim,
                                        assets,
                                        sender_id,
                                        &stimulus,
                                        &ctx,
                                        &fallback_tick,
                                    );
                                }
                            }
                            continue;
                        };
                        build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
                            self.world.weather.is_forest_level,
                            self.world.weather.ambiance,
                            self.ai.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.world.fast_grid,
                            &assets.hiking_paths,
                            &self.ai.global.all_soldier_handles,
                            self.control.sim_config.difficulty,
                        )
                    };
                    // SendStimulus → enemy soldier target: the
                    // stimulus may be EVENT_VIEW / EVENT_REPORT /
                    // alert-forwarding which feeds BattleDecisions.
                    let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
                    let handled = self.dispatch_filtered_stimulus(
                        sim, assets, target_id, &stimulus, &ctx, &tick_data,
                    );
                    // Fallback: if target couldn't handle the stimulus,
                    // redeliver to the sender (e.g. conversation chains).
                    if !handled && let Some(sender) = fallback_to_sender {
                        let sender_id = EntityId::Soldier(SoldierId(sender));
                        let ctx2 = {
                            let Some(entity @ Entity::Soldier(_)) =
                                self.world.entities.get(sender_id)
                            else {
                                continue;
                            };
                            build_ai_context_from_entity(
                                entity,
                                frame,
                                None,
                                self.world.weather.is_forest_level,
                                self.world.weather.ambiance,
                                self.ai.standard_view_polygon_radius,
                                &scratch.ai_entity_views,
                                &scratch.ai_sight_obstacles,
                                &self.world.fast_grid,
                                &assets.hiking_paths,
                                &self.ai.global.all_soldier_handles,
                                self.control.sim_config.difficulty,
                            )
                        };
                        let fallback_tick =
                            self.build_npc_tick_data(sim, sender_id, &scratch, assets);
                        self.dispatch_filtered_stimulus(
                            sim,
                            assets,
                            sender_id,
                            &stimulus,
                            &ctx2,
                            &fallback_tick,
                        );
                    }
                }

                crate::ai::CrossNpcAction::SetLeftCombatNeighbour { target, neighbour } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.left_combat_neighbour = neighbour;
                    }
                }

                crate::ai::CrossNpcAction::SetRightCombatNeighbour { target, neighbour } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.right_combat_neighbour = neighbour;
                    }
                }

                crate::ai::CrossNpcAction::SetArcherBehindMe { target, archer } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.archer_behind_me = archer;
                    }
                }

                crate::ai::CrossNpcAction::SetShieldBearerBeforeMe {
                    target,
                    shield_bearer,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.shield_bearer_before_me = shield_bearer;
                    }
                }

                // Full reciprocal update.  Four steps:
                //   1. clear old_left's right pointer
                //   2. store new_left on target's left pointer (caller
                //      may also have written it eagerly for immediate
                //      visibility)
                //   3. pre-clean new_left's existing right (recursive
                //      `update_right_combat_neighbour(NULL)`) — clear
                //      that-right's left pointer
                //   4. wire new_left's right back to target
                crate::ai::CrossNpcAction::UpdateLeftCombatNeighbour {
                    target,
                    old_left,
                    new_left,
                } => self.apply_update_left_combat_neighbour(target, old_left, new_left),

                // Same shape as `update_left_combat_neighbour`, for
                // the right side.
                crate::ai::CrossNpcAction::UpdateRightCombatNeighbour {
                    target,
                    old_right,
                    new_right,
                } => self.apply_update_right_combat_neighbour(target, old_right, new_right),

                crate::ai::CrossNpcAction::SetPrimaryTarget {
                    target,
                    primary_target,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.base.primary_target = primary_target;
                    }
                }

                crate::ai::CrossNpcAction::SetPhalanxThemList {
                    target,
                    them,
                    primary_target,
                } => self.process_synchronous_set_phalanx_them_list(target, them, primary_target),

                crate::ai::CrossNpcAction::Say { target, remark } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Entity::Soldier(s) =
                        self.world.entities.get_mut(target_id).unwrap_or_else(|| {
                            panic!("cross-NPC speech target {target} is missing")
                        })
                    else {
                        panic!("cross-NPC speech target {target} is not a soldier")
                    };
                    s.npc
                        .ai_brain
                        .enemy_mut()
                        .unwrap_or_else(|| {
                            panic!("cross-NPC speech target {target} has no EnemyAi")
                        })
                        .base
                        .say(remark);
                    self.drain_ai_owner_work_for(sim, assets, target_id);
                }

                crate::ai::CrossNpcAction::SetLootedAfterMoneyFight { target, looted } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.base.looted_after_money_fight = looted;
                    }
                }

                // Legacy serialized pending work. Production no longer emits
                // this incomplete shape, but retaining the arm preserves old
                // save/checkpoint compatibility and later enum ordinals.
                crate::ai::CrossNpcAction::UpdateReport {
                    target,
                    report_type,
                    seek_position,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai
                            .base
                            .my_reconnaissance_report
                            .update(report_type, seek_position);
                    }
                }

                crate::ai::CrossNpcAction::ConsiderReport {
                    target,
                    report,
                    flags,
                } => {
                    let frame = self.control.frame_counter;
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        // Use the AiController-level helper: it merges
                        // the report AND queues the per-body
                        // `delete_detectable(body, DETECTABLE_BODY)`
                        // side effects.  The bare
                        // `ReconnaissanceReport::consider_report`
                        // skipped those side effects, leaving stale
                        // body detectables on the NPC after a peer
                        // report merge.
                        enemy_ai.base.consider_report_merged_at_frame(
                            &report,
                            flags,
                            scratch.ai_entity_views.as_ref(),
                            frame,
                        );
                    }
                }

                crate::ai::CrossNpcAction::RegisterSynchronizingActor { target, actor } => {
                    // `register_synchronizing_actor` pushes the
                    // calling NPC onto the target's
                    // `synchronizing_actors` so the target's
                    // macro-complete dispatch can wake all waiters.
                    // Dedup the push for safety since the original
                    // list pushes unconditionally.
                    let target_id = EntityId::Soldier(SoldierId(target));
                    if let Some(entity) = self.world.entities.get_mut(target_id)
                        && let Some(ai) = entity.ai_controller_mut()
                        && !ai.synchronizing_actors.contains(&actor)
                    {
                        ai.synchronizing_actors.push(actor);
                    }
                }
                crate::ai::CrossNpcAction::ReportBackToOfficer { .. } => {
                    panic!("synchronous officer report leaked into deferred cross-NPC actions")
                }
            }
        }

        self.drain_pending_self_stimuli(sim, assets);
    }

    /// Dispatch `stimulus` to `npc_id` via
    /// [`Self::dispatch_filtered_stimulus`], then run a synchronous
    /// side-effect drain pass so handler side effects (LaunchSequence,
    /// SetAttentiveMode, Face, quit/enter swordfight, look-sidewards,
    /// …) and any condolations / re-entrant `Think(EVENT_DONE)` they
    /// trigger happen inside the same call stack as the outer Think —
    /// matching the original `think()`, where handlers invoke
    /// `launch_sequence`, `halt`, `face`, `set_attentive_mode` inline
    /// and `send_condolation_card` fires `think(EVENT_DONE)`
    /// re-entrantly.
    ///
    /// The loop re-runs the drain while the NPC keeps generating new
    /// pending side effects (e.g. one condolation's `EventDone` handler
    /// queues another sequence that is preempted in the next iteration),
    /// bounded at 8 iterations to guard against a pathological cascade.
    ///
    /// Returns `dispatch_filtered_stimulus`'s handled bool — unchanged
    /// by the drain pass.
    pub(super) fn dispatch_think_with_drain(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        self.dispatch_think_with_drain_mode(
            sim, npc_id, stimulus, ctx, tick_data, assets, false, true,
        )
    }

    pub(super) fn dispatch_think_with_drain_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        self.dispatch_think_with_drain_mode(
            sim, npc_id, stimulus, ctx, tick_data, assets, true, false,
        )
    }

    /// Owner-local Think before the current frame's SequenceManager hourglass.
    /// Keep standalone Turns registered but uninstructed just like the
    /// detection FIFO that originally produced a retained stimulus.
    pub(super) fn dispatch_think_with_drain_without_forecast_deferred_turn(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        self.dispatch_think_with_drain_mode(
            sim, npc_id, stimulus, ctx, tick_data, assets, true, true,
        )
    }

    fn dispatch_think_with_drain_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) -> bool {
        // Original's ComputeViewRadius memo lives on the target surface, so a
        // synchronous IsDetecting inside Think must see a RefreshDetection
        // result produced earlier in this universal frame. Keep AiContext's
        // immutable-handler facade bounded exactly by the synchronous Think
        // call, then commit any newly computed surfaces before later callbacks.
        ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
        let had_ai_at_entry = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_some();
        let handled = self.dispatch_filtered_stimulus_with_owner_mode(
            sim,
            assets,
            npc_id,
            stimulus,
            ctx,
            Some(tick_data),
            owner_local_no_forecast,
            defer_turn_instruction,
        );
        ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);

        // `RHArtificialIntelligence::ExecuteWaypointScript` invokes the
        // waypoint VM directly from the active Think handler. Close that
        // authored callback before the generic post-Think effect drain:
        // script natives such as AssignPath recursively enter
        // EVENT_RETURN_TO_DUTY before later orders or condolations from the
        // outer handler can settle.
        self.dispatch_pending_waypoint_script_for_owner(sim, npc_id, assets);

        // A missing entity or NPC shell without AI is a legitimate unhandled
        // no-op. An existing AI may also return false after running a handler,
        // so only the entry-state no-AI case can skip the synchronous drain.
        if !handled && !had_ai_at_entry {
            return false;
        }

        // EventViewStandardProcedure explicitly marks an accepted VIEW after
        // all StartThink and handler guards. Mirror that one-shot onto the
        // engine-owned NPC record before draining its other synchronous
        // effects. Locked, frozen, script-filtered, and handler-rejected VIEWs
        // never set the flag.
        let mark_alerted = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "handled Think recipient {} lost its entity or AI controller before drain",
                    npc_id.index()
                )
            });
        let mark_alerted = std::mem::take(&mut mark_alerted.outbox.detection.mark_alerted);
        if mark_alerted {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "accepted EVENT_VIEW recipient {} disappeared after its synchronous Think",
                    npc_id.index()
                )
            });
            let npc = entity.npc_data_mut().unwrap_or_else(|| {
                panic!(
                    "accepted EVENT_VIEW recipient {} lost its NPC data after synchronous Think",
                    npc_id.index()
                )
            });
            npc.alerted = true;
        }

        const MAX_ITERS: u32 = 8;
        for iter in 0..MAX_ITERS {
            // Drain the per-NPC pending-flags pass (launches sequences,
            // commands, turn orders, attentive-mode transitions, etc.).
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
            // `drain_pending_for_npc` launches the first order barrier in its
            // original position. Close the boundary again because later
            // effect application and civilian handlers share the same base
            // order outbox. Owner-local SetState notifications are also part
            // of this fixed point, so late script-seek callbacks cannot leak
            // into a global batch or strand in the outbox.
            self.launch_pending_orders_for_npc_mode(sim, assets, npc_id, defer_turn_instruction);
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
            self.surface_synchronous_completion_events_for_owner(npc_id);

            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );

            // Any condolations the drain above queued (sequences that
            // got preempted by the side effects) fire here — which may
            // push EventDone / EventImpossible into pending_self_stimuli.
            self.dispatch_condolations_for_npc(sim, npc_id, assets);

            // Re-enter Think for each self-stimulus (EventDone, MYTALK,
            // etc.).  This may queue more pending flags — loop again.
            let has_self_stimuli = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} disappeared before self-stimulus recheck",
                        npc_id.index()
                    )
                });
                let ai = entity.ai_controller().unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} lost its AI controller before self-stimulus recheck",
                        npc_id.index()
                    )
                });
                !ai.outbox.reentrant.self_stimuli.is_empty()
            };
            if has_self_stimuli {
                if owner_local_no_forecast {
                    self.drain_self_stimuli_for_npc_without_forecast(sim, npc_id, assets);
                } else {
                    self.drain_self_stimuli_for_npc(sim, npc_id, assets);
                }
            }

            // A re-entrant self stimulus can itself call another NPC. Close
            // those direct C++ call boundaries before deciding this owner has
            // stabilised; otherwise the result-bearing request can escape to
            // the global cross-action batch.
            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );

            let still_pending = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} disappeared before fixed-point recheck",
                        npc_id.index()
                    )
                });
                let ai = entity.ai_controller().unwrap_or_else(|| {
                    panic!(
                        "handled Think recipient {} lost its AI controller before fixed-point recheck",
                        npc_id.index()
                    )
                });
                ai.outbox.actor.has_boundary_work()
                    || !ai.outbox.reentrant.self_stimuli.is_empty()
                    || !ai.outbox.reentrant.owner_work.is_empty()
                    || ai.has_pending_synchronous_cross_npc_actions()
            };
            if !still_pending {
                break;
            }
            assert!(
                iter + 1 < MAX_ITERS,
                "Think-drain NPC {} did not stabilise after {MAX_ITERS} passes",
                npc_id.index()
            );
        }

        handled
    }

    /// Deliver completion latches produced after the typed `EndThink` at the
    /// same logical boundary as the operation that produced them.
    ///
    /// Original `AppendMoveToSequence` performs path construction inline, so
    /// failures and already-at-destination results are visible to the
    /// enclosing `EndThink`. Rust may discover either result only after the
    /// controller borrow is released: path construction is engine-owned, and
    /// owner-work continuations such as patrol initialization resume outside
    /// the typed Think call. Surface all three `EndThink` latches before a
    /// sibling synchronous event can enter `StartThink` and clear them.
    fn surface_synchronous_completion_events_for_owner(&mut self, npc_id: EntityId) {
        let ai = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous move owner {} disappeared before path-result delivery",
                    npc_id.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| {
                panic!(
                    "synchronous move owner {} lost AI before path-result delivery",
                    npc_id.index()
                )
            });
        // Only an EndThink delivers these latches, so one whose operation ran
        // outside a Think is discarded exactly as the next Think entry would.
        // Dispatching a completion also re-enters Think, whose entry gate
        // clears all three latches before the nested handler runs, so a single
        // boundary surfaces at most one event even when several were set.
        let typed_tail_pending = ai.outbox.reentrant.reconsider_approach_completion_pending
            || ai.outbox.reentrant.look_for_help_completion_pending;
        let retain_couldnt_reachpoint =
            ai.completion_latch_inside_think && ai.couldnt_reachpoint && typed_tail_pending;
        let event = if !ai.completion_latch_inside_think {
            None
        } else if retain_couldnt_reachpoint {
            None
        } else if ai.couldnt_reachpoint {
            Some(crate::ai::StimulusType::EventCouldntReachPoint)
        } else if ai.already_on_point {
            Some(crate::ai::StimulusType::EventReachPoint)
        } else if ai.already_turned {
            Some(crate::ai::StimulusType::EventDone)
        } else {
            None
        };
        if !retain_couldnt_reachpoint {
            ai.couldnt_reachpoint = false;
        }
        ai.already_on_point = false;
        ai.already_turned = false;
        if let Some(event) = event {
            ai.outbox.reentrant.self_stimuli.push(event);
        }
    }

    pub(super) fn process_synchronous_reentrant_actions_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.process_synchronous_reentrant_actions_for_mode(sim, source_id, assets, false);
    }

    fn process_synchronous_reentrant_actions_for_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        loop {
            let actions = self
                .world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .map(crate::ai::AiController::take_pending_synchronous_cross_npc_actions)
                .unwrap_or_else(|| {
                    panic!(
                        "synchronous action source {} has no AI controller",
                        source_id.index()
                    )
                });
            if actions.is_empty() {
                break;
            }

            let deferred = {
                let ai = self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "synchronous action source {} lost its AI controller",
                            source_id.index()
                        )
                    });
                std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions)
            };
            let mut deferred = deferred;
            let mut alert_formation_targets = Vec::new();
            for action in actions {
                if let crate::ai::CrossNpcAction::RequestThinkResult {
                    target,
                    continuation:
                        crate::ai::ThinkResultContinuation::OfficerAlertedSoldier { .. }
                        | crate::ai::ThinkResultContinuation::OfficerCombatAlertedSoldier { .. },
                    ..
                } = &action
                {
                    alert_formation_targets.push(*target);
                }
                match action {
                    crate::ai::CrossNpcAction::InstructGatherPosition {
                        target,
                        position,
                        direction,
                        call_instruction,
                    } => {
                        // Alert formations queue their result requests before
                        // the sibling gather instructions. Remember those
                        // exact targets while draining this saved batch, then
                        // suppress an instruction if its direct Think result
                        // pruned it. Phalanx instructions have no preceding
                        // alert-result request and remain unconditional.
                        let alert_formation = alert_formation_targets.contains(&target);
                        let still_alerted = !alert_formation
                            || self
                                .world
                                .entities
                                .get(source_id)
                                .and_then(Entity::enemy_ai)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "alert InstructGatherPosition source {source_id:?} is not an enemy soldier"
                                    )
                                })
                                .alerted_us
                                .contains(&target);
                        if still_alerted {
                            self.process_synchronous_gather_instruction(
                                sim,
                                target,
                                position,
                                direction,
                                call_instruction,
                                assets,
                            );
                        }
                    }
                    crate::ai::CrossNpcAction::BreakPhalanx {
                        target,
                        refresh_them_list,
                    } => self.process_synchronous_break_phalanx(
                        sim,
                        target,
                        refresh_them_list,
                        assets,
                    ),
                    crate::ai::CrossNpcAction::ConsiderReport {
                        target,
                        report,
                        flags,
                    } => {
                        self.process_synchronous_consider_report(sim, target, report, flags, assets)
                    }
                    crate::ai::CrossNpcAction::FinalizeAlertSoldiers {
                        caller,
                        use_formation,
                        failure,
                    } => self.process_synchronous_finalize_alert_soldiers(
                        sim,
                        source_id,
                        caller,
                        use_formation,
                        failure,
                        assets,
                    ),
                    crate::ai::CrossNpcAction::ResumeTowerGuardBattleDecisions { caller } => self
                        .process_synchronous_tower_guard_battle_decisions(
                            sim, source_id, caller, assets,
                        ),
                    crate::ai::CrossNpcAction::SetPhalanxThemList {
                        target,
                        them,
                        primary_target,
                    } => {
                        self.process_synchronous_set_phalanx_them_list(target, them, primary_target)
                    }
                    crate::ai::CrossNpcAction::ResumeAfterLookThere {
                        caller,
                        continuation,
                    } => self.process_synchronous_look_there_resume(
                        sim,
                        source_id,
                        caller,
                        continuation,
                        assets,
                    ),
                    crate::ai::CrossNpcAction::BroadcastLookThere {
                        caller,
                        position,
                        radius,
                        continuation,
                    } => self.process_synchronous_look_there_broadcast(
                        sim,
                        source_id,
                        caller,
                        position,
                        radius,
                        continuation,
                        assets,
                        defer_turn_instruction,
                    ),
                    crate::ai::CrossNpcAction::UpdateLeftCombatNeighbour {
                        target,
                        old_left,
                        new_left,
                    } => self.apply_update_left_combat_neighbour(target, old_left, new_left),
                    crate::ai::CrossNpcAction::UpdateRightCombatNeighbour {
                        target,
                        old_right,
                        new_right,
                    } => self.apply_update_right_combat_neighbour(target, old_right, new_right),
                    crate::ai::CrossNpcAction::SetLeftCombatNeighbour { target, neighbour } => {
                        let target_id = EntityId::Soldier(SoldierId(target));
                        let enemy_ai = self
                            .world
                            .entities
                            .get_mut(target_id)
                            .unwrap_or_else(|| {
                                panic!(
                                    "synchronous left-neighbour setter target {target} is missing"
                                )
                            })
                            .enemy_ai_mut()
                            .unwrap_or_else(|| {
                                panic!(
                                    "synchronous left-neighbour setter target {target} has no EnemyAi"
                                )
                            });
                        enemy_ai.left_combat_neighbour = neighbour;
                    }
                    crate::ai::CrossNpcAction::SetRightCombatNeighbour { target, neighbour } => {
                        let target_id = EntityId::Soldier(SoldierId(target));
                        let enemy_ai = self
                            .world
                            .entities
                            .get_mut(target_id)
                            .unwrap_or_else(|| {
                                panic!(
                                    "synchronous right-neighbour setter target {target} is missing"
                                )
                            })
                            .enemy_ai_mut()
                            .unwrap_or_else(|| {
                                panic!(
                                    "synchronous right-neighbour setter target {target} has no EnemyAi"
                                )
                            });
                        enemy_ai.right_combat_neighbour = neighbour;
                    }
                    crate::ai::CrossNpcAction::SendStimulus { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_stimuli_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::RelayStimulusToPatrolMembers { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_patrol_member_relay_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::RequestPatrolDispatch { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_patrol_dispatch_requests_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::RequestAlert { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_alert_requests_for(sim, source_id, assets)
                    }
                    crate::ai::CrossNpcAction::RequestThinkResult { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_think_results_for(
                            sim,
                            source_id,
                            assets,
                            defer_turn_instruction,
                        )
                    }
                    crate::ai::CrossNpcAction::ReportBackToOfficer { .. } => {
                        self.requeue_isolated_synchronous_action(source_id, action.clone());
                        self.process_synchronous_officer_reports_for(sim, source_id, assets)
                    }
                    crate::ai::CrossNpcAction::Say { target, remark } => {
                        let target_id = EntityId::Soldier(SoldierId(target));
                        let Entity::Soldier(s) =
                            self.world.entities.get_mut(target_id).unwrap_or_else(|| {
                                panic!("cross-NPC speech target {target} is missing")
                            })
                        else {
                            panic!("cross-NPC speech target {target} is not a soldier")
                        };
                        s.npc
                            .ai_brain
                            .enemy_mut()
                            .unwrap_or_else(|| {
                                panic!("cross-NPC speech target {target} has no EnemyAi")
                            })
                            .base
                            .say(remark);
                        self.drain_ai_owner_work_for(sim, assets, target_id);
                    }
                    _ => unreachable!("ordered synchronous drain received deferred action"),
                }

                // Direct C++ calls are depth-first: if A emits C while B was
                // already queued, C closes before B. Isolate A's generated
                // work, recursively drain it, then continue the saved batch.
                self.process_synchronous_reentrant_actions_for_mode(
                    sim,
                    source_id,
                    assets,
                    defer_turn_instruction,
                );
                let ai = self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "synchronous action source {} lost its AI controller",
                            source_id.index()
                        )
                    });
                deferred.extend(std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions));
            }
            let ai = self
                .world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "synchronous action source {} lost its AI controller",
                        source_id.index()
                    )
                });
            ai.outbox.reentrant.cross_npc_actions = deferred;
        }
    }

    fn requeue_isolated_synchronous_action(
        &mut self,
        source_id: crate::element::EntityId,
        action: crate::ai::CrossNpcAction,
    ) {
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous action source {} lost its AI controller",
                    source_id.index()
                )
            })
            .outbox
            .reentrant
            .cross_npc_actions
            .push(action);
    }

    /// Install the completed phalanx them-list and its head target on one
    /// member. This has to stay ordered against the `BreakPhalanx` batch:
    /// the assignment happens while the rebuild recursion unwinds, before
    /// any member's `BattleDecisions` prunes entries that can no longer
    /// fight.
    fn process_synchronous_set_phalanx_them_list(
        &mut self,
        target: u32,
        them: Vec<crate::ai::HumanHandle>,
        primary_target: crate::ai::HumanHandle,
    ) {
        let target_id = EntityId::Soldier(SoldierId(target));
        let Some(Entity::Soldier(s)) = self.world.entities.get_mut(target_id) else {
            return;
        };
        if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
            tracing::trace!(
                target: "robin_engine::ai_enemy::phalanx",
                member = target,
                ?them,
                primary_target,
                "phalanx them-list: installing on member"
            );
            enemy_ai.list_them = them;
            enemy_ai.base.primary_target = primary_target;
        }
    }

    /// Execute one member of Original's recursive
    /// `RHArtificialMalignity::BreakPhalanx` call. Every neighbour clears its
    /// links and immediately runs `BattleDecisions`; delaying this to the
    /// global cross-NPC batch moves swordfight entry and panic RNG by a frame.
    fn process_synchronous_break_phalanx(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        target: u32,
        refresh_them_list: bool,
        assets: &LevelAssets,
    ) {
        let target_id = EntityId::Soldier(SoldierId(target));
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let Some(entity @ Entity::Soldier(_)) = self.world.entities.get(target_id) else {
            return;
        };
        let mut ctx = build_ai_context_from_entity(
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
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        self.refresh_selected_default_wait_identity(target_id, &mut ctx);
        // This is a direct recursive `BreakPhalanx` call rather than a typed
        // Think dispatch, but its `PhalanxReinitializeThemList` performs live
        // `IsDetecting180Degrees` checks. Those checks share the Original's
        // surface-owned `ComputeViewRadius` memo with later RefreshDetection
        // in the same frame, so bracket the direct AI call with the same
        // cache handoff as the Think wrapper.
        ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
        let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
        {
            let ai_global = &mut self.ai.global;
            let grid = &self.world.fast_grid;
            let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(target_id) else {
                return;
            };
            let Some(enemy_ai) = soldier.npc.ai_brain.enemy_mut() else {
                return;
            };
            enemy_ai.break_phalanx_from_neighbour(
                sim,
                ai_global,
                &ctx,
                &tick_data,
                Some(grid),
                refresh_them_list,
            );
        }
        ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);
        self.drain_direct_ai_owner_boundary(sim, target_id, assets);
    }

    /// Resume the statement immediately following Original
    /// `TowerGuardCallAlert`. The alert routine directly enters every
    /// recipient's Think before returning, so rebuilding the caller context
    /// here is necessary: `BattleDecisions` can synchronously enter SeekArea,
    /// whose nearby-friend multiplier reads the recipients' new alert status.
    fn process_synchronous_tower_guard_battle_decisions(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "tower-guard battle continuation caller must be its owner"
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("tower-guard caller {caller} disappeared"));
        let mut ctx = {
            let entity = self
                .world
                .entities
                .get(source_id)
                .unwrap_or_else(|| panic!("tower-guard caller {caller} disappeared"));
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        let tick = self.build_npc_tick_data(sim, source_id, &scratch, assets);
        let global = &mut self.ai.global;
        let grid = &self.world.fast_grid;
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("tower-guard caller {caller} lost its EnemyAi"))
            .battle_decisions(sim, global, &ctx, &tick, Some(grid));
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
    }

    fn process_synchronous_consider_report(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        target: u32,
        report: crate::ai::ReconnaissanceReport,
        flags: u16,
        assets: &LevelAssets,
    ) {
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let frame = self.control.frame_counter;
        let target_id = EntityId::Soldier(SoldierId(target));
        self.world
            .entities
            .get_mut(target_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("ConsiderReport target {target} is not an enemy soldier"))
            .base
            .consider_report_merged_at_frame(
                &report,
                flags,
                scratch.ai_entity_views.as_ref(),
                frame,
            );
        self.drain_direct_ai_owner_boundary_without_forecast(sim, target_id, assets);
    }

    fn process_synchronous_finalize_alert_soldiers(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        use_formation: bool,
        failure: crate::ai::AlertSoldiersFailureContinuation,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "AlertSoldiers finalization caller must be its owner"
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("AlertSoldiers caller {caller} disappeared"));
        let mut ctx = {
            let entity = self
                .world
                .entities
                .get(source_id)
                .unwrap_or_else(|| panic!("AlertSoldiers caller {caller} disappeared"));
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        let tick = self.build_npc_tick_data(sim, source_id, &scratch, assets);
        let global = &mut self.ai.global;
        let grid = use_formation.then_some(&*self.world.fast_grid);
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("AlertSoldiers caller {caller} lost its EnemyAi"))
            .finalize_alert_soldiers(sim, failure, global, grid, &ctx, &tick);
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
    }

    fn process_synchronous_look_there_broadcast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        position: crate::ai::Position,
        radius: u16,
        continuation: crate::ai::LookThereContinuation,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "HeyFolksLookThere broadcast caller must be its owner"
        );
        let (caller_camp, caller_position) = self
            .world
            .entities
            .get(source_id)
            .map(|entity| (entity.camp(), entity.element_data().position()))
            .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} disappeared"));
        let radius_sq = f32::from(radius) * f32::from(radius);
        let candidates = self.world.fighter_registry_order();

        for target_id in candidates {
            if target_id == source_id {
                continue;
            }
            let eligible = self
                .world
                .entities
                .get(target_id)
                .filter(|entity| {
                    matches!(entity, Entity::Soldier(_)) && entity.camp() == caller_camp
                })
                .and_then(Entity::enemy_ai)
                .is_some_and(|enemy| {
                    matches!(
                        enemy.base.current_state,
                        crate::ai::AiState::Default | crate::ai::AiState::Wondering
                    ) || (enemy.base.current_state == crate::ai::AiState::Seeking
                        && matches!(
                            enemy.base.current_substate,
                            crate::ai::Substate::SeekingJustWatching
                                | crate::ai::Substate::SeekingJustWatchingSidewards
                        ))
                });
            if !eligible {
                continue;
            }

            // Original evaluates the state and range together immediately
            // before the direct Think call. An earlier recipient may have
            // re-entered and changed this soldier since the loop began.
            let target_position = self
                .world
                .entities
                .get(target_id)
                .unwrap_or_else(|| {
                    panic!(
                        "HeyFolksLookThere target {} disappeared during the registry walk",
                        target_id.index()
                    )
                })
                .element_data()
                .position();
            let dx = target_position.x - caller_position.x;
            let dy = target_position.y - caller_position.y;
            let dz = target_position.z - caller_position.z;
            if dx * dx + dy * dy + dz * dz >= radius_sq {
                continue;
            }

            let hint = crate::ai::Hint {
                seek_point: position,
                seek_flags: 0,
                who_tells_me: caller,
            };
            self.world
                .entities
                .get_mut(source_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} lost its AI"))
                .outbox
                .reentrant
                .cross_npc_actions
                .push(crate::ai::CrossNpcAction::SendStimulus {
                    target: target_id.index(),
                    stimulus_type: crate::ai::StimulusType::CallLookThere,
                    info: crate::ai::StimulusInfo::Hint(hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
            self.process_synchronous_stimuli_for(sim, source_id, assets, defer_turn_instruction);
        }

        self.process_synchronous_look_there_resume(sim, source_id, caller, continuation, assets);
    }

    fn process_synchronous_look_there_resume(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        caller: u32,
        continuation: crate::ai::LookThereContinuation,
        assets: &LevelAssets,
    ) {
        assert_eq!(
            source_id.index(),
            caller,
            "HeyFolksLookThere resume caller must be its owner"
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(source_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} disappeared"));
        let mut ctx = {
            let entity = self
                .world
                .entities
                .get(source_id)
                .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} disappeared"));
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
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(source_id, &mut ctx);
        // The tail of `EVENT_VIEW` is what adopts the sighted enemy as the
        // primary target, so at this point the AI still carries whatever
        // target it had before the sighting. Reconstructing the per-tick
        // combat data off that stale handle leaves the enemy distances
        // unseeded, and `BattleDecisions` then reads an infinite
        // nearest-enemy distance and holds the soldier back in reserve
        // instead of engaging a target standing right next to it. Resolve
        // the tick data against the enemy the tail is about to adopt.
        let target_override = match continuation {
            crate::ai::LookThereContinuation::EventView { enemy, .. } => {
                self.entity_id_for_index(enemy)
            }
            _ => None,
        };
        let tick =
            self.build_npc_tick_data_for_target(sim, source_id, &scratch, assets, target_override);
        let global = &mut self.ai.global;
        let grid = &self.world.fast_grid;
        self.world
            .entities
            .get_mut(source_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| panic!("HeyFolksLookThere caller {caller} lost its EnemyAi"))
            .resume_after_look_there(sim, continuation, global, Some(grid), &ctx, &tick);
        self.drain_direct_ai_owner_boundary_without_forecast(sim, source_id, assets);
    }

    /// Whether a soldier is still holding its place in a phalanx.
    ///
    /// The phalanx-correction loops re-read this for every member right before
    /// announcing the new slot, because an earlier member's `CALL_INSTRUCTION`
    /// can re-enter and pull later members out of the formation.
    fn soldier_stands_in_phalanx(&self, target_id: EntityId) -> bool {
        self.world
            .entities
            .get(target_id)
            .and_then(Entity::enemy_ai)
            .is_some_and(|enemy| {
                enemy.base.current_substate == crate::ai::Substate::AttackingPhalanx
            })
    }

    fn process_synchronous_gather_instruction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        target: u32,
        position: crate::ai::Position,
        direction: u16,
        call_instruction: bool,
        assets: &LevelAssets,
    ) {
        let target_id = EntityId::Soldier(SoldierId(target));
        tracing::trace!(
            target: "robin_engine::ai_enemy::phalanx",
            instructed = target,
            frame = self.control.frame_counter,
            ?position,
            direction,
            call_instruction,
            "synchronous InstructGatherPosition"
        );
        if call_instruction && !self.soldier_stands_in_phalanx(target_id) {
            return;
        }
        let enemy = self
            .world
            .entities
            .get_mut(target_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| {
                panic!("InstructGatherPosition target {target} is not an enemy soldier")
            });
        enemy.gather_position = position;
        enemy.gather_direction = direction;
        enemy.gather_position_instructed = true;
        if !call_instruction {
            return;
        }

        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self
            .world
            .entities
            .get(target_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("gather-instruction target {target} disappeared"));
        let ctx = build_ai_context_from_entity(
            self.world
                .entities
                .get(target_id)
                .unwrap_or_else(|| panic!("gather-instruction target {target} disappeared")),
            self.control.frame_counter,
            building_sector,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        let tick = self.build_npc_tick_data(sim, target_id, &scratch, assets);
        self.dispatch_think_with_drain_without_forecast(
            sim,
            target_id,
            &crate::ai::Stimulus::new(crate::ai::StimulusType::CallInstruction),
            &ctx,
            &tick,
            assets,
        );
    }

    fn process_synchronous_stimuli_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let actions = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_synchronous_stimuli)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous stimulus source {} has no AI controller",
                    source_id.index()
                )
            });

        for action in actions {
            let crate::ai::CrossNpcAction::SendStimulus {
                target,
                stimulus_type,
                info,
                fallback_to_sender,
                to_whole_patrol,
            } = action
            else {
                unreachable!("synchronous-stimulus drain returned a different cross-NPC action")
            };
            let target_id = self.entity_id_for_index(target).unwrap_or_else(|| {
                panic!(
                    "synchronous {stimulus_type:?} from NPC {} references missing target {target}",
                    source_id.index()
                )
            });
            assert!(
                matches!(self.world.entities.get(target_id), Some(Entity::Soldier(_))),
                "synchronous {stimulus_type:?} target {target} is not a soldier"
            );

            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let building_sector = self
                .world
                .entities
                .get(target_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| {
                    panic!("synchronous {stimulus_type:?} target {target} disappeared")
                });
            let ctx = {
                let entity = self.world.entities.get(target_id).unwrap_or_else(|| {
                    panic!("synchronous {stimulus_type:?} target {target} disappeared")
                });
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
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let tick_data = self.build_npc_tick_data(sim, target_id, &scratch, assets);
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            stimulus.to_whole_patrol = to_whole_patrol;
            tracing::trace!(
                target: "patrol_relay",
                source = source_id.index(),
                target,
                ?stimulus_type,
                to_whole_patrol,
                "synchronous SendStimulus drain"
            );
            let handled = self.dispatch_think_with_drain_mode(
                sim,
                target_id,
                &stimulus,
                &ctx,
                &tick_data,
                assets,
                true,
                defer_turn_instruction,
            );
            if !handled && let Some(sender) = fallback_to_sender {
                let sender_id = self.entity_id_for_index(sender).unwrap_or_else(|| {
                    panic!(
                        "synchronous {stimulus_type:?} fallback references missing sender {sender}"
                    )
                });
                let scratch = self.build_owner_context_scratch_without_forecast(assets);
                let building_sector = self
                    .world
                    .entities
                    .get(sender_id)
                    .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                    .unwrap_or_else(|| panic!("synchronous fallback sender {sender} disappeared"));
                let sender_ctx = {
                    let entity = self.world.entities.get(sender_id).unwrap_or_else(|| {
                        panic!("synchronous fallback sender {sender} disappeared")
                    });
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
                        &self.ai.global.all_soldier_handles,
                        self.control.sim_config.difficulty,
                    )
                };
                let sender_tick = self.build_npc_tick_data(sim, sender_id, &scratch, assets);
                self.dispatch_think_with_drain_mode(
                    sim,
                    sender_id,
                    &stimulus,
                    &sender_ctx,
                    &sender_tick,
                    assets,
                    true,
                    defer_turn_instruction,
                );
            }
        }
    }

    /// Walk a patrol chief's members, delivering the whole-patrol stimulus to
    /// each one that the chief currently 360-degree detects.
    ///
    /// The detection gate is evaluated inside the loop, immediately before the
    /// member's own `think`, so a member whose visibility changed because an
    /// earlier member's dispatch moved somebody is judged on the state that
    /// dispatch left behind.
    fn process_synchronous_patrol_member_relay_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let relays = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_patrol_member_relays)
            .unwrap_or_else(|| {
                panic!(
                    "patrol broadcast source {} has no AI controller",
                    source_id.index()
                )
            });

        for relay in relays {
            let crate::ai::CrossNpcAction::RelayStimulusToPatrolMembers {
                members,
                stimulus_type,
                info,
            } = relay
            else {
                unreachable!("patrol-broadcast drain returned a different cross-NPC action")
            };
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            stimulus.to_whole_patrol = true;

            for member in members {
                let member_id = self.entity_id_for_index(member).unwrap_or_else(|| {
                    panic!(
                        "patrol broadcast from chief {} references missing member {member}",
                        source_id.index()
                    )
                });

                let scratch = self.build_owner_context_scratch_without_forecast(assets);
                let detected = {
                    let building_sector = self
                        .world
                        .entities
                        .get(source_id)
                        .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                        .unwrap_or_else(|| {
                            panic!("patrol broadcast chief {} disappeared", source_id.index())
                        });
                    let entity = self.world.entities.get(source_id).unwrap_or_else(|| {
                        panic!("patrol broadcast chief {} disappeared", source_id.index())
                    });
                    let chief_ctx = build_ai_context_from_entity(
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
                        &self.ai.global.all_soldier_handles,
                        self.control.sim_config.difficulty,
                    );
                    let chief_ai = entity.enemy_ai().unwrap_or_else(|| {
                        panic!(
                            "patrol broadcast chief {} is not an enemy soldier",
                            source_id.index()
                        )
                    });
                    chief_ai.detects_patrol_member_360(member, &chief_ctx)
                };
                tracing::trace!(
                    target: "patrol_relay",
                    chief = source_id.index(),
                    member,
                    ?stimulus_type,
                    detected,
                    "patrol broadcast member gate"
                );
                if !detected {
                    continue;
                }

                let building_sector = self
                    .world
                    .entities
                    .get(member_id)
                    .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                    .unwrap_or_else(|| panic!("patrol broadcast member {member} disappeared"));
                let ctx = {
                    let entity =
                        self.world.entities.get(member_id).unwrap_or_else(|| {
                            panic!("patrol broadcast member {member} disappeared")
                        });
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
                        &self.ai.global.all_soldier_handles,
                        self.control.sim_config.difficulty,
                    )
                };
                let tick_data = self.build_npc_tick_data(sim, member_id, &scratch, assets);
                self.dispatch_think_with_drain_mode(
                    sim,
                    member_id,
                    &stimulus,
                    &ctx,
                    &tick_data,
                    assets,
                    true,
                    defer_turn_instruction,
                );
            }
        }
    }

    /// Invoke a patrol chief's dispatch routine exactly as the subordinate's
    /// direct C++ call does. The routine's return value, rather than a
    /// prediction from the chief's state, decides whether the subordinate
    /// resumes its local handler.
    fn process_synchronous_patrol_dispatch_requests_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let requests = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_patrol_dispatch_requests)
            .unwrap_or_else(|| {
                panic!(
                    "patrol-dispatch source {} has no AI controller",
                    source_id.index()
                )
            });

        for request in requests {
            let crate::ai::CrossNpcAction::RequestPatrolDispatch {
                chief,
                caller,
                stimulus_type,
                info,
            } = request
            else {
                unreachable!("patrol-dispatch drain returned a different action")
            };
            assert_eq!(
                source_id.index(),
                caller,
                "patrol-dispatch caller must be its owner"
            );
            let chief_id = self.entity_id_for_index(chief).unwrap_or_else(|| {
                panic!(
                    "synchronous patrol dispatch from NPC {caller} references missing chief {chief}"
                )
            });
            if !matches!(self.world.entities.get(chief_id), Some(Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some())
            {
                panic!("patrol chief {chief} is not an enemy soldier");
            }

            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let chief_building_sector = self
                .world
                .entities
                .get(chief_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("patrol chief {chief} disappeared"));
            let mut chief_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(chief_id)
                    .unwrap_or_else(|| panic!("patrol chief {chief} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    chief_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(chief_id, &mut chief_ctx);
            let chief_tick = self.build_npc_tick_data(sim, chief_id, &scratch, assets);
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            chief_ctx.seed_view_radius_cache(&self.ai.view_radius_cache);
            let dispatched = {
                let global = &mut self.ai.global;
                let grid = &self.world.fast_grid;
                self.world
                    .entities
                    .get_mut(chief_id)
                    .and_then(Entity::enemy_ai_mut)
                    .unwrap_or_else(|| panic!("patrol chief {chief} lost its EnemyAi"))
                    .dispatch_stimulus_to_whole_patrol(
                        sim,
                        &stimulus,
                        global,
                        &chief_ctx,
                        &chief_tick,
                        Some(grid),
                    )
            };
            chief_ctx.commit_view_radius_cache(&mut self.ai.view_radius_cache);

            // A successful chief routine can recursively Think and queue the
            // member walk. Close those effects before the direct call returns.
            if dispatched {
                self.drain_direct_ai_owner_boundary_mode(
                    sim,
                    chief_id,
                    assets,
                    true,
                    defer_turn_instruction,
                );
                continue;
            }

            // The caller's outer handler resumes after the chief returned
            // false. Re-enter with the patrol flag set solely as a recursion
            // guard; no other handler observes that flag.
            let caller_scratch = self.build_owner_context_scratch_without_forecast(assets);
            let caller_building_sector = self
                .world
                .entities
                .get(source_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("patrol-dispatch caller {caller} disappeared"));
            let caller_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(source_id)
                    .unwrap_or_else(|| panic!("patrol-dispatch caller {caller} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    caller_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &caller_scratch.ai_entity_views,
                    &caller_scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let caller_tick = self.build_npc_tick_data(sim, source_id, &caller_scratch, assets);
            stimulus.to_whole_patrol = true;
            self.dispatch_think_with_drain_mode(
                sim,
                source_id,
                &stimulus,
                &caller_ctx,
                &caller_tick,
                assets,
                true,
                defer_turn_instruction,
            );
        }
    }

    fn process_synchronous_think_results_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
        defer_turn_instruction: bool,
    ) {
        let requests = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(|ai| {
                let mut requests = Vec::new();
                let mut deferred = Vec::new();
                for action in ai.outbox.reentrant.cross_npc_actions.drain(..) {
                    if matches!(action, crate::ai::CrossNpcAction::RequestThinkResult { .. }) {
                        requests.push(action);
                    } else {
                        deferred.push(action);
                    }
                }
                ai.outbox.reentrant.cross_npc_actions = deferred;
                requests
            })
            .unwrap_or_else(|| {
                panic!(
                    "Think-result source {} has no AI controller",
                    source_id.index()
                )
            });

        for request in requests {
            let crate::ai::CrossNpcAction::RequestThinkResult {
                target,
                caller,
                stimulus_type,
                info,
                continuation,
            } = request
            else {
                unreachable!("Think-result drain returned a different action")
            };
            assert_eq!(
                source_id.index(),
                caller,
                "Think-result caller must be its owner"
            );
            let target_id = self.entity_id_for_index(target).unwrap_or_else(|| {
                panic!(
                    "synchronous {stimulus_type:?} from NPC {caller} references missing target {target}"
                )
            });
            if !matches!(self.world.entities.get(target_id), Some(Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some())
            {
                panic!(
                    "synchronous {stimulus_type:?} from enemy NPC {caller} requires enemy-soldier target {target}"
                );
            }
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let target_building_sector = self
                .world
                .entities
                .get(target_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("{stimulus_type:?} target {target} disappeared"));
            let target_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(target_id)
                    .unwrap_or_else(|| panic!("{stimulus_type:?} target {target} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    target_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let target_tick = self.build_npc_tick_data(sim, target_id, &scratch, assets);
            let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
            stimulus.info = info;
            let accepted = self.dispatch_think_with_drain_without_forecast(
                sim,
                target_id,
                &stimulus,
                &target_ctx,
                &target_tick,
                assets,
            );

            let source_scratch = self.build_owner_context_scratch_without_forecast(assets);
            let source_building_sector = self
                .world
                .entities
                .get(source_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("Think-result caller {caller} disappeared"));
            let mut source_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(source_id)
                    .unwrap_or_else(|| panic!("Think-result caller {caller} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    source_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &source_scratch.ai_entity_views,
                    &source_scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(source_id, &mut source_ctx);
            let source_tick = self.build_npc_tick_data(sim, source_id, &source_scratch, assets);
            let global = &mut self.ai.global;
            let grid = &self.world.fast_grid;
            self.world
                .entities
                .get_mut(source_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| panic!("Think-result caller {caller} lost its EnemyAi"))
                .resolve_think_result(
                    sim,
                    accepted,
                    target,
                    continuation,
                    global,
                    Some(grid),
                    &source_ctx,
                    &source_tick,
                );

            // The continuation is the caller's C++ stack frame resuming
            // immediately after `target->Think(...)` returned. SetState,
            // Say, and timer work emitted there must therefore close before
            // the next queued group member is called (and certainly before
            // this source owner's Hourglass slot returns).
            self.drain_ai_owner_work_for_mode(sim, assets, source_id, true, defer_turn_instruction);
        }
    }

    fn process_synchronous_alert_requests_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let requests = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_alert_requests)
            .unwrap_or_else(|| {
                panic!(
                    "CALL_ALERT source {} has no AI controller",
                    source_id.index()
                )
            });

        for request in requests {
            let crate::ai::CrossNpcAction::RequestAlert {
                target,
                caller,
                continuation,
            } = request
            else {
                unreachable!("alert-request drain returned a deferred action")
            };
            assert_eq!(
                source_id.index(),
                caller,
                "CALL_ALERT request caller must be its owner"
            );

            let target_id = self.entity_id_for_index(target).unwrap_or_else(|| {
                panic!(
                    "synchronous CALL_ALERT from NPC {caller} references missing target {target}"
                )
            });
            assert!(
                matches!(self.world.entities.get(target_id), Some(Entity::Soldier(_))),
                "synchronous CALL_ALERT target {target} is not a soldier"
            );
            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let target_building_sector = self
                .world
                .entities
                .get(target_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("CALL_ALERT target {target} disappeared"));
            let target_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(target_id)
                    .unwrap_or_else(|| panic!("CALL_ALERT target {target} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    target_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let target_tick = self.build_npc_tick_data(sim, target_id, &scratch, assets);
            let accepted = self.dispatch_think_with_drain_without_forecast(
                sim,
                target_id,
                &crate::ai::Stimulus::with_human(crate::ai::StimulusType::CallAlert, caller),
                &target_ctx,
                &target_tick,
                assets,
            );

            let source_scratch = self.build_owner_context_scratch_without_forecast(assets);
            let source_building_sector = self
                .world
                .entities
                .get(source_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("CALL_ALERT caller {caller} disappeared"));
            let mut source_ctx = {
                let entity = self
                    .world
                    .entities
                    .get(source_id)
                    .unwrap_or_else(|| panic!("CALL_ALERT caller {caller} disappeared"));
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    source_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &source_scratch.ai_entity_views,
                    &source_scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(source_id, &mut source_ctx);
            let source_tick = self.build_npc_tick_data(sim, source_id, &source_scratch, assets);
            match continuation {
                crate::ai::AlertContinuation::CivilianReachedSoldier
                | crate::ai::AlertContinuation::CivilianSawSoldier => self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::friendly_ai_mut)
                    .unwrap_or_else(|| {
                        panic!("civilian CALL_ALERT caller {caller} lost its FriendlyAi")
                    })
                    .resolve_alert_request(sim, accepted, continuation, &source_ctx),
                crate::ai::AlertContinuation::SoldierSawOfficer => self
                    .world
                    .entities
                    .get_mut(source_id)
                    .and_then(Entity::enemy_ai_mut)
                    .unwrap_or_else(|| {
                        panic!("soldier CALL_ALERT caller {caller} lost its EnemyAi")
                    })
                    .resolve_alert_request(sim, accepted, continuation, &source_ctx, &source_tick),
            }
        }
    }

    fn process_synchronous_officer_reports_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        source_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let reports = self
            .world
            .entities
            .get_mut(source_id)
            .and_then(Entity::ai_controller_mut)
            .map(crate::ai::AiController::take_pending_officer_reports)
            .unwrap_or_else(|| {
                panic!(
                    "officer-report source {} has no AI controller",
                    source_id.index()
                )
            });

        for report in reports {
            let crate::ai::CrossNpcAction::ReportBackToOfficer { officer, charly } = report else {
                unreachable!("take_pending_officer_reports returned a deferred action")
            };
            assert_eq!(
                source_id.index(),
                charly,
                "officer report source must be the reporting Charly"
            );

            let scratch = self.build_owner_context_scratch_without_forecast(assets);
            let officer_id = EntityId::Soldier(SoldierId(officer));
            let officer_building_sector = self
                .world
                .entities
                .get(officer_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("reporting Charly requires missing officer {officer}"));
            let officer_ctx = {
                let entity = self.world.entities.get(officer_id).unwrap_or_else(|| {
                    panic!("reporting Charly requires missing officer {officer}")
                });
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    officer_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            let officer_tick = self.build_npc_tick_data(sim, officer_id, &scratch, assets);
            let officer_stimulus = crate::ai::Stimulus::with_human(
                crate::ai::StimulusType::CallMrOfficerIAmBack,
                charly,
            );
            let accepted = self.dispatch_think_with_drain_without_forecast(
                sim,
                officer_id,
                &officer_stimulus,
                &officer_ctx,
                &officer_tick,
                assets,
            );

            let charly_id = EntityId::Soldier(SoldierId(charly));
            let charly_building_sector = self
                .world
                .entities
                .get(charly_id)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| panic!("officer response requires missing Charly {charly}"));
            let mut charly_ctx = {
                let entity =
                    self.world.entities.get(charly_id).unwrap_or_else(|| {
                        panic!("officer response requires missing Charly {charly}")
                    });
                build_ai_context_from_entity(
                    entity,
                    self.control.frame_counter,
                    charly_building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };
            self.refresh_selected_default_wait_identity(charly_id, &mut charly_ctx);
            let charly_tick = self.build_npc_tick_data(sim, charly_id, &scratch, assets);
            let entity = self
                .world
                .entities
                .get_mut(charly_id)
                .unwrap_or_else(|| panic!("officer response requires missing Charly {charly}"));
            let enemy = entity
                .enemy_ai_mut()
                .unwrap_or_else(|| panic!("reporting Charly {charly} requires enemy AI"));
            enemy.resolve_charly_officer_report(sim, accepted, &charly_ctx, &charly_tick);
        }
    }

    /// Drain each NPC's `pending_self_stimuli` queue and re-dispatch each
    /// stimulus through `think` on the same frame.  Matches
    /// `Think()`-from-within-handler calls (MYTALK callbacks from
    /// `say()`, deferred `EventDone` from `SendCondolationCard`, etc.)
    /// which in the original engine immediately re-enter the AI but in
    /// Rust are queued to avoid nested `&mut AiGlobalState` borrows.
    ///
    /// Called unconditionally each tick.  Each NPC is drained to a fixed
    /// point so a Think call that recursively fires another self-stimulus
    /// observes that stimulus in the originating frame, matching the
    /// original direct `Think(...)` call.
    pub(super) fn drain_pending_self_stimuli(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.drain_self_stimuli_for_npc(sim, npc_id, assets);
        }
    }

    /// Per-NPC half of [`Self::drain_pending_self_stimuli`] — drains the
    /// pending self-stimulus queue for a single NPC and re-dispatches
    /// each through `think`.  Called both from the global end-of-tick
    /// drain and from [`Self::dispatch_think_with_drain`] so the
    /// re-entrant `think(EVENT_DONE)` that `send_condolation_card`
    /// fires lands inside the same call stack as the outer think.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn drain_self_stimuli_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_self_stimuli_for_npc_mode(sim, npc_id, assets, false, false);
    }

    /// Native `SetAIState` StartThink/EndThink recursion must remain
    /// owner-local: forecasting unrelated actors here would advance their
    /// authoritative BuildingExitGate RNG before the native returns.
    pub(super) fn drain_self_stimuli_for_npc_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_self_stimuli_for_npc_mode(sim, npc_id, assets, true, false);
    }

    fn drain_self_stimuli_for_npc_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        const MAX_REENTRANT_STIMULI: usize = 111;
        let mut dispatched = 0usize;

        loop {
            let stimulus_type = {
                let Some(entity) = self.world.entities.get_mut(npc_id) else {
                    return;
                };
                let Some(ai) = entity.ai_controller_mut() else {
                    return;
                };
                if ai.outbox.reentrant.self_stimuli.is_empty() {
                    break;
                }
                ai.outbox.reentrant.self_stimuli.remove(0)
            };

            dispatched += 1;
            if dispatched > MAX_REENTRANT_STIMULI {
                tracing::warn!(
                    npc = npc_id.index(),
                    "self-stimulus recursion exceeded the original 111-call guard"
                );
                break;
            }

            let scratch = if owner_local_no_forecast {
                self.build_owner_context_scratch_without_forecast(assets)
            } else {
                self.build_sim_scratch(sim, assets)
            };
            let frame = self.control.frame_counter;
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let ctx = {
                let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("re-entrant self-Think NPC {} disappeared", npc_id.index())
                });
                let building_sector = self.entity_building_sector(entity.element_data().sector());
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    frame,
                    building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                ctx
            };
            let stimulus = crate::ai::Stimulus::new(stimulus_type);
            if owner_local_no_forecast {
                match self.world.entities.get(npc_id) {
                    Some(Entity::Soldier(_)) => {
                        let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
                        self.dispatch_filtered_stimulus_without_forecast(
                            sim, assets, npc_id, &stimulus, &ctx, &tick_data,
                        );
                    }
                    Some(Entity::Civilian(_)) => {
                        self.dispatch_filtered_friendly_stimulus_without_forecast(
                            sim, assets, npc_id, &stimulus, &ctx,
                        );
                    }
                    Some(other) => panic!(
                        "owner-local self-stimulus recipient {} has invalid kind {:?}",
                        npc_id.index(),
                        other.element_data().kind
                    ),
                    None => panic!(
                        "owner-local self-stimulus recipient {} disappeared",
                        npc_id.index()
                    ),
                };
            } else {
                let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
                self.dispatch_filtered_stimulus(sim, assets, npc_id, &stimulus, &ctx, &tick_data);
            }

            // A recursive Think can itself reach an authored waypoint. The
            // Original runs ReachPoint on this same call stack, before the
            // recursive Think's generic effects are allowed to escape.
            self.dispatch_pending_waypoint_script_for_owner(sim, npc_id, assets);

            // Original Think calls execute their engine-facing side effects
            // before returning.  Close that window after every recursive
            // stimulus so a newly launched sequence participates in
            // arbitration before the next sibling stimulus is delivered.
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
            self.launch_pending_orders_for_npc_mode(sim, assets, npc_id, defer_turn_instruction);
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
            self.surface_synchronous_completion_events_for_owner(npc_id);
            self.process_synchronous_reentrant_actions_for_mode(
                sim,
                npc_id,
                assets,
                defer_turn_instruction,
            );
            self.dispatch_condolations_for_npc(sim, npc_id, assets);
        }

        let finish_macro = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .is_some_and(|ai| {
                std::mem::take(&mut ai.outbox.reentrant.finish_macro_after_self_stimuli)
            });
        if finish_macro {
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "post-reentrant macro owner {} lost its AI controller",
                        npc_id.index()
                    )
                })
                .finish_patrol_macro();
        }
    }

    // ── Per-waypoint ReachPoint dispatch ──────────────────────────
    //
    // Drain `pending_waypoint_script_reach_point` on every NPC:
    // dispatch `ReachPoint(actor)` on the waypoint's bound VM, then
    // synchronously re-enter `think(EventAfterScriptGoOn)` unless the
    // script transitioned the NPC into `DefaultScriptDriven`.  Runs
    // `execute_waypoint_script`, including the `script_enabled` gate
    // and the recursive `think()` call.  If no script is bound for
    // the waypoint (class missing), the recursive `think` still fires
    // — the "script was a no-op" branch when the bound class doesn't
    // transition state.
    pub(super) fn dispatch_pending_waypoint_scripts(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let owners: Vec<_> = self
            .world
            .entities
            .npcs()
            .filter_map(|(npc_id, entity)| {
                entity
                    .ai_controller()
                    .and_then(|ai| ai.outbox.reentrant.waypoint_script_reach_point)
                    .map(|_| EntityId::from(npc_id))
            })
            .collect();
        for owner in owners {
            self.dispatch_pending_waypoint_script_for_owner(sim, owner, assets);
        }
    }

    /// Close one NPC's authored waypoint callback on the same owner-local
    /// stack that selected it. `ExecuteWaypointScript` in the Original calls
    /// `ReachPoint` and then `Think(EventAfterScriptGoOn)` directly.
    pub(super) fn dispatch_pending_waypoint_script_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let request = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(|entity| entity.ai_controller_mut())
            .and_then(|ai| ai.outbox.reentrant.waypoint_script_reach_point.take());
        let Some((path_idx, wp_idx)) = request else {
            return;
        };
        if !sim.config().script_enabled {
            return;
        }

        self.with_suspended_waypoint_think(npc_id, |engine| {
            engine
                .dispatch_waypoint_script_on_suspended_think(sim, npc_id, assets, path_idx, wp_idx);
        });
    }

    /// Keep the route-arrival Think logically live while Rust releases its AI
    /// borrow to enter the waypoint VM. The restoration is unwind-safe so a
    /// script panic cannot leak a fake recursion level into later AI work.
    fn with_suspended_waypoint_think<T>(
        &mut self,
        npc_id: EntityId,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        // ExecuteWaypointScript is called from inside the route-arrival
        // Think in the Original. Rust has to release the AI borrow before it
        // can enter the waypoint VM, but native AI calls made by ReachPoint
        // must still observe that suspended outer Think. In particular, a
        // close-point GoTo sets `already_on_point` for the enclosing EndThink
        // instead of immediately queueing a second EVENT_REACHPOINT. The
        // recursively entered EVENT_AFTER_SCRIPT_GO_ON resets that latch in
        // StartThink, exactly as the C++ call stack does.
        {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "waypoint-script owner {} lost its AI before ReachPoint",
                        npc_id.index()
                    )
                });
            ai.think_recursion_depth = ai
                .think_recursion_depth
                .checked_add(1)
                .expect("waypoint-script suspended Think depth overflow");
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));
        if let Some(ai) = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
        {
            // This is the EndThink of the outer route-arrival Think that was
            // suspended while the waypoint VM and its recursive
            // EventAfterScriptGoOn ran.  Merely restoring the depth strands
            // completion latches produced by the final recursive action
            // (for example AssignNewPatrolPath(-1) returning to a post at the
            // actor's current position).  Original consumes those latches
            // here and recursively dispatches the matching event.
            assert!(
                ai.end_think_completion_events(),
                "waypoint-script suspended Think unexpectedly hit the typed recursion fallback"
            );
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn dispatch_waypoint_script_on_suspended_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        path_idx: crate::ai::PathId,
        wp_idx: u8,
    ) {
        let actor_handle = crate::natives::ScriptHandleCodec::actor_handle(npc_id);
        tracing::trace!(
            frame = self.control.frame_counter,
            owner = npc_id.index(),
            path = ?path_idx,
            wp = wp_idx,
            "waypoint ReachPoint dispatch"
        );
        if let Err(error) = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Waypoint(path_idx, wp_idx),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        ) {
            tracing::warn!(
                "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {error}"
            );
            debug_assert!(
                false,
                "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {error}"
            );
        }

        // The script may have spawned or deactivated entities, so rebuild the
        // context only after its VM call returns.
        let scratch = self.build_sim_scratch(sim, assets);
        let frame = self.control.frame_counter;
        let is_forest_level = self.world.weather.is_forest_level;
        let ambiance = self.world.weather.ambiance;
        let standard_view_polygon_radius = self.ai.standard_view_polygon_radius;
        let script_driven = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .is_none_or(|ai| ai.current_substate == crate::ai::Substate::DefaultScriptDriven);
        if script_driven {
            return;
        }
        let ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                frame,
                None,
                is_forest_level,
                ambiance,
                standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterScriptGoOn);
        let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
        self.dispatch_think_with_drain(sim, npc_id, &stimulus, &ctx, &tick_data, assets);
        self.dispatch_synchronous_owner_moves(sim, assets, npc_id, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "waypoint-script owner {} synchronous Move dispatch failed: {error:?}",
                    npc_id.index()
                )
            });
    }

    /// Finish engine-facing work queued by a direct AI method that is not
    /// itself entered through `dispatch_think_with_drain` (ambush checks,
    /// ladder recovery, The16thFrame, and macro continuation). This remains
    /// owner-local and includes the shared SetState/Say FIFO before later
    /// effects, orders, condolations, and recursive self-stimuli.
    pub(super) fn drain_direct_ai_owner_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_direct_ai_owner_boundary_mode(sim, npc_id, assets, false, false);
    }

    pub(super) fn drain_direct_ai_owner_boundary_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_direct_ai_owner_boundary_mode(sim, npc_id, assets, true, false);
    }

    pub(super) fn drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.drain_direct_ai_owner_boundary_mode(sim, npc_id, assets, true, true);
    }

    /// Continue common route-arrival code after its virtual SetState barrier.
    ///
    /// This is a fresh engine-facing context because `FilterAIEvent` may have
    /// synchronously reassigned the patrol path or otherwise mutated the
    /// actor before Original resumes the caller after `SetState`.
    pub(super) fn resume_goto_route_reach_point_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }
        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!(
                    "route-arrival continuation owner {} disappeared",
                    npc_id.index()
                )
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "route-arrival continuation owner {} lost its AI",
                    npc_id.index()
                )
            })
            .resume_goto_route_reach_point(sim, &ctx);

        // Original calls InitializePatrol inline, immediately after the
        // virtual SetState callback returns. Delaying this to the next
        // RHArtificialIntelligence::Hourglass changes which side of the
        // formation equally-close members occupy because later legacy slots
        // have moved by then.
        self.initialize_patrol_for_npc_from_owner_views(assets, npc_id, &scratch.ai_entity_views);
    }

    /// Invoke the concrete Enemy/Friendly `ReturnToDuty` override requested
    /// by shared AI code. Enemy queues its patrol-initialization continuation
    /// on the same owner FIFO; Friendly applies its busy gate before entering
    /// the common tail. In both cases the caller observes the complete virtual
    /// call before later owner work runs.
    pub(super) fn virtual_return_to_duty_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        flags: crate::ai::DutyFlags,
        owner_position_override: Option<crate::ai::Position>,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        // Work already behind this virtual call belongs to the caller after
        // ReturnToDuty returns. Detach it so work emitted by the override
        // (notably ResumeReturnToDutyAfterPatrolInit) stays nested ahead of
        // that caller tail instead of being appended after it.
        let later_owner_work = {
            let ai = self
                .world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!("virtual ReturnToDuty owner {} lost its AI", npc_id.index())
                });
            std::mem::take(&mut ai.outbox.reentrant.owner_work)
        };
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }

        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("virtual ReturnToDuty owner {} disappeared", npc_id.index())
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            if let Some(owner_position) = owner_position_override {
                ctx.position = owner_position;
            }
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        let is_enemy = self
            .world
            .entities
            .get(npc_id)
            .is_some_and(|entity| entity.enemy_ai().is_some());
        if is_enemy {
            let tick = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::enemy_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "virtual ReturnToDuty enemy owner {} lost its AI",
                        npc_id.index()
                    )
                })
                .return_to_duty(sim, flags, &ctx, &tick);
        } else {
            self.world
                .entities
                .get_mut(npc_id)
                .and_then(Entity::friendly_ai_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "virtual ReturnToDuty owner {} has neither Enemy nor Friendly AI",
                        npc_id.index()
                    )
                })
                .return_to_duty(sim, flags, &ctx);
        }

        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| {
                panic!(
                    "virtual ReturnToDuty owner {} lost its AI after override",
                    npc_id.index()
                )
            })
            .outbox
            .reentrant
            .owner_work
            .extend(later_owner_work);
    }

    /// Run Enemy `ReturnToDuty`'s synchronous patrol initialization and then
    /// resume the common tail at the same owner boundary.
    pub(super) fn resume_return_to_duty_after_patrol_init_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        flags: crate::ai::DutyFlags,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }

        // This is an inline C++ call, not a future Hourglass request. It must
        // settle before ReturnToDutyCommonStuff checks `patrol_chief` and may
        // issue the reciprocal visibility query.
        self.initialize_patrol_for_npc_from_owner_views(assets, npc_id, &scratch.ai_entity_views);

        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!(
                    "return-to-duty continuation owner {} disappeared",
                    npc_id.index()
                )
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            if let Some((_, owner_position)) = owner_boundary_positions
                .iter()
                .find(|(handle, _)| *handle == npc_id.index())
            {
                ctx.position = *owner_position;
            }
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::enemy_ai_mut)
            .unwrap_or_else(|| {
                panic!(
                    "return-to-duty continuation owner {} lost its Enemy AI",
                    npc_id.index()
                )
            })
            .resume_return_to_duty_after_patrol_init(sim, flags, &ctx);
    }

    /// Run `CMD_PATROL_START`'s inline patrol rebuild, then continue the
    /// waypoint macro at the same owner boundary.
    pub(super) fn resume_macro_after_patrol_init_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_boundary_positions: &[(u32, crate::ai::Position)],
    ) {
        let mut scratch = self.build_owner_context_scratch_without_forecast(assets);
        let views = std::sync::Arc::make_mut(&mut scratch.ai_entity_views);
        for &(handle, position) in owner_boundary_positions {
            if let Some(view) = views.get_mut(&handle) {
                view.position = position;
            }
        }

        self.initialize_patrol_for_npc_from_owner_views(assets, npc_id, &scratch.ai_entity_views);

        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let mut ctx = {
            let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("patrol-start macro owner {} disappeared", npc_id.index())
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            ctx
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        self.world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::ai_controller_mut)
            .unwrap_or_else(|| panic!("patrol-start macro owner {} lost its AI", npc_id.index()))
            .execute_next_macro_command(sim, &ctx);
    }

    /// Run Original's `InitializePatrol` at a captured owner boundary.
    ///
    /// Non-position fields are intentionally resolved from the live world
    /// after `FilterAIEvent`: that callback is authoritative and may change
    /// actor state. Positions instead come from the owner-boundary views
    /// because Rust's globally batched movement has already advanced later
    /// legacy slots.
    pub(super) fn initialize_patrol_for_npc_from_owner_views(
        &mut self,
        assets: &LevelAssets,
        chief_id: EntityId,
        views: &crate::ai_entity_view::AiEntityViewMap,
    ) {
        let theoretical = self
            .world
            .entities
            .get(chief_id)
            .and_then(Entity::ai_controller)
            .unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} has no AI",
                    chief_id.index()
                )
            })
            .theoretical_patrol
            .clone();
        self.initialize_patrol_for_npc_over_members(assets, chief_id, views, &theoretical);
    }

    /// `InitializePatrol` restricted to an explicit slice of theoretical
    /// members. `AddPatrolMember` runs one initialization per appended
    /// member, so each of its passes sees only the prefix of the theoretical
    /// list that existed at that point.
    pub(super) fn initialize_patrol_for_npc_over_members(
        &mut self,
        assets: &LevelAssets,
        chief_id: EntityId,
        views: &crate::ai_entity_view::AiEntityViewMap,
        theoretical: &[EntityId],
    ) {
        #[derive(Clone, Copy)]
        struct PatrolSnap {
            position: crate::ai::Position,
            detection_position_world: crate::coordinates::WorldPoint3D,
            direction: u16,
            ground_z: f32,
            posture: crate::element::Posture,
            is_rider: bool,
            in_building: bool,
            ai_state: crate::ai::AiState,
            is_alive: bool,
            is_active: bool,
            is_civilian: bool,
            is_able_to_fight: bool,
        }

        let chief_entity = self.world.entities.get(chief_id).unwrap_or_else(|| {
            panic!(
                "synchronous patrol initialization owner {} disappeared",
                chief_id.index()
            )
        });
        // `InitializePatrol` admits members through IsDetecting360Degrees,
        // whose distance gate is the post-RefreshView real radius, not the
        // pre-factor base radius the growing cone animates towards.
        let chief_real_view_radius = chief_entity
            .npc_data()
            .unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} is not an NPC",
                    chief_id.index()
                )
            })
            .view_radius;

        let snapshot = |id: EntityId| {
            let view = views.get(&id.index()).unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} lacks boundary view for member {}",
                    chief_id.index(),
                    id.index()
                )
            });
            let entity = self.world.entities.get(id).unwrap_or_else(|| {
                panic!(
                    "synchronous patrol initialization owner {} references missing member {}",
                    chief_id.index(),
                    id.index()
                )
            });
            PatrolSnap {
                position: view.position,
                detection_position_world: view.detection_position_world,
                direction: entity.element_data().direction() as u16,
                ground_z: entity.element_data().position().z,
                posture: entity.element_data().posture,
                is_rider: entity.soldier_data().is_some_and(|soldier| soldier.rider),
                in_building: self.entity_data_in_building_sector(entity.element_data()),
                ai_state: entity
                    .npc_data()
                    .unwrap_or_else(|| {
                        panic!(
                            "patrol member {} referenced by owner {} is not an NPC",
                            id.index(),
                            chief_id.index()
                        )
                    })
                    .ai_state(),
                is_alive: !entity.is_dead(),
                is_active: entity.is_active(),
                is_civilian: entity.is_civilian(),
                is_able_to_fight: match entity {
                    crate::element::Entity::Soldier(soldier) => {
                        use crate::element::Human as _;
                        soldier.is_able_to_fight()
                    }
                    crate::element::Entity::Pc(pc) => {
                        use crate::element::Human as _;
                        pc.is_able_to_fight()
                    }
                    _ => false,
                },
            }
        };

        let chief_snap = snapshot(chief_id);
        let obstacles_owned = self.build_ai_sight_obstacles(assets);
        let obstacles = obstacles_owned.list();
        let mut patrol = Vec::new();
        let mut missed = Vec::new();

        for &member in theoretical {
            if member == chief_id {
                continue;
            }
            let snap = snapshot(member);
            // Original evaluates IsDetecting360Degrees first in the `&&`
            // chain. An active, outdoor member therefore emits its LOS query
            // even when its later AI-state / able-to-fight gate rejects it.
            // Pre-gating visibility on those later predicates loses the
            // chief-to-member prefix and lets ReturnToDutyCommonStuff's
            // reciprocal member queries appear first in the frame trace.
            let admit = patrol_member_admitted(
                chief_snap.is_active && snap.is_active,
                || {
                    patrol_member_visible_from_raw_world(
                        chief_snap.detection_position_world,
                        chief_snap.is_rider,
                        chief_real_view_radius,
                        chief_snap.in_building,
                        snap.detection_position_world,
                        snap.posture,
                        snap.is_rider,
                        snap.direction as i16,
                        snap.in_building,
                        obstacles,
                    )
                },
                snap.ai_state,
                snap.is_civilian,
                snap.is_able_to_fight,
            );
            if admit {
                patrol.push((member, snap));
            } else if snap.is_alive {
                missed.push(member);
            }
        }

        let square_distance = |snap: PatrolSnap| {
            let dx = snap.position.x - chief_snap.position.x;
            let dy_world =
                (snap.position.y + snap.ground_z) - (chief_snap.position.y + chief_snap.ground_z);
            let dy = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = snap.ground_z - chief_snap.ground_z;
            dx * dx + dy * dy + dz * dz
        };
        let mut sorted: Vec<(EntityId, PatrolSnap)> = Vec::with_capacity(patrol.len());
        for entry in patrol {
            let distance = square_distance(entry.1);
            let insert_at = sorted
                .iter()
                .position(|existing| distance <= square_distance(existing.1))
                .unwrap_or(sorted.len());
            sorted.insert(insert_at, entry);
        }
        for pair_end in (1..sorted.len()).step_by(2) {
            let even = sorted[pair_end - 1].1.position;
            let odd = sorted[pair_end].1.position;
            let ex = even.x - chief_snap.position.x;
            let ey = even.y - chief_snap.position.y;
            let ox = odd.x - chief_snap.position.x;
            let oy = odd.y - chief_snap.position.y;
            if ex * oy - ey * ox < 0.0 {
                sorted.swap(pair_end - 1, pair_end);
            }
        }

        let patrol_ids: Vec<_> = sorted.into_iter().map(|(id, _)| id).collect();
        {
            let ai = self
                .world
                .entities
                .get_mut(chief_id)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "synchronous patrol initialization owner {} lost its AI",
                        chief_id.index()
                    )
                });
            ai.needs_patrol_reinit = false;
            ai.patrol = patrol_ids.clone();
            ai.missed_patrol_members = missed;
        }
        for member in patrol_ids {
            self.world
                .entities
                .get_mut(member)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "patrol member {} admitted by owner {} lost its AI",
                        member.index(),
                        chief_id.index()
                    )
                })
                .patrol_chief = Some(chief_id);
        }

        // TODO: share this sorting/admission core with the initialization
        // paths that run before the per-owner hourglass instead of retaining
        // the equivalent delayed-path implementation in patrol coordination.
    }

    pub(super) fn drain_direct_ai_owner_boundary_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
        owner_local_no_forecast: bool,
        defer_turn_instruction: bool,
    ) {
        // This entry point models one direct, synchronous member-call stack.
        // Cards that were already queued for other owners belong to their
        // established later Hourglass boundaries; nested helpers below still
        // use the global drain because cards they create on this stack are
        // causal and must close re-entrantly. Detach only the pre-existing
        // foreign backlog for the duration of the fixed point.
        let pending = self.orders.sequence_manager.drain_pending_condolations();
        let (owner_roots, foreign_backlog): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|dispatch| dispatch.card.owner == npc_id);
        self.orders
            .sequence_manager
            .restore_pending_condolations(owner_roots);

        const MAX_ITERS: u32 = 8;
        for iter in 0..MAX_ITERS {
            self.drain_pending_for_npc_mode(
                sim,
                npc_id,
                assets,
                owner_local_no_forecast,
                defer_turn_instruction,
            );
            self.launch_pending_orders_for_npc_mode(sim, assets, npc_id, defer_turn_instruction);
            let _ = self.drain_pending_move_requests_for_owner(sim, npc_id);
            self.surface_synchronous_completion_events_for_owner(npc_id);
            self.process_synchronous_reentrant_actions_for(sim, npc_id, assets);
            // All foreign cards that predated this direct boundary are held
            // aside above. Any foreign-owner card visible here was therefore
            // produced causally on this call stack and must close now.
            self.dispatch_condolations_for_npc(sim, npc_id, assets);
            let has_self_stimuli = {
                let ai = self
                    .world
                    .entities
                    .get(npc_id)
                    .unwrap_or_else(|| panic!("direct-drain NPC {} disappeared", npc_id.index()))
                    .ai_controller()
                    .unwrap_or_else(|| {
                        panic!("direct-drain NPC {} has no AI controller", npc_id.index())
                    });
                !ai.outbox.reentrant.self_stimuli.is_empty()
            };
            if has_self_stimuli {
                self.drain_self_stimuli_for_npc_mode(
                    sim,
                    npc_id,
                    assets,
                    owner_local_no_forecast,
                    defer_turn_instruction,
                );
            }

            let still_pending = {
                let ai = self
                    .world
                    .entities
                    .get(npc_id)
                    .unwrap_or_else(|| panic!("direct-drain NPC {} disappeared", npc_id.index()))
                    .ai_controller()
                    .unwrap_or_else(|| {
                        panic!("direct-drain NPC {} has no AI controller", npc_id.index())
                    });
                ai.outbox.actor.has_boundary_work()
                    || !ai.outbox.reentrant.self_stimuli.is_empty()
                    || !ai.outbox.reentrant.owner_work.is_empty()
                    || ai.has_pending_synchronous_cross_npc_actions()
            };
            if !still_pending {
                break;
            }
            assert!(
                iter + 1 < MAX_ITERS,
                "direct AI drain for NPC {} did not stabilise after {MAX_ITERS} passes",
                npc_id.index()
            );
        }

        self.orders
            .sequence_manager
            .restore_pending_condolations(foreign_backlog);
    }

    /// Apply one AI `StopAll` prefix as a synchronous owner boundary.
    ///
    /// Existing cards for unrelated owners belong to their established
    /// Hourglass slots. Cards produced while draining this owner's queued
    /// SetState/StopAll work are causal, including cross-owner callbacks, and
    /// remain visible to the ordinary global condolence drain.
    pub(super) fn drain_ai_owner_halt_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        npc_id: EntityId,
    ) {
        let pending = self.orders.sequence_manager.drain_pending_condolations();
        let (owner_roots, foreign_backlog): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|dispatch| dispatch.card.owner == npc_id);
        self.orders
            .sequence_manager
            .restore_pending_condolations(owner_roots);

        self.drain_ai_owner_work_for(sim, assets, npc_id);
        self.apply_pending_ai_halt(npc_id);
        self.dispatch_condolations_for_npc(sim, npc_id, assets);

        self.orders
            .sequence_manager
            .restore_pending_condolations(foreign_backlog);
    }

    // ── The16thFrame — periodic AI tasks (staggered) ──────────────
    //
    // `the_16th_frame` runs every 16th frame from the NPC's
    // `hourglass`, staggered by NPC index so not all soldiers run on
    // the same frame.

    pub(super) fn tick_periodic_ai_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;

        let entity = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("periodic NPC {} disappeared", npc_id.index()));

        // Exact original phase:
        //   (frame & 255) - ((register_number + 100) & 255)
        // with unsigned-byte wrap. Passing the full phase matters:
        // The16thFrame uses bits 4..5 to reduce some work to every
        // 64th frame, so substituting `frame % 16` ran that work 4x.
        let register_number = entity
            .npc_data()
            .unwrap_or_else(|| panic!("periodic entity {} is not an NPC", npc_id.index()))
            .register_number;
        let frame_phase = npc_hourglass_frame_phase(current_frame, u32::from(register_number));
        if (frame_phase & 15) != 0 {
            return;
        }

        if entity.is_dead() {
            return;
        }

        // `sequence_element_is_about_to_be_launched(self, NULL)`
        // — used by the civilian stuck-counter suppression.
        // Query once up front so we can hand it to the AI layer
        // without holding a sequence-manager borrow across the
        // AI tick.
        let sequence_null_about_to_launch = self
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(npc_id, crate::element::Command::Null);

        // `command == Wait` — entity is idle.  Read the live
        // sequence-element command via `actor_command` rather
        // than `action_state == Waiting` so we don't get a
        // false-positive on `WaitTimer` (which sets `action_state
        // = Waiting` via the animation map but is not
        // `Command::Wait`) or a false-negative on the brief
        // window where a teardown nulls the sequence-element
        // before the next animation tick resets `action_state`.
        let actor_command = self.actor_command(npc_id);
        let is_idle = actor_command == crate::element::Command::Wait;
        let receiving_wasp_sting = actor_command == crate::element::Command::ReceiveWaspSting;
        let stuck_command_active = matches!(
            actor_command,
            crate::element::Command::Wait
                | crate::element::Command::SwordstrikeSmalltalkLeft
                | crate::element::Command::SwordstrikeSmalltalkRight
                | crate::element::Command::ParrySmalltalkLeft
                | crate::element::Command::ParrySmalltalkRight
        );
        // C++ `RHElementActor::GetAnimation()` returns `mpOrder->action`, not
        // the sprite row most recently performed. A transition may complete
        // during Actor::Execute and promote its successor before NPC
        // Hourglass reaches The16thFrame; in that window `Sprite::last_action`
        // still names the transition.
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        // The16thFrame's only combat-context consumer is
        // RefreshArrowProtection. Original gathers its live fighter data
        // without ForecastDestinationForIA, so resolving door exits here
        // would consume unrelated BuildingExitGate RNG merely because an
        // idle soldier reached its staggered periodic slot.
        let tick_data = if matches!(entity, Entity::Soldier(_)) {
            self.build_npc_tick_data_without_forecasts(sim, npc_id, &scratch, assets)
        } else {
            crate::ai::AiPerTickData::stub()
        };

        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("periodic NPC {} disappeared", npc_id.index()));
        let entity =
            self.world.entities.get(npc_id).unwrap_or_else(|| {
                panic!("periodic NPC {} disappeared before call", npc_id.index())
            });

        let mut ctx = build_ai_context_from_entity(
            entity,
            current_frame,
            building_sector,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);

        let entity =
            self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!("periodic NPC {} disappeared before call", npc_id.index())
            });

        match entity {
            Entity::Soldier(s) => {
                s.npc
                    .ai_brain
                    .enemy_mut()
                    .unwrap_or_else(|| {
                        panic!("periodic soldier {} has no enemy AI", npc_id.index())
                    })
                    .the_16th_frame(
                        sim,
                        frame_phase,
                        &ctx,
                        &self.ai.global,
                        &tick_data,
                        Some(&self.world.fast_grid),
                        is_idle,
                        receiving_wasp_sting,
                        stuck_command_active,
                        sequence_null_about_to_launch,
                    );
            }
            Entity::Civilian(c) => {
                c.npc
                    .ai_brain
                    .friendly_mut()
                    .unwrap_or_else(|| {
                        panic!("periodic civilian {} has no friendly AI", npc_id.index())
                    })
                    .the_16th_frame(
                        frame_phase,
                        &mut self.ai.global,
                        &ctx,
                        is_idle,
                        sequence_null_about_to_launch,
                    );
                // `tick_data` is only used for enemies; civilians
                // don't need it.
                let _ = &tick_data;
            }
            _ => unreachable!("post-detection owner must remain an NPC"),
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    /// Civilian `RandomSpeech(ubFramePhase)` call from NPC Hourglass.
    /// It sits before the lock gate and only acts at exact phase zero.
    pub(super) fn tick_civilian_random_speech_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;
        let entity = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("random-speech NPC {} disappeared", npc_id.index()));
        let Entity::Civilian(civilian) = entity else {
            return;
        };
        if npc_hourglass_frame_phase(current_frame, u32::from(civilian.npc.register_number)) != 0 {
            return;
        }

        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let building_sector = self.entity_building_sector(entity.element_data().sector());
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "random-speech NPC {} disappeared before call",
                npc_id.index()
            )
        });
        let ctx = build_ai_context_from_entity(
            entity,
            current_frame,
            building_sector,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        entity
            .friendly_ai_mut()
            .unwrap_or_else(|| panic!("civilian {} has no friendly AI", npc_id.index()))
            .random_speech(sim, 0, &ctx);
        // Original RandomSpeech calls Say synchronously before the following
        // NPC lock gate. Rust's AI borrow records Say in owner_work, so close
        // that same owner-local boundary here even when the lock gate will
        // short-circuit the remainder of Hourglass.
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    // ── RefreshAmbushPoints — per-frame ambush peek scan ─────────
    //
    // `refresh_ambush_points` runs every frame for each NPC from
    // `hourglass`.  Civilians have a no-op virtual stub, so this only
    // fires for enemies (soldiers).  The per-NPC method updates the
    // slot status vector and may transition the AI substate via
    // `check_ambush_point`.

    pub(super) fn tick_refresh_ambush_points_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        if self.actors_frozen() {
            return;
        }
        if self.ai.global.ambush_points.is_empty() {
            return;
        }

        // Civilian RefreshAmbushPoints is the Original virtual no-op. Check
        // that before scratch construction, which can draw BuildingExitGate
        // RNG while forecasting unrelated door-passing actors.
        let owner = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("ambush-refresh NPC {} disappeared", npc_id.index()));
        if matches!(owner, Entity::Civilian(_)) {
            return;
        }
        assert!(
            owner.enemy_ai().is_some(),
            "soldier {} has no enemy AI for ambush refresh",
            npc_id.index()
        );
        let scratch = self.build_owner_context_scratch_without_forecast(assets);

        let frame = self.control.frame_counter;
        let is_forest_level = self.world.weather.is_forest_level;
        let ambiance = self.world.weather.ambiance;
        let standard_view_polygon_radius = self.ai.standard_view_polygon_radius;
        // Phase 1: read-only — gather context + eyes point + LOS scope.
        let (ctx, eyes) = {
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("ambush-refresh NPC {} disappeared", npc_id.index()));
            assert!(
                entity.enemy_ai().is_some(),
                "soldier {} has no enemy AI for ambush refresh",
                npc_id.index()
            );
            let eyes = entity.compute_eyes_point(None).unwrap_or_else(|| {
                panic!(
                    "soldier {} has no eye point for ambush refresh",
                    npc_id.index()
                )
            });
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let ctx = build_ai_context_from_entity(
                entity,
                frame,
                building_sector,
                is_forest_level,
                ambiance,
                standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            (ctx, eyes)
        };

        // Build the obstacle view from individual disjoint fields
        // so the borrow checker can split it from the mut borrow
        // on `self.world.entities` below.
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let ambush_points = self.ai.global.ambush_points.as_slice();

        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "ambush-refresh NPC {} disappeared before apply",
                npc_id.index()
            )
        });
        entity
            .enemy_ai_mut()
            .unwrap_or_else(|| panic!("soldier {} lost enemy AI", npc_id.index()))
            .refresh_ambush_points(&ctx, eyes, ambush_points, sight_obstacles);
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    // ── Macro timer hourglass ────────────────────────────────────
    //
    // `hourglass` polls `macro_timer_is_running` each frame and, when
    // the timer has rung and the NPC is still in
    // `SUBSTATE_DEFAULT_INMACRO`, calls `execute_next_macro_command(sim, )`
    // directly — **bypassing** the Think stimulus dispatch so
    // CMD_WAIT / CMD_BEND resume without going through EVENT_TIMER.
    //
    // We iterate both soldier and civilian NPCs because civilians use
    // the common macro opcodes too (REVERSE_PATH, WAIT, GOTO_POINT,
    // FACE_TO, ...).
    pub(super) fn tick_ai_macro_timer_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        let current_frame = self.control.frame_counter;

        // Read macro-timer state without holding a borrow. The original stops
        // an elapsed macro timer even outside DefaultInMacro; only execution
        // is substate-gated.
        let (fire, execute) = {
            let entity = self
                .world
                .entities
                .get(npc_id)
                .unwrap_or_else(|| panic!("macro-timer NPC {} disappeared", npc_id.index()));
            let ai = entity.ai_controller().unwrap_or_else(|| {
                panic!("macro-timer NPC {} has no AI controller", npc_id.index())
            });
            let fire = ai.macro_timer_is_running && ai.when_does_macro_timer_ring <= current_frame;
            (
                fire,
                fire && ai.current_substate == crate::ai::Substate::DefaultInMacro,
            )
        };
        if !fire {
            return;
        }

        let scratch = self.build_owner_context_scratch_without_forecast(assets);

        // Build the AI context before we take the mut AI borrow.
        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("macro-timer NPC {} disappeared", npc_id.index()));
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "macro-timer NPC {} disappeared before execute",
                npc_id.index()
            )
        });
        let mut ctx = build_ai_context_from_entity(
            entity,
            current_frame,
            building_sector,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);

        // Stop the timer and resume the macro VM.  `execute_next_
        // macro_command` may transition the substate (e.g. to
        // `DefaultEnroute` when the byte stream ends) — we don't
        // post-process beyond that; any downstream state changes
        // ride the normal think dispatch.
        let base = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| {
                panic!(
                    "macro-timer NPC {} disappeared before execute",
                    npc_id.index()
                )
            })
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("macro-timer NPC {} lost its AI controller", npc_id.index()));
        base.macro_timer_is_running = false;
        if execute {
            base.execute_next_macro_command(sim, &ctx);
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }

    // ── Locked-frame timer bumps ─────────────────────────────────
    //
    // `hourglass` short-circuits the post-Refresh tail when any lock
    // is held (`locks_flag_field > 0 || script_locked || frozen_all`)
    // but still bumps `when_does_timer_ring`,
    // `when_does_macro_timer_ring`, and `emoticon_expiration_date`
    // per locked frame.  Without this, the per-piece tick guards
    // skip everything (no bumps), so ring-times shift -N once the
    // lock clears — a script-locked civilian's EVENT_TIMER would
    // fire immediately on unlock instead of N frames later.
    //
    // The decision returned here is the one and only lock sample for this
    // owner suffix. Once it is false, later The16thFrame/Think side effects
    // may acquire locks or FrozenAll without suppressing the already-entered
    // normal timer, macro timer, or emoticon phases. Only the retained FIFO
    // intentionally samples AI/script locks again before every item.
    /// Original `GetDeafness()` call immediately after
    /// `RefreshAmbushPoints`. This runs for every non-frozen owner even when
    /// acoustic detection's staggered cadence did not open this frame.
    pub(super) fn tick_npc_refresh_deafness_for_npc(&mut self, npc_id: EntityId) {
        if self.actors_frozen() {
            return;
        }
        let (position, elevation) = {
            let entity =
                self.world.entities.get(npc_id).unwrap_or_else(|| {
                    panic!("deafness-refresh NPC {} disappeared", npc_id.index())
                });
            assert!(
                entity.npc_data().is_some(),
                "deafness-refresh owner {} has no NPC data",
                npc_id.index()
            );
            (
                entity.element_data().position_map(),
                entity.element_data().position().z,
            )
        };
        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(position.x, position.y, elevation);
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "deafness-refresh NPC {} disappeared before apply",
                npc_id.index()
            )
        });
        entity
            .npc_data_mut()
            .unwrap_or_else(|| panic!("deafness-refresh owner {} lost NPC data", npc_id.index()))
            .get_deafness(self.control.frame_counter, cover_volume);
    }

    pub(super) fn tick_npc_lock_gate_for_npc(&mut self, npc_id: EntityId) -> bool {
        let frozen = self.actors_frozen();
        let entity = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| panic!("lock-gate NPC {} disappeared", npc_id.index()));
        let ai = entity
            .ai_controller_mut()
            .unwrap_or_else(|| panic!("lock-gate NPC {} has no AI controller", npc_id.index()));
        let locked = frozen || !ai.locks_flag_field.is_empty() || ai.script_locked;
        if locked {
            // C++ UDWORD `++` wraps. Saturation would pin a deadline forever
            // after one overflow and break the later elapsed checks.
            ai.when_does_timer_ring = ai.when_does_timer_ring.wrapping_add(1);
            ai.when_does_macro_timer_ring = ai.when_does_macro_timer_ring.wrapping_add(1);
            ai.emoticon_expiration_date = ai.emoticon_expiration_date.wrapping_add(1);
        }
        locked
    }

    pub(super) fn tick_npc_emoticon_expiration_for_npc(&mut self, npc_id: EntityId) {
        let current_frame = self.control.frame_counter;
        let entity = self
            .world
            .entities
            .get_mut(npc_id)
            .unwrap_or_else(|| panic!("emoticon-expiry NPC {} disappeared", npc_id.index()));
        let ai = entity.ai_controller_mut().unwrap_or_else(|| {
            panic!(
                "emoticon-expiry NPC {} has no AI controller",
                npc_id.index()
            )
        });
        if ai.emoticon_has_expiration_date && ai.emoticon_expiration_date <= current_frame {
            ai.set_emoticon(crate::ai::EmoticonType::None);
            assert!(!ai.emoticon_has_expiration_date);
        }
    }

    // ── Stuck-on-ladder emergency counter ────────────────────────
    //
    // `hourglass` bumps `stuck_on_ladder_emergency_counter` every
    // frame an NPC is on a ladder in a non-building sector with
    // command `Wait`/`MoveWaiting` and not script-locked; otherwise
    // resets to 0.  After 25 frames it calls `force_return_to_duty()`
    // (== `return_to_duty(sim, )`) and resets the counter so
    // outdoor-ladder hangs self-recover.
    //
    // Note: this checks only `script_locked`, *not* `locks_flag_field`
    // — so the freshly-set BUSY lock from the edge detector earlier in
    // the same frame does not suppress this counter (the BUSY lock is
    // exactly what we want to escape from).
    pub(super) fn tick_npc_stuck_on_ladder_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        // Snapshot the gating predicates without holding a borrow.
        let entity = self
            .world
            .entities
            .get(npc_id)
            .unwrap_or_else(|| panic!("ladder-tail NPC {} disappeared", npc_id.index()));
        let on_ladder = entity.element_data().posture == crate::element::Posture::OnLadder;
        let cmd = self.actor_command(npc_id);
        let in_wait_or_move_waiting = matches!(
            cmd,
            crate::element::Command::Wait | crate::element::Command::MoveWaiting
        );
        let script_locked = entity
            .ai_controller()
            .unwrap_or_else(|| panic!("ladder-tail NPC {} has no AI", npc_id.index()))
            .script_locked;
        let in_building = self.entity_data_in_building_sector(entity.element_data());
        let qualifies = on_ladder && in_wait_or_move_waiting && !script_locked && !in_building;

        // Bump or reset the counter; remember whether to fire.
        let trigger = {
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "ladder-tail NPC {} disappeared before counter",
                    npc_id.index()
                )
            });
            let npc = entity
                .npc_data_mut()
                .unwrap_or_else(|| panic!("ladder-tail owner {} has no NPC data", npc_id.index()));
            if qualifies {
                npc.stuck_on_ladder_emergency_counter =
                    npc.stuck_on_ladder_emergency_counter.saturating_add(1);
                if npc.stuck_on_ladder_emergency_counter > 25 {
                    npc.stuck_on_ladder_emergency_counter = 0;
                    true
                } else {
                    false
                }
            } else {
                npc.stuck_on_ladder_emergency_counter = 0;
                false
            }
        };
        if !trigger {
            return;
        }

        // `force_return_to_duty == return_to_duty`.  Dispatch via
        // the AI subclass to mirror the virtual call.  Build the
        // ctx + tick data the way `tick_periodic_ai` does.
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
        let frame = self.control.frame_counter;
        let in_uninterruptible_command = self.is_very_very_busy(npc_id);
        let building_sector = self
            .world
            .entities
            .get(npc_id)
            .map(|entity| self.entity_building_sector(entity.element_data().sector()))
            .unwrap_or_else(|| panic!("ladder-tail NPC {} disappeared", npc_id.index()));
        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "ladder-tail NPC {} disappeared before recovery",
                npc_id.index()
            )
        });
        let mut ctx = build_ai_context_from_entity(
            entity,
            frame,
            building_sector,
            self.world.weather.is_forest_level,
            self.world.weather.ambiance,
            self.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &self.world.fast_grid,
            &assets.hiking_paths,
            &self.ai.global.all_soldier_handles,
            self.control.sim_config.difficulty,
        );
        ctx.in_uninterruptible_command = in_uninterruptible_command;
        self.refresh_selected_default_wait_identity(npc_id, &mut ctx);
        let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
            panic!(
                "ladder-tail NPC {} disappeared before recovery",
                npc_id.index()
            )
        });
        match entity {
            Entity::Soldier(s) => {
                s.npc
                    .ai_brain
                    .enemy_mut()
                    .unwrap_or_else(|| {
                        panic!("ladder-tail soldier {} has no enemy AI", npc_id.index())
                    })
                    .return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx, &tick_data);
            }
            Entity::Civilian(c) => {
                c.npc
                    .ai_brain
                    .friendly_mut()
                    .unwrap_or_else(|| {
                        panic!("ladder-tail civilian {} has no friendly AI", npc_id.index())
                    })
                    .return_to_duty(sim, crate::ai::DutyFlags::empty(), &ctx);
            }
            _ => unreachable!("post-detection owner must remain an NPC"),
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
    }
}
