//! Sequence execution contexts and ordered runtime dispatch.
//!
//! The original engine drains registered sequence elements after entity
//! hourglasses and permits `Ready()` to launch successor command levels
//! synchronously. These modules keep that ordering local while borrowing the
//! existing [`EngineInner`] domains directly.

mod immediate;
mod phase;
mod script_sync;

use super::movement::MovePathOutcome;
use super::*;
use crate::abilities::{self, BeginResult as AbilityBeginResult};
use crate::bow_shot::{self, BeginShotResult};
use crate::element::{Command, Entity, EntityId};

/// Transient ordered work owned by the sequence phase.
///
/// The context owns no simulation state and cannot reach `EngineInner`.
/// Sequence-manager mutation is borrowed only at the two ordering barriers:
/// initial `Hourglass` collection and the after-action synchronous splice.
/// Keeping those operations here makes the same-call front-of-queue rule
/// explicit without inventing a deferred gameplay queue.
struct SequencePhase {
    initial_actions: Vec<crate::sequence::SequenceAction>,
    actions: std::collections::VecDeque<crate::sequence::SequenceAction>,
}

impl SequencePhase {
    fn begin(orders: &mut OrderRuntime) -> Self {
        Self {
            initial_actions: orders.sequence_manager.hourglass(),
            actions: std::collections::VecDeque::new(),
        }
    }

    fn initial_actions(&self) -> &[crate::sequence::SequenceAction] {
        &self.initial_actions
    }

    fn begin_dispatch(&mut self) {
        debug_assert!(self.actions.is_empty());
        self.actions = std::mem::take(&mut self.initial_actions).into();
    }

    fn pop_action(&mut self) -> Option<crate::sequence::SequenceAction> {
        self.actions.pop_front()
    }

    /// Splice newly registered synchronous work before every older action,
    /// preserving the manager's registration order.
    fn splice_synchronous_actions(&mut self, orders: &mut OrderRuntime) {
        let pending = orders.sequence_manager.take_pending_synchronous_actions();
        for action in pending.into_iter().rev() {
            self.actions.push_front(action);
        }
    }
}

impl EngineInner {
    /// Complete the ordinary Move/Seek path translation after its destination
    /// has been resolved. Both the regular hourglass and SetAIState's exact
    /// owner-local native barrier use this same outcome handling.
    fn dispatch_prepared_move_instruction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        dest: crate::coordinates::MapPoint,
        move_action: crate::order::OrderType,
    ) {
        match self.try_dispatch_move_path(sim, owner, seq_id, elem_idx, dest, move_action) {
            MovePathOutcome::Success | MovePathOutcome::Pending => {}
            MovePathOutcome::ActorGone => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
            MovePathOutcome::Failed => {
                let source = self.get_entity(owner).map(|e| {
                    let elem = e.element_data();
                    (
                        elem.position_map(),
                        elem.layer(),
                        elem.sector().map(u16::from),
                    )
                });
                let movement_meta = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|elem| match &elem.data {
                        crate::sequence::SequenceElementData::Movement {
                            flags,
                            line_id,
                            gate_id,
                            sector,
                            layer,
                            ..
                        } => Some((*flags, *line_id, *gate_id, *sector, *layer)),
                        _ => None,
                    });
                tracing::warn!(
                    actor = ?owner,
                    ?seq_id,
                    elem_idx,
                    dest_x = dest.x,
                    dest_y = dest.y,
                    src_x = source.map(|(p, _, _)| p.x),
                    src_y = source.map(|(p, _, _)| p.y),
                    src_layer = source.map(|(_, layer, _)| layer),
                    src_sector = source.and_then(|(_, _, sector)| sector),
                    elem_flags = ?movement_meta.map(|(flags, _, _, _, _)| flags),
                    elem_line = ?movement_meta.and_then(|(_, line, _, _, _)| line),
                    elem_gate = ?movement_meta.and_then(|(_, _, gate, _, _)| gate),
                    elem_sector = ?movement_meta.and_then(|(_, _, _, sector, _)| sector),
                    elem_layer = ?movement_meta.map(|(_, _, _, _, layer)| layer),
                    action = ?move_action,
                    frame = self.control.frame_counter,
                    "Move path dispatch failed; queuing 100-frame failed_path timeout"
                );
                self.orders
                    .failed_path_requests
                    .push(crate::engine::movement::FailedPathRequest {
                        owner,
                        seq_id,
                        elem_idx,
                        first_fail_frame: self.control.frame_counter,
                    });
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);
            }
        }
    }
}

/// Whether an extracted owner-command dispatcher reaches the synchronous
/// successor splice at the bottom of the action loop.
///
/// Several legacy command paths deliberately `continue` after updating their
/// sequence element. Keeping that distinction in the return type prevents a
/// helper extraction from silently changing same-call action ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::engine) enum OwnerActionBarrier {
    Reach,
    Skip,
}

pub(in crate::engine) fn required_canonical_door<'a>(
    doors: &'a [crate::gate::Door],
    door_id: crate::gate::DoorIndex,
    context: &'static str,
) -> &'a crate::gate::Door {
    doors
        .get(usize::from(door_id))
        .unwrap_or_else(|| panic!("{context} references missing canonical door {door_id}"))
}

pub(in crate::engine) fn required_canonical_door_mut<'a>(
    doors: &'a mut [crate::gate::Door],
    door_id: crate::gate::DoorIndex,
    context: &'static str,
) -> &'a mut crate::gate::Door {
    doors
        .get_mut(usize::from(door_id))
        .unwrap_or_else(|| panic!("{context} references missing canonical door {door_id}"))
}

fn required_unlock_door_id(
    element: Option<&crate::sequence::SequenceElement>,
    seq_id: crate::sequence::SequenceId,
    elem_idx: usize,
) -> crate::gate::DoorIndex {
    let element = element.unwrap_or_else(|| {
        panic!("UnlockDoor sequence element {seq_id:?}/{elem_idx} disappeared during dispatch")
    });
    match element.get_property(crate::sequence::Field::Door) {
        Some(crate::sequence::FieldValue::DoorId(id)) => *id,
        Some(crate::sequence::FieldValue::Integer(id)) => crate::gate::DoorIndex(*id),
        _ => panic!("UnlockDoor sequence element {seq_id:?}/{elem_idx} has no Door property"),
    }
}

fn read_sequence_map_point_property(
    element: &crate::sequence::SequenceElement,
    field: crate::sequence::Field,
) -> Option<crate::coordinates::MapPoint> {
    match element.get_property(field)? {
        crate::sequence::FieldValue::GeoPoint2D { x, y }
        | crate::sequence::FieldValue::Point3D { x, y, .. } => {
            Some(crate::coordinates::MapPoint::new(*x, *y))
        }
        _ => None,
    }
}

/// Synchronous position assertion against entity state and its owning
/// sequence element.
///
/// Original provenance: `RHelementactor.cpp:3006-3035` performs this check
/// directly from `Translate`: a sector-less assertion compares max-norm
/// distance against `tolerance + 5`, while a sector assertion compares only
/// the sector. Both paths interrupt on mismatch and terminate on success.
pub(in crate::engine) struct PositionAssertionContext<'a> {
    pub(in crate::engine) entities: &'a crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl PositionAssertionContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let movement = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement {
                    destination,
                    sector,
                    tolerance,
                    ..
                } => Some((*destination, *sector, *tolerance)),
                _ => None,
            });

        let matches = movement.is_none_or(|(destination, expected_sector, tolerance)| {
            let entity = self.entities.get(owner);
            if let Some(expected_sector) = expected_sector {
                entity.and_then(|entity| entity.element_data().sector()) == Some(expected_sector)
            } else {
                let position = entity
                    .map(|entity| entity.element_data().position_map())
                    .unwrap_or_default();
                let delta_x = position.x - destination.x;
                let delta_y = position.y - destination.y;
                delta_x.abs().max(delta_y.abs()) < tolerance + 5.0
            }
        });

        if matches {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_interrupted(
                seq_id,
                elem_idx,
                crate::sequence::CascadeFlags::NEXT_LEVEL,
            );
        }
        OwnerActionBarrier::Reach
    }
}

