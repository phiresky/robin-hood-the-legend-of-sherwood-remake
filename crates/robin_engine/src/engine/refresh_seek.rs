//! Per-tick RefreshSeek scan.
//!
//! ## Semantics
//!
//! A seek arms its `seek_refresh_wait` countdown to `TIME_SEEK_REFRESH`
//! (=25) at launch and again at the tail of each refresh. Entity-target
//! `PerformSeek` decrements it at the owner movement boundary unless an
//! in-range post-seek interaction returns first. Once it is zero AND the
//! target has moved more than 10 units (MaxNorm) since the last launch,
//! this pre-owner scan rebuilds a fresh single-element seek sequence
//! bound to the target's *current* position, sets the previous movement
//! element to interrupted, and launches the new sequence at info
//! priority. The Original tests the unsigned counter through a signed cast,
//! so zero and wrapped high-bit values are all considered expired.
//!
//! Runs once per tick for every actor holding an `InProgress` movement
//! element with a populated `element` (target) field and the
//! `MoveFlags::SEEK` bit set.  Initial user-facing seeks arrive as
//! `Command::Seek`; gate-expanded `AppendMoveToSequence`-style seek
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

#[inline]
fn seek_refresh_wait_elapsed(wait: u32) -> bool {
    (wait as i32) <= 0
}

/// Original gate routing compares the `RHSector*` stored by each RHposition.
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

/// `RHElementActor::PerformSeek`'s entity arm opens with
///
/// ```text
/// // Wait for the post seek sequence to be launched !
/// if( mpSeekTarget == 0 )
/// {
///     return RHMOTION_TERMINATED;
/// }
/// ```
///
/// (`original-code/RHelementactor.cpp:7820-7826`).  The seek target lives on
/// the actor, not on the sequence element: `StartPostSeekSequence` clears
/// `mpSeekTarget` while the element keeps its own `GetElement()` pointer, so
/// a later Move|SEEK leg of the same route still looks like an entity seek to
/// the element but is already finished as far as `PerformSeek` is concerned.
/// Original then terminates that Execute before computing any distance,
/// before ageing `mulWaitTime`, and before `RefreshSeek`.
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
    // `mbSeekToPoint` picks the point arm, which has no null-target guard.
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
    // PerformMotion directly and never reach this guard.
    let Some(order_type) = element
        .current_order()
        .map(|order| order.order_type)
        .filter(|order| super::movement::perform_seek_calls_per_execute(*order) > 0)
    else {
        return false;
    };

    // PerformSeek's null-target guard is only the nested motion producer.
    // The surrounding Execute switch still receives TERMINATED and applies
    // the selected animation arm's state effect before Actor::Hourglass runs
    // DoNextOrder (RHelementactor.cpp:1690-1713, :7820-7826). Returning from
    // the split Rust owner envelope without this step left a finished running
    // stop transition in MovingFast even after its MoveOk element retired.
    if let Some((posture, action_state)) =
        super::movement::movement_execute_state_effect(order_type, MotionState::Terminated)
    {
        let entity = engine
            .get_entity_mut(owner)
            .unwrap_or_else(|| panic!("lost-target PerformSeek owner {owner:?} disappeared"));
        entity.set_posture(posture);
        entity
            .actor_data_mut()
            .expect("lost-target PerformSeek owner is not an actor")
            .action_state = action_state;
    }
    true
}

