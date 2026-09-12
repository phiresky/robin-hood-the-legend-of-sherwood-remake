//! Per-tick seek-refresh scan.
//!
//! ## Semantics
//!
//! A seek arms its `seek_refresh_wait` countdown to `TIME_SEEK_REFRESH`
//! (=25) at launch and again at the tail of each refresh. Entity-target
//! Seeking decrements it at the owner movement boundary unless an
//! in-range post-seek interaction returns first. Once it is zero AND the
//! target has moved more than 10 units (maximum-axis norm) since the last launch,
//! this pre-owner scan rebuilds a fresh single-element seek sequence
//! bound to the target's *current* position, sets the previous movement
//! element to interrupted, and launches the new sequence at info
//! priority. Zero and wrapped high-bit counter values are considered expired.
//!
//! Runs once per tick for every actor holding an `InProgress` movement
//! element with a populated `element` (target) field and the
//! `MoveFlags::SEEK` bit set.  Initial user-facing seeks arrive as
//! `Command::Seek`; gate-expanded movement-sequence seek
//! legs arrive as `Command::Move` with the same flag/target pair.  The
//! entity-target seeks share a single destination resolver: `USE_POINT`
//! seeks go to the target's current point, moving actor targets adjust
//! tolerance/speed by chase speed, `SEEK_SHIELD` aims at the protected
//! side point, and `SEEK_STOP_NPC` keeps the distance gate.
//!
//! The point-target overload runs at Seek translation only. A goal sector
//! that differs from the actor's own goes through
//! [`EngineInner::try_dispatch_cross_sector_point_seek`], which expands the
//! route across gates; anything else keeps the flat interrupt-and-relaunch
//! primitive.

use super::movement::GoalShape;
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{ActionState, Entity, EntityId};
use crate::engine::LevelAssets;
use crate::order::OrderType;
use crate::sequence::{
    CascadeFlags, MoveFlags, Sequence, SequenceElement, SequenceElementData, SequenceId,
};
use crate::sprite::MotionState;
use serde::{Deserialize, Serialize};

/// Identity and movement policy shared by initial entity-seek lowering and
/// subsequent refreshes. This is data only: callers retain simulation/assets
/// borrows, and dispatch still resolves the target at its live owner slot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct EntitySeekRequest {
    pub(super) owner: EntityId,
    pub(super) sequence_id: SequenceId,
    pub(super) element_index: usize,
    pub(super) target: EntityId,
    pub(super) action: OrderType,
    pub(super) flags: MoveFlags,
}

/// An admitted point-seek route attempt. Recorded outcomes travel by value so
/// the dispatcher consumes exactly the evidence supplied by this attempt.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PointSeekRequest {
    pub(super) owner: EntityId,
    pub(super) sequence_id: SequenceId,
    pub(super) element_index: usize,
    pub(super) destination: MapPoint,
    pub(super) goal_sector: Option<crate::position_interface::SectorHandle>,
    pub(super) goal_layer: u16,
    pub(super) action: OrderType,
    pub(super) flags: MoveFlags,
    pub(super) seek_distance: f32,
    pub(super) recorded_gate_path: Option<crate::gate::RecordedGatePath>,
    pub(super) route_provenance: crate::sequence::PointSeekRouteProvenance,
}

#[inline]
fn seek_refresh_wait_elapsed(wait: u32) -> bool {
    (wait as i32) <= 0
}