/// Owner-local WAIT_FREE_LIFT arbitration after Actor::Execute.
///
/// Translation books the same stationary order as WAIT. This context is then
/// invoked once per actual owner Execute while the element remains current,
/// matching `RHelementactor.cpp:624-657`.
pub(in crate::engine) struct LiftWaitCommandContext<'a> {
    pub(in crate::engine) entities: &'a mut crate::entities::Entities,
    pub(in crate::engine) fast_grid: &'a mut crate::fast_find_grid::FastFindGrid,
    pub(in crate::engine) doors: &'a [crate::gate::Door],
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl LiftWaitCommandContext<'_> {
    pub(in crate::engine) fn authorize_and_reserve(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> bool {
        let element = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap_or_else(|| {
                panic!("WAIT_FREE_LIFT owner {owner:?} lost current element {seq_id:?}/{elem_idx}")
            });
        assert_eq!(
            element.command,
            Command::WaitFreeLift,
            "WAIT_FREE_LIFT owner {owner:?} modifier received {:?} at {seq_id:?}/{elem_idx}",
            element.command
        );
        let (gate_id, target_sector) = match &element.data {
            crate::sequence::SequenceElementData::Movement {
                gate_id: Some(gate_id),
                sector: Some(sector),
                ..
            } => (*gate_id, *sector),
            data => panic!(
                "WAIT_FREE_LIFT owner {owner:?} requires movement gate and sector at {seq_id:?}/{elem_idx}, found {data:?}"
            ),
        };
        let door = self.doors.get(usize::from(gate_id)).unwrap_or_else(|| {
            panic!(
                "WAIT_FREE_LIFT owner {owner:?} references missing door {gate_id} at {seq_id:?}/{elem_idx}"
            )
        });
        let is_high = match door.door_type {
            crate::gate::DoorType::LiftHigh => true,
            crate::gate::DoorType::LiftLow => false,
            other => panic!(
                "WAIT_FREE_LIFT owner {owner:?} door {gate_id} must be LiftHigh or LiftLow, found {other:?}"
            ),
        };
        assert_eq!(
            i16::from(target_sector),
            i16::from(door.sector_in),
            "WAIT_FREE_LIFT owner {owner:?} target sector {} disagrees with door {gate_id} inside sector {}",
            u16::from(target_sector),
            i16::from(door.sector_in)
        );
        let owner_sector = self
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("WAIT_FREE_LIFT owner {owner:?} is missing"))
            .element_data()
            .sector()
            .unwrap_or_else(|| panic!("WAIT_FREE_LIFT owner {owner:?} has no current sector"));
        if i16::from(owner_sector) != i16::from(door.sector_out) {
            return false;
        }
        let sector_number = door.sector_in;
        let grid_idx = *self
            .fast_grid
            .level
            .sector_number_map
            .get(&sector_number)
            .unwrap_or_else(|| {
                panic!(
                    "WAIT_FREE_LIFT owner {owner:?} door {gate_id} references missing lift sector {sector_number:?}"
                )
            });
        let sector = self
            .fast_grid
            .level
            .sectors
            .get(grid_idx)
            .unwrap_or_else(|| {
                panic!(
                    "WAIT_FREE_LIFT owner {owner:?} door {gate_id} resolved invalid sector index {grid_idx}"
                )
            });
        assert!(
            sector.lift_type.is_some(),
            "WAIT_FREE_LIFT owner {owner:?} door {gate_id} inside sector {sector_number:?} is not a lift"
        );

        // Authorization decrements the cooldown while blocked. Once free,
        // occupancy is recorded before the element terminates so another
        // actor dispatched in the same frame observes the reservation.
        let authorized = {
            let lift = self.fast_grid.lift_state_mut(grid_idx as u32);
            if is_high {
                lift.is_authorized_downwards()
            } else {
                lift.is_authorized_upwards()
            }
        };

        if authorized {
            let lift = self.fast_grid.lift_state_mut(grid_idx as u32);
            if is_high {
                lift.set_occupied_downwards(true);
            } else {
                lift.set_occupied_upwards(true);
            }
            let actor = self
                .entities
                .get_mut(owner)
                .unwrap_or_else(|| {
                    panic!("WAIT_FREE_LIFT owner {owner:?} vanished during reservation")
                })
                .actor_data_mut()
                .unwrap_or_else(|| panic!("WAIT_FREE_LIFT owner {owner:?} is not an actor"));
            actor.active_lift = Some(crate::element::ActiveLiftClimb {
                sector_number: u16::from(target_sector),
                upwards: !is_high,
            });
        }
        authorized
    }
}

/// Translate one WAIT-priority smalltalk strike/parry at the synchronous
/// owner boundary where it was launched.
pub(in crate::engine) struct SmalltalkCommandContext<'a> {
    pub(in crate::engine) entities: &'a crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
}

impl SmalltalkCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let antagonist = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap_or_else(|| {
                panic!(
                    "smalltalk owner {owner:?} lost element {seq_id:?}/{elem_idx} during translation"
                )
            });
        let antagonist = match antagonist.data {
            crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(antagonist),
            } => antagonist,
            ref data => panic!(
                "smalltalk owner {owner:?} command {command:?} requires an interaction antagonist at {seq_id:?}/{elem_idx}, found {data:?}"
            ),
        };
        let owner_entity = self
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("smalltalk command {command:?} owner {owner:?} is missing"));
        let opponent = self.entities.get(antagonist).unwrap_or_else(|| {
            panic!(
                "smalltalk command {command:?} owner {owner:?} references missing antagonist {antagonist:?}"
            )
        });
        assert!(
            opponent.human_data().is_some(),
            "smalltalk command {command:?} owner {owner:?} antagonist {antagonist:?} is not human"
        );
        let owner_higher =
            owner_entity.element_data().position().z >= opponent.element_data().position().z + 20.0;
        let order_type = match command {
            Command::SwordstrikeSmalltalkLeft if owner_higher => {
                crate::order::OrderType::StrikingLowLeftSmalltalk
            }
            Command::SwordstrikeSmalltalkLeft => crate::order::OrderType::StrikingLeftSmalltalk,
            Command::SwordstrikeSmalltalkRight if owner_higher => {
                crate::order::OrderType::StrikingLowRightSmalltalk
            }
            Command::SwordstrikeSmalltalkRight => crate::order::OrderType::StrikingRightSmalltalk,
            Command::ParrySmalltalkLeft if owner_higher => {
                crate::order::OrderType::ParryingLowLeftSmalltalk
            }
            Command::ParrySmalltalkLeft => crate::order::OrderType::ParryingLeftSmalltalk,
            Command::ParrySmalltalkRight if owner_higher => {
                crate::order::OrderType::ParryingLowRightSmalltalk
            }
            Command::ParrySmalltalkRight => crate::order::OrderType::ParryingRightSmalltalk,
            _ => unreachable!("non-smalltalk command passed to SmalltalkCommandContext"),
        };
        let blocked = owner_entity
            .actor_data()
            .unwrap_or_else(|| {
                panic!("smalltalk command {command:?} owner {owner:?} is not an actor")
            })
            .active_melee
            .is_active();
        if blocked {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return;
        }

        let order = crate::order::Order::new(
            order_type,
            0.0,
            0.0,
            crate::order::alloc_order_id(self.next_order_id),
        );
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
    }
}

/// Bow-transition command translation with only the owners it actually uses.
///
/// The original command bodies read actor posture/action state, append
/// transition orders, and update the sequence element. They do not need the
/// mission, scripts, AI, players, feedback, or spatial world domains.
struct BowTransitionContext<'a> {
    entities: &'a crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl BowTransitionContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let command_body_already_queued = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|element| {
                element.orders.iter().any(|order| {
                    use crate::order::OrderType as OT;
                    match command {
                        Command::EquipBow => matches!(
                            order.order_type,
                            OT::TransitionEquipBow | OT::TransitionEquipBowAnonymous
                        ),
                        Command::EquipBowDown => {
                            order.order_type == OT::TransitionLoweringBowLeaningOut
                        }
                        Command::UnequipBow => matches!(
                            order.order_type,
                            OT::TransitionUnloadBow
                                | OT::TransitionUnloadBowAnonymous
                                | OT::TransitionUnequipBow
                                | OT::TransitionUnequipBowAnonymous
                        ),
                        Command::RaiseBow => matches!(
                            order.order_type,
                            OT::TransitionRaisingBow | OT::TransitionRaisingBowAnonymous
                        ),
                        Command::LowerBow => matches!(
                            order.order_type,
                            OT::TransitionLoweringBow | OT::TransitionLoweringBowAnonymous
                        ),
                        _ => false,
                    }
                })
            });

        if !command_body_already_queued {
            let owner_entity = self
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("bow command owner missing: {owner:?}"));
            let posture = owner_entity.element_data().posture;
            let owner_action_state = owner_entity
                .actor_data()
                .map(|actor| actor.action_state)
                .unwrap_or_else(|| panic!("bow command owner missing actor data: {owner:?}"));
            if matches!(command, Command::EquipBow | Command::EquipBowDown)
                && owner_action_state.is_bow()
            {
                // C++ `Translate(EQUIP_BOW*)` terminates non-transition
                // command bodies when the actor is already aiming.
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            }

            let anonymous = posture == crate::element::Posture::AnonymousArcher;
            let target_xy = self
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| element.orders.back())
                .map(|order| (order.target_x, order.target_y))
                .unwrap_or((0.0, 0.0));

            use crate::element::ActionState;
            use crate::order::OrderType;
            match command {
                Command::EquipBow => {
                    if anonymous {
                        self.push_order(
                            seq_id,
                            elem_idx,
                            OrderType::TransitionEquipBowAnonymous,
                            0.0,
                            0.0,
                        );
                        self.push_order(
                            seq_id,
                            elem_idx,
                            OrderType::TransitionLoadingBowAnonymous,
                            0.0,
                            0.0,
                        );
                    } else {
                        self.push_order(seq_id, elem_idx, OrderType::TransitionEquipBow, 0.0, 0.0);
                        self.push_order(
                            seq_id,
                            elem_idx,
                            OrderType::TransitionLoadingBow,
                            0.0,
                            0.0,
                        );
                    }
                    self.set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBow,
                    );
                }
                Command::EquipBowDown => {
                    self.push_order(seq_id, elem_idx, OrderType::TransitionEquipBow, 0.0, 0.0);
                    self.push_order(seq_id, elem_idx, OrderType::TransitionLoadingBow, 0.0, 0.0);
                    self.push_order(
                        seq_id,
                        elem_idx,
                        OrderType::TransitionLoweringBowLeaningOut,
                        0.0,
                        0.0,
                    );
                    self.set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBowDown,
                    );
                }
                Command::UnequipBow => {
                    let (x, y) = target_xy;
                    if anonymous {
                        self.push_order(
                            seq_id,
                            elem_idx,
                            OrderType::TransitionUnloadBowAnonymous,
                            x,
                            y,
                        );
                        self.push_order(
                            seq_id,
                            elem_idx,
                            OrderType::TransitionUnequipBowAnonymous,
                            x,
                            y,
                        );
                    } else {
                        self.push_order(seq_id, elem_idx, OrderType::TransitionUnloadBow, x, y);
                        self.push_order(seq_id, elem_idx, OrderType::TransitionUnequipBow, x, y);
                    }
                    self.set_action_state_after_transition(seq_id, elem_idx, ActionState::Waiting);
                }
                Command::RaiseBow => {
                    self.push_order(
                        seq_id,
                        elem_idx,
                        if anonymous {
                            OrderType::TransitionRaisingBowAnonymous
                        } else {
                            OrderType::TransitionRaisingBow
                        },
                        0.0,
                        0.0,
                    );
                    self.set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBowUp,
                    );
                }
                Command::LowerBow => {
                    self.push_order(
                        seq_id,
                        elem_idx,
                        if anonymous {
                            OrderType::TransitionLoweringBowAnonymous
                        } else {
                            OrderType::TransitionLoweringBow
                        },
                        0.0,
                        0.0,
                    );
                    self.set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBow,
                    );
                }
                _ => unreachable!("non-bow command passed to bow transition context"),
            }
        }

        let has_orders = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|element| !element.orders.is_empty());
        if has_orders {
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
        OwnerActionBarrier::Reach
    }

    fn push_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
        target_x: f32,
        target_y: f32,
    ) {
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(order_type, target_x, target_y, id);
        order.compute_direction = false;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
    }

    fn set_action_state_after_transition(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        state: crate::element::ActionState,
    ) {
        let element = self
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("bow transition sequence element disappeared during dispatch");
        element.action_state_after_transition = state;
    }
}

