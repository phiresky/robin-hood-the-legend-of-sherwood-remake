//! Door transition START, DONE, and completion boundaries.
use super::super::sequence_runtime::required_canonical_door;
use super::*;

impl EngineInner {
    pub(in crate::engine) fn apply_door_pass_transition_done_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        let entity = self
            .world
            .entities
            .get(entity_id)
            .expect("door transition owner disappeared");
        let action = entity
            .actor_data()
            .expect("door transition owner is not an actor")
            .installed_order
            .expect("door transition has no installed order")
            .order_type;
        let is_pc = entity.is_pc();
        let state = match action {
            OT::TransitionWaitingUprightClimbingWallUp => {
                Some((Posture::OnWall, ActionState::Moving))
            }
            OT::TransitionClimbingWallDownWaitingUpright => {
                Some((Posture::Upright, ActionState::Waiting))
            }
            OT::TransitionClimbingLadderUpWaitingCrouched
            | OT::TransitionClimbingLadderUpWaitingUprightAlerted => Some((
                if is_pc {
                    Posture::Crouched
                } else {
                    Posture::Upright
                },
                ActionState::Waiting,
            )),
            _ => None,
        };
        if let Some((posture, action_state)) = state {
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("door transition owner disappeared");
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .expect("door transition owner is not an actor")
                .action_state = action_state;
            return;
        }
        let door_index = entity
            .position_iface()
            .get_door()
            .expect("door transition geometry requires a live door");

        let door = required_canonical_door(
            &self.script_domains.interactables.doors,
            door_index,
            "PassDoor transition side effects",
        );
        let (layer_in, layer_out, sector_in, sector_out, point_in, point_mid, point_out) = (
            door.layer_in,
            door.layer_out,
            door.sector_in,
            door.sector_out,
            MapPoint {
                x: door.point_in.x,
                y: door.point_in.y,
            },
            MapPoint {
                x: door.point_mid.x,
                y: door.point_mid.y,
            },
            MapPoint {
                x: door.point_out.x,
                y: door.point_out.y,
            },
        );

