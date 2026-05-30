//! Per-tick RefreshSeek scan.
//!
//! ## Semantics
//!
//! A seek arms its `seek_refresh_wait` countdown to `TIME_SEEK_REFRESH`
//! (=25) at launch and again at the tail of each refresh.  While a seek
//! is translating, the per-tick driver decrements the counter and, once
//! it hits zero AND the target has moved more than 10 units (MaxNorm)
//! since the last launch, rebuilds a fresh single-element seek sequence
//! bound to the target's *current* position, sets the previous movement
//! element to interrupted, and launches the new sequence at info
//! priority.
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
//! The point-target overload ([`refresh_seek_point`]) uses the same
//! interrupt-and-relaunch primitive for classical point seeks and now
//! preserves door-sector and line-goal metadata when rebuilding those
//! seek variants.

use crate::coordinates::MapPoint;
use crate::element::{ActionState, EntityId};
use crate::engine::LevelAssets;
use crate::order::OrderType;
use crate::sequence::{
    CascadeFlags, MoveFlags, Sequence, SequenceElement, SequenceElementData, SequenceId,
};

pub(crate) struct ResolvedEntitySeek {
    pub(crate) destination: MapPoint,
    pub(crate) tolerance: f32,
    pub(crate) speed_factor: f32,
    pub(crate) stop_npc: bool,
}