/// Script-target activation collection with no mutable world access.
struct TargetActivationContext<'a> {
    entities: &'a crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    pending_activations: &'a mut Vec<(i32, i32, &'static str)>,
}

impl TargetActivationContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        antagonist: Option<EntityId>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let method = match command {
            Command::ActivateApple => "ActivatedByApple",
            Command::ActivateArrow => "ActivatedByArrow",
            Command::ActivateHandle => "ActivatedByHand",
            Command::ActivateHeal => "ActivatedByHeal",
            Command::ActivateLever => "ActivatedByLever",
            Command::ActivateMoney => "ActivatedByMoney",
            Command::ActivateSearch => "ActivatedBySearch",
            Command::ActivateStone => "ActivatedByStone",
            Command::ActivateSword => "ActivatedBySword",
            _ => unreachable!("non-activation command passed to target activation context"),
        };
        debug_assert!(
            self.entities
                .get(owner)
                .is_some_and(|entity| entity.kind().is_fx_target()),
            "{method} dispatched on non-FX-target owner {owner:?}",
        );
        let target_handle = crate::natives::ScriptHandleCodec::actor_handle(owner);
        let pc_handle = antagonist
            .map(crate::natives::ScriptHandleCodec::actor_handle)
            .unwrap_or(0);
        self.pending_activations
            .push((target_handle, pc_handle, method));
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}

/// Actor/FX animation and target-interaction translation.
struct TargetAnimationContext<'a> {
    entities: &'a mut crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl TargetAnimationContext<'_> {
    fn dispatch_play_animation(
        &mut self,
        owner: EntityId,
        command: Command,
        animation: Option<crate::order::OrderType>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let Some(animation) = animation else {
            tracing::warn!(
                entity = ?owner,
                cmd = ?command,
                "PlayAnim*: missing/invalid AnimationId — terminating",
            );
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };

        let Some(owner_entity) = self.entities.get(owner) else {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        if owner_entity.is_human() {
            let id = crate::order::alloc_order_id(self.next_order_id);
            let mut order = crate::order::Order::new(animation, 0.0, 0.0, id);
            order.compute_direction = false;
            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }

        if !owner_entity.kind().is_fx_target() {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }

        let progression_ordinal = match command {
            Command::PlayAnim => crate::sprite::FrameProgression::Default as u32,
            Command::PlayAnimLoop => crate::sprite::FrameProgression::Cyclically as u32,
            Command::PlayAnimFreeze => crate::sprite::FrameProgression::FreezeWhenTerminated as u32,
            Command::PlayAnimFrozen => crate::sprite::FrameProgression::FrozenLastFrame as u32,
            _ => unreachable!("non-animation command passed to target animation context"),
        };
        let entity = self
            .entities
            .get_mut(owner)
            .expect("FX target disappeared during PlayAnim dispatch");
        let direction = entity.element_data().direction() as u16;
        if let crate::element::Entity::Target(target) = entity {
            target.target.progression = progression_ordinal;
        }
        let sprite = &mut entity.element_data_mut().sprite;
        if sprite.has_animation(animation) {
            sprite.force_animation(animation, direction);
            sprite.reset_sprite_frame(false);
        } else {
            tracing::warn!(
                ?owner,
                ?animation,
                profile = %sprite.frame_profile_name,
                "PlayAnim*: animation unmapped for this sprite profile — skipping",
            );
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }
}

/// PC-side FX-target orders need read-only entity classification plus the two
/// order fields they mutate; they never need mutable world access.
struct TargetInteractionContext<'a> {
    entities: &'a crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl TargetInteractionContext<'_> {
    fn dispatch(
        &mut self,
        owner_command: Command,
        target: Option<EntityId>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let Some(target) = target else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        if !self
            .entities
            .get(target)
            .is_some_and(|entity| entity.kind().is_fx_target())
        {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }
        let order_type = match owner_command {
            Command::HitTarget => crate::order::OrderType::HittingTarget,
            Command::HandleTarget => crate::order::OrderType::HandlingTarget,
            Command::UseLever => crate::order::OrderType::UsingLever,
            Command::TakeTarget => crate::order::OrderType::TakingTarget,
            Command::SearchCmd => crate::order::OrderType::Searching,
            _ => unreachable!("non-target command passed to target interaction context"),
        };
        let id = crate::order::alloc_order_id(self.next_order_id);
        let order = crate::order::Order::new(order_type, 0.0, 0.0, id).with_antagonist(target);
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }
}

/// Directional and one-shot owner commands that only mutate entity facing
/// plus the owning sequence/order allocator.
pub(in crate::engine) struct TurnCommandContext<'a> {
    pub(in crate::engine) entities: &'a mut crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
}

impl TurnCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        match command {
            Command::Turn | Command::TurnFast => {
                let element = self.sequence_manager.get_element(seq_id, elem_idx);
                let camera_point = element
                    .and_then(|element| {
                        read_sequence_map_point_property(
                            element,
                            crate::sequence::Field::CameraPoint,
                        )
                    })
                    .map(|point| (point.x, point.y));
                let explicit_direction = element
                    .and_then(|element| element.get_property(crate::sequence::Field::Direction))
                    .and_then(|value| match value {
                        crate::sequence::FieldValue::Integer(direction) => Some(*direction as i16),
                        _ => None,
                    });
                if let Some(entity) = self.entities.get_mut(owner) {
                    if let Some(direction) = explicit_direction {
                        entity.element_data_mut().set_direction_goal(direction);
                    } else if let Some((target_x, target_y)) = camera_point {
                        let position = entity.element_data().position_map();
                        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                            target_x - position.x,
                            target_y - position.y,
                        );
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                }
                self.push_order(seq_id, elem_idx, crate::order::OrderType::Turning, true);
            }
            Command::TurnElement => {
                let antagonist = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| match &element.data {
                        crate::sequence::SequenceElementData::Interaction { antagonist } => {
                            *antagonist
                        }
                        _ => None,
                    });
                if let Some(antagonist) = antagonist {
                    let antagonist_position = self
                        .entities
                        .get(antagonist)
                        .map(|entity| entity.element_data().position_map());
                    if let (Some(antagonist_position), Some(entity)) =
                        (antagonist_position, self.entities.get_mut(owner))
                    {
                        let position = entity.element_data().position_map();
                        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                            antagonist_position.x - position.x,
                            antagonist_position.y - position.y,
                        );
                        entity.element_data_mut().set_direction_instantly(direction);
                    }
                }
                self.push_order(seq_id, elem_idx, crate::order::OrderType::Turning, true);
            }
            Command::Freeze => {
                self.push_order(seq_id, elem_idx, crate::order::OrderType::Freezing, true);
            }
            Command::Point | Command::GatherSoldiers => {
                let order_type = match command {
                    Command::Point => crate::order::OrderType::Pointing,
                    Command::GatherSoldiers => crate::order::OrderType::GatheringSoldiers,
                    _ => unreachable!(),
                };
                if command == Command::Point {
                    let explicit_direction = self
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .and_then(|element| element.get_property(crate::sequence::Field::Direction))
                        .and_then(|value| match value {
                            crate::sequence::FieldValue::Integer(direction) => {
                                Some(*direction as i16)
                            }
                            _ => None,
                        });
                    if let (Some(entity), Some(direction)) =
                        (self.entities.get_mut(owner), explicit_direction)
                    {
                        entity.element_data_mut().set_direction_instantly(direction);
                    }
                }
                self.push_order(seq_id, elem_idx, order_type, false);
            }
            _ => unreachable!("non-turn command passed to turn command context"),
        }
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    fn push_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
        compute_direction: bool,
    ) {
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        order.compute_direction = compute_direction;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
}

/// WAIT/WAIT_TIMER translation against entity state, sequence state, and the
/// immutable profile table used by the carried-VIP animation branch.
pub(in crate::engine) struct WaitCommandContext<'a> {
    pub(in crate::engine) entities: &'a mut crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
    pub(in crate::engine) profiles: &'a crate::profiles::ProfileManager,
}