/// Original-game gate routing compares the sector stored by each position.
/// Public sector numbers are only a compatibility identity when neither
/// position has been resolved to a live arena object.
#[inline]
fn seek_sectors_match(
    source: crate::position_interface::SectorHandle,
    goal: crate::position_interface::SectorHandle,
) -> bool {
    match (source.arena_index(), goal.arena_index()) {
        (Some(source), Some(goal)) => source == goal,
        (None, None) => source == goal,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// Validate and recover an Original-recorded gate-search outcome. The outer
/// `None` means there is no replay override and the caller must run live A*;
/// `Some(None)` preserves an observed original-game search failure.
fn recorded_point_seek_gate_path(
    doors: &[crate::gate::Door],
    source_sector: crate::position_interface::SectorHandle,
    source_layer: u16,
    goal_sector: crate::position_interface::SectorHandle,
    recorded: Option<crate::gate::RecordedGatePath>,
) -> Option<Option<Vec<crate::gate::GatePathStep>>> {
    let recorded = recorded?;
    validate_recorded_point_seek_source(&recorded, source_sector, source_layer);
    let crate::gate::RecordedGateOutcome::Success(path) = recorded.outcome else {
        return Some(None);
    };
    assert!(
        !path.is_empty(),
        "recorded cross-sector point Seek has an empty successful gate path"
    );
    let mut expected_entry = source_sector;
    for step in &path {
        let door = doors.get(usize::from(step.door_index)).unwrap_or_else(|| {
            panic!(
                "recorded point-Seek gate {} is absent from the Rust mission",
                step.door_index
            )
        });
        let (entry_number, entry_index, exit_number, exit_index) = if step.direct {
            (
                door.sector_out,
                door.sector_out_index,
                door.sector_in,
                door.sector_in_index,
            )
        } else {
            (
                door.sector_in,
                door.sector_in_index,
                door.sector_out,
                door.sector_out_index,
            )
        };
        let mut entry = crate::position_interface::SectorHandle::new(u16::from(entry_number))
            .unwrap_or_else(|| {
                panic!(
                    "recorded point-Seek gate {} has invalid entry sector {entry_number}",
                    step.door_index
                )
            });
        if let Some(index) = entry_index {
            entry = entry.with_arena_index(index);
        }
        assert!(
            seek_sectors_match(expected_entry, entry),
            "recorded point-Seek gate {} does not continue from sector {:?}",
            step.door_index,
            expected_entry,
        );
        let mut exit = crate::position_interface::SectorHandle::new(u16::from(exit_number))
            .unwrap_or_else(|| {
                panic!(
                    "recorded point-Seek gate {} has invalid exit sector {exit_number}",
                    step.door_index
                )
            });
        if let Some(index) = exit_index {
            exit = exit.with_arena_index(index);
        }
        expected_entry = exit;
    }
    assert!(
        seek_sectors_match(expected_entry, goal_sector),
        "recorded point-Seek gate path ends in {expected_entry:?}, not goal {goal_sector:?}"
    );
    Some(Some(path))
}

fn validate_recorded_point_seek_source(
    recorded: &crate::gate::RecordedGatePath,
    source_sector: crate::position_interface::SectorHandle,
    source_layer: u16,
) {
    assert_eq!(
        u16::from(recorded.source_sector),
        u16::from(source_sector),
        "recorded point-Seek route public source sector differs at dispatch"
    );
    assert_eq!(
        recorded.source_sector_index,
        source_sector.arena_index(),
        "recorded point-Seek route exact source sector differs at dispatch"
    );
    assert_eq!(
        recorded.source_layer, source_layer,
        "recorded point-Seek route source layer differs at dispatch"
    );
}

#[allow(clippy::too_many_arguments)]
fn find_seek_gate_path(
    doors: &[crate::gate::Door],
    source: MapPoint,
    source_sector: crate::position_interface::SectorHandle,
    goal: MapPoint,
    goal_sector: crate::position_interface::SectorHandle,
    auth: Option<&crate::gate::ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(crate::sector::SectorNumber) -> bool,
    sector_lift_type: &impl Fn(crate::sector::SectorNumber) -> Option<crate::sector::LiftType>,
) -> Option<Vec<crate::gate::GatePathStep>> {
    crate::gate::find_path_gates_with_sector_indices(
        doors,
        (source.x, source.y),
        u16::from(source_sector),
        source_sector.arena_index(),
        (goal.x, goal.y),
        u16::from(goal_sector),
        goal_sector.arena_index(),
        auth,
        allow_leave_map,
        building_is_authorized,
        sector_lift_type,
    )
}

/// Entity seeking terminates immediately when the actor has no seek target.
/// The seek target lives on the actor, not on the sequence element:
/// starting the post-seek sequence clears
/// the actor's seek target while the element keeps its own target reference, so
/// a later Move|SEEK leg of the same route still looks like an entity seek to
/// the element but is already finished as far as seeking is concerned.
/// Execution then terminates before computing any distance,
/// before aging the wait timer, and before seek refresh.
///
/// Returns `true` when this owner's selected movement is in that state.
pub(super) fn perform_seek_lost_actor_target(
    engine: &mut super::EngineInner,
    owner: EntityId,
    selected: super::movement::MovementOwnerSelection,
) -> bool {
    let Some(actor) = engine
        .get_entity(owner)
        .and_then(|entity| entity.actor_data())
    else {
        return false;
    };
    // The seek-to-point flag picks the point branch, which has no null-target guard.
    if actor.continuation.seek_to_point || actor.seek_target.is_some() {
        return false;
    }
    let Some(element) = engine
        .orders
        .sequence_manager
        .get_element(selected.seq_id, selected.elem_idx)
    else {
        return false;
    };
    let SequenceElementData::Movement { flags, .. } = &element.data else {
        return false;
    };
    if !flags.contains(MoveFlags::SEEK) {
        return false;
    }
    // The seek wrapper is chosen per animation arm, not per element: wall and
    // ladder orders keep the SEEK flag while their Execute arms call
    // motion processing directly and never reach this guard.
    let Some(order_type) = element
        .current_order()
        .map(|order| order.order_type)
        .filter(|order| super::movement::perform_seek_calls_per_execute(*order) > 0)
    else {
        return false;
    };

    // Seeking's absent-target guard is only the nested motion producer.
    // The surrounding Execute switch still receives TERMINATED and applies
    // the selected animation branch's state effect before the actor update runs
    // advancing to the next order. Returning from
    // the split Rust owner envelope without this step left a finished running
    // stop transition in MovingFast even after its MoveOk element retired.
    if let Some((posture, action_state)) =
        super::movement::movement_execute_state_effect(order_type, MotionState::Terminated)
    {
        let entity = engine
            .get_entity_mut(owner)
            .unwrap_or_else(|| panic!("lost-target seeking owner {owner:?} disappeared"));
        entity.set_posture(posture);
        entity
            .actor_data_mut()
            .expect("lost-target seeking owner is not an actor")
            .action_state = action_state;
    }
    true
}

/// Actor seeking tests the live target tolerance before its
/// moved-target refresh arm.  Keep the same test at the pre-owner refresh
/// seam so an expired stale route cannot replace a seek whose target has
/// already entered interaction range.
fn entity_seek_live_tolerance_reached(
    owner: &Entity,
    target: &Entity,
    flags: MoveFlags,
    seek_distance: f32,
) -> bool {
    // The original game's shield-seeking arm is intentionally different: it tests
    // moved-target refresh before computing distance to the saved shield
    // destination.
    if seek_distance <= 0.0 || flags.contains(MoveFlags::SEEK_SHIELD) {
        return false;
    }
    let owner_position = owner.element_data().position_map();
    let owner_sector = owner.element_data().sector();
    let target_position = target.element_data().position_map();
    let target_sector = target.element_data().sector();
    if owner_sector != target_sector {
        return false;
    }

    let target_point = if flags.contains(MoveFlags::USE_POINT) {
        target
            .current_gameplay_point_map()
            .filter(|point| *point != target_position)
            .unwrap_or(target_position)
    } else {
        target_position
    };
    let delta = target_point - owner_position;
    let dy = if flags.contains(MoveFlags::DIRECTIONAL_TOLERANCE) {
        delta.y * 1.743_446_8
    } else {
        delta.y
    };
    delta.x * delta.x + dy * dy < seek_distance * seek_distance * 1.1025
}

pub(crate) struct ResolvedEntitySeek {
    pub(crate) destination: MapPoint,
    pub(crate) tolerance: f32,
    pub(crate) speed_factor: f32,
}

impl crate::engine::EngineInner {
    /// Execute the original game's explicit seek-refresh hold.
    ///
    /// Unlike ordinary movement arms, this order has no countdown or
    /// target-displacement gate. It does retain the entity-arm null-target
    /// sentinel: post-seek startup clears the actor's seek target
    /// while the route element keeps its stale target reference, and
    /// execution terminates instead of refreshing in that state. It is installed when a
    /// final Move|SEEK reaches a building and executes on the actor's following
    /// update slot.
    pub(super) fn tick_refreshing_seek_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<MotionState> {
        let (seq_id, elem_idx) = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)?;
        let (action, flags) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| {
                let order = element.current_order()?;
                if order.order_type != crate::order::OrderType::RefreshingSeek {
                    return None;
                }
                let SequenceElementData::Movement {
                    action,
                    flags,
                    ..
                } = &element.data
                else {
                    panic!(
                        "RefreshingSeek owner {owner:?} selected non-movement element {seq_id:?}/{elem_idx}"
                    )
                };
                Some((*action, *flags))
            })?;

        let actor = self
            .get_entity(owner)
            .and_then(|entity| entity.actor_data())
            .unwrap_or_else(|| panic!("RefreshingSeek owner {owner:?} is not an actor"));
        let target = if actor.continuation.seek_to_point {
            None
        } else {
            actor.seek_target
        };
        let Some(target) = target else {
            // The point-target arm and an entity arm whose post-seek handoff
            // cleared seek target both return TERMINATED without refreshing.
            // The actor update owns the subsequent order advance; retiring the
            // element here would skip that base completion boundary and lose
            // a synchronously exposed successor.
            return Some(MotionState::Terminated);
        };
        let target_position = self
            .get_entity(target)
            .unwrap_or_else(|| {
                panic!("RefreshingSeek owner {owner:?} requires missing target {target:?}")
            })
            .element_data()
            .position_map();
        self.apply_seek_refresh(
            sim,
            assets,
            EntitySeekRequest {
                owner,
                sequence_id: seq_id,
                element_index: elem_idx,
                target,
                action,
                flags,
            },
            target_position,
        );
        Some(MotionState::InProgress)
    }

    /// Prepare a selected seek for the terminal state used by the original
    /// game's seek refresh. The original game sends the actor's condolence card
    /// synchronously before launching any replacement, so the still-selected
    /// Seeking clears the map goal. Rust queues that card and eagerly
    /// detaches active mechanics; preserve the same observable cleanup at the
    /// exact seek-refresh boundary.
    fn stop_selected_seek_for_refresh(&mut self, owner: EntityId) {
        // The initial Translate(SEEK) wrapper is semantically selected in
        // Original before it reaches seek refresh, but Rust deliberately
        // leaves ordered elements Todo until concrete movement dispatch.
        // Therefore manager `current_element_for_actor` cannot recognize the
        // initial wrapper here. Callers have already identified the selected
        // Seek; this cleanup only clears its owner's active mechanics.
        let frame = self.control.frame_counter;
        if let Some(entity) = self.get_entity_mut(owner) {
            tracing::trace!(
                target: "parity_owner_handoff",
                frame,
                ?owner,
                goal = ?entity.position_iface().map_goal(),
                "refresh seek clearing selected movement goal"
            );
            entity.position_iface_mut().set_map_goal(MapPoint::ZERO);
        }
        self.stop_owner_active_mechanics(owner);
    }

    /// Resolve the destination/tolerance/speed tuple for an entity-target
    /// seek.  Handles USE_POINT, moving-target chase speed, shield-danger
    /// offset, synchronous SEEK_STOP_NPC, and authorized-position snapping.
    pub(crate) fn resolve_entity_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
        flags: MoveFlags,
        seek_distance: f32,
    ) -> Option<ResolvedEntitySeek> {
        let owner_entity = self.get_entity(owner)?;
        let target_entity = self.get_entity(target)?;
        let target_elem = target_entity.element_data();
        let target_pos = target_elem.position_map();
        let target_layer = target_elem.layer();

        let owner_move_box = *owner_entity.position_iface().get_move_box();

        if flags.contains(MoveFlags::USE_POINT) {
            // The original game's current map point starts at integer
            // sprite origin (`floor(position_map - center)`) and
            // adds the current row's hotspot. It disables USE_POINT only
            // when that complete map-space point equals the map position; a zero
            // local hotspot alone is not the disabling condition.
            let current_point = target_entity
                .current_gameplay_point_map()
                .filter(|point| *point != target_pos)
                .unwrap_or(target_pos);
            let mut target_box = owner_move_box.translated(current_point);
            if self.world.fast_grid.find_authorized_position_toward(
                &mut target_box,
                target_pos,
                target_layer,
            ) {
                return Some(ResolvedEntitySeek {
                    destination: target_box.center(),
                    tolerance: seek_distance,
                    speed_factor: 1.0,
                });
            }
            tracing::warn!(
                ?owner,
                ?target,
                "resolve_entity_seek: USE_POINT target has no authorized position"
            );
            return None;
        }

        let (mut tolerance, speed_factor, send_stop_sqr) = if let Some(target_actor) =
            target_entity.actor_data()
        {
            let owner_state = owner_entity
                .actor_data()
                .map(|a| a.action_state)
                .unwrap_or(ActionState::Waiting);
            let (chase_speed, walking_behind_running_enemy) = match target_actor.action_state {
                ActionState::MovingFast => match owner_state {
                    ActionState::MovingFast => (ActionState::MovingFast, false),
                    ActionState::Moving => (ActionState::Moving, true),
                    _ => (ActionState::Waiting, false),
                },
                ActionState::Moving => match owner_state {
                    ActionState::MovingFast | ActionState::Moving => (ActionState::Moving, false),
                    _ => (ActionState::Waiting, false),
                },
                _ => (ActionState::Waiting, false),
            };
            match chase_speed {
                ActionState::MovingFast => (1.0, 1.2, seek_distance * seek_distance * 9.0),
                ActionState::Moving => (
                    seek_distance / 2.0,
                    if walking_behind_running_enemy {
                        1.0
                    } else {
                        1.2
                    },
                    seek_distance * seek_distance * 4.0,
                ),
                _ => (seek_distance, 1.0, -1.0),
            }
        } else {
            (seek_distance, 1.0, -1.0)
        };

        let stop_npc = if flags.contains(MoveFlags::SEEK_STOP_NPC)
            && send_stop_sqr > 0.0
            && target_entity.npc_data().is_some()
        {
            // Seek refresh uses the stored 3D
            // world positions here, not their projected map positions.
            // In particular, actors on different elevations can be close in
            // map space while remaining far outside the stop radius.
            let owner_pos = owner_entity.element_data().position();
            let target_pos = target_entity.element_data().position();
            let dx = target_pos.x - owner_pos.x;
            let dy = target_pos.y - owner_pos.y;
            let dz = target_pos.z - owner_pos.z;
            dx * dx + dy * dy + dz * dz < send_stop_sqr
        } else {
            false
        };

        // Original computes the chase speed/tolerance and distance gate from
        // the target's pre-event moving state, then calls
        // sends the target `EVENT_STOP` synchronously, and only afterwards
        // samples the destination and authorizes/builds the replacement.
        if stop_npc {
            self.send_seek_stop_to_npc(sim, assets, target);
        }

        let owner_entity = self.get_entity(owner)?;
        let target_entity = self.get_entity(target)?;
        let target_elem = target_entity.element_data();
        let target_pos = target_elem.position_map();
        let target_layer = target_elem.layer();

        let mut destination = target_pos;
        if owner_entity.is_pc() && flags.contains(MoveFlags::SEEK_SHIELD) {
            let danger = owner_entity
                .pc_data()
                .map(|pc| pc.shield_danger_point)
                .unwrap_or_default();
            let protected_elevation = target_entity.position_iface().get_elevation();
            let vx = danger.x - target_pos.x;
            let vy = (danger.y - protected_elevation) - target_pos.y;
            let len = (vx * vx + vy * vy).sqrt();
            if len > f32::EPSILON {
                destination = target_pos + MapVec::new(vx / len * 50.0, vy / len * 50.0);
            } else {
                tolerance = seek_distance;
            }
        }

        let mut target_box = owner_move_box.translated(destination);
        if self.world.fast_grid.find_authorized_position_toward(
            &mut target_box,
            target_pos,
            target_layer,
        ) {
            Some(ResolvedEntitySeek {
                destination: target_box.center(),
                tolerance,
                speed_factor,
            })
        } else {
            tracing::warn!(
                ?owner,
                ?target,
                "resolve_entity_seek: target has no authorized seek position"
            );
            None
        }
    }

    /// Refresh one selected entity-target seek at its live actor update
    /// slot.
    ///
    /// The original game evaluates this inside actor seeking, so a
    /// target with an earlier creation slot has already moved this frame,
    /// while a later target has not. Returns `true` when seek refresh replaced
    /// the selected movement; its old Execute arm must then return without
    /// moving the replacement in the same owner slot.
    pub(super) fn tick_refresh_seek_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        let Some(refresh) = self.selected_seek_refresh_decision(owner) else {
            return false;
        };
        let (seq_id, elem_idx, target, action, flags, new_target_pos) = refresh;
        tracing::trace!(
            ?owner,
            ?target,
            new_x = new_target_pos.x,
            new_y = new_target_pos.y,
            "owner-slot seek target moved >10u; re-launching seek",
        );
        // Seeking restores the actor's moving state immediately before
        // seek refresh. This is observable on the refresh frame even though
        // the replacement path does not execute until a later owner slot.
        if let Some(actor) = self.get_entity_mut(owner).and_then(|e| e.actor_data_mut()) {
            actor.action_state = actor.action_state.set_moving(false, false);
        }
        self.apply_seek_refresh(
            sim,
            assets,
            EntitySeekRequest {
                owner,
                sequence_id: seq_id,
                element_index: elem_idx,
                target,
                action,
                flags,
            },
            new_target_pos,
        );
        true
    }

    /// Decide whether the actor seek's moved-target branch
    /// fires for this owner's selected seek, without
    /// applying it.
    ///
    /// Split out of [`Self::tick_refresh_seek_for_owner`] so the sword- and
    /// shield-walking execution arms can run their pre-seeking facing
    /// prologue on exactly the frames that branch preempts the motion.
    #[allow(clippy::type_complexity)]
    pub(super) fn selected_seek_refresh_decision(
        &self,
        owner: EntityId,
    ) -> Option<(
        crate::sequence::SequenceId,
        usize,
        EntityId,
        crate::order::OrderType,
        MoveFlags,
        crate::coordinates::MapPoint,
    )> {
        let entity = self.get_entity(owner)?;
        let actor = entity.actor_data()?;
        let seq_id = actor.active_movement.sequence_id?;
        let elem_idx = actor.active_movement.element_index;
        if self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            != Some((seq_id, elem_idx))
        {
            return None;
        }
        let elem = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
        if !matches!(
            elem.command,
            crate::element::Command::Move
                | crate::element::Command::MoveOk
                | crate::element::Command::Seek
        ) {
            return None;
        }
        let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
            return None;
        };
        if !flags.contains(MoveFlags::SEEK) {
            return None;
        }
        // Original evaluates moved-target refresh only inside
        // actor seeking. The SEEK flag remains attached to
        // cross-sector wall and ladder orders, but their Execute arms call
        // motion processing directly and must neither refresh nor consume the
        // route-construction RNG draws. Sampling solely from the element
        // flags made a climbing PC rebuild a cross-building chase while
        // Original kept climbing.
        let order = elem.orders.front()?;
        if super::movement::perform_seek_calls_per_execute(order.order_type) == 0 {
            return None;
        }
        // Seek handling follows the actor-owned seek target, not the target
        // pointer stored on the selected movement element. Competing seeks
        // can replace the actor slot while an older route remains selected;
        // Original deliberately keeps following that live owner target until
        // the route is interrupted or the next seek refresh replaces it.
        let target_id = actor.seek_target?;
        let target_entity = self.get_entity(target_id)?;
        let target_pos = target_entity.element_data().position_map();

        // Seeking's same-sector distance gate precedes its expired
        // timer / moved-target test. In particular, a target can enter
        // range while this route still points at its old position; the
        // ensuing owner Execute must consume the post-seek handoff rather
        // than seek refresh replacing the route first.
        if entity_seek_live_tolerance_reached(entity, target_entity, *flags, actor.seek_distance) {
            return None;
        }

        // Zero and wrapped high-bit wait-timer values are expired. They
        // remain expired rather than delaying refresh for another 2^32
        // owner ticks.
        if !seek_refresh_wait_elapsed(actor.seek_refresh_wait) {
            return None;
        }
        let last = actor.last_seek_target_position;
        let dx = (target_pos.x - last.x).abs();
        let dy = (target_pos.y - last.y).abs();
        if dx.max(dy) <= 10.0 {
            return None;
        }

        Some((seq_id, elem_idx, target_id, *action, *flags, target_pos))
    }

    /// Per-entity body of `tick_refresh_seeks`: re-resolve the seek
    /// destination, build a fresh single-element seek sequence,
    /// stamp `last_seek_target_position`, and re-launch via
    /// [`Self::relaunch_seek_replacement`].  Honours the
    /// `SEEK_IN_BUILDINGS` short-circuit (teleport + post-seek).
    /// Extracted from the per-r loop above so the same dispatch is
    /// reusable from same-tick refresh callers (the
    /// transition-animation refresh check in
    /// [`EngineInner::process_per_tick_movement`]).
    pub(super) fn apply_seek_refresh(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        request: EntitySeekRequest,
        new_target_pos: crate::coordinates::MapPoint,
    ) {
        let EntitySeekRequest {
            owner,
            sequence_id: seq_id,
            element_index: elem_idx,
            target,
            action,
            flags,
        } = request;
        let seek_distance = self
            .get_entity(owner)
            .and_then(|entity| entity.actor_data())
            .map(|actor| actor.seek_distance)
            .filter(|distance| *distance > 0.0)
            .unwrap_or_else(|| {
                panic!(
                    "entity-target seek-refresh owner {owner:?} has no positive base seek distance"
                )
            });
        if self.try_handle_same_sector_actor_seek_wait(
            sim, assets, owner, seq_id, elem_idx, target, flags,
        ) {
            return;
        }

        // Original stamps these immediately after its same-sector early
        // returns and before deciding whether movement-sequence construction needs a
        // cross-sector door route. The route builder can synchronously
        // replace the selected element, so delaying this until the direct
        // same-sector path loses the observable TIME_SEEK_REFRESH value.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.last_seek_target_position = new_target_pos;
            actor.wait_time = 25;
            actor.seek_refresh_wait = 25;
        }

        if self.try_dispatch_cross_sector_entity_seek(sim, assets, request, seek_distance) {
            return;
        }

        let Some(resolved) =
            self.resolve_entity_seek(sim, assets, owner, target, flags, seek_distance)
        else {
            self.stop_selected_seek_for_refresh(owner);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return;
        };
        // Seek refresh's transient selected seek is replaced by the concrete
        // movement built by movement-sequence construction. Keep it as Move + SEEK
        // flags rather than another Seek wrapper, otherwise ordered dispatch
        // would recursively lower the replacement again.
        let mut new_elem =
            SequenceElement::new_movement(1, crate::element::Command::Move, Some(owner), action);
        if let SequenceElementData::Movement {
            flags: f,
            element,
            tolerance: t,
            speed_factor,
            destination,
            ..
        } = &mut new_elem.data
        {
            *f = flags;
            *element = Some(target);
            *t = resolved.tolerance;
            *speed_factor = resolved.speed_factor;
            *destination = resolved.destination;
        }

        self.relaunch_seek_replacement(owner, seq_id, elem_idx, new_elem);
    }

    /// Original-game seek refresh waits instead of rebuilding
    /// a path for two actor-target cases in the same sector:
    ///
    /// * actor and target are both inside a building, except that
    ///   `SEEK_IN_BUILDINGS` with a post-seek tail teleports to the
    ///   target and starts the tail;
    /// * target actor is currently passing a door, where the next
    ///   refresh should see the target's post-door sector/position.
    ///
    /// Returns `true` when seek refresh should stop without normal path
    /// re-resolution.
    pub(super) fn try_handle_same_sector_actor_seek_wait(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        target: EntityId,
        flags: MoveFlags,
    ) -> bool {
        let (owner_sector, target_sector, target_is_actor) =
            match self.get_entity(owner).zip(self.get_entity(target)) {
                Some((owner_e, target_e)) => (
                    owner_e.element_data().sector(),
                    target_e.element_data().sector(),
                    target_e.actor_data().is_some(),
                ),
                None => return false,
            };
        if !target_is_actor || owner_sector != target_sector {
            return false;
        }

        if self.sector_is_building(owner_sector) {
            let has_post_seek = self
                .get_entity(owner)
                .and_then(|e| e.actor_data())
                .is_some_and(|a| a.post_seek_sequence.is_some());
            if flags.contains(MoveFlags::SEEK_IN_BUILDINGS)
                && has_post_seek
                && let Some(pos) = self
                    .get_entity(target)
                    .map(|e| e.element_data().position_map())
                && let Some(owner_e) = self.get_entity_mut(owner)
            {
                owner_e.position_iface_mut().set_map_position(pos);
                self.start_post_seek_sequence(sim, assets, owner, Some((seq_id, elem_idx)));
            }
            return true;
        }

        self.get_entity(target)
            .and_then(|e| e.actor_data())
            .and_then(|a| {
                a.active_movement
                    .sequence_id
                    .map(|seq_id| (seq_id, a.active_movement.element_index))
            })
            .and_then(|(seq_id, elem_idx)| {
                self.orders.sequence_manager.get_element(seq_id, elem_idx)
            })
            .is_some_and(|elem| elem.command == crate::element::Command::PassDoor)
    }

    /// Central entity-target Seek lowering, matching original
    /// seek refresh followed by movement-sequence construction for cross-sector targets.
    ///
    /// Returns `true` when the current Seek was fully consumed: either
    /// replaced by a gate/lift/jump traversal sequence or marked
    /// impossible after an authorized-position / gate-path failure.
    pub(super) fn try_dispatch_cross_sector_entity_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        request: EntitySeekRequest,
        seek_distance: f32,
    ) -> bool {
        let EntitySeekRequest {
            owner,
            sequence_id: seq_id,
            element_index: elem_idx,
            target,
            action,
            flags,
        } = request;
        let (owner_pos, owner_sector, door_source) = match self.get_entity(owner) {
            Some(e) => {
                let elem = e.element_data();
                (
                    elem.position_map(),
                    super::ai::ai_view_position_sector(self, elem),
                    super::movement::current_door_for_route_source(e),
                )
            }
            None => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return true;
            }
        };
        let (target_pos, target_sector, target_layer) = match self.get_entity(target) {
            Some(e) => {
                let elem = e.element_data();
                (
                    elem.position_map(),
                    super::ai::ai_view_position_sector(self, elem),
                    elem.layer(),
                )
            }
            None => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return true;
            }
        };
        let (Some(owner_sector), Some(target_sector)) = (owner_sector, target_sector) else {
            return false;
        };
        if seek_sectors_match(owner_sector, target_sector) {
            return false;
        }

        let Some(resolved) =
            self.resolve_entity_seek(sim, assets, owner, target, flags, seek_distance)
        else {
            // The original game marks the current movement during seek refresh
            // impossible silently when position authorization fails.
            // The unable-to-do bark belongs to movement-sequence construction's
            // gate-path failure below.
            self.stop_selected_seek_for_refresh(owner);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };

        let (path_src_pos, path_src_sector) = {
            let adapted = self.scripts.mission.as_ref().and_then(|_| {
                door_source.and_then(|(door_handle, door_direction)| {
                    crate::engine::movement::adapt_source_to_current_door_with_identity(
                        &self.script_domains.interactables.doors,
                        door_handle,
                        door_direction,
                    )
                })
            });
            match adapted {
                Some((adj, sector, _layer)) => (adj, sector),
                None => (owner_pos, owner_sector),
            }
        };

        let owner_auth = self.get_entity(owner).map(|e| e.actor_auth_info());
        let level = self.world.fast_grid.level.clone();
        let gate_path = {
            self.scripts.mission.as_ref().and_then(|_| {
                find_seek_gate_path(
                    &self.script_domains.interactables.doors,
                    path_src_pos,
                    path_src_sector,
                    resolved.destination,
                    target_sector,
                    owner_auth.as_ref(),
                    false,
                    &|sector| self.building_sector_is_authorized(sector),
                    &|sector| {
                        level
                            .sectors
                            .iter()
                            .find(|candidate| candidate.sector_number == sector)
                            .and_then(|candidate| candidate.lift_type)
                    },
                )
            })
        };

        let Some(gate_path) = gate_path else {
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            self.stop_selected_seek_for_refresh(owner);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };

        self.stop_selected_seek_for_refresh(owner);
        self.orders.sequence_manager.element_interrupted(
            seq_id,
            elem_idx,
            CascadeFlags::NEXT_LEVEL,
        );

        let _ = self.build_gate_movement_sequence(sim, crate::engine::movement::GateRouteRequest { entity_id: owner, source_sector: Some(path_src_sector), gate_path: gate_path, goal: GoalShape::Seek {
                point: resolved.destination,
                target,
                tolerance: resolved.tolerance,
            }, goal_layer: target_layer, base_action: action, move_after_last_door: true, speed_factor: resolved.speed_factor, initial_flags: flags, prefix_elements: Vec::new(), tail_elements: // Seek refresh leaves the post-seek sequence on the
            // actor while replacing the path sequence. It may refresh the
            // seek repeatedly as a moving target crosses sectors, and every
            // replacement must retain the same eventual interaction.
            //
            // Appending the interaction to this transient gate route instead
            // loses it when the next seek refresh interrupts that route.
            Vec::new(), append_arrival_speech: false, append_recovery: false });

        tracing::trace!(
            ?owner,
            ?target,
            target_x = target_pos.x,
            target_y = target_pos.y,
            from_sector = u16::from(owner_sector),
            to_sector = u16::from(target_sector),
            "try_dispatch_cross_sector_entity_seek: launched gate traversal"
        );
        true
    }

    /// Point-target seek refresh: expand a seek whose goal sector differs
    /// from the actor's own into a gate route.
    ///
    /// The point overload always hands its goal to the shared move builder,
    /// which short-circuits to a single MOVE while source and goal share a
    /// sector and otherwise walks the gate chain — including the
    /// building-exit `WaitTimer` pair each building gate contributes. Rust
    /// previously relaunched every point seek as a flat MOVE, so a
    /// cross-sector drop-ale / walk-here click never crossed a door and never
    /// consumed the route-construction RNG draws.
    ///
    /// Returns `true` when the gate route (or its failure) has consumed the
    /// element, leaving nothing for the flat relaunch to do.
    pub(super) fn try_dispatch_cross_sector_point_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        request: PointSeekRequest,
    ) -> bool {
        let PointSeekRequest {
            owner,
            sequence_id: seq_id,
            element_index: elem_idx,
            destination,
            goal_sector,
            goal_layer,
            action,
            flags,
            seek_distance,
            recorded_gate_path,
            route_provenance,
        } = request;
        let Some(goal_sector) = goal_sector else {
            return false;
        };
        if self.scripts.mission.is_none() {
            return false;
        }
        let Some((owner_pos, owner_layer, Some(owner_sector))) = self.get_entity(owner).map(|e| {
            (
                e.element_data().position_map(),
                e.element_data().layer(),
                e.element_data().sector(),
            )
        }) else {
            return false;
        };
        // A door-sector goal takes the original game's door-movement construction
        // shape, which this expansion does not build yet.
        let goal_is_door_sector = self
            .world
            .fast_grid
            .level
            .sectors
            .iter()
            .find(|candidate| u16::from(candidate.sector_number) == u16::from(goal_sector))
            .is_some_and(|candidate| candidate.sector_type.is_door());
        if goal_is_door_sector {
            tracing::debug!(
                ?owner,
                goal_sector = u16::from(goal_sector),
                "cross-sector point seek to a door sector is not expanded yet"
            );
            return false;
        }

        let door_source = self
            .get_entity(owner)
            .and_then(crate::engine::movement::current_door_for_route_source);
        let (src_pos, src_sector, src_layer) =
            match door_source.and_then(|(door_handle, door_direction)| {
                crate::engine::movement::adapt_source_to_current_door_with_identity(
                    &self.script_domains.interactables.doors,
                    door_handle,
                    door_direction,
                )
            }) {
                Some((adjusted, sector, layer)) => (adjusted, sector, layer),
                None => (owner_pos, owner_sector, owner_layer),
            };
        if let Some(recorded) = recorded_gate_path.as_ref() {
            validate_recorded_point_seek_source(recorded, src_sector, src_layer);
        }
        if seek_sectors_match(src_sector, goal_sector) {
            return false;
        }

        assert!(
            route_provenance != crate::sequence::PointSeekRouteProvenance::OriginalReplay
                || recorded_gate_path.is_some(),
            "Original-replay cross-sector point Seek for {owner:?} to ({}, {}) has no admitted RecordedDropAleRoute ExternalFact",
            destination.x,
            destination.y,
        );

        let gate_path = match recorded_point_seek_gate_path(
            &self.script_domains.interactables.doors,
            src_sector,
            src_layer,
            goal_sector,
            recorded_gate_path,
        ) {
            Some(recorded) => recorded,
            None => {
                let owner_auth = self.get_entity(owner).map(|e| e.actor_auth_info());
                let level = self.world.fast_grid.level.clone();
                find_seek_gate_path(
                    &self.script_domains.interactables.doors,
                    src_pos,
                    src_sector,
                    destination,
                    goal_sector,
                    owner_auth.as_ref(),
                    flags.contains(MoveFlags::MAP),
                    &|sector| self.building_sector_is_authorized(sector),
                    &|sector| {
                        level
                            .sectors
                            .iter()
                            .find(|candidate| candidate.sector_number == sector)
                            .and_then(|candidate| candidate.lift_type)
                    },
                )
            }
        };

        let Some(gate_path) = gate_path else {
            // Movement-sequence construction speaks the unable bark for a PC before
            // returning false, and seek refresh then marks the element
            // impossible.
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            self.stop_selected_seek_for_refresh(owner);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };
        if gate_path.is_empty() {
            return false;
        }

        self.stop_selected_seek_for_refresh(owner);
        self.orders.sequence_manager.element_interrupted(
            seq_id,
            elem_idx,
            CascadeFlags::NEXT_LEVEL,
        );

        let _ = self.build_gate_movement_sequence(sim, crate::engine::movement::GateRouteRequest { entity_id: owner, source_sector: Some(src_sector), gate_path: gate_path, goal: GoalShape::Point {
                point: destination,
                tolerance: seek_distance,
            }, goal_layer: goal_layer, base_action: action, move_after_last_door: true, speed_factor: 1.0, initial_flags: flags | MoveFlags::SEEK, prefix_elements: Vec::new(), tail_elements: // The post-seek interaction lives on the actor, not on this
            // transient route, so a later refresh that replaces the route
            // keeps it.
            Vec::new(), append_arrival_speech: false, append_recovery: false });

        tracing::trace!(
            ?owner,
            from_sector = u16::from(owner_sector),
            to_sector = u16::from(goal_sector),
            "try_dispatch_cross_sector_point_seek: launched gate traversal"
        );
        true
    }

    /// Shared tail of both seek-refresh variants: interrupt the
    /// actor's current movement element and launch a fresh single-
    /// element seek sequence at info priority.  The
    /// selected-seek cleanup cancels any in-flight path request belonging to
    /// the interrupted element and clears its old sprite goal before the
    /// replacement becomes current.
    pub(super) fn relaunch_seek_replacement(
        &mut self,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
        new_elem: SequenceElement,
    ) {
        self.stop_selected_seek_for_refresh(owner);
        self.orders.sequence_manager.element_interrupted(
            seq_id,
            elem_idx,
            CascadeFlags::NEXT_LEVEL,
        );

        let mut seq = Sequence::new();
        seq.append_element(new_elem);
        self.launch_sequence(seq);
    }
}

#[cfg(test)]
#[path = "refresh_seek/tests.rs"]
mod tests;