impl crate::engine::EngineInner {
    /// Resolve the destination/tolerance/speed tuple for an entity-target
    /// seek.  Handles USE_POINT, moving-target chase speed, shield-danger
    /// offset, and authorized-position snapping.
    pub(crate) fn resolve_entity_seek(
        &self,
        owner: EntityId,
        target: EntityId,
        flags: MoveFlags,
        seek_distance: f32,
    ) -> Option<ResolvedEntitySeek> {
        let owner_entity = self.get_entity(owner)?;
        let target_entity = self.get_entity(target)?;
        let target_elem = target_entity.element_data();
        let target_pos = target_elem.position_map();
        let target_geo = crate::geo2d::pt(target_pos.x, target_pos.y);
        let target_layer = target_elem.layer();

        let owner_move_box = *owner_entity.position_iface().get_move_box();

        if flags.contains(MoveFlags::USE_POINT) {
            let current_point = target_elem
                .sprite
                .current_hotspot()
                .filter(|p| p.x != 0.0 || p.y != 0.0)
                .map(|p| crate::geo2d::pt(target_pos.x + p.x, target_pos.y + p.y))
                .unwrap_or(target_geo);
            let mut target_box = owner_move_box.translated(current_point);
            if self.fast_grid.find_authorized_position_toward(
                &mut target_box,
                target_pos,
                target_layer,
            ) {
                return Some(ResolvedEntitySeek {
                    destination: target_box.center().into(),
                    tolerance: seek_distance,
                    speed_factor: 1.0,
                    stop_npc: false,
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
            let owner_pos = owner_entity.element_data().position_map();
            let dx = target_pos.x - owner_pos.x;
            let dy = target_pos.y - owner_pos.y;
            dx * dx + dy * dy < send_stop_sqr
        } else {
            false
        };

        let mut destination = target_geo;
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
                destination = crate::geo2d::pt(
                    target_pos.x + vx / len * 50.0,
                    target_pos.y + vy / len * 50.0,
                );
            } else {
                tolerance = seek_distance;
            }
        }

        let mut target_box = owner_move_box.translated(destination);
        if self
            .fast_grid
            .find_authorized_position_toward(&mut target_box, target_pos, target_layer)
        {
            Some(ResolvedEntitySeek {
                destination: target_box.center().into(),
                tolerance,
                speed_factor,
                stop_npc,
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

    /// Scan every actor with an in-flight seek movement and re-launch
    /// it when the target has moved more than 10 units (MaxNorm) since
    /// the seek was last (re-)issued.
    ///
    /// Runs once per tick before the sequence manager hourglass so the
    /// freshly-launched seek sequence gets picked up in the same tick.
    pub(super) fn tick_refresh_seeks(&mut self, assets: &LevelAssets) {
        if self.freeze_all {
            return;
        }

        struct Refresh {
            owner: EntityId,
            seq_id: crate::sequence::SequenceId,
            elem_idx: usize,
            target: EntityId,
            action: crate::order::OrderType,
            flags: MoveFlags,
            tolerance: f32,
            new_target_pos: crate::coordinates::MapPoint,
        }

        let mut refreshes: Vec<Refresh> = Vec::new();

        for (owner_id, entity) in self.entities.actors() {
            let owner_id = EntityId::from(owner_id);
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let Some(seq_id) = actor.active_movement.sequence_id else {
                continue;
            };
            let elem_idx = actor.active_movement.element_index;
            let Some(elem) = self.sequence_manager.get_element(seq_id, elem_idx) else {
                continue;
            };
            if !matches!(
                elem.command,
                crate::element::Command::Move | crate::element::Command::Seek
            ) {
                continue;
            }
            let SequenceElementData::Movement {
                flags,
                element: target,
                tolerance,
                action,
                ..
            } = &elem.data
            else {
                continue;
            };
            if !flags.contains(MoveFlags::SEEK) {
                continue;
            }
            let Some(target_id) = *target else {
                continue;
            };
            let Some(target_entity) = self.get_entity(target_id) else {
                continue;
            };
            let target_pos = target_entity.element_data().position_map();

            // Countdown gate — when still >0, just decrement and skip
            // (collected below via `decrement_only`).
            if actor.seek_refresh_wait > 0 {
                continue;
            }
            let last = actor.last_seek_target_position;
            let dx = (target_pos.x - last.x).abs();
            let dy = (target_pos.y - last.y).abs();
            if dx.max(dy) <= 10.0 {
                continue;
            }

            refreshes.push(Refresh {
                owner: owner_id,
                seq_id,
                elem_idx,
                target: target_id,
                action: *action,
                flags: *flags,
                tolerance: *tolerance,
                new_target_pos: target_pos,
            });
        }

        // Decrement `seek_refresh_wait` for every actor with an active
        // seek, regardless of whether it triggered.
        for (_, entity) in self.entities.actors_mut() {
            let Some(actor) = entity.actor_data_mut() else {
                continue;
            };
            if actor.active_movement.sequence_id.is_none() {
                continue;
            }
            if actor.seek_refresh_wait > 0 {
                actor.seek_refresh_wait -= 1;
            }
        }

        for r in refreshes {
            tracing::trace!(
                owner = ?r.owner,
                target = ?r.target,
                new_x = r.new_target_pos.x,
                new_y = r.new_target_pos.y,
                "tick_refresh_seeks: target moved >10u, re-launching seek",
            );
            self.apply_seek_refresh(
                assets,
                r.owner,
                r.seq_id,
                r.elem_idx,
                r.target,
                r.action,
                r.flags,
                r.tolerance,
                r.new_target_pos,
            );
        }
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
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        target: EntityId,
        action: crate::order::OrderType,
        flags: MoveFlags,
        tolerance: f32,
        new_target_pos: crate::coordinates::MapPoint,
    ) {
        if self.try_handle_same_sector_actor_seek_wait(owner, seq_id, elem_idx, target, flags) {
            return;
        }

        if self.try_dispatch_cross_sector_entity_seek(
            assets, owner, seq_id, elem_idx, target, action, flags, tolerance,
        ) {
            return;
        }

        let Some(resolved) = self.resolve_entity_seek(owner, target, flags, tolerance) else {
            self.stop_owner_active_mechanics(owner);
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        };
        if resolved.stop_npc {
            self.send_seek_stop_to_npc(target);
        }

        let mut new_elem =
            SequenceElement::new_movement(1, crate::element::Command::Seek, Some(owner), action);
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

        // Stamp the new last-seek-position so the next tick's
        // threshold check measures against this launch, and re-arm
        // `seek_refresh_wait` (every refresh re-arms the countdown).
        // `try_dispatch_move_path` will overwrite both when the new
        // element dispatches, but stamp them here too so a dispatch
        // failure still leaves coherent state.
        if let Some(Some(entity)) = self.entities.get_mut(owner.index() as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.last_seek_target_position = new_target_pos;
            actor.seek_refresh_wait = 25;
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
                self.start_post_seek_sequence(owner, Some((seq_id, elem_idx)));
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
            .and_then(|(seq_id, elem_idx)| self.sequence_manager.get_element(seq_id, elem_idx))
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
                let pi = e.position_iface();
                (
                    elem.position_map().to_geo(),
                    elem.sector(),
                    pi.get_door(),
                    pi.get_door_direction(),
                )
            }
            None => {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return true;
            }
        };
        let (target_pos, target_sector, target_layer) = match self.get_entity(target) {
            Some(e) => {
                let elem = e.element_data();
                (elem.position_map().to_geo(), elem.sector(), elem.layer())
            }
            None => {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return true;
            }
        };
        let (Some(owner_sector), Some(target_sector)) = (owner_sector, target_sector) else {
            return false;
        };
        if owner_sector == target_sector {
            return false;
        }

        let Some(resolved) = self.resolve_entity_seek(owner, target, flags, seek_distance) else {
            // Original RefreshSeek marks the current movement
            // impossible silently when FindAutorizedPosition fails.
            // The unable-to-do bark belongs to AppendMoveToSequence's
            // gate-path failure below.
            self.stop_owner_active_mechanics(owner);
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return true;
        };

        let (path_src_pos, path_src_sector) = {
            let host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
            let adapted = host.and_then(|h| {
                crate::engine::movement::adapt_source_to_current_door(
                    &h.doors,
                    door_handle,
                    door_direction,
                )
            });
            match adapted {
                Some((adj, sector, _layer)) => (adj, sector),
                None => (owner_pos.into(), u16::from(owner_sector)),
            }
        };

        let owner_auth = self.get_entity(owner).map(|e| e.actor_auth_info());
        let gate_path = {
            let host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
            host.and_then(|h| {
                crate::gate::find_path_gates(
                    &h.doors,
                    (path_src_pos.x, path_src_pos.y),
                    path_src_sector,
                    (resolved.destination.x, resolved.destination.y),
                    u16::from(target_sector),
                    owner_auth.as_ref(),
                    false,
                    &|sector| {
                        h.sector_kinds
                            .get(&u16::from(sector))
                            .and_then(|k| k.lift_type)
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
            self.stop_owner_active_mechanics(owner);
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return true;
        };

        let tail_elements = self
            .get_entity_mut(owner)
            .and_then(|e| e.actor_data_mut())
            .and_then(|actor| actor.post_seek_sequence.take())
            .map(|seq| seq.elements)
            .unwrap_or_default();

        self.stop_owner_active_mechanics(owner);
        self.sequence_manager
            .element_interrupted(seq_id, elem_idx, CascadeFlags::NEXT_LEVEL);

        self.build_gate_movement_sequence(
            owner,
            gate_path,
            crate::engine::movement::GoalShape::Seek {
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
            tail_elements,
            false,
            true,
        );

        if resolved.stop_npc {
            self.send_seek_stop_to_npc(target);
        }

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

    /// Shared tail of both `RefreshSeek` overloads: interrupt the
    /// actor's current movement element and launch a fresh single-
    /// element seek sequence at info priority.  The
    /// `stop_owner_active_mechanics` call cancels any in-flight path
    /// request belonging to the interrupted element.
    pub(super) fn relaunch_seek_replacement(
        &mut self,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
        new_elem: SequenceElement,
    ) {
        self.stop_owner_active_mechanics(owner);
        self.sequence_manager
            .element_interrupted(seq_id, elem_idx, CascadeFlags::NEXT_LEVEL);

        let mut seq = Sequence::new();
        seq.append_element(new_elem);
        self.launch_sequence(seq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorData, ActorPc, Command, ElementData, ElementKind, Entity, HumanData, PcData, Posture,
    };
    use crate::movement::ActiveMovement;
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElementData, SequenceState};

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

    #[test]
    fn refresh_seek_waits_when_same_sector_actor_target_is_passing_door() {
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
        let seek_seq = engine.sequence_manager.launch_element(seek);
        engine.sequence_manager.element_in_progress(seek_seq, 0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(seek_seq, 0);

        let pass = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(target),
            OrderType::WalkingUpright,
        );
        let pass_seq = engine.sequence_manager.launch_element(pass);
        engine.sequence_manager.element_in_progress(pass_seq, 0);
        engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_movement = ActiveMovement::new(pass_seq, 0);

        engine.apply_seek_refresh(
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

        assert_eq!(engine.sequence_manager.sequence_count(), 2);
        assert_eq!(
            engine
                .sequence_manager
                .get_element(seek_seq, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
    }
}