impl WaitCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let (
            is_soldier,
            is_pc,
            posture,
            action_state,
            is_attentive,
            is_dead,
            is_unconscious,
            is_swordfighting,
            is_stuck_under_net,
            carrier_is_vip,
        ) = {
            let entity = self.entities.get(owner).unwrap_or_else(|| {
                panic!(
                    "Wait translation owner {owner:?} is missing for {command:?} at {seq_id:?}/{elem_idx}"
                )
            });
            let actor = entity.actor_data().unwrap_or_else(|| {
                panic!(
                    "Wait translation owner {owner:?} is not an actor for {command:?} at {seq_id:?}/{elem_idx}"
                )
            });
            let carrier = entity.human_data().and_then(|human| human.carrier);
            (
                entity.is_soldier(),
                entity.is_pc(),
                entity.element_data().posture,
                actor.action_state,
                entity.enemy_ai().is_some_and(|enemy| enemy.attentive),
                entity.is_dead(),
                entity
                    .human_data()
                    .is_some_and(|human| human.unconscious),
                entity
                    .human_data()
                    .is_some_and(|human| !human.opponents.is_empty()),
                entity
                    .human_data()
                    .is_some_and(|human| human.stuck_under_nets_counter > 0),
                carrier.is_some_and(|carrier_id| {
                    let carrier = self.entities.get(carrier_id).unwrap_or_else(|| {
                        panic!(
                            "Wait translation owner {owner:?} references missing carrier {carrier_id:?} at {seq_id:?}/{elem_idx}"
                        )
                    });
                    self.is_entity_vip(carrier)
                }),
            )
        };

        let wait_element = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap_or_else(|| {
                panic!(
                    "Wait translation owner {owner:?} lost sequence element {seq_id:?}/{elem_idx} for {command:?}"
                )
            });
        assert_eq!(
            wait_element.owner,
            Some(owner),
            "Wait translation owner {owner:?} does not own {seq_id:?}/{elem_idx}"
        );
        assert_eq!(
            wait_element.command, command,
            "Wait translation owner {owner:?} dispatched {command:?} for {:?} at {seq_id:?}/{elem_idx}",
            wait_element.command
        );
        let after_state = wait_element.action_state_after_transition;
        let pc_posture_animation = if is_pc {
            use crate::element::{ActionState as AS, Posture as P};
            use crate::order::OrderType as OT;
            match posture {
                P::HelpingToClimb => Some(OT::WaitingHelpingClimbing),
                P::CarryingOnShoulders => Some(OT::WaitingCarryingOnShoulders),
                P::OnShoulders => Some(OT::WaitingOnShoulders),
                P::CarryingCorpse => Some(OT::WaitingWithCorpse),
                P::SimulatingBeggar => Some(OT::SimulatingBeggar),
                P::Spy => Some(OT::WaitingCape),
                P::AnonymousArcher => Some(match after_state {
                    AS::AimingWithBow => OT::AimingWithBowAnonymous,
                    AS::AimingWithBowUp => OT::AimingWithBowUpAnonymous,
                    _ => OT::WaitingCapeAnonymousArcher,
                }),
                P::Tree => Some(OT::WaitingHidden),
                P::Upright if action_state == AS::Listening => Some(OT::Listening),
                _ => None,
            }
        } else {
            None
        };

        let mut set_posture_stuck_under_net = false;
        let animation = if let Some(pc_animation) = pc_posture_animation {
            Some(pc_animation)
        } else if is_soldier
            && is_attentive
            && posture == crate::element::Posture::Upright
            && action_state == crate::element::ActionState::Waiting
            && !is_dead
            && !is_unconscious
        {
            Some(crate::order::OrderType::WaitingAlerted)
        } else if is_soldier && posture == crate::element::Posture::LeaningOut {
            Some(match after_state {
                crate::element::ActionState::AimingWithBow
                | crate::element::ActionState::AimingWithBowDown => {
                    crate::order::OrderType::AimingWithBowLeaningOut
                }
                _ => crate::order::OrderType::LeaningOut,
            })
        } else {
            use crate::element::{ActionState as AS, Posture as P};
            use crate::order::OrderType as OT;
            let upright_animation = if is_swordfighting {
                match after_state {
                    AS::ParryingSword | AS::ParryingSwordLow => OT::ParryingSword,
                    AS::WaitingSword | AS::MovingSword | AS::MovingFastSword => OT::WaitingSword,
                    _ => OT::TransitionRaisingSword,
                }
            } else {
                match after_state {
                    AS::HoldingShield | AS::ParryingShield | AS::MovingShield => OT::WaitingShield,
                    AS::AimingWithBow => OT::AimingWithBow,
                    AS::AimingWithBowUp => OT::AimingWithBowUp,
                    AS::WaitingSword | AS::MovingSword | AS::MovingFastSword => OT::WaitingSword,
                    AS::Menacing => OT::Menacing,
                    AS::Sleeping => OT::SleepingUpright,
                    AS::ParryingSword | AS::ParryingSwordLow => OT::ParryingSword,
                    _ => OT::WaitingUprightBored,
                }
            };
            match posture {
                P::Upright => Some(upright_animation),
                P::Crouched => Some(OT::WaitingCrouched),
                P::OnWall | P::OnLadder => Some(OT::Freezing),
                P::Sitting => Some(OT::Sitting),
                P::Lying if is_unconscious || command == Command::WaitTimer => {
                    Some(match after_state {
                        state if state.is_sword() || state == AS::Menacing => {
                            OT::BeingUnconsciousSword
                        }
                        state if state.is_bow() => OT::BeingUnconsciousBow,
                        _ => OT::BeingUnconscious,
                    })
                }
                P::Lying => {
                    if is_stuck_under_net {
                        set_posture_stuck_under_net = true;
                        Some(OT::LyingStuckUnderNet)
                    } else {
                        Some(match after_state {
                            state if state.is_sword() || state == AS::Menacing => {
                                OT::StandingUpSword
                            }
                            state if state.is_bow() => OT::StandingUpBow,
                            _ => OT::StandingUp,
                        })
                    }
                }
                P::DeadBack => Some(match after_state {
                    AS::WaitingSword | AS::Menacing => OT::BeingDeadFallenBackSword,
                    AS::AimingWithBow | AS::AimingWithBowDown => OT::BeingDeadFallenBackBow,
                    _ => OT::BeingDeadFallenBack,
                }),
                P::Dead => Some(match after_state {
                    AS::WaitingSword => OT::BeingDeadSword,
                    AS::AimingWithBow | AS::AimingWithBowDown => OT::BeingDeadBow,
                    _ => OT::BeingDead,
                }),
                P::Carried => {
                    tracing::warn!(
                        ?owner,
                        "Wait/Translate: CARRIED posture reached (asserted unreachable upstream); \
                         queuing BeingCarried{{LittleJohn|PeasantC}}"
                    );
                    Some(if carrier_is_vip {
                        OT::BeingCarriedLittleJohn
                    } else {
                        OT::BeingCarriedPeasantC
                    })
                }
                P::Tied => Some(OT::BeingTied),
                P::Leisure => Some(OT::Special),
                P::StuckUnderNet => Some(OT::LyingStuckUnderNet),
                _ => None,
            }
        };

        if command == Command::WaitTimer {
            let timer = self
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("WAIT_TIMER owner {owner:?} lost sequence element {seq_id:?}/{elem_idx}")
                })
                .get_property(crate::sequence::Field::Timer)
                .unwrap_or_else(|| {
                    panic!("WAIT_TIMER owner {owner:?} has no Timer at {seq_id:?}/{elem_idx}")
                });
            let timer = match timer {
                crate::sequence::FieldValue::Integer(timer) => *timer,
                other => panic!(
                    "WAIT_TIMER owner {owner:?} requires integer Timer at {seq_id:?}/{elem_idx}, found {other:?}"
                ),
            };
            let actor = self
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("WAIT_TIMER owner {owner:?} vanished during translation")
                });
            actor.wait_time = timer;
        }
        if is_pc
            && posture == crate::element::Posture::Upright
            && action_state == crate::element::ActionState::Listening
        {
            let actor = self
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| panic!("Wait translation listening owner {owner:?} vanished"));
            const TIME_LISTEN_WAIT: u32 = 25;
            actor.wait_time = TIME_LISTEN_WAIT;
        }
        if set_posture_stuck_under_net {
            let entity = self.entities.get_mut(owner).unwrap_or_else(|| {
                panic!("Wait translation owner {owner:?} vanished while setting net posture")
            });
            entity
                .element_data_mut()
                .set_posture(crate::element::Posture::StuckUnderNet);
        }

        if let Some(animation) = animation {
            let id = crate::order::alloc_order_id(self.next_order_id);
            let mut order = crate::order::Order::new(animation, 0.0, 0.0, id);
            order.compute_direction = false;
            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
        OwnerActionBarrier::Reach
    }

    fn is_entity_vip(&self, entity: &Entity) -> bool {
        match entity {
            Entity::Soldier(soldier) => self
                .profiles
                .get_soldier(soldier.soldier.soldier_profile_index)
                .is_some_and(|profile| profile.vip),
            Entity::Civilian(civilian) => self
                .profiles
                .civilians
                .get(usize::from(civilian.civilian.civilian_profile_index))
                .is_some_and(|profile| profile.civilian_type == crate::profiles::CivilianType::Vip),
            _ => false,
        }
    }
}

/// Fixed NPC posture/action-state transition order translation.
pub(in crate::engine) struct NpcStateCommandContext<'a> {
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
}

impl NpcStateCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        match command {
            Command::SitDown | Command::BeggarShowFace | Command::EnterLeisure => {
                let order_type = match command {
                    Command::SitDown => crate::order::OrderType::TransitionWaitingUprightSitting,
                    Command::BeggarShowFace => crate::order::OrderType::BeggarShowingFace,
                    Command::EnterLeisure => {
                        crate::order::OrderType::TransitionWaitingUprightSpecial
                    }
                    _ => unreachable!(),
                };
                self.push_order(seq_id, elem_idx, order_type);
                self.sequence_manager.element_in_progress(seq_id, elem_idx);
            }
            Command::StartMenace
            | Command::StopMenace
            | Command::StopSleep
            | Command::LowerBowLeanOut
            | Command::RaiseBowLeanOut => {
                match command {
                    Command::StartMenace => {
                        self.push_order(
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionRaisingSword,
                        );
                        self.push_order(
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionWaitingSwordMenacing,
                        );
                    }
                    Command::StopMenace => {
                        self.push_order(
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionMenacingWaitingSword,
                        );
                        self.push_order(
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionLoweringSword,
                        );
                    }
                    Command::StopSleep => self.push_order(
                        seq_id,
                        elem_idx,
                        crate::order::OrderType::TransitionSleepingWaitingUpright,
                    ),
                    Command::LowerBowLeanOut => self.push_order(
                        seq_id,
                        elem_idx,
                        crate::order::OrderType::TransitionLoweringBowLeaningOut,
                    ),
                    Command::RaiseBowLeanOut => self.push_order(
                        seq_id,
                        elem_idx,
                        crate::order::OrderType::TransitionRaisingBowLeaningOut,
                    ),
                    _ => unreachable!(),
                }
                if matches!(command, Command::LowerBowLeanOut | Command::RaiseBowLeanOut) {
                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                } else {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                }
            }
            _ => unreachable!("non-NPC-state command passed to NPC state context"),
        }
        OwnerActionBarrier::Reach
    }

    fn push_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
    ) {
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        order.compute_direction = false;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
}