/// `RHElementActor::PerformSeek` tests the live target tolerance before its
/// moved-target refresh arm.  Keep the same test at the pre-owner refresh
/// seam so an expired stale route cannot replace a seek whose target has
/// already entered interaction range.
fn entity_seek_live_tolerance_reached(
    owner: &Entity,
    target: &Entity,
    flags: MoveFlags,
    seek_distance: f32,
) -> bool {
    // The Original's SEEK_SHIELD arm is intentionally different: it tests
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
            .cxx_current_point_map()
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
    /// Execute Original's explicit `RHNONANIMATION_REFRESHING_SEEK` hold.
    ///
    /// Unlike ordinary movement arms, this order calls `RefreshSeek`
    /// unconditionally: there is no countdown or target-displacement gate.
    /// It is installed when a final Move|SEEK reaches a building and executes
    /// on the actor's following Hourglass slot.
    pub(super) fn tick_refreshing_seek_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<MotionState> {
        let Some((seq_id, elem_idx)) = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
        else {
            return None;
        };
        let Some((target, action, flags, tolerance)) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| {
                let order = element.current_order()?;
                if order.order_type != crate::order::OrderType::RefreshingSeek {
                    return None;
                }
                let SequenceElementData::Movement {
                    element: target,
                    action,
                    flags,
                    tolerance,
                    ..
                } = &element.data
                else {
                    panic!(
                        "RefreshingSeek owner {owner:?} selected non-movement element {seq_id:?}/{elem_idx}"
                    )
                };
                Some((*target, *action, *flags, *tolerance))
            })
        else {
            return None;
        };

        let Some(target) = target else {
            // The point-target arm returns TERMINATED without refreshing.
            // Actor::Hourglass owns the subsequent DoNextOrder; retiring the
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
            owner,
            seq_id,
            elem_idx,
            target,
            action,
            flags,
            tolerance,
            target_position,
        );
        Some(MotionState::InProgress)
    }

    /// Prepare a selected Seek for the terminal `SetState` used by Original's
    /// `RefreshSeek` overloads. Original sends the actor's condolence card
    /// synchronously before launching any replacement, so the still-selected
    /// Seek clears `PositionGoalMap`. Rust queues that card and eagerly
    /// detaches active mechanics; preserve the same observable cleanup at the
    /// exact RefreshSeek boundary.
    fn stop_selected_seek_for_refresh(
        &mut self,
        owner: EntityId,
        _seq_id: SequenceId,
        _elem_idx: usize,
    ) {
        // The initial Translate(SEEK) wrapper is semantically selected in
        // Original before it reaches RefreshSeek, but Rust deliberately
        // leaves ordered elements Todo until concrete movement dispatch.
        // Therefore manager `current_element_for_actor` cannot recognize the
        // initial wrapper here; the explicit sequence identity supplied by
        // every caller is the authoritative selected Seek.
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
            // C++ `GetCurrentPointMap()` starts at integer
            // `GetPositionSprite()` (`floor(position_map - center)`) and
            // adds the current row's hotspot. It disables USE_POINT only
            // when that complete map-space point equals PositionMap; a zero
            // local hotspot alone is not the disabling condition.
            let current_point = target_entity
                .cxx_current_point_map()
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
            // RHElementActor::RefreshSeek uses the literal stored 3D
            // GetPosition values here, not their projected map positions.
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
        // target->Think(EVENT_STOP) synchronously, and only afterwards
        // samples the destination and authorizes/builds the replacement.
        // End the immutable entity borrows before entering that nested Think.
        let _ = owner_entity;
        let _ = target_entity;
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

    /// Refresh one selected entity-target seek at its live actor Hourglass
    /// slot.
    ///
    /// Original evaluates this inside `RHElementActor::PerformSeek`, so a
    /// target with an earlier creation slot has already moved this frame,
    /// while a later target has not. Returns `true` when RefreshSeek replaced
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
        let (seq_id, elem_idx, target, action, flags, tolerance, new_target_pos) = refresh;
        tracing::trace!(
            ?owner,
            ?target,
            new_x = new_target_pos.x,
            new_y = new_target_pos.y,
            "owner-slot seek target moved >10u; re-launching seek",
        );
        // PerformSeek restores the actor's moving state immediately before
        // RefreshSeek. This is observable on the refresh frame even though
        // the replacement path does not execute until a later owner slot.
        if let Some(actor) = self.get_entity_mut(owner).and_then(|e| e.actor_data_mut()) {
            actor.action_state = actor.action_state.set_moving(false, false);
        }
        self.apply_seek_refresh(
            sim,
            assets,
            owner,
            seq_id,
            elem_idx,
            target,
            action,
            flags,
            tolerance,
            new_target_pos,
        );
        true
    }

    /// Decide whether `RHElementActor::PerformSeek`'s moved-target branch
    /// (RHelementactor.cpp:7913) fires for this owner's selected seek, without
    /// applying it.
    ///
    /// Split out of [`Self::tick_refresh_seek_for_owner`] so the sword- and
    /// shield-walking Execute arms can run their pre-`PerformSeek` facing
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
        f32,
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
        let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
            return None;
        };
        if !matches!(
            elem.command,
            crate::element::Command::Move
                | crate::element::Command::MoveOk
                | crate::element::Command::Seek
        ) {
            return None;
        }
        let SequenceElementData::Movement {
            flags,
            element: target,
            tolerance,
            action,
            ..
        } = &elem.data
        else {
            return None;
        };
        if !flags.contains(MoveFlags::SEEK) {
            return None;
        }
        // Original evaluates moved-target refresh only inside
        // RHElementActor::PerformSeek. The SEEK flag remains attached to
        // cross-sector wall and ladder orders, but their Execute arms call
        // PerformMotion directly and must neither refresh nor consume the
        // route-construction RNG draws. Sampling solely from the element
        // flags made a climbing PC rebuild a cross-building chase while
        // Original kept climbing.
        let Some(order) = elem.orders.front() else {
            return None;
        };
        if super::movement::perform_seek_calls_per_execute(order.order_type) == 0 {
            return None;
        }
        let Some(target_id) = *target else {
            return None;
        };
        let Some(target_entity) = self.get_entity(target_id) else {
            return None;
        };
        let target_pos = target_entity.element_data().position_map();

        // PerformSeek's same-sector distance gate precedes its expired
        // timer / moved-target test. In particular, a target can enter
        // range while this route still points at its old position; the
        // ensuing owner Execute must consume the post-seek handoff rather
        // than RefreshSeek replacing the route first.
        if entity_seek_live_tolerance_reached(entity, target_entity, *flags, actor.seek_distance) {
            return None;
        }

        // Original stores this as ULONG but tests expiry through
        // `(SLONG)mulWaitTime <= 0`. Wrapped high-bit values therefore
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

        Some((
            seq_id, elem_idx, target_id, *action, *flags, *tolerance, target_pos,
        ))
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_seek_refresh(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        target: EntityId,
        action: crate::order::OrderType,
        flags: MoveFlags,
        _concrete_tolerance: f32,
        new_target_pos: crate::coordinates::MapPoint,
    ) {
        let seek_distance = self
            .get_entity(owner)
            .and_then(|entity| entity.actor_data())
            .map(|actor| actor.seek_distance)
            .filter(|distance| *distance > 0.0)
            .unwrap_or_else(|| {
                panic!(
                    "entity-target RefreshSeek owner {owner:?} has no positive base seek distance"
                )
            });
        if self.try_handle_same_sector_actor_seek_wait(
            sim, assets, owner, seq_id, elem_idx, target, flags,
        ) {
            return;
        }

        // Original stamps these immediately after its same-sector early
        // returns and before deciding whether AppendMoveToSequence needs a
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

        if self.try_dispatch_cross_sector_entity_seek(
            sim,
            assets,
            owner,
            seq_id,
            elem_idx,
            target,
            action,
            flags,
            seek_distance,
        ) {
            return;
        }

        let Some(resolved) =
            self.resolve_entity_seek(sim, assets, owner, target, flags, seek_distance)
        else {
            self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return;
        };
        // RefreshSeek's transient selected Seek is replaced by the concrete
        // movement built by AppendMoveToSequence. Keep it as Move + SEEK
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

    /// Original `RefreshSeek(RHElement*)` waits instead of rebuilding
    /// a path for two actor-target cases in the same sector:
    ///
    /// * actor and target are both inside a building, except that
    ///   `SEEK_IN_BUILDINGS` with a post-seek tail teleports to the
    ///   target and starts the tail;
    /// * target actor is currently passing a door, where the next
    ///   refresh should see the target's post-door sector/position.
    ///
    /// Returns `true` when RefreshSeek should stop without normal path
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

        let target_passing_door = self
            .get_entity(target)
            .and_then(|e| e.actor_data())
            .and_then(|a| {
                a.active_movement
                    .sequence_id
                    .map(|seq_id| (seq_id, a.active_movement.element_index))
            })
            .and_then(|(seq_id, elem_idx)| {
                self.orders.sequence_manager.get_element(seq_id, elem_idx)
            })
            .is_some_and(|elem| elem.command == crate::element::Command::PassDoor);
        if target_passing_door {
            return true;
        }

        false
    }

    /// Central entity-target Seek lowering, matching original
    /// `RefreshSeek -> AppendMoveToSequence` for cross-sector targets.
    ///
    /// Returns `true` when the current Seek was fully consumed: either
    /// replaced by a gate/lift/jump traversal sequence or marked
    /// impossible after an authorized-position / gate-path failure.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_dispatch_cross_sector_entity_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        target: EntityId,
        action: OrderType,
        flags: MoveFlags,
        seek_distance: f32,
    ) -> bool {
        let (owner_pos, owner_sector, door_handle, door_direction) = match self.get_entity(owner) {
            Some(e) => {
                let elem = e.element_data();
                let (door_handle, door_direction) =
                    super::movement::current_door_for_route_source(e);
                (
                    elem.position_map(),
                    super::ai::ai_view_position_sector(self, elem),
                    door_handle,
                    door_direction,
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
            // Original RefreshSeek marks the current movement
            // impossible silently when FindAutorizedPosition fails.
            // The unable-to-do bark belongs to AppendMoveToSequence's
            // gate-path failure below.
            self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };

        let (path_src_pos, path_src_sector) = {
            let adapted = self.scripts.mission.as_ref().and_then(|_| {
                crate::engine::movement::adapt_source_to_current_door_with_identity(
                    &self.script_domains.interactables.doors,
                    door_handle,
                    door_direction,
                )
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
            self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };

        self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
        self.orders.sequence_manager.element_interrupted(
            seq_id,
            elem_idx,
            CascadeFlags::NEXT_LEVEL,
        );

        let _ = self.build_gate_movement_sequence(
            sim,
            owner,
            Some(path_src_sector),
            gate_path,
            GoalShape::Seek {
                point: resolved.destination,
                target,
                tolerance: resolved.tolerance,
            },
            target_layer,
            action,
            true,
            resolved.speed_factor,
            flags,
            Vec::new(),
            // RHElementActor::RefreshSeek leaves mpPostSeekSequence on the
            // actor while replacing the path sequence. It may refresh the
            // seek repeatedly as a moving target crosses sectors, and every
            // replacement must retain the same eventual interaction.
            //
            // Appending the interaction to this transient gate route instead
            // loses it when the next RefreshSeek interrupts that route.
            Vec::new(),
            false,
            false,
        );

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

    /// Point-target `RefreshSeek`: expand a seek whose goal sector differs
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_dispatch_cross_sector_point_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
        destination: MapPoint,
        goal_sector: Option<crate::position_interface::SectorHandle>,
        goal_layer: u16,
        action: OrderType,
        flags: MoveFlags,
        seek_distance: f32,
    ) -> bool {
        let Some(goal_sector) = goal_sector else {
            return false;
        };
        if self.scripts.mission.is_none() {
            return false;
        }
        let Some((owner_pos, Some(owner_sector))) = self
            .get_entity(owner)
            .map(|e| (e.element_data().position_map(), e.element_data().sector()))
        else {
            return false;
        };
        if seek_sectors_match(owner_sector, goal_sector) {
            return false;
        }
        // A door-sector goal takes the original's `AppendMoveToDoorToSequence`
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

        let (door_handle, door_direction) = self
            .get_entity(owner)
            .map(crate::engine::movement::current_door_for_route_source)
            .unwrap_or((crate::position_interface::DoorHandle::NULL, false));
        let (src_pos, src_sector) =
            match crate::engine::movement::adapt_source_to_current_door_with_identity(
                &self.script_domains.interactables.doors,
                door_handle,
                door_direction,
            ) {
                Some((adjusted, sector, _layer)) => (adjusted, sector),
                None => (owner_pos, owner_sector),
            };

        let owner_auth = self.get_entity(owner).map(|e| e.actor_auth_info());
        let level = self.world.fast_grid.level.clone();
        let gate_path = find_seek_gate_path(
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
        );

        let Some(gate_path) = gate_path else {
            // AppendMoveToSequence speaks the unable bark for a PC before
            // returning false, and RefreshSeek then marks the element
            // impossible.
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return true;
        };
        if gate_path.is_empty() {
            return false;
        }

        self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
        self.orders.sequence_manager.element_interrupted(
            seq_id,
            elem_idx,
            CascadeFlags::NEXT_LEVEL,
        );

        let _ = self.build_gate_movement_sequence(
            sim,
            owner,
            Some(src_sector),
            gate_path,
            GoalShape::Point {
                point: destination,
                tolerance: seek_distance,
            },
            goal_layer,
            action,
            true,
            1.0,
            flags | MoveFlags::SEEK,
            Vec::new(),
            // The post-seek interaction lives on the actor, not on this
            // transient route, so a later refresh that replaces the route
            // keeps it.
            Vec::new(),
            false,
            false,
        );

        tracing::trace!(
            ?owner,
            from_sector = u16::from(owner_sector),
            to_sector = u16::from(goal_sector),
            "try_dispatch_cross_sector_point_seek: launched gate traversal"
        );
        true
    }

    /// Shared tail of both `RefreshSeek` overloads: interrupt the
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
        self.stop_selected_seek_for_refresh(owner, seq_id, elem_idx);
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
mod tests {
    use super::*;
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, AiBrain, Command, ElementData, ElementKind, Entity,
        HumanData, PcData, Posture, SoldierData,
    };
    use crate::movement::ActiveMovement;
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElementData, SequenceState};

    #[test]
    fn seek_refresh_expiry_uses_original_signed_view_of_unsigned_counter() {
        assert!(!seek_refresh_wait_elapsed(25));
        assert!(!seek_refresh_wait_elapsed(1));
        assert!(seek_refresh_wait_elapsed(0));
        assert!(seek_refresh_wait_elapsed(u32::MAX));
        assert!(seek_refresh_wait_elapsed(i32::MIN as u32));
    }

    #[test]
    fn lost_target_moveok_stop_transition_publishes_waiting_before_terminal_handoff() {
        let mut engine = crate::engine::EngineInner::new();
        let mut owner_entity = test_pc_at(100.0, 100.0, 1);
        {
            let actor = owner_entity.actor_data_mut().unwrap();
            actor.action_state = ActionState::MovingFast;
            actor.seek_target = None;
            actor.continuation.seek_to_point = false;
        }
        owner_entity.element_data_mut().sprite.last_action = OrderType::WalkingStairs;
        let owner = engine.add_entity(owner_entity);

        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        let SequenceElementData::Movement { flags, .. } = &mut movement.data else {
            panic!("MoveOk fixture must be a movement element")
        };
        flags.insert(MoveFlags::SEEK);
        let order_id = engine.orders.allocate_order_id();
        movement.orders.push_back(crate::order::Order::new(
            OrderType::TransitionRunningUprightWaitingUpright,
            100.0,
            100.0,
            order_id,
        ));
        let sequence_id = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.active_movement = ActiveMovement::new(sequence_id, 0);
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id,
                order_type: OrderType::TransitionRunningUprightWaitingUpright,
            });
            actor.continuation.motion_state = MotionState::InProgress;
        }

        let mut tail_order = None;
        engine.tick_actor_animation_action_change_slots_with_hooks(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            |_, _| {},
            |_, _| {},
            |engine, execute_owner, movement, _, _, _, _| {
                assert_eq!(execute_owner, owner);
                let movement = movement.expect("live MoveOk must own the actor Execute slot");
                assert!(perform_seek_lost_actor_target(
                    engine,
                    execute_owner,
                    movement,
                ));
                Some(MotionState::Terminated)
            },
            |_, tail_owner, order_type| {
                assert_eq!(tail_owner, owner);
                tail_order = Some(order_type);
            },
        );
        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(entity.element_data().posture, Posture::Upright);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting
        );
        assert_eq!(
            entity.element_data().sprite.last_action,
            OrderType::WalkingStairs,
            "the null-target PerformSeek branch never enters RHSprite; its surrounding Execute arm owns only SetStates"
        );
        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert_eq!(actor.installed_order, None);
        assert_eq!(actor.continuation.motion_state, MotionState::Terminated);
        assert_eq!(
            tail_order,
            Some(OrderType::NonanimationEnd),
            "the exhausted live MoveOk publishes the null mpOrder tail as NONANIMATION_END"
        );
    }

    fn test_pc_at(x: f32, y: f32, sector: u16) -> Entity {
        let mut pc = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        pc.element
            .set_position_map(crate::coordinates::MapPoint { x, y });
        pc.element.set_sector(SectorHandle::new(sector));
        Entity::Pc(pc)
    }

    fn test_moving_soldier_at(position: crate::coordinates::WorldPoint3D) -> Entity {
        let mut soldier = ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: ActorData {
                action_state: ActionState::Moving,
                ..Default::default()
            },
            human: HumanData::default(),
            npc: Default::default(),
            soldier: SoldierData::default(),
        };
        soldier.element.set_position(position);
        soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
        Entity::Soldier(soldier)
    }

    fn minimal_mission() -> crate::engine::MissionScript {
        use crate::scb::{ClassEntry, Function, SCB_VERSION, ScbFile};
        use crate::vm::{Opcode, Quad};

        crate::engine::MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![ClassEntry {
                source_file: "refresh_seek_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: vec![Function {
                    name: "Initialize".into(),
                    address: 0,
                    num_parameters: 0,
                    size_of_return_value: 0,
                    size_of_parameters: 0,
                    size_of_volatile: 0,
                    size_of_temporary: 0,
                }],
                quads: vec![
                    Quad {
                        operation: Opcode::BeginFunction as u8,
                        operands: [0; 8],
                    },
                    Quad {
                        operation: Opcode::Return as u8,
                        operands: [0; 8],
                    },
                ],
            }],
        })
        .expect("minimal mission script")
    }

    #[test]
    fn entity_seek_gate_search_keeps_pc126_exact_route_identity() {
        use crate::fast_find_grid::SectorIndex;
        use crate::gate::{Door, DoorIndex, build_gate_links};
        use crate::sector::SectorNumber;

        let arena = |index| SectorIndex::new(index).unwrap();
        let sector = |public, index| {
            SectorHandle::new(public)
                .unwrap()
                .with_arena_index(arena(index))
        };
        let mut doors = (0..74)
            .map(|_| Door {
                active: false,
                ..Door::default()
            })
            .collect::<Vec<_>>();

        // Cyrdach Pc126's Original route at frame 1427 starts by crossing
        // gate 73 in reverse (62 -> 24), then gate 18 directly (24 -> 27).
        doors[73] = Door {
            active: true,
            point_in: MapPoint::new(0.0, 0.0),
            point_out: MapPoint::new(20.0, 0.0),
            sector_in: SectorNumber::new(62),
            sector_out: SectorNumber::new(24),
            sector_in_index: Some(arena(162)),
            sector_out_index: Some(arena(124)),
            ..Door::default()
        };
        doors[18] = Door {
            active: true,
            point_out: MapPoint::new(30.0, 0.0),
            point_in: MapPoint::new(50.0, 0.0),
            sector_out: SectorNumber::new(24),
            sector_in: SectorNumber::new(27),
            sector_out_index: Some(arena(124)),
            sector_in_index: Some(arena(127)),
            ..Door::default()
        };
        build_gate_links(&mut doors);

        let path = find_seek_gate_path(
            &doors,
            MapPoint::new(0.0, 0.0),
            sector(62, 162),
            MapPoint::new(50.0, 0.0),
            sector(27, 127),
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .expect("exact Pc126 Seek topology must find the two-gate route");
        assert_eq!(
            path,
            vec![
                crate::gate::GatePathStep {
                    door_index: DoorIndex(73),
                    direct: false,
                },
                crate::gate::GatePathStep {
                    door_index: DoorIndex(18),
                    direct: true,
                },
            ]
        );

        assert!(
            find_seek_gate_path(
                &doors,
                MapPoint::new(0.0, 0.0),
                // Same public sector as gate 73's in-side, but a distinct
                // Original arena object. Numeric routing would falsely use
                // gate 73 here; exact routing must reject it.
                sector(62, 262),
                MapPoint::new(50.0, 0.0),
                sector(27, 127),
                None,
                false,
                &|_| true,
                &|_| None,
            )
            .is_none()
        );
    }

    #[test]
    fn refresh_seek_recovers_moved_owner_and_target_sectors_before_indexed_route() {
        use crate::coordinates::MapBBox;
        use crate::fast_find_grid::{GridSector, SectorIndex};
        use crate::gate::{Door, DoorIndex, build_gate_links};
        use crate::sector::{SectorNumber, SectorType};

        let arena = |index| SectorIndex::new(index).unwrap();
        let grid_sector = |number, layer, min_x, min_y, max_x, max_y| GridSector {
            points: vec![
                MapPoint::new(min_x, min_y),
                MapPoint::new(max_x, min_y),
                MapPoint::new(max_x, max_y),
                MapPoint::new(min_x, max_y),
            ],
            bounding_box: MapBBox::from_coords(min_x, min_y, max_x, max_y),
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
        };

        let sim = crate::sim_rng::test_context();
        let mut engine = crate::engine::EngineInner::new();
        engine.scripts.mission = Some(minimal_mission());
        engine.world.fast_grid_mut().size_map(10, 10);
        engine.world.fast_grid_mut().allocate_layers(3);
        let wrong_target = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(88, 2, 450.0, 450.0, 500.0, 500.0), 2);
        let wrong_source = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(0, 0, 450.0, 150.0, 500.0, 200.0), 0);
        let source = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(0, 0, 10.0, 10.0, 100.0, 100.0), 0);
        let middle = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(70, 1, 150.0, 10.0, 250.0, 100.0), 1);
        let exact_target = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(88, 2, 300.0, 10.0, 400.0, 100.0), 2);
        assert_ne!(wrong_target, exact_target);
        assert_ne!(wrong_source, source);

        let mut doors = (0..114)
            .map(|_| Door {
                active: false,
                ..Door::default()
            })
            .collect::<Vec<_>>();
        doors[111] = Door {
            active: true,
            point_out: MapPoint::new(90.0, 50.0),
            point_in: MapPoint::new(160.0, 50.0),
            layer_out: 0,
            layer_in: 1,
            sector_out: SectorNumber::new(0),
            sector_in: SectorNumber::new(70),
            sector_out_index: Some(arena(source)),
            sector_in_index: Some(arena(middle)),
            ..Door::default()
        };
        doors[113] = Door {
            active: true,
            point_in: MapPoint::new(240.0, 50.0),
            point_out: MapPoint::new(310.0, 50.0),
            layer_in: 1,
            layer_out: 2,
            sector_in: SectorNumber::new(70),
            sector_out: SectorNumber::new(88),
            sector_in_index: Some(arena(middle)),
            sector_out_index: Some(arena(exact_target)),
            ..Door::default()
        };
        build_gate_links(&mut doors);
        engine.script_domains.interactables.doors = doors;

        let owner = engine.add_entity(test_pc_at(50.0, 50.0, 0));
        {
            let owner_position = engine.get_entity_mut(owner).unwrap().position_iface_mut();
            // The adopted PC carries only the public sector. Original's
            // GetSector() still supplies the exact RHSector* to
            // AppendMoveToSequence, so recover the containing duplicate here.
            owner_position.set_sector(SectorHandle::new(0));
            owner_position.set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        }
        let target = engine.add_entity(test_pc_at(350.0, 50.0, 88));
        {
            let target_element = engine.get_entity_mut(target).unwrap().element_data_mut();
            target_element.set_layer(2);
            target_element.set_sector(SectorHandle::new(88));
        }

        let mut seek =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::RunningUpright);
        if let SequenceElementData::Movement {
            element,
            flags,
            tolerance,
            ..
        } = &mut seek.data
        {
            *element = Some(target);
            *flags = MoveFlags::SEEK;
            *tolerance = 0.0;
        }
        let seek_id = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_id, 0);

        assert!(engine.try_dispatch_cross_sector_entity_seek(
            &sim,
            &LevelAssets::new(),
            owner,
            seek_id,
            0,
            target,
            OrderType::RunningUpright,
            MoveFlags::SEEK,
            0.0,
        ));
        let gates = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter_map(|element| match element.data {
                SequenceElementData::Movement {
                    gate_id: Some(gate),
                    ..
                } if element.owner == Some(owner) => Some(gate),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(gates, vec![DoorIndex(111), DoorIndex(113)]);
    }

    #[test]
    fn cross_sector_refresh_seek_does_not_append_pc_posture_recovery() {
        use crate::gate::Door;
        use crate::sector::SectorNumber;

        let sim = crate::sim_rng::test_context();
        let mut engine = crate::engine::EngineInner::new();
        engine.scripts.mission = Some(minimal_mission());
        engine.script_domains.interactables.doors = vec![Door {
            point_out: MapPoint::new(20.0, 0.0),
            point_in: MapPoint::new(30.0, 0.0),
            sector_out: SectorNumber::new(1),
            sector_in: SectorNumber::new(2),
            ..Door::default()
        }];

        let mut owner_entity = test_pc_at(0.0, 0.0, 1);
        owner_entity.element_data_mut().posture = Posture::HelpingToClimb;
        owner_entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        let owner = engine.add_entity(owner_entity);
        let mut target_entity = test_pc_at(50.0, 0.0, 2);
        target_entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                46.0, -4.0, 54.0, 4.0,
            ));
        let target = engine.add_entity(target_entity);

        let mut seek =
            SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
        if let SequenceElementData::Movement {
            element,
            flags,
            tolerance,
            ..
        } = &mut seek.data
        {
            *element = Some(target);
            *flags = MoveFlags::SEEK;
            *tolerance = 10.0;
        }
        let seek_id = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_id, 0);

        assert!(engine.try_dispatch_cross_sector_entity_seek(
            &sim,
            &LevelAssets::new(),
            owner,
            seek_id,
            0,
            target,
            OrderType::WalkingUpright,
            MoveFlags::SEEK,
            10.0,
        ));

        let replacement = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .find(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.owner == Some(owner) && element.command == Command::PassDoor
                })
            })
            .expect("cross-sector RefreshSeek must build a gate route");
        assert!(
            replacement
                .elements
                .iter()
                .all(|element| element.command != Command::EnterHelpingClimb),
            "Original RefreshSeek calls AppendMoveToSequence directly and does not append PC posture recovery"
        );
    }

    #[test]
    fn ordinary_cross_sector_pc_move_still_appends_posture_recovery() {
        use crate::gate::{Door, DoorIndex, GatePathStep};
        use crate::sector::SectorNumber;

        let sim = crate::sim_rng::test_context();
        let mut engine = crate::engine::EngineInner::new();
        engine.scripts.mission = Some(minimal_mission());
        engine.script_domains.interactables.doors = vec![Door {
            point_out: MapPoint::new(20.0, 0.0),
            point_in: MapPoint::new(30.0, 0.0),
            sector_out: SectorNumber::new(1),
            sector_in: SectorNumber::new(2),
            ..Door::default()
        }];
        let mut owner_entity = test_pc_at(0.0, 0.0, 1);
        owner_entity.element_data_mut().posture = Posture::HelpingToClimb;
        let owner = engine.add_entity(owner_entity);

        let sequence_id = engine
            .build_gate_movement_sequence(
                &sim,
                owner,
                crate::position_interface::SectorHandle::new(1),
                vec![GatePathStep {
                    door_index: DoorIndex(0),
                    direct: true,
                }],
                GoalShape::Point {
                    point: MapPoint::new(50.0, 0.0),
                    tolerance: 0.0,
                },
                0,
                OrderType::WalkingUpright,
                true,
                1.0,
                MoveFlags::empty(),
                Vec::new(),
                Vec::new(),
                true,
                true,
            )
            .expect("ordinary cross-sector move route");
        let route = engine
            .orders
            .sequence_manager
            .get_sequence(sequence_id)
            .expect("ordinary route remains queued");
        assert!(
            route
                .elements
                .iter()
                .any(|element| element.command == Command::EnterHelpingClimb)
        );
    }

    fn resolve_stop_npc_seek_with_target_at(
        target_position: crate::coordinates::WorldPoint3D,
    ) -> (crate::ai::AiState, crate::ai::Substate) {
        let sim = crate::sim_rng::test_context();
        let mut engine = crate::engine::EngineInner::new();
        let mut assets = LevelAssets::new();
        let mut owner_entity = test_pc_at(0.0, 0.0, 1);
        owner_entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::ZERO);
        owner_entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -6.0, -4.0, 6.0, 4.0,
            ));
        owner_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        let owner = engine.add_entity(owner_entity);
        let target = engine.add_entity(test_moving_soldier_at(target_position));
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

        let _ = engine.resolve_entity_seek(
            &sim,
            &assets,
            owner,
            target,
            MoveFlags::SEEK | MoveFlags::SEEK_STOP_NPC,
            32.0,
        );

        let ai = engine
            .get_entity(target)
            .unwrap()
            .npc_data()
            .unwrap()
            .ai_brain
            .base()
            .unwrap();
        (ai.current_state, ai.current_substate)
    }

    #[test]
    fn seek_stop_npc_uses_raw_3d_distance_across_elevations() {
        // Map projection is (20, 16), inside the 64-unit moving-target
        // stop radius, while the literal 3D distance is about 224.6.
        assert_eq!(
            resolve_stop_npc_seek_with_target_at(crate::coordinates::WorldPoint3D::new(
                20.0, 166.0, 150.0,
            )),
            (
                crate::ai::AiState::Default,
                crate::ai::Substate::DefaultOnPost,
            ),
            "a map-near actor on another elevation must not receive EventStop"
        );
    }

    #[test]
    fn seek_stop_npc_still_stops_raw_3d_near_target() {
        assert_eq!(
            resolve_stop_npc_seek_with_target_at(crate::coordinates::WorldPoint3D::new(
                20.0, 16.0, 0.0,
            )),
            (
                crate::ai::AiState::Seeking,
                crate::ai::Substate::SeekingGotStopEvent,
            ),
            "a raw-3D-near moving NPC must receive EventStop"
        );
    }

    #[test]
    fn refresh_seek_waits_when_same_sector_actor_target_is_passing_door() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = crate::engine::EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
        let target = engine.add_entity(test_pc_at(80.0, 10.0, 1));

        let mut seek =
            SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
        if let SequenceElementData::Movement {
            flags,
            element,
            tolerance,
            ..
        } = &mut seek.data
        {
            *flags = MoveFlags::SEEK;
            *element = Some(target);
            *tolerance = 10.0;
        }
        let seek_seq = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_seq, 0);
        {
            // Seek translation stamps the owner's base seek distance before
            // any refresh can run; RefreshSeek requires that live value.
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.active_movement = ActiveMovement::new(seek_seq, 0);
            actor.seek_target = Some(target);
            actor.seek_distance = 10.0;
        }

        let pass = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(target),
            OrderType::WalkingUpright,
        );
        let pass_seq = engine.orders.sequence_manager.launch_element(pass);
        engine
            .orders
            .sequence_manager
            .element_in_progress(pass_seq, 0);
        engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(pass_seq, 0);

        engine.apply_seek_refresh(
            sim,
            &assets,
            owner,
            seek_seq,
            0,
            target,
            OrderType::WalkingUpright,
            MoveFlags::SEEK,
            10.0,
            crate::coordinates::MapPoint { x: 90.0, y: 10.0 },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 2);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seek_seq, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
    }

    fn assert_moved_target_refresh_returns_explicit_in_progress(
        stale_sprite_motion: MotionState,
        target_sector: u16,
        expected_entry_state: SequenceState,
    ) {
        let sim = crate::sim_rng::test_context();
        let mut engine = crate::engine::EngineInner::new();
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .characters
            .push(crate::profiles::CharacterProfile::default());
        let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
        let target = engine.add_entity(test_pc_at(80.0, 10.0, target_sector));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -6.0, -4.0, 6.0, 4.0,
            ));

        let mut seek =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        seek.orders.push_back(crate::order::Order::test_new(
            OrderType::WalkingUpright,
            0.0,
            0.0,
        ));
        if let SequenceElementData::Movement {
            flags,
            element,
            tolerance,
            ..
        } = &mut seek.data
        {
            *flags = MoveFlags::SEEK;
            *element = Some(target);
            *tolerance = 10.0;
        }
        let seek_seq = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_seq, 0);
        {
            let owner_entity = engine.get_entity_mut(owner).unwrap();
            owner_entity.element_data_mut().sprite.last_motion_state = Some(stale_sprite_motion);
            let actor = owner_entity.actor_data_mut().unwrap();
            actor.active_movement = ActiveMovement::new(seek_seq, 0);
            actor.seek_target = Some(target);
            actor.seek_distance = 10.0;
            actor.seek_refresh_wait = 0;
            actor.last_seek_target_position = MapPoint::new(60.0, 10.0);
        }

        let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
        engine.tick_actor_owner_envelopes(&sim, &assets, &positions);

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert_eq!(actor.continuation.motion_state, MotionState::InProgress);
        if expected_entry_state == SequenceState::Impossible {
            assert_eq!(
                actor.installed_order, None,
                "failed RefreshSeek returns before installing replacement work"
            );
        }
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .element_data()
                .sprite
                .last_motion_state,
            Some(stale_sprite_motion),
            "RefreshSeek returns before Sprite::PerformMotion"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seek_seq, 0)
                .unwrap()
                .state,
            expected_entry_state
        );
        if expected_entry_state == SequenceState::Interrupted {
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .current_element_for_actor(owner),
                None,
                "RefreshSeek registers its replacement for the later manager phase"
            );
            let replacement = engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .find(|element| {
                    element.owner == Some(owner)
                        && element.command == Command::Move
                        && element.state == SequenceState::Todo
                })
                .expect("RefreshSeek must leave a fresh movement queued for Hourglass");
            let SequenceElementData::Movement {
                flags,
                element,
                destination,
                ..
            } = &replacement.data
            else {
                panic!("RefreshSeek replacement changed element kind")
            };
            assert!(flags.contains(MoveFlags::SEEK));
            assert_eq!(*element, Some(target));
            assert_eq!(*destination, MapPoint::new(80.0, 10.0));
        }
    }

    #[test]
    fn moved_target_refresh_returns_in_progress_over_stale_terminated_sprite_motion() {
        assert_moved_target_refresh_returns_explicit_in_progress(
            MotionState::Terminated,
            1,
            SequenceState::Interrupted,
        );
    }

    #[test]
    fn moved_target_refresh_returns_in_progress_over_stale_done_sprite_motion() {
        assert_moved_target_refresh_returns_explicit_in_progress(
            MotionState::Done,
            1,
            SequenceState::Interrupted,
        );
    }

    #[test]
    fn failed_moved_target_refresh_keeps_explicit_in_progress_motion() {
        assert_moved_target_refresh_returns_explicit_in_progress(
            MotionState::Aborted,
            2,
            SequenceState::Impossible,
        );
    }

    #[test]
    fn climbing_seek_flag_does_not_run_perform_seek_refresh() {
        let sim = crate::sim_rng::test_context();
        let mut engine = crate::engine::EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
        let target = engine.add_entity(test_pc_at(80.0, 10.0, 2));

        let mut seek = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::ClimbingWallUp,
        );
        if let SequenceElementData::Movement { flags, element, .. } = &mut seek.data {
            *flags = MoveFlags::SEEK;
            *element = Some(target);
        }
        seek.orders.push_back(crate::order::Order::test_new(
            OrderType::ClimbingWallUp,
            80.0,
            10.0,
        ));
        let seek_seq = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_seq, 0);
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.active_movement = ActiveMovement::new(seek_seq, 0);
            actor.seek_refresh_wait = 0;
            actor.last_seek_target_position = MapPoint::ZERO;
        }

        assert!(!engine.tick_refresh_seek_for_owner(&sim, &assets, owner));
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seek_seq, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .seek_refresh_wait,
            0
        );
    }

    /// `RHElementActorHuman::Execute` calls `FaceOpponent`
    /// (RHelementactorhuman.cpp:3662) before it enters `PerformSeek`
    /// (RHelementactorhuman.cpp:3667), and `RHElementActor::PerformSeek`'s
    /// moved-target branch returns `RHMOTION_IN_PROGRESS` without reaching
    /// `PerformMotion`.  `FaceOpponent`'s
    /// `SetDirection( ... GetSector0to15( ASPECT_RATIO ) )` plus `Turn()`
    /// (RHelementactorhuman.cpp:7511-7512) therefore still run on the
    /// RefreshSeek frame.
    #[test]
    fn sword_walk_seek_refresh_still_faces_the_opponent() {
        let mut engine = crate::engine::EngineInner::new();

        let mut owner_entity = test_pc_at(0.0, 0.0, 1);
        owner_entity
            .actor_data_mut()
            .expect("test PC is an actor")
            .action_state = ActionState::MovingSword;
        owner_entity.element_data_mut().set_direction_instantly(0);
        let owner = engine.add_entity(owner_entity);
        let target = engine.add_entity(test_pc_at(100.0, 0.0, 1));
        engine
            .get_entity_mut(owner)
            .expect("owner")
            .human_data_mut()
            .expect("PC has human payload")
            .opponents
            .push(target);

        let mut seek = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        if let SequenceElementData::Movement { flags, element, .. } = &mut seek.data {
            *flags = MoveFlags::SEEK;
            *element = Some(target);
        }
        seek.orders.push_back(crate::order::Order::test_new(
            OrderType::WalkingWithSword,
            90.0,
            0.0,
        ));
        let seek_seq = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_seq, 0);
        {
            let actor = engine
                .get_entity_mut(owner)
                .expect("owner")
                .actor_data_mut()
                .expect("owner is an actor");
            actor.active_movement = ActiveMovement::new(seek_seq, 0);
            actor.seek_refresh_wait = 0;
            actor.seek_distance = 10.0;
            actor.last_seek_target_position = MapPoint::ZERO;
        }

        assert!(
            engine.selected_seek_refresh_decision(owner).is_some(),
            "the >10u moved target must select PerformSeek's RefreshSeek branch"
        );

        engine.apply_pre_perform_seek_facing_prologue(owner);

        let owner_entity = engine.get_entity(owner).expect("owner");
        assert_eq!(
            owner_entity.position_iface().get_direction_goal().as_u8(),
            4,
            "FaceOpponent aims the goal at the principal opponent"
        );
        assert_eq!(
            owner_entity.position_iface().get_direction().as_u8(),
            1,
            "FaceOpponent's Turn advances one sector on the RefreshSeek frame"
        );
    }

    #[test]
    fn relaunch_seek_replacement_clears_selected_seek_goal_before_queuing_replacement() {
        let mut engine = crate::engine::EngineInner::new();
        let owner = engine.add_entity(test_pc_at(10.0, 10.0, 1));
        let stale_goal = MapPoint::new(70.0, 80.0);

        let seek =
            SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
        let seek_seq = engine.orders.sequence_manager.launch_element(seek);
        engine
            .orders
            .sequence_manager
            .element_in_progress(seek_seq, 0);
        {
            let entity = engine.get_entity_mut(owner).unwrap();
            entity.actor_data_mut().unwrap().active_movement = ActiveMovement::new(seek_seq, 0);
            entity.position_iface_mut().set_map_goal(stale_goal);
        }

        let replacement =
            SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
        engine.relaunch_seek_replacement(owner, seek_seq, 0, replacement);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            MapPoint::ZERO,
            "Original's synchronous selected-Seek condolence clears the old sprite goal"
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_movement,
            ActiveMovement::none()
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seek_seq, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted
        );

        let replacement_seq = SequenceId(seek_seq.0 + 1);
        let replacement = engine
            .orders
            .sequence_manager
            .get_element(replacement_seq, 0)
            .expect("replacement Seek should remain queued for dispatch");
        assert_eq!(replacement.command, Command::Move);
        assert_eq!(replacement.state, SequenceState::Todo);
        assert!(
            engine
                .orders
                .sequence_manager
                .is_registered_to_go(replacement_seq, 0)
        );
    }
}