        let lift_direction = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector_in)))
            .and_then(|sector| {
                if sector.lift_type == Some(crate::sector::LiftType::Wall) {
                    Some(sector.lift_direction)
                } else {
                    None
                }
            });

        match action {
            OT::TransitionWaitingCrouchedClimbingWallDown => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets,
                    entity_id,
                    crate::coordinates::MapPoint {
                        x: point_in.x,
                        y: point_in.y,
                    },
                );
            }
            OT::TransitionWaitingCrouchedClimbingWallDownCrenel => {
                let point_in = crate::coordinates::MapPoint::new(point_in.x, point_in.y);
                // The crenel variant re-latches the old position across the
                // teleport, so the wall-height jump to the door's entry point
                // is not reported as this frame's movement. Only the map half
                // of the latch sees the teleported point: the 3D position is
                // still the pre-teleport one when the latch happens and is
                // re-derived from the map afterwards.
                let pre_teleport_position = self
                    .get_entity(entity_id)
                    .map(|entity| entity.position_iface().get_position());
                self.finalize_special_move_position_using_projection_sector(
                    assets,
                    entity_id,
                    super::super::special_motion::SpecialMovePosition::Map(point_in),
                    layer_in,
                    u16::from(sector_in),
                    point_in,
                    "crenel climb-down transition",
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    let pi = entity.position_iface_mut();
                    pi.set_old_map_position(point_in);
                    if let Some(position) = pre_teleport_position {
                        pi.set_old_position(position);
                    }
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    let elem = entity.element_data_mut();
                    if let Some(dir) = lift_direction {
                        super::animation::direction_provenance_snapshot(
                            &elem.sprite.position_iface,
                            entity_id,
                            self.control.frame_counter,
                            "writer:crenel_completion_instant:before",
                        );
                        elem.set_direction_instantly(dir);
                        super::animation::direction_provenance_snapshot(
                            &elem.sprite.position_iface,
                            entity_id,
                            self.control.frame_counter,
                            "writer:crenel_completion_instant:after",
                        );
                    }
                    // The teleported position is re-aimed at the map goal the
                    // actor was already standing on; the direction is the one
                    // just latched from the lift, so it is not recomputed.
                    elem.sprite.position_iface.compute_increment_all(false);
                }
            }
            OT::TransitionClimbingWallUpWaitingCrouched => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(if is_pc {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    });
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Waiting;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets,
                    entity_id,
                    crate::coordinates::MapPoint {
                        x: point_mid.x,
                        y: point_mid.y,
                    },
                );
            }
            OT::TransitionClimbingWallUpWaitingCrouchedCrenel => {
                let point_out_probe = crate::coordinates::MapPoint::new(point_out.x, point_out.y);
                let point_mid_map = crate::coordinates::MapPoint::new(point_mid.x, point_mid.y);
                self.finalize_special_move_position_using_projection_sector(
                    assets,
                    entity_id,
                    super::super::special_motion::SpecialMovePosition::Map(point_mid_map),
                    layer_out,
                    u16::from(sector_out),
                    point_out_probe,
                    "crenel climb-up transition",
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::Flying);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    {
                        let pi = &mut entity.element_data_mut().sprite.position_iface;
                        let point_out = crate::coordinates::MapPoint {
                            x: point_out.x,
                            y: point_out.y,
                        };
                        pi.set_old_map_position(point_out);
                        pi.set_map_goal(point_out);
                        pi.compute_increment_all(true);
                    }
                }
            }
            _ => {}
        }
    }

    pub(in crate::engine) fn apply_door_pass_transition_start_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::coordinates::MapPoint;
        use crate::order::OrderType as OT;

        let (door_index, action) = self
            .get_entity(entity_id)
            .and_then(|entity| {
                let door = entity.position_iface().get_door()?;
                entity
                    .actor_data()?
                    .installed_order
                    .map(|order| (door, order.order_type))
            })
            .unwrap_or_else(|| {
                panic!(
                    "queued PassDoor transition START effect for {entity_id:?} has no active pass"
                )
            });
        assert!(
            matches!(
                action,
                OT::TransitionClimbingLadderUpWaitingCrouched
                    | OT::TransitionClimbingLadderUpWaitingUprightAlerted
            ),
            "queued PassDoor ladder START effect for {entity_id:?} has action {action:?}"
        );

        let door = required_canonical_door(
            &self.script_domains.interactables.doors,
            door_index,
            "PassDoor transition START",
        );
        let midpoint = MapPoint::new(door.point_mid.x, door.point_mid.y);
        // These two ladder-exit transition Execute arms align the actor to
        // the gate midpoint on the initial motion tick. This is a positional
        // alignment only: the later PassingDoor order remains responsible
        // for changing sector/layer membership.
        self.set_transition_position_map_and_compute_position_all(assets, entity_id, midpoint);
    }

    fn set_transition_position_map_and_compute_position_all(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        point: crate::coordinates::MapPoint,
    ) {
        self.finalize_special_move_position(
            assets,
            entity_id,
            super::super::special_motion::SpecialMovePosition::Map(point),
            None,
            None,
            None,
            "door transition",
        );
    }

    pub(in crate::engine) fn apply_door_pass_transition_completion_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        action: crate::order::OrderType,
    ) {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        // Exit orders can run after crossing has consumed the live door.
        // They publish actor state without reading door geometry.
        let is_pc = self
            .world
            .entities
            .get(entity_id)
            .expect("door transition owner disappeared")
            .is_pc();
        let terminal_posture = match action {
            OT::TransitionClimbingWallUpWaitingCrouchedCrenel => Some(if is_pc {
                Posture::Crouched
            } else {
                Posture::Upright
            }),
            OT::TransitionClimbingWallDownWaitingUpright
            | OT::TransitionClimbingLadderDownWaitingUpright
            | OT::TransitionClimbingLadderDownWaitingUprightAlerted => Some(Posture::Upright),
            _ => None,
        };
        if let Some(posture) = terminal_posture {
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("door transition owner disappeared");
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .expect("door transition owner is not an actor")
                .action_state = ActionState::Waiting;
            return;
        }
        let door_index = match action {
            OT::TransitionWaitingUprightClimbingWallUp
            | OT::TransitionWaitingCrouchedClimbingLadderDown
            | OT::TransitionWaitingUprightClimbingLadderDownAlerted => self
                .world
                .entities
                .get(entity_id)
                .expect("door transition owner disappeared")
                .position_iface()
                .get_door()
                .expect("door entry transition requires a live door"),
            _ => return,
        };

        let door = required_canonical_door(
            &self.script_domains.interactables.doors,
            door_index,
            "PassDoor transition completion",
        );
        let (snap_point, posture) = match action {
            OT::TransitionWaitingUprightClimbingWallUp => (
                MapPoint::new(door.point_mid.x, door.point_mid.y),
                Posture::OnWall,
            ),
            OT::TransitionWaitingCrouchedClimbingLadderDown
            | OT::TransitionWaitingUprightClimbingLadderDownAlerted => (
                MapPoint::new(door.point_in.x, door.point_in.y),
                Posture::OnLadder,
            ),
            _ => unreachable!("door entry action was classified above"),
        };
        let action_state = ActionState::Moving;
        tracing::trace!(
            ?entity_id,
            ?action,
            ?snap_point,
            ?posture,
            "door transition completion side effects"
        );
        self.set_transition_position_map_and_compute_position_all(assets, entity_id, snap_point);

        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        let elem = entity.element_data_mut();
        elem.update_grid_cell();
        entity.set_posture(posture);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = action_state;
        }
    }
}