/// NPC look/lean and attentive-mode translation against only entity state,
/// sequence state, and order-id allocation.
///
/// Original provenance: `RHelementactorsoldier.cpp:329-386` appends these
/// orders synchronously from `Translate`. The sequence phase must therefore
/// still reach its after-action splice immediately after this context returns.
pub(in crate::engine) struct NpcAttentionCommandContext<'a> {
    pub(in crate::engine) entities: &'a mut crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
}

impl NpcAttentionCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        match command {
            Command::LookLeft | Command::LookRight | Command::LeanOut => {
                let order_type = self.entities.get(owner).map(|entity| {
                    let attentive = entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
                    match command {
                        Command::LookLeft if attentive => {
                            crate::order::OrderType::LookingLeftAlerted
                        }
                        Command::LookLeft => crate::order::OrderType::LookingLeft,
                        Command::LookRight if attentive => {
                            crate::order::OrderType::LookingRightAlerted
                        }
                        Command::LookRight => crate::order::OrderType::LookingRight,
                        Command::LeanOut => {
                            crate::order::OrderType::TransitionWaitingAlertedLeaningOut
                        }
                        _ => unreachable!(),
                    }
                });
                if let Some(order_type) = order_type {
                    self.push_order(seq_id, elem_idx, order_type);
                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                } else {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                }
            }
            Command::EnterAttentiveMode
            | Command::LeaveAttentiveMode
            | Command::LeaveAttentiveModeOfficer => {
                let posture_after_transition = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| element.posture_after_transition)
                    .unwrap_or_default();
                if self.dispatch_attentive(
                    owner,
                    command,
                    posture_after_transition,
                    seq_id,
                    elem_idx,
                ) {
                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                } else {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                }
            }
            _ => unreachable!("non-attention command passed to NPC attention context"),
        }
        OwnerActionBarrier::Reach
    }

    fn dispatch_attentive(
        &mut self,
        owner: EntityId,
        command: Command,
        posture_after_transition: crate::element::Posture,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> bool {
        let target_attentive = command == Command::EnterAttentiveMode;
        let animation = match command {
            Command::EnterAttentiveMode => {
                crate::order::OrderType::TransitionWaitingUprightWaitingAlerted
            }
            Command::LeaveAttentiveMode => {
                crate::order::OrderType::TransitionWaitingAlertedWaitingUpright
            }
            Command::LeaveAttentiveModeOfficer => {
                crate::order::OrderType::TransitionWaitingAlertedWaitingUprightOfficer
            }
            _ => unreachable!(),
        };

        // The officer salute-and-drop transition is unconditional in the
        // original translator and in the pre-split Rust path.
        if command == Command::LeaveAttentiveModeOfficer {
            self.push_order(seq_id, elem_idx, animation);
            return true;
        }

        let posture_upright_after = posture_after_transition == crate::element::Posture::Upright;
        let Some(entity) = self.entities.get(owner) else {
            return false;
        };
        let currently_attentive = entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
        let idle = self
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(current_seq, current_idx)| {
                self.sequence_manager.get_element(current_seq, current_idx)
            })
            .map(|element| element.command == Command::Wait)
            .unwrap_or(true);
        let needs_change = currently_attentive != target_attentive;
        let can_play_transition = posture_upright_after && idle && needs_change;

        // TODO(parity): `RHElementActorSoldier::Translate` at
        // `RHelementactorsoldier.cpp:329-370` does not inspect the current Wait
        // element, and its normal Leave arm animates whenever posture-after is
        // Upright. Verify whether instruct arbitration makes Rust's retained
        // `idle && needs_change` gate redundant before changing tick behavior.
        tracing::trace!(
            owner = owner.index(),
            ?command,
            ?posture_after_transition,
            posture_upright_after,
            currently_attentive,
            target_attentive,
            needs_change,
            idle,
            can_play_transition,
            "dispatch attentive transition"
        );

        if can_play_transition {
            self.push_order(seq_id, elem_idx, animation);
            true
        } else {
            if let Some(enemy) = self.entities.get_mut(owner).and_then(Entity::enemy_ai_mut) {
                enemy.attentive = target_attentive;
            }
            false
        }
    }

    fn push_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
    ) {
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        order.compute_direction = false;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
}

/// Stealth-posture command translation with the exact mutable domains it
/// touches: entities, the owning sequence, order allocation, and HIDDEN
/// titbits. Character profiles are read only to derive the HIDDEN phase.
pub(in crate::engine) struct StealthCommandContext<'a> {
    pub(in crate::engine) entities: &'a mut crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
    pub(in crate::engine) titbit_manager: &'a mut crate::titbit::TitbitManager,
    pub(in crate::engine) profiles: &'a crate::profiles::ProfileManager,
}

impl StealthCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        use crate::element::ActionState;

        let Some(entity) = self.entities.get(owner) else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Reach;
        };
        let posture = entity.element_data().posture;
        let action_state = entity
            .actor_data()
            .map(|actor| actor.action_state)
            .unwrap_or(ActionState::Waiting);
        let is_swordfighting = action_state.is_sword();

        if !crate::stealth::can_execute_stealth_command(
            command,
            posture,
            action_state,
            is_swordfighting,
        ) {
            tracing::debug!(
                ?owner,
                ?command,
                ?posture,
                ?action_state,
                "stealth command rejected: preconditions not met"
            );
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Reach;
        }

        let Some(transition) = crate::stealth::stealth_transition(command) else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Reach;
        };
        let hidden_phase = if transition.result_posture.is_hidden() {
            let Some(Entity::Pc(pc)) = self.entities.get(owner) else {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return OwnerActionBarrier::Reach;
            };
            let profile = self
                .profiles
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "stealth command owner {} has unknown profile_index {}",
                        owner.index(),
                        pc.pc.profile_index
                    )
                });
            Some(crate::titbit::HiddenCharacter::for_pc(pc.pc.robin, &profile.filename).to_phase())
        } else {
            None
        };

        let old_posture = posture;
        let entity = self
            .entities
            .get_mut(owner)
            .expect("stealth command owner disappeared during dispatch");
        entity.set_posture(transition.result_posture);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = transition.result_action_state;
        }
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(transition.animation, 0.0, 0.0, id);
        order.compute_direction = false;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);

        tracing::debug!(
            ?owner,
            ?command,
            posture = ?transition.result_posture,
            animation = ?transition.animation,
            "stealth transition applied"
        );

        use crate::coordinates::WorldPoint3D;
        use crate::titbit::{ElementHandle, TitbitKind};
        let handle = ElementHandle(owner.index());
        if transition.result_posture.is_hidden() && !old_posture.is_hidden() {
            self.titbit_manager.add_titbit(
                WorldPoint3D::default(),
                0,
                TitbitKind::Hidden,
                handle,
                hidden_phase.expect("hidden phase resolved before entering hidden posture"),
                handle,
                false,
                0,
                true,
                None,
                None,
            );
        } else if !transition.result_posture.is_hidden() && old_posture.is_hidden() {
            self.titbit_manager
                .remove_titbit(TitbitKind::Hidden, handle);
        }

        // TODO(parity): C++ applies posture/action, HIDDEN, and nearby-coin
        // side effects on transition-animation DONE
        // (`RHelementactorpc.cpp:6005-6034`). Rust has historically snapped
        // them during command dispatch while leaving the animation order on
        // the terminated element. Move all four effects together only after
        // the animation completion path can retain that order safely.
        if transition.result_posture == crate::element::Posture::SimulatingBeggar
            && old_posture != crate::element::Posture::SimulatingBeggar
        {
            super::beggar::set_flags_of_near_coins_on_ground(self.entities, owner, true);
        } else if old_posture == crate::element::Posture::SimulatingBeggar
            && transition.result_posture != crate::element::Posture::SimulatingBeggar
        {
            super::beggar::set_flags_of_near_coins_on_ground(self.entities, owner, false);
        }

        self.sequence_manager.element_terminated(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }
}

/// Direct actor abilities whose translation is confined to entity state,
/// sequence state, order-id allocation, and immutable animation profiles.
/// Campaign-backed ammo checks are evaluated by the caller and passed as a
/// value so this context cannot reach mission or campaign ownership.
pub(in crate::engine) struct DirectAbilityCommandContext<'a> {
    pub(in crate::engine) entities: &'a mut crate::entities::Entities,
    pub(in crate::engine) sequence_manager: &'a mut crate::sequence::SequenceManager,
    pub(in crate::engine) next_order_id: &'a mut u32,
    pub(in crate::engine) profiles: &'a crate::profiles::ProfileManager,
}

impl DirectAbilityCommandContext<'_> {
    pub(in crate::engine) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        ammo_available: bool,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        match command {
            Command::TieCmd => {
                let Some(target) = self.interaction_target(seq_id, elem_idx) else {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Reach;
                };
                let result = abilities::begin_tie(
                    self.entities,
                    self.sequence_manager,
                    owner,
                    target,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                );
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::HealCmd => {
                let Some(target) = self.interaction_target(seq_id, elem_idx) else {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Reach;
                };
                if !ammo_available {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Reach;
                }
                let result = abilities::begin_heal(
                    self.entities,
                    self.sequence_manager,
                    owner,
                    target,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                );
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::WhistleCmd => {
                let result = abilities::begin_whistle(
                    self.entities,
                    self.sequence_manager,
                    owner,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                );
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::EatCmd => {
                if !ammo_available {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                }
                let result = abilities::begin_eat(
                    self.entities,
                    self.sequence_manager,
                    owner,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                );
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::HitCmd | Command::StrangleCmd => {
                let Some(target) = self.interaction_target(seq_id, elem_idx) else {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Reach;
                };
                let result = match command {
                    Command::HitCmd => abilities::begin_hit(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    Command::StrangleCmd => abilities::begin_strangle(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    _ => unreachable!(),
                };
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::ReceivePurse => {
                let result = abilities::begin_receive_purse(
                    self.entities,
                    self.sequence_manager,
                    owner,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                );
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::EnterListen => {
                let result = abilities::begin_listen(
                    self.entities,
                    self.profiles,
                    self.sequence_manager,
                    owner,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                );
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::LeaveListen => {
                if abilities::begin_leave_listen(
                    self.entities,
                    self.sequence_manager,
                    owner,
                    seq_id,
                    elem_idx,
                    self.next_order_id,
                ) {
                    tracing::debug!(
                        ?owner,
                        "Listen: LeaveListen flipped phase to ExitTransition"
                    );
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                OwnerActionBarrier::Reach
            }
            Command::ThrowNet | Command::ThrowPurse | Command::ThrowWaspNest => {
                let field = match command {
                    Command::ThrowNet => crate::sequence::Field::NetTarget,
                    Command::ThrowPurse => crate::sequence::Field::PurseTarget,
                    Command::ThrowWaspNest => crate::sequence::Field::WaspNestTarget,
                    _ => unreachable!(),
                };
                let Some(target) = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| read_sequence_map_point_property(element, field))
                else {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Reach;
                };
                if !ammo_available {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Reach;
                }
                let result = match command {
                    Command::ThrowNet => abilities::begin_throw_net(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    Command::ThrowPurse => abilities::begin_throw_purse(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    Command::ThrowWaspNest => abilities::begin_throw_wasp_nest(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    _ => unreachable!(),
                };
                self.finish_begin(result, seq_id, elem_idx)
            }
            Command::ThrowApple | Command::ThrowStone => {
                let Some(target) = self.interaction_target(seq_id, elem_idx) else {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                };
                if !ammo_available {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                }
                let result = match command {
                    Command::ThrowApple => abilities::begin_throw_apple(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    Command::ThrowStone => abilities::begin_throw_stone(
                        self.entities,
                        self.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        self.next_order_id,
                    ),
                    _ => unreachable!(),
                };
                self.finish_begin(result, seq_id, elem_idx)
            }
            _ => unreachable!("non-direct ability passed to direct ability context"),
        }
    }

    fn interaction_target(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<EntityId> {
        self.sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
                _ => None,
            })
    }

    fn finish_begin(
        &mut self,
        result: AbilityBeginResult,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        match result {
            AbilityBeginResult::Started => {
                self.sequence_manager.element_in_progress(seq_id, elem_idx)
            }
            AbilityBeginResult::Impossible => {
                self.sequence_manager.element_impossible(seq_id, elem_idx)
            }
        }
        OwnerActionBarrier::Reach
    }
}

/// Owner recovery and wake-up animations touch only entity state, sequence
/// state, and the deterministic order-id stream.
struct RecoveryCommandContext<'a> {
    entities: &'a mut crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl RecoveryCommandContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        use crate::order::OrderType;

        match command {
            Command::Fainted => {
                self.push_order(seq_id, elem_idx, OrderType::BeingUnconsciousSword, None);
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Command::Recover | Command::StandUp => {
                let already_queued = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .is_some_and(|element| !element.orders.is_empty());
                if !already_queued {
                    let standing_up = self
                        .entities
                        .get(owner)
                        .and_then(Entity::actor_data)
                        .map(|actor| {
                            let action_state = actor.action_state;
                            if action_state.is_sword()
                                || action_state == crate::element::ActionState::Menacing
                            {
                                OrderType::StandingUpSword
                            } else if action_state.is_bow() {
                                OrderType::StandingUpBow
                            } else {
                                OrderType::StandingUp
                            }
                        })
                        .unwrap_or_else(|| {
                            tracing::warn!(
                                ?owner,
                                ?seq_id,
                                elem_idx,
                                "StandUp/Recover owner has no actor data; defaulting to StandingUp"
                            );
                            OrderType::StandingUp
                        });
                    self.push_order(seq_id, elem_idx, standing_up, None);
                }
                if let Some(entity) = self.entities.get_mut(owner) {
                    entity.set_posture(crate::element::Posture::Upright);
                }
                if self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .is_some_and(|element| !element.orders.is_empty())
                {
                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                } else {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                }
            }
            Command::WakeUp => {
                let target = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| match element.data {
                        crate::sequence::SequenceElementData::Interaction { antagonist } => {
                            antagonist
                        }
                        _ => None,
                    });
                let Some(target) = target else {
                    tracing::warn!(?owner, ?seq_id, elem_idx, "WakeUp element has no target");
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                };
                let Some(target_position) = self
                    .entities
                    .get(target)
                    .map(|entity| entity.element_data().position_map())
                else {
                    tracing::warn!(
                        ?owner,
                        ?target,
                        ?seq_id,
                        elem_idx,
                        "WakeUp target is missing"
                    );
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                };
                let id = crate::order::alloc_order_id(self.next_order_id);
                let mut order = crate::order::Order::new(
                    OrderType::WakingUp,
                    target_position.x,
                    target_position.y,
                    id,
                )
                .with_antagonist(target);
                order.compute_direction = false;
                self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                self.sequence_manager.element_in_progress(seq_id, elem_idx);
            }
            Command::Knee => {
                self.push_order(seq_id, elem_idx, OrderType::FallingBackSword, None);
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            _ => unreachable!("non-recovery command passed to recovery context"),
        }
        OwnerActionBarrier::Reach
    }

    fn push_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
        antagonist: Option<EntityId>,
    ) {
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        if let Some(antagonist) = antagonist {
            order = order.with_antagonist(antagonist);
        }
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
}

/// Drink/take translation reads only the interaction pair and object payload,
/// then books one deterministic animation order on the owning element.
struct ObjectInteractionCommandContext<'a> {
    entities: &'a mut crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl ObjectInteractionCommandContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let owner_is_pc = self.entities.get(owner).is_some_and(Entity::is_pc);
        let antagonist = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => antagonist,
                _ => None,
            });
        if let Some(antagonist) = antagonist {
            let object_type = self
                .entities
                .get(antagonist)
                .and_then(|entity| entity.object_data().map(|object| object.object_type));
            match command {
                Command::DrinkAle => assert!(
                    matches!(object_type, Some(crate::element::ObjectType::Ale)),
                    "DrinkAle: antagonist {antagonist:?} has object_type {object_type:?}; expected Ale"
                ),
                Command::Take if !owner_is_pc => assert!(
                    matches!(
                        object_type,
                        Some(
                            crate::element::ObjectType::Net
                                | crate::element::ObjectType::Purse
                                | crate::element::ObjectType::Coin
                        )
                    ),
                    "Take (soldier): antagonist {antagonist:?} has object_type {object_type:?}; expected Net/Purse/Coin"
                ),
                Command::Take => assert!(
                    object_type.is_some(),
                    "Take (PC): antagonist {antagonist:?} is not an object"
                ),
                _ => unreachable!(),
            }
        }

        if command == Command::DrinkAle || command == Command::Take && !owner_is_pc {
            let antagonist =
                antagonist.unwrap_or_else(|| panic!("{command:?}: missing interaction antagonist"));
            let owner_position = self
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("{command:?}: owner {owner:?} is missing"))
                .element_data()
                .position_map();
            let antagonist_position = self
                .entities
                .get(antagonist)
                .unwrap_or_else(|| panic!("{command:?}: antagonist {antagonist:?} is missing"))
                .element_data()
                .position_map();
            let direction_goal = crate::position_interface::vector_to_sector_0_to_15_iso(
                antagonist_position.x - owner_position.x,
                antagonist_position.y - owner_position.y,
            );
            self.entities
                .get_mut(owner)
                .expect("object interaction owner disappeared")
                .element_data_mut()
                .set_direction_goal(direction_goal);
        }

        let antagonist_is_net = antagonist
            .and_then(|id| self.entities.get(id))
            .is_some_and(|entity| matches!(entity, Entity::Net(_)));
        let order_type = match command {
            Command::DrinkAle => crate::order::OrderType::DrinkingAle,
            Command::Take if antagonist_is_net => crate::order::OrderType::TakingNet,
            Command::Take => crate::order::OrderType::Taking,
            _ => unreachable!(),
        };
        let id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        if let Some(antagonist) = antagonist {
            order = order.with_antagonist(antagonist);
        }
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }
}

/// Immediate mobile controls cannot reach any other world or mission state.
struct MobileImmediateContext<'a> {
    entities: &'a mut crate::entities::Entities,
    mobiles: &'a mut [crate::mobile::MobileElement],
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl MobileImmediateContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let mobile_index = self
            .entities
            .get(owner)
            .and_then(Entity::as_fx)
            .and_then(|fx| fx.fx.mobile_index)
            .unwrap_or_else(|| panic!("{command:?} owner {owner} is not a mobile child FX"));
        let mobile = self
            .mobiles
            .get_mut(usize::from(mobile_index))
            .unwrap_or_else(|| panic!("{command:?} references missing mobile {mobile_index}"));
        match command {
            Command::StartMobile => mobile.start(),
            Command::StopMobile => mobile.stop(),
            Command::ActivateMobile => {
                mobile.set_active(true);
            }
            Command::DeactivateMobile => {
                mobile.set_active(false);
            }
            _ => unreachable!("non-mobile command passed to mobile context"),
        }
        let active = mobile.active;
        for child_id in mobile.sprite_ids.clone() {
            self.entities
                .get_mut(child_id)
                .unwrap_or_else(|| panic!("mobile {mobile_index} child {child_id} is missing"))
                .element_data_mut()
                .active = active;
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}

/// Immediate sprite metadata commands are isolated from mobile, AI, camera,
/// script, and mission ownership.
struct SpriteImmediateContext<'a> {
    entities: &'a mut crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl SpriteImmediateContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match command {
            Command::Unblip => {
                if let Some(entity) = self.entities.get_mut(owner)
                    && entity.element_data().blipped
                {
                    entity.reveal_blip();
                }
            }
            Command::ReplaceAnim => {
                let old =
                    self.animation_property(seq_id, elem_idx, crate::sequence::Field::OldAnimation);
                let new =
                    self.animation_property(seq_id, elem_idx, crate::sequence::Field::NewAnimation);
                if let (Some(old), Some(new), Some(entity)) =
                    (old, new, self.entities.get_mut(owner))
                {
                    entity.element_data_mut().sprite.replace_anim(old, new);
                }
            }
            Command::RestoreAnim => {
                let old =
                    self.animation_property(seq_id, elem_idx, crate::sequence::Field::OldAnimation);
                if let (Some(old), Some(entity)) = (old, self.entities.get_mut(owner)) {
                    entity.element_data_mut().sprite.restore_anim(old);
                }
            }
            _ => unreachable!("non-sprite command passed to sprite context"),
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }

    fn animation_property(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        field: crate::sequence::Field,
    ) -> Option<crate::order::OrderType> {
        self.sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(field))
            .and_then(|value| match value {
                crate::sequence::FieldValue::Integer(value) => {
                    crate::order::OrderType::try_from(*value).ok()
                }
                _ => None,
            })
    }
}

struct AiLockImmediateContext<'a> {
    entities: &'a mut crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl AiLockImmediateContext<'_> {
    fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let lock = command == Command::LockAi;
        if let Some(entity) = self.entities.get_mut(owner)
            && entity.is_npc()
        {
            let unconscious = entity.human_data().is_some_and(|human| human.unconscious);
            if let Some(ai) = entity.ai_controller_mut() {
                if lock {
                    ai.script_lock(false, true);
                } else if ai.script_locked {
                    ai.script_unlock(unconscious);
                }
            }
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}

#[cfg(test)]
struct UserLockImmediateContext<'a> {
    user_locked: &'a mut bool,
    side_effects: &'a mut SideEffects,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

fn synchronous_action_element_ref(
    action: &crate::sequence::SequenceAction,
) -> Option<(crate::sequence::SequenceId, usize)> {
    use crate::sequence::SequenceAction;
    match *action {
        SequenceAction::InstructOwner {
            sequence_id,
            element_index,
            ..
        }
        | SequenceAction::EngineCommand {
            sequence_id,
            element_index,
        }
        | SequenceAction::ExecuteImmediateOwner {
            sequence_id,
            element_index,
            ..
        }
        | SequenceAction::ExecuteImmediateEngine {
            sequence_id,
            element_index,
        } => Some((sequence_id, element_index)),
    }
}

#[cfg(test)]
impl UserLockImmediateContext<'_> {
    fn dispatch(&mut self, command: Command, seq_id: crate::sequence::SequenceId, elem_idx: usize) {
        *self.user_locked = command == Command::LockUser;
        if command == Command::UnlockUser {
            self.side_effects.pending_reset_input = true;
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}

struct TimerImmediateContext<'a> {
    sequence_manager: &'a crate::sequence::SequenceManager,
}

impl TimerImmediateContext<'_> {
    fn entry(&self, seq_id: crate::sequence::SequenceId, elem_idx: usize) -> TimerEntry {
        let remaining = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(crate::sequence::Field::Timer))
            .and_then(|value| match value {
                crate::sequence::FieldValue::Integer(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(0);
        TimerEntry {
            remaining,
            element_ref: crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
        }
    }
}

/// Map/dialog/popup commands need host minimap access plus only the
/// deterministic presentation outputs and messenger reset-input edge.
struct PresentationCommandContext<'a> {
    display: &'a mut HostDisplayState,
    fast_forward: bool,
    side_effects: &'a mut SideEffects,
    messenger: &'a mut crate::messenger::Messenger,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl PresentationCommandContext<'_> {
    fn dispatch(&mut self, command: Command, seq_id: crate::sequence::SequenceId, elem_idx: usize) {
        match command {
            Command::DisplayMap => {
                let show = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| element.get_property(crate::sequence::Field::MapDisplay))
                    .and_then(|value| match value {
                        crate::sequence::FieldValue::Bool(value) => Some(*value),
                        _ => None,
                    })
                    .unwrap_or(false);
                self.display.minimap.display_map(show, false);
            }
            Command::PlayDialog => {
                if !self.fast_forward {
                    let id =
                        self.integer_property(seq_id, elem_idx, crate::sequence::Field::DialogId);
                    self.side_effects.pending_dialogues.push(id);
                }
                self.reset_input();
            }
            Command::DisplayPopupText => {
                if !self.fast_forward {
                    let id = self.integer_property(
                        seq_id,
                        elem_idx,
                        crate::sequence::Field::PopupTextId,
                    );
                    self.side_effects.pending_popup_texts.push(id);
                }
                self.reset_input();
            }
            _ => unreachable!("non-presentation command passed to presentation context"),
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }

    fn integer_property(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        field: crate::sequence::Field,
    ) -> i32 {
        self.sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(field))
            .and_then(|value| match value {
                crate::sequence::FieldValue::Integer(value) => Some(*value as i32),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn reset_input(&mut self) {
        self.messenger
            .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
    }
}

struct FreezeImmediateContext<'a> {
    control: &'a mut SimulationControl,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl FreezeImmediateContext<'_> {
    fn dispatch(&mut self, seq_id: crate::sequence::SequenceId, elem_idx: usize) {
        let frozen = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(crate::sequence::Field::Freeze))
            .and_then(|value| match value {
                crate::sequence::FieldValue::Bool(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(false);
        self.control.set_actors_frozen(frozen);
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}

/// Character/action availability affects only PC metadata and the ordered
/// messenger stream consumed after the simulation tick.
struct AvailabilityImmediateContext<'a> {
    entities: &'a mut crate::entities::Entities,
    messenger: &'a mut crate::messenger::Messenger,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
}

impl AvailabilityImmediateContext<'_> {
    fn dispatch(&mut self, command: Command, seq_id: crate::sequence::SequenceId, elem_idx: usize) {
        let element = self.sequence_manager.get_element(seq_id, elem_idx);
        let owner = element.and_then(|element| element.owner);
        match command {
            Command::CharacterAvailable => {
                let available = element
                    .and_then(|element| {
                        element.get_property(crate::sequence::Field::CharacterAvailable)
                    })
                    .and_then(|value| match value {
                        crate::sequence::FieldValue::Bool(value) => Some(*value),
                        _ => None,
                    })
                    .unwrap_or(false);
                if let Some(owner) = owner
                    && let Some(pc) = self.entities.get_mut(owner).and_then(Entity::pc_data_mut)
                {
                    pc.playable = available;
                    let message = if available {
                        crate::messenger::PcMessage::EnableCharacter
                    } else {
                        crate::messenger::PcMessage::DisableCharacter
                    };
                    self.messenger.send(Message::pc(message, Some(owner)));
                }
            }
            Command::ActionAvailable => {
                let action_id = element
                    .and_then(|element| element.get_property(crate::sequence::Field::ActionId))
                    .and_then(|value| match value {
                        crate::sequence::FieldValue::Integer(value) => Some(*value),
                        _ => None,
                    })
                    .unwrap_or(0);
                let available = element
                    .and_then(|element| {
                        element.get_property(crate::sequence::Field::ActionAvailable)
                    })
                    .and_then(|value| match value {
                        crate::sequence::FieldValue::Bool(value) => Some(*value),
                        _ => None,
                    })
                    .unwrap_or(false);
                if let Some(owner) = owner {
                    let message = if available {
                        crate::messenger::PcMessage::EnableAction
                    } else {
                        crate::messenger::PcMessage::DisableAction
                    };
                    self.messenger
                        .send(Message::pc_with_value(message, Some(owner), action_id));
                }
            }
            _ => unreachable!("non-availability command passed to availability context"),
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}

#[cfg(test)]
mod sequence_phase_context_tests {
    use super::*;

    fn shield_pc(action_state: crate::element::ActionState) -> Entity {
        Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                active: true,
                posture: crate::element::Posture::Upright,
                ..Default::default()
            },
            actor: crate::element::ActorData {
                action_state,
                ..Default::default()
            },
            human: crate::element::HumanData::default(),
            pc: crate::element::PcData {
                life_points: crate::combat::LIFEPOINTS_PC,
                ..Default::default()
            },
        })
    }

    #[test]
    fn synchronous_successor_is_spliced_before_older_hourglass_work() {
        let older = crate::sequence::SequenceAction::EngineCommand {
            sequence_id: crate::sequence::SequenceId(900),
            element_index: 0,
        };
        let mut phase = SequencePhase {
            initial_actions: vec![older],
            actions: std::collections::VecDeque::new(),
        };
        phase.begin_dispatch();

        let mut orders = OrderRuntime::new();
        let mut sequence = crate::sequence::Sequence::new();
        let mut immediate = crate::sequence::SequenceElement::new(1, Command::Wait, None);
        immediate.priority = crate::sequence::SequencePriority::Wait;
        sequence.append_element(immediate);
        let synchronous_sequence = orders.sequence_manager.launch_sequence(sequence);

        phase.splice_synchronous_actions(&mut orders);

        assert!(matches!(
            phase.pop_action(),
            Some(crate::sequence::SequenceAction::EngineCommand {
                sequence_id,
                element_index: 0,
            }) if sequence_id == synchronous_sequence
        ));
        assert!(matches!(
            phase.pop_action(),
            Some(crate::sequence::SequenceAction::EngineCommand {
                sequence_id: crate::sequence::SequenceId(900),
                element_index: 0,
            })
        ));
        assert!(phase.pop_action().is_none());
    }

    #[test]
    fn immediate_family_successor_is_spliced_before_older_hourglass_work() {
        use crate::sequence::{Field, FieldValue, Sequence, SequenceAction, SequenceElement};

        let older = SequenceAction::EngineCommand {
            sequence_id: crate::sequence::SequenceId(900),
            element_index: 0,
        };
        let mut phase = SequencePhase {
            initial_actions: vec![older],
            actions: std::collections::VecDeque::new(),
        };
        phase.begin_dispatch();

        let mut orders = OrderRuntime::new();
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::LockUser, None));
        let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
        timer.set_property(Field::Timer, FieldValue::Integer(7));
        sequence.append_element(timer);
        let sequence_id = orders.sequence_manager.launch_sequence(sequence);

        phase.splice_synchronous_actions(&mut orders);
        let lock_action = phase.pop_action().expect("LockUser is synchronous");
        assert!(matches!(
            lock_action,
            SequenceAction::ExecuteImmediateEngine {
                sequence_id: id,
                element_index: 0,
            } if id == sequence_id
        ));

        let mut user_locked = false;
        let mut side_effects = SideEffects::default();
        UserLockImmediateContext {
            user_locked: &mut user_locked,
            side_effects: &mut side_effects,
            sequence_manager: &mut orders.sequence_manager,
        }
        .dispatch(Command::LockUser, sequence_id, 0);
        assert!(user_locked);

        // Terminating LockUser synchronously starts Timer. The sequence-phase
        // splice must put that continuation ahead of the older manager action.
        phase.splice_synchronous_actions(&mut orders);
        let timer_action = phase.pop_action().expect("Timer successor is synchronous");
        assert!(matches!(
            timer_action,
            SequenceAction::ExecuteImmediateEngine {
                sequence_id: id,
                element_index: 1,
            } if id == sequence_id
        ));
        assert!(matches!(
            phase.pop_action(),
            Some(SequenceAction::EngineCommand {
                sequence_id: crate::sequence::SequenceId(900),
                element_index: 0,
            })
        ));

        let timer = TimerImmediateContext {
            sequence_manager: &orders.sequence_manager,
        }
        .entry(sequence_id, 1);
        assert_eq!(timer.remaining, 7);
        assert_eq!(timer.element_ref.sequence_id, sequence_id);
        assert_eq!(timer.element_ref.element_index, 1);
    }

    #[test]
    fn availability_executor_preserves_command_message_order() {
        use crate::messenger::{MessageType, PcMessage};
        use crate::sequence::{Field, FieldValue, SequenceElement};

        let mut engine = EngineInner::new();
        let owner = engine.add_entity(shield_pc(crate::element::ActionState::Waiting));
        let mut character =
            SequenceElement::new_generic(1, Command::CharacterAvailable, Some(owner));
        character.set_property(Field::CharacterAvailable, FieldValue::Bool(false));
        let character_sequence = engine.orders.sequence_manager.launch_element(character);
        let mut action = SequenceElement::new_generic(1, Command::ActionAvailable, Some(owner));
        action.set_property(Field::ActionId, FieldValue::Integer(12));
        action.set_property(Field::ActionAvailable, FieldValue::Bool(true));
        let action_sequence = engine.orders.sequence_manager.launch_element(action);

        AvailabilityImmediateContext {
            entities: &mut engine.world.entities,
            messenger: &mut engine.orders.messenger,
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .dispatch(Command::CharacterAvailable, character_sequence, 0);
        AvailabilityImmediateContext {
            entities: &mut engine.world.entities,
            messenger: &mut engine.orders.messenger,
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .dispatch(Command::ActionAvailable, action_sequence, 0);

        let messages = engine.orders.messenger.drain();
        assert!(matches!(
            messages.as_slice(),
            [
                Message {
                    msg_type: MessageType::Pc(PcMessage::DisableCharacter, Some(id)),
                    ..
                },
                Message {
                    msg_type: MessageType::Pc(PcMessage::EnableAction, Some(action_id)),
                    value: 12,
                    ..
                }
            ] if *id == owner && *action_id == owner
        ));
    }

    #[test]
    fn shield_context_preserves_transition_orders_and_states() {
        use crate::element::ActionState;
        use crate::order::OrderType;
        use crate::sequence::{SequenceElement, SequenceState};

        let mut engine = EngineInner::new();
        let owner = engine.add_entity(shield_pc(ActionState::Waiting));
        let target = engine.add_entity(shield_pc(ActionState::Waiting));
        let seq_id =
            engine
                .orders
                .sequence_manager
                .launch_element(SequenceElement::new_interaction(
                    1,
                    Command::RaiseShield,
                    Some(owner),
                    Some(target),
                ));

        let follow_up = crate::engine::melee::ShieldCommandContext::new(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, Command::RaiseShield, seq_id, 0);

        assert!(follow_up.is_none());
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("raise-shield element remains live");
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.orders.front().map(|order| order.order_type),
            Some(OrderType::RaisingShield)
        );
        let owner_entity = engine.get_entity(owner).expect("shield owner exists");
        assert_eq!(
            owner_entity.element_data().posture,
            crate::element::Posture::Upright
        );
        assert_eq!(
            owner_entity
                .actor_data()
                .expect("shield owner has actor data")
                .action_state,
            ActionState::Waiting,
            "raising completion, not translation, enters HoldingShield"
        );

        let instant_seq = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(
                1,
                Command::RaiseShieldInstantly,
                Some(owner),
            ));
        crate::engine::melee::ShieldCommandContext::new(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, Command::RaiseShieldInstantly, instant_seq, 0);
        let instant = engine
            .orders
            .sequence_manager
            .get_element(instant_seq, 0)
            .expect("instant raise-shield element remains inspectable");
        assert_eq!(instant.state, SequenceState::Terminated);
        assert_eq!(
            instant.orders.front().map(|order| order.order_type),
            Some(OrderType::WaitingShield)
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .expect("shield owner exists")
                .actor_data()
                .expect("shield owner has actor data")
                .action_state,
            ActionState::HoldingShield
        );

        let lower_seq = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::LowerShield, Some(owner)));
        engine
            .get_entity_mut(owner)
            .expect("shield owner exists")
            .actor_data_mut()
            .expect("shield owner has actor data")
            .action_state = ActionState::HoldingShield;
        crate::engine::melee::ShieldCommandContext::new(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, Command::LowerShield, lower_seq, 0);
        let lower = engine
            .orders
            .sequence_manager
            .get_element(lower_seq, 0)
            .expect("lower-shield element remains live");
        assert_eq!(lower.state, SequenceState::InProgress);
        assert_eq!(
            lower.orders.front().map(|order| order.order_type),
            Some(OrderType::LoweringShield)
        );

        let parry_seq = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::ParryShield, Some(owner)));
        crate::engine::melee::ShieldCommandContext::new(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, Command::ParryShield, parry_seq, 0);
        let parry = engine
            .orders
            .sequence_manager
            .get_element(parry_seq, 0)
            .expect("parry-shield element remains live");
        assert_eq!(parry.state, SequenceState::InProgress);
        assert_eq!(
            parry.orders.front().map(|order| order.order_type),
            Some(OrderType::ParryingShield)
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .expect("shield owner exists")
                .actor_data()
                .expect("shield owner has actor data")
                .action_state,
            ActionState::ParryingShield
        );
    }

    #[test]
    fn shield_refresh_seek_is_registered_before_the_action_splice() {
        use crate::element::ActionState;
        use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement};

        let mut engine = EngineInner::new();
        let owner = engine.add_entity(shield_pc(ActionState::HoldingShield));
        let protected = engine.add_entity(shield_pc(ActionState::Waiting));
        let mut raise = SequenceElement::new_generic(1, Command::RaiseShield, Some(owner));
        raise.set_property(
            Field::ShieldDangerPoint,
            FieldValue::Point3D {
                x: 100.0,
                y: 50.0,
                z: 7.0,
            },
        );
        raise.set_property(Field::ShieldDangerPointLayer, FieldValue::Integer(3));
        raise.set_property(Field::ShieldProtected, FieldValue::Element(protected));
        let raise_seq = engine.orders.sequence_manager.launch_element(raise);

        let mut phase = SequencePhase::begin(&mut engine.orders);
        phase.begin_dispatch();
        assert!(matches!(
            phase.pop_action(),
            Some(crate::sequence::SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 0,
            }) if action_owner == owner && sequence_id == raise_seq
        ));

        let follow_up = crate::engine::melee::ShieldCommandContext::new(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, Command::RaiseShield, raise_seq, 0)
        .expect("already-shielding protector gets an immediate Seek");
        match &follow_up.data {
            crate::sequence::SequenceElementData::Movement {
                element,
                tolerance,
                flags,
                ..
            } => {
                assert_eq!(*element, Some(protected));
                assert_eq!(*tolerance, 0.0);
                assert!(flags.contains(MoveFlags::SEEK));
                assert!(flags.contains(MoveFlags::SEEK_SHIELD));
            }
            data => panic!("shield follow-up must be movement, got {data:?}"),
        }
        let follow_up_seq = engine.launch_element(follow_up);

        // A normal Move/Seek launch registers with the manager FIFO rather
        // than the immediate-action splice. The pre-split helper launched at
        // this same point, so the current action loop must remain empty and
        // the next manager hourglass must produce the Seek instruction.
        phase.splice_synchronous_actions(&mut engine.orders);
        assert!(phase.pop_action().is_none());
        let next_actions = engine.orders.sequence_manager.hourglass();
        assert!(matches!(
            next_actions.as_slice(),
            [crate::sequence::SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 0,
            }] if *action_owner == owner && *sequence_id == follow_up_seq
        ));

        let owner_pc = engine
            .get_entity(owner)
            .expect("shield owner exists")
            .pc_data()
            .expect("shield owner is a PC");
        assert_eq!(owner_pc.shield_protected, Some(protected));
        assert_eq!(owner_pc.shield_danger_point_layer, 3);
        assert_eq!(owner_pc.shield_danger_point.z, 7.0);
    }
}

#[cfg(test)]
mod canonical_door_invariant_tests {
    use super::*;

    #[test]
    #[should_panic(expected = "UnlockDoor dispatch references missing canonical door 4")]
    fn required_door_lookup_rejects_stale_unlock_target() {
        required_canonical_door(&[], crate::gate::DoorIndex(4), "UnlockDoor dispatch");
    }

    #[test]
    #[should_panic(expected = "UnlockDoor completion references missing canonical door 9")]
    fn required_mutable_door_lookup_rejects_stale_completion_target() {
        required_canonical_door_mut(&mut [], crate::gate::DoorIndex(9), "UnlockDoor completion");
    }

    #[test]
    #[should_panic(expected = "has no Door property")]
    fn unlock_dispatch_rejects_missing_required_door_property() {
        let element = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::UnlockDoor,
            None,
        );
        required_unlock_door_id(Some(&element), crate::sequence::SequenceId(3), 0);
    }
}
