//! Sequence execution contexts and ordered runtime dispatch.
//!
//! The original engine drains registered sequence elements after entity
//! hourglasses and permits `Ready()` to launch successor command levels
//! synchronously. These modules keep that ordering local while borrowing the
//! existing [`EngineInner`] domains directly.

mod immediate;
mod instruct_commands;
#[cfg(test)]
mod instruction_tests;
mod owner_dispatch;
mod owner_preflight;
use owner_preflight::PreparedOwnerInstruction;
mod phase;
mod script_sync;
mod teleport;

use super::movement::MovePathOutcome;
use super::*;
use crate::abilities::{self, BeginResult as AbilityBeginResult};
use crate::bow_shot::{self, BeginShotResult};
use crate::element::{Command, Entity, EntityId};
use crate::messenger::{Message, MessageType, SimpleMessage};

impl EngineInner {
    /// Apply the immediate actor-side effect authored by
    /// actor instruction for map-movement elements.
    ///
    /// This happens when the Move/Seek is instructed, before path
    /// translation or execution. It is therefore not derivable from the
    /// eventual concrete movement order: a pending path, a failed path, and
    /// a delayed script sequence must all already have anti-collision off.
    fn apply_map_move_instruction_side_effect(
        &mut self,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) {
        let is_map_move = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .is_some_and(|element| {
                matches!(
                    element.data,
                    crate::sequence::SequenceElementData::Movement { flags, .. }
                        if flags.contains(crate::sequence::MoveFlags::MAP)
                )
            });
        if is_map_move {
            self.world
                .entities
                .expect_entity_mut(owner, format_args!("MAP movement owner during Instruct"))
                .position_iface_mut()
                .set_anti_collision_on(false);
        }
    }

    /// Complete the ordinary Move/Seek path translation after its destination
    /// has been resolved. Both the regular hourglass and SetAIState's exact
    /// owner-local native barrier use this same outcome handling.
    fn dispatch_prepared_move_instruction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        dest: crate::coordinates::MapPoint,
        move_action: crate::order::OrderType,
    ) {
        match self.try_dispatch_move_path(sim, assets, owner, seq_id, elem_idx, dest, move_action) {
            MovePathOutcome::Success | MovePathOutcome::Pending => (),
            MovePathOutcome::ActorGone | MovePathOutcome::Refused => {
                self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
            }
            MovePathOutcome::Failed => {
                // Path request insertion calls Stop + Wait when it
                // cannot extract the actor, then returns without adding the
                // request to either path queue. `try_dispatch_move_path`
                // already performed those owner effects; there is no failed
                // A* request to retain or time out here.
            }
        }
    }
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
        Some(crate::sequence::FieldValue::Integer(id)) => {
            crate::gate::DoorIndex::new(*id).expect("valid door index")
        }
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
/// This check is performed
/// directly from `Translate`: a sector-less assertion compares max-norm
/// distance against `tolerance + 5`, while a sector assertion compares only
/// the sector. Both paths interrupt on mismatch and terminate on success.

impl EngineInner {
    pub(in crate::engine) fn dispatch_position_assertion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let movement = self
            .orders
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

        let Some(entity) = self.world.entities.get(owner) else {
            tracing::warn!(
                ?owner,
                ?seq_id,
                elem_idx,
                "interrupting AssertPosition because its owner no longer exists"
            );
            self.element_interrupted(
                sim,
                assets,
                active_scripts,
                seq_id,
                elem_idx,
                crate::sequence::CascadeFlags::NEXT_LEVEL,
            );
            return;
        };
        let live_position = entity.element_data().position_map();
        let live_sector = entity.element_data().sector();
        let mismatches = movement.is_some_and(|(destination, expected_sector, tolerance)| {
            if let Some(expected_sector) = expected_sector {
                live_sector != Some(expected_sector)
            } else {
                let delta_x = live_position.x - destination.x;
                let delta_y = live_position.y - destination.y;
                // Preserve Original's literal mismatch predicate from
                // actor translation. Its maximum-norm comparison
                // comparison is false for qNaN, so a route built from a
                // transient qNaN source accepts the gate-entry assertion and
                // continues to PassDoor. Rewriting this as the positive
                // `< limit` test is not IEEE-equivalent.
                delta_x.abs().max(delta_y.abs()) >= tolerance + 5.0
            }
        });

        if mismatches {
            self.element_interrupted(
                sim,
                assets,
                active_scripts,
                seq_id,
                elem_idx,
                crate::sequence::CascadeFlags::NEXT_LEVEL,
            );
        } else {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
        }
        // Position assertion changes state directly during translation. That
        // synchronous condolence clears the actor's selected sequence element, so
        // instruction handling observes the changed pointer and skips its ordinary
        // IN_PROGRESS/order-publication epilogue.
    }
}

/// Owner-local WAIT_FREE_LIFT arbitration after actor action execution.
///
/// Translation books the same stationary order as WAIT. This context is then
/// invoked once per actual owner Execute while the element remains current,
/// matching the game's projectile-launch behavior.

impl EngineInner {
    pub(in crate::engine) fn authorize_and_reserve_lift_wait(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> bool {
        let element = self
            .orders
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
        let door = self.script_domains.interactables.doors.get(usize::from(gate_id)).unwrap_or_else(|| {
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
            .world
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
        let grid_idx = *self.world.fast_grid_mut()
            .level
            .sector_number_map
            .get(&sector_number)
            .unwrap_or_else(|| {
                panic!(
                    "WAIT_FREE_LIFT owner {owner:?} door {gate_id} references missing lift sector {sector_number:?}"
                )
            });
        let sector = self.world.fast_grid_mut()
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
            let lift = self.world.fast_grid_mut().lift_state_mut(grid_idx as u32);
            if is_high {
                lift.is_authorized_downwards()
            } else {
                lift.is_authorized_upwards()
            }
        };

        if authorized {
            let owner_is_pc = self
                .world
                .entities
                .expect_entity(
                    owner,
                    format_args!("WAIT_FREE_LIFT owner during reservation"),
                )
                .is_pc();
            let lift = self.world.fast_grid_mut().lift_state_mut(grid_idx as u32);
            if is_high {
                lift.set_occupied_downwards(true, owner_is_pc);
            } else {
                lift.set_occupied_upwards(true, owner_is_pc);
            }
            let actor = self.world.entities.expect_actor_data_mut(
                owner,
                format_args!("WAIT_FREE_LIFT owner during reservation"),
            );
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

impl EngineInner {
    pub(in crate::engine) fn dispatch_smalltalk_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let antagonist = self.orders.sequence_manager
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
        let owner_entity =
            self.world.entities.get(owner).unwrap_or_else(|| {
                panic!("smalltalk command {command:?} owner {owner:?} is missing")
            });
        let opponent = self.world.entities.expect_entity(
            antagonist,
            format_args!("smalltalk command {command:?} owner {owner:?} antagonist"),
        );
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
        let blocked = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
            .is_some_and(|(_, _, order)| {
                super::melee::sword_strike_from_animation(order.order_type).is_some()
            });
        if blocked {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        }

        let mut order = crate::order::Order::new(
            order_type,
            0.0,
            0.0,
            crate::order::alloc_order_id(&mut self.orders.next_order_id),
        );
        // Human-actor translation stores the interaction antagonist on
        // every smalltalk strike/parry order. Execute uses it for live strike
        // facing and later wound geometry; parry variants retain the same
        // authored pointer even though their live facing uses the principal
        // opponent.
        order.antagonist = Some(antagonist);
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);
    }
}

/// Bow-transition command translation with only the owners it actually uses.
///
/// The original command bodies read actor posture/action state, append
/// transition orders, and update the sequence element. They do not need the
/// mission, scripts, AI, players, feedback, or spatial world domains.

impl EngineInner {
    fn dispatch_bow_transition(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let owner_entity = self
            .world
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("bow command owner missing: {owner:?}"));
        let posture = owner_entity.element_data().posture();
        let owner_action_state = owner_entity
            .actor_data()
            .map(|actor| actor.action_state)
            .unwrap_or_else(|| panic!("bow command owner missing actor data: {owner:?}"));
        if matches!(command, Command::EquipBow | Command::EquipBowDown)
            && owner_action_state.is_bow()
        {
            // Transition generation may already have queued an equip transition,
            // but the original game still enters ordinary translation with
            // transitions-only mode disabled) command body afterward. That body
            // terminates redundant equip commands whenever the actor is
            // already aiming.
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        }

        let command_body_already_queued = self
            .orders
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
            let anonymous = posture == crate::element::Posture::AnonymousArcher;
            let target_xy = self
                .orders
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
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionEquipBowAnonymous,
                            (0.0, 0.0),
                            false,
                        );
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionLoadingBowAnonymous,
                            (0.0, 0.0),
                            false,
                        );
                    } else {
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionEquipBow,
                            (0.0, 0.0),
                            false,
                        );
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionLoadingBow,
                            (0.0, 0.0),
                            false,
                        );
                    }
                    self.sequence_bow_set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBow,
                    );
                }
                Command::EquipBowDown => {
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionEquipBow,
                        (0.0, 0.0),
                        false,
                    );
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionLoadingBow,
                        (0.0, 0.0),
                        false,
                    );
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionLoweringBowLeaningOut,
                        (0.0, 0.0),
                        false,
                    );
                    self.sequence_bow_set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBowDown,
                    );
                }
                Command::UnequipBow => {
                    let (x, y) = target_xy;
                    if anonymous {
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionUnloadBowAnonymous,
                            (x, y),
                            false,
                        );
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionUnequipBowAnonymous,
                            (x, y),
                            false,
                        );
                    } else {
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionUnloadBow,
                            (x, y),
                            false,
                        );
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionUnequipBow,
                            (x, y),
                            false,
                        );
                    }
                    self.sequence_bow_set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::Waiting,
                    );
                }
                Command::RaiseBow => {
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        if anonymous {
                            OrderType::TransitionRaisingBowAnonymous
                        } else {
                            OrderType::TransitionRaisingBow
                        },
                        (0.0, 0.0),
                        false,
                    );
                    self.sequence_bow_set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBowUp,
                    );
                }
                Command::LowerBow => {
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        if anonymous {
                            OrderType::TransitionLoweringBowAnonymous
                        } else {
                            OrderType::TransitionLoweringBow
                        },
                        (0.0, 0.0),
                        false,
                    );
                    self.sequence_bow_set_action_state_after_transition(
                        seq_id,
                        elem_idx,
                        ActionState::AimingWithBow,
                    );
                }
                _ => unreachable!("non-bow command passed to bow transition context"),
            }
        }
    }
    fn sequence_bow_set_action_state_after_transition(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        state: crate::element::ActionState,
    ) {
        let element = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("bow transition sequence element disappeared during dispatch");
        element.action_state_after_transition = state;
    }
}

/// Script-target activation resolution with no mutable world access.

impl EngineInner {
    fn dispatch_target_activation(
        &mut self,
        owner: EntityId,
        command: Command,
        antagonist: Option<EntityId>,
    ) -> (i32, i32, &'static str) {
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
            self.world
                .entities
                .get(owner)
                .is_some_and(|entity| entity.kind().is_fx_target()),
            "{method} dispatched on non-FX-target owner {owner:?}",
        );
        let target_handle = crate::natives::ScriptHandleCodec::actor_handle(owner);
        let pc_handle = antagonist
            .map(crate::natives::ScriptHandleCodec::actor_handle)
            .unwrap_or(0);
        (target_handle, pc_handle, method)
    }
}

/// Actor/FX animation and target-interaction translation.

impl EngineInner {
    fn dispatch_play_animation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        animation: Option<crate::order::OrderType>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        preserve_trigger_visual: bool,
    ) {
        let Some(animation) = animation else {
            tracing::warn!(
                entity = ?owner,
                cmd = ?command,
                "PlayAnim*: missing/invalid AnimationId — terminating",
            );
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        };

        let Some(owner_entity) = self.world.entities.get(owner) else {
            self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        };
        if owner_entity.is_human() {
            let wrapper = match command {
                Command::PlayAnim => crate::order::OrderType::PlayCustom,
                Command::PlayAnimLoop => crate::order::OrderType::PlayCustomLooped,
                Command::PlayAnimFreeze => crate::order::OrderType::PlayCustomFreeze,
                Command::PlayAnimFrozen => crate::order::OrderType::PlayCustomFrozen,
                _ => unreachable!("non-animation command passed to target animation context"),
            };
            let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
            let mut order = crate::order::Order::new(wrapper, 0.0, 0.0, id);
            order.compute_direction = false;
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);

            // This is an ordinary accepted actor-instruction boundary. Let the
            // outer dispatcher publish the actor order and project IN_PROGRESS after
            // the manager drain, just like every other translated actor
            // command. Returning Skip here used to strand the outgoing
            // animation's terminal motion edge on PlayAnim instructions.
            return;
        }

        if !owner_entity.kind().is_fx_target() {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        }

        if preserve_trigger_visual
            && matches!(command, Command::PlayAnimFreeze | Command::PlayAnimFrozen)
        {
            // One-shot mechanisms can freeze on an entirely transparent spent
            // sprite (Lincoln's drawbridge uses action 160). Keep the reusable
            // control visible and pixel-pickable. This queued command executes
            // after ActivatedBy* returns and captures its reversible patches.
            // Complete normally so following mission messages still run once.
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        }

        let progression_ordinal = match command {
            Command::PlayAnim => crate::sprite::FrameProgression::Default as u32,
            Command::PlayAnimLoop => crate::sprite::FrameProgression::Cyclically as u32,
            Command::PlayAnimFreeze => crate::sprite::FrameProgression::FreezeWhenTerminated as u32,
            Command::PlayAnimFrozen => crate::sprite::FrameProgression::FrozenLastFrame as u32,
            _ => unreachable!("non-animation command passed to target animation context"),
        };
        let entity = self
            .world
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
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }
}

/// PC-side FX-target orders need read-only entity classification plus the two
/// order fields they mutate; they never need mutable world access.

impl EngineInner {
    fn dispatch_sequence_target_interaction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        owner_command: Command,
        target: Option<EntityId>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        // SEARCH is the one command in this group that does not belong to
        // a target. Human-actor search translation
        // inserts a single SEARCHING / SEARCHING_CROUCHED order carrying
        // whatever antagonist the element holds
        // and callers legitimately pass a
        // corpse, including every PC click on a
        // lying body) or no antagonist at all (the SEARCH x4 net-pickup
        // sequence). Applying the
        // fx-target gate to it terminated all of those before they animated.
        if owner_command == Command::SearchCmd {
            let crouched = self.world.entities.get(owner).is_some_and(|entity| {
                entity.element_data().posture() == crate::element::Posture::Crouched
            });
            let order_type = if crouched {
                crate::order::OrderType::SearchingCrouched
            } else {
                crate::order::OrderType::Searching
            };
            let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
            let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
            order.compute_direction = false;
            order.antagonist = target;
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);

            return;
        }
        let Some(target) = target else {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        };
        if !self
            .world
            .entities
            .get(target)
            .is_some_and(|entity| entity.kind().is_fx_target())
        {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        }
        let order_types: &[crate::order::OrderType] = match owner_command {
            // Player-character hit-target translation raises the sword,
            // performs the target hit, then lowers it again. Besides the
            // visible transitions, those orders drive WaitingSword back to
            // Waiting at the same points as the original action-state
            // machine.
            Command::HitTarget => &[
                crate::order::OrderType::TransitionRaisingSword,
                crate::order::OrderType::HittingTarget,
                crate::order::OrderType::TransitionLoweringSword,
            ],
            Command::HandleTarget => &[crate::order::OrderType::HandlingTarget],
            Command::UseLever => &[crate::order::OrderType::UsingLever],
            Command::TakeTarget => &[crate::order::OrderType::TakingTarget],
            _ => unreachable!("non-target command passed to target interaction context"),
        };
        for &order_type in order_types {
            let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
            let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
            order.compute_direction = false;
            if order_type == crate::order::OrderType::HittingTarget || order_types.len() == 1 {
                order.antagonist = Some(target);
            }
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
        }
    }
}

/// Directional and one-shot owner commands that only mutate entity facing
/// plus the owning sequence/order allocator.

impl EngineInner {
    pub(in crate::engine) fn dispatch_turn_command(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match command {
            Command::Turn | Command::TurnFast => {
                let element = self.orders.sequence_manager.get_element(seq_id, elem_idx);
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
                if let Some(entity) = self.world.entities.get_mut(owner) {
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
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    crate::order::OrderType::Turning,
                    (0.0, 0.0),
                    false,
                );
            }
            Command::TurnElement => {
                let antagonist = self
                    .orders
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
                        .world
                        .entities
                        .get(antagonist)
                        .map(|entity| entity.element_data().position_map());
                    if let (Some(antagonist_position), Some(entity)) =
                        (antagonist_position, self.world.entities.get_mut(owner))
                    {
                        let position = entity.element_data().position_map();
                        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                            antagonist_position.x - position.x,
                            antagonist_position.y - position.y,
                        );
                        // The original game's turn translation updates the direction goal only.
                        // The actor's later `Turn()` step advances the current
                        // direction toward it; snapping both values here
                        // skips that observable turn frame.
                        entity.element_data_mut().set_direction_goal(direction);
                    }
                }
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    crate::order::OrderType::Turning,
                    (0.0, 0.0),
                    false,
                );
            }
            Command::Freeze => {
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    crate::order::OrderType::Freezing,
                    (0.0, 0.0),
                    true,
                );
            }
            Command::Point | Command::GatherSoldiers => {
                let order_type = match command {
                    Command::Point => crate::order::OrderType::Pointing,
                    Command::GatherSoldiers => crate::order::OrderType::GatheringSoldiers,
                    _ => unreachable!(),
                };
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    order_type,
                    (0.0, 0.0),
                    false,
                );
            }
            _ => unreachable!("non-turn command passed to turn command context"),
        }
    }
}

/// WAIT/WAIT_TIMER translation against entity state, sequence state, and the
/// immutable profile table used by the carried-VIP animation branch.

/// The base actor WAIT posture switch.
///
/// Human WAIT translation normally handles dead/unconscious holds itself.
/// Its upright-dead emergency fall has one deliberate default fallthrough to
/// this base switch, which re-reads the element's stamped transition state
/// rather than the Human translator's local collapsed posture.
fn base_wait_animation(
    posture: crate::element::Posture,
    action_state: crate::element::ActionState,
) -> crate::order::OrderType {
    use crate::element::{ActionState as AS, Posture as P};
    use crate::order::OrderType as OT;

    match posture {
        P::Upright => OT::WaitingUprightBored,
        P::Crouched => OT::WaitingCrouched,
        P::OnWall | P::OnLadder => OT::Freezing,
        P::Sitting => OT::Sitting,
        P::Lying => OT::BeingUnconscious,
        P::DeadBack => match action_state {
            state if state.is_sword() || state == AS::Menacing => OT::BeingDeadFallenBackSword,
            state if state.is_bow() => OT::BeingDeadFallenBackBow,
            _ => OT::BeingDeadFallenBack,
        },
        P::Dead => match action_state {
            state if state.is_sword() => OT::BeingDeadSword,
            state if state.is_bow() => OT::BeingDeadBow,
            _ => OT::BeingDead,
        },
        other => panic!("base WAIT translation does not support posture {other:?}"),
    }
}

impl EngineInner {
    pub(in crate::engine) fn dispatch_wait_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
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
            // Keep "Wait translation owner" in the context: asserted by
            // `wait_context_rejects_stale_owner_contextually`.
            let entity = self.world.entities.expect_entity(
                owner,
                format_args!("Wait translation owner for {command:?} at {seq_id:?}/{elem_idx}"),
            );
            let actor = entity.actor_data().unwrap_or_else(|| {
                panic!(
                    "Wait translation owner {owner:?} is not an actor for {command:?} at {seq_id:?}/{elem_idx}"
                )
            });
            let carrier = entity.human_data().and_then(|human| human.carrier);
            (
                entity.is_soldier(),
                entity.is_pc(),
                entity.element_data().posture(),
                actor.action_state,
                entity.enemy_ai().is_some_and(|enemy| enemy.attentive),
                entity.is_dead(),
                entity.is_unconscious(),
                entity
                    .human_data()
                    .is_some_and(|human| !human.opponents.is_empty()),
                entity
                    .human_data()
                    .is_some_and(|human| human.stuck_under_nets_counter > 0),
                carrier.is_some_and(|carrier_id| {
                    let carrier = self.world.entities.expect_entity(
                        carrier_id,
                        format_args!(
                            "Wait translation owner {owner:?} carrier at {seq_id:?}/{elem_idx}"
                        ),
                    );
                    self.sequence_wait_is_entity_vip(assets, carrier)
                }),
            )
        };

        let wait_element = self.orders.sequence_manager
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
        tracing::trace!(
            ?owner,
            ?command,
            ?posture,
            posture_after_transition = ?wait_element.posture_after_transition,
            ?action_state,
            action_state_after_transition = ?wait_element.action_state_after_transition,
            "translating actor wait"
        );
        let stamped_posture = wait_element.posture_after_transition;
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
                P::Spy | P::Cloaked => Some(OT::WaitingCape),
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

        // An actor that reaches an upright wait while already dead or
        // unconscious never got a damage sequence to fall through, so the
        // wait itself has to stage the collapse: drop whatever transition
        // the movement machinery queued, play the harder falling animation,
        // and translate the rest of this wait as if the actor were already
        // down. Only the animation is redirected here — the posture write
        // still happens when the fall lands.
        let heart_attack_animation =
            if posture == crate::element::Posture::Upright && (is_unconscious || is_dead) {
                crate::engine::melee::select_hit_fall_animation(posture, action_state, true)
            } else {
                None
            };
        // Human's local posture switch sees the collapsed posture. Its
        // default dead-back arm delegates to the base Actor translator,
        // which re-reads the element's original stamped posture instead.
        let switch_posture = if heart_attack_animation.is_some() {
            if is_unconscious {
                crate::element::Posture::Lying
            } else {
                crate::element::Posture::DeadBack
            }
        } else {
            posture
        };

        // Human-actor translation allocates its wait order before
        // switching on posture/action state. A few arms then delegate to
        // base actor translation and return without inserting that order;
        // the base translator allocates the real order separately. The
        // leaked object still advances the next order ID
        // in the original game. Preserve that
        // allocation side effect so every later runtime order retains
        // Original's identity.
        let soldier_handles_wait = is_soldier
            && ((is_attentive
                && posture == crate::element::Posture::Upright
                && action_state == crate::element::ActionState::Waiting
                && !is_dead
                && !is_unconscious)
                || posture == crate::element::Posture::LeaningOut);
        let human_wait_discards_preallocated_order = pc_posture_animation.is_none()
            && !soldier_handles_wait
            && match switch_posture {
                crate::element::Posture::Upright => {
                    !is_swordfighting
                        && !after_state.is_shield()
                        && !after_state.is_sword()
                        && !matches!(
                            after_state,
                            crate::element::ActionState::AimingWithBow
                                | crate::element::ActionState::AimingWithBowUp
                                | crate::element::ActionState::Menacing
                                | crate::element::ActionState::Sleeping
                        )
                }
                crate::element::Posture::DeadBack => {
                    !after_state.is_sword()
                        && !after_state.is_bow()
                        && after_state != crate::element::ActionState::Menacing
                }
                crate::element::Posture::Crouched
                | crate::element::Posture::OnWall
                | crate::element::Posture::OnLadder
                | crate::element::Posture::Sitting => true,
                _ => false,
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
            match switch_posture {
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
                    state if state.is_sword() || state == AS::Menacing => {
                        OT::BeingDeadFallenBackSword
                    }
                    state if state.is_bow() => OT::BeingDeadFallenBackBow,
                    _ if heart_attack_animation.is_some() && is_dead => {
                        base_wait_animation(stamped_posture, after_state)
                    }
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
                .orders
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
                .world
                .entities
                .expect_actor_data_mut(owner, format_args!("WAIT_TIMER owner during translation"));
            actor.wait_time = timer;
        }
        if is_pc
            && posture == crate::element::Posture::Upright
            && action_state == crate::element::ActionState::Listening
        {
            let actor = self
                .world
                .entities
                .expect_actor_data_mut(owner, format_args!("Wait translation listening owner"));
            const TIME_LISTEN_WAIT: u32 = 25;
            actor.wait_time = TIME_LISTEN_WAIT;
        }
        if set_posture_stuck_under_net {
            let entity = self.world.entities.expect_entity_mut(
                owner,
                format_args!("Wait translation owner while setting net posture"),
            );
            entity
                .element_data_mut()
                .set_posture(crate::element::Posture::StuckUnderNet);
        }

        if let Some(falling) = heart_attack_animation {
            tracing::trace!(
                ?owner,
                ?command,
                ?falling,
                is_dead,
                is_unconscious,
                "upright wait on a downed actor; collapsing"
            );
            self.orders
                .sequence_manager
                .clear_orders_on(seq_id, elem_idx);
            let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
            let mut order = crate::order::Order::new(falling, 0.0, 0.0, id);
            order.compute_direction = false;
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
        }

        if human_wait_discards_preallocated_order {
            let _discarded_original_order_id =
                crate::order::alloc_order_id(&mut self.orders.next_order_id);
        }

        if let Some(animation) = animation {
            let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
            let mut order = crate::order::Order::new(animation, 0.0, 0.0, id);
            // Human command translation deliberately delegates a plain dead-back wait
            // to base actor translation. The base translator leaves the order's
            // default direction-computation setting intact, unlike the sword/bow
            // dead-back arms handled above
            // by the original game. This can refresh the corpse's
            // facing from a newly stamped placement destination.
            order.compute_direction = animation == crate::order::OrderType::BeingDeadFallenBack;
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
        } else {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
        }
    }

    fn sequence_wait_is_entity_vip(&self, assets: &LevelAssets, entity: &Entity) -> bool {
        match entity {
            Entity::Soldier(soldier) => assets
                .profile_manager
                .get_soldier(soldier.soldier.soldier_profile_index)
                .is_some_and(|profile| profile.vip),
            Entity::Civilian(civilian) => assets
                .profile_manager
                .civilians
                .get(usize::from(civilian.civilian.civilian_profile_index))
                .is_some_and(|profile| profile.civilian_type == crate::profiles::CivilianType::Vip),
            _ => false,
        }
    }
}

/// Fixed NPC posture/action-state transition order translation.

impl EngineInner {
    pub(in crate::engine) fn dispatch_npc_state_command(
        &mut self,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
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
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    order_type,
                    (0.0, 0.0),
                    false,
                );
            }
            Command::StartMenace
            | Command::StopMenace
            | Command::StopSleep
            | Command::LowerBowLeanOut
            | Command::RaiseBowLeanOut => {
                match command {
                    Command::StartMenace => {
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionRaisingSword,
                            (0.0, 0.0),
                            false,
                        );
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionWaitingSwordMenacing,
                            (0.0, 0.0),
                            false,
                        );
                    }
                    Command::StopMenace => {
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionMenacingWaitingSword,
                            (0.0, 0.0),
                            false,
                        );
                        OrderEmitter::new(&mut self.orders.next_order_id).push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionLoweringSword,
                            (0.0, 0.0),
                            false,
                        );
                    }
                    Command::StopSleep => OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        crate::order::OrderType::TransitionSleepingWaitingUpright,
                        (0.0, 0.0),
                        false,
                    ),
                    Command::LowerBowLeanOut => OrderEmitter::new(&mut self.orders.next_order_id)
                        .push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionLoweringBowLeaningOut,
                            (0.0, 0.0),
                            false,
                        ),
                    Command::RaiseBowLeanOut => OrderEmitter::new(&mut self.orders.next_order_id)
                        .push(
                            &mut self.orders.sequence_manager,
                            seq_id,
                            elem_idx,
                            crate::order::OrderType::TransitionRaisingBowLeaningOut,
                            (0.0, 0.0),
                            false,
                        ),
                    _ => unreachable!(),
                }
                // These translated orders remain owned by the command's
                // selected sequence element until the actor executes the
                // final order. The original game only appends the translated orders;
                // it never terminates START_MENACE / STOP_MENACE /
                // STOP_SLEEP at instruction time.
            }
            _ => unreachable!("non-NPC-state command passed to NPC state context"),
        }
    }
}

/// NPC look/lean and attentive-mode translation against only entity state,
/// sequence state, and order-id allocation.
///
/// Soldier swordfight setup appends these
/// orders synchronously from `Translate`. The sequence phase must therefore
/// still reach its after-action splice immediately after this context returns.

impl EngineInner {
    pub(in crate::engine) fn dispatch_npc_attention_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match command {
            Command::LookLeft | Command::LookRight | Command::LeanOut => {
                let order_type = self.world.entities.get(owner).map(|entity| {
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
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        order_type,
                        (0.0, 0.0),
                        false,
                    );
                } else {
                    self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
                }
            }
            Command::EnterAttentiveMode
            | Command::LeaveAttentiveMode
            | Command::LeaveAttentiveModeOfficer => {
                let posture_after_transition = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| element.posture_after_transition)
                    .unwrap_or_default();
                if !self.sequence_dispatch_attentive(
                    owner,
                    command,
                    posture_after_transition,
                    seq_id,
                    elem_idx,
                ) {
                    // Soldier translation terminates the element in this
                    // branch. That can replace the selected sequence element while
                    // Translation is still active, so actor instruction handling
                    // returns before publishing the actor order or IN_PROGRESS.
                    self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                }
            }
            _ => unreachable!("non-attention command passed to NPC attention context"),
        }
    }

    fn sequence_dispatch_attentive(
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
            OrderEmitter::new(&mut self.orders.next_order_id).push(
                &mut self.orders.sequence_manager,
                seq_id,
                elem_idx,
                animation,
                (0.0, 0.0),
                false,
            );
            return true;
        }

        let posture_upright_after = posture_after_transition == crate::element::Posture::Upright;
        let Some(entity) = self.world.entities.get(owner) else {
            return false;
        };
        let currently_attentive = entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
        // Soldier translation has deliberately asymmetric arms:
        // ENTER checks that the actor is not attentive, while LEAVE checks only the
        // stamped post-transition posture.  A corrective LEAVE can therefore
        // still carry its alerted-to-upright animation after an earlier
        // officer transition has already cleared attentiveness.
        let needs_change = currently_attentive != target_attentive;
        let can_play_transition =
            posture_upright_after && (command == Command::LeaveAttentiveMode || needs_change);
        tracing::trace!(
            owner = owner.index(),
            ?command,
            ?posture_after_transition,
            posture_upright_after,
            currently_attentive,
            target_attentive,
            needs_change,
            can_play_transition,
            "dispatch attentive transition"
        );

        if can_play_transition {
            OrderEmitter::new(&mut self.orders.next_order_id).push(
                &mut self.orders.sequence_manager,
                seq_id,
                elem_idx,
                animation,
                (0.0, 0.0),
                false,
            );
            true
        } else {
            if let Some(enemy) = self
                .world
                .entities
                .get_mut(owner)
                .and_then(Entity::enemy_ai_mut)
            {
                enemy.attentive = target_attentive;
            }
            false
        }
    }
}

/// Stealth-posture command translation with the exact mutable domains it
/// touches: entities, the owning sequence, order allocation, and HIDDEN
/// titbits. Character profiles are read only to derive the HIDDEN phase.

impl EngineInner {
    pub(in crate::engine) fn dispatch_stealth_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let Some(entity) = self.world.entities.get(owner) else {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        };
        let live_posture = entity.element_data().posture();

        // Command translation appends the stance body order
        // unconditionally.  Admission gating happened earlier: the
        // transition-generation pass either produced a valid
        // exit-action/posture prefix or marked the element Impossible,
        // and Execute-time sequence validity remains the authoritative
        // per-frame gate.  Re-validating the body against the element's
        // saved post-prefix state here would wrongly reject prefixes
        // that deliberately retain a masking posture: leaving the spy
        // cape, tree hide, or anonymous-archer disguise keeps the saved
        // posture on the element even though the prefix animation makes
        // the actor upright before the body executes.

        let Some(transition) = crate::stealth::stealth_transition(command) else {
            self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        };

        // These PC commands are animation bodies in Original, following any
        // transition prefix produced above.  Their posture/action and toolbar
        // side effects happen only when the body animation reaches DONE; the
        // sequence element remains selected until TERMINATED.  Keeping that
        // lifetime is also what makes the command query expose ENTER_* while an old
        // Bored-to-Waiting prefix is still playing.
        if matches!(
            command,
            Command::EnterBeggar
                | Command::LeaveBeggar
                | Command::EnterHelpingClimb
                | Command::LeaveHelpingClimb
                | Command::EnterCloak
        ) {
            let has_carried = entity.pc_data().is_some_and(|pc| pc.carried.is_some());
            let animations: &[crate::order::OrderType] = if command == Command::LeaveHelpingClimb {
                crate::stealth::leave_helping_climb_orders(live_posture, has_carried)
            } else {
                std::slice::from_ref(&transition.animation)
            };
            for &animation in animations {
                let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
                let mut order = crate::order::Order::new(animation, 0.0, 0.0, id);
                order.compute_direction = false;
                self.orders
                    .sequence_manager
                    .push_order_on(seq_id, elem_idx, order);
            }

            return;
        }

        // Player command translation only appends the crouch transition order. The actor's
        // posture changes after motion completion, not while instruction is
        // translating the command. Keep the element selected/in-progress so
        // The command query reports CROUCH_{DOWN,UP} during that interval.
        if matches!(command, Command::CrouchDown | Command::CrouchUp) {
            let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
            let mut order = crate::order::Order::new(transition.animation, 0.0, 0.0, id);
            order.compute_direction = false;
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);

            return;
        }

        let hidden_phase = if transition.result_posture.is_hidden() {
            let Some(Entity::Pc(pc)) = self.world.entities.get(owner) else {
                self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
                return;
            };
            let profile = assets
                .profile_manager
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

        let old_posture = live_posture;
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("stealth command owner disappeared during dispatch");
        entity.set_posture(transition.result_posture);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = transition.result_action_state;
        }
        let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
        let mut order = crate::order::Order::new(transition.animation, 0.0, 0.0, id);
        order.compute_direction = false;
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);

        tracing::debug!(
            ?owner,
            ?command,
            posture = ?transition.result_posture,
            animation = ?transition.animation,
            "stealth transition applied"
        );

        use crate::coordinates::WorldPoint3D;
        use crate::titbit::{ElementHandle, TitbitKind};
        let layer = entity.element_data().layer();
        let handle = ElementHandle(owner.index());
        if transition.result_posture.is_hidden() && !old_posture.is_hidden() {
            self.feedback.titbit_manager.add_titbit(
                WorldPoint3D::default(),
                layer,
                TitbitKind::Hidden,
                handle,
                hidden_phase.expect("hidden phase resolved before entering hidden posture"),
                handle,
                false,
                None,
                true,
                None,
                Some(layer),
            );
        } else if !transition.result_posture.is_hidden() && old_posture.is_hidden() {
            self.feedback
                .titbit_manager
                .remove_titbit(TitbitKind::Hidden, handle);
        }

        // TODO(parity): The original game applies posture/action, HIDDEN, and nearby-coin
        // side effects on transition-animation DONE
        // during the action. This implementation has historically snapped
        // them during command dispatch while leaving the animation order on
        // the terminated element. Move all four effects together only after
        // the animation completion path can retain that order safely.
        if transition.result_posture == crate::element::Posture::SimulatingBeggar
            && old_posture != crate::element::Posture::SimulatingBeggar
        {
            super::beggar::set_flags_of_near_coins_on_ground(&mut self.world.entities, owner, true);
        } else if old_posture == crate::element::Posture::SimulatingBeggar
            && transition.result_posture != crate::element::Posture::SimulatingBeggar
        {
            super::beggar::set_flags_of_near_coins_on_ground(
                &mut self.world.entities,
                owner,
                false,
            );
        }

        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }
}

/// Direct actor abilities whose translation is confined to entity state,
/// sequence state, order-id allocation, and immutable animation profiles.
/// Campaign-backed ammo checks are evaluated by the caller and passed as a
/// value so this context cannot reach mission or campaign ownership.

impl EngineInner {
    pub(in crate::engine) fn dispatch_direct_ability_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        ammo_available: bool,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        // Every arm either bails out through `element_impossible` /
        // `element_terminated`, or picks the begin function whose result the
        // shared tail below reports.
        type TargetedBegin = fn(
            &mut crate::entities::Entities,
            &mut crate::sequence::SequenceManager,
            EntityId,
            EntityId,
            crate::sequence::SequenceId,
            usize,
            &mut u32,
        ) -> AbilityBeginResult;
        type UntargetedBegin = fn(
            &mut crate::entities::Entities,
            &mut crate::sequence::SequenceManager,
            EntityId,
            crate::sequence::SequenceId,
            usize,
            &mut u32,
        ) -> AbilityBeginResult;
        type GroundBegin = fn(
            &mut crate::entities::Entities,
            &mut crate::sequence::SequenceManager,
            EntityId,
            crate::coordinates::MapPoint,
            crate::sequence::SequenceId,
            usize,
            &mut u32,
        ) -> AbilityBeginResult;

        let result = match command {
            Command::TieCmd
            | Command::Untie
            | Command::HealCmd
            | Command::HitCmd
            | Command::StrangleCmd => {
                let Some(target) = self.sequence_ability_interaction_target(seq_id, elem_idx)
                else {
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                };
                if command == Command::HealCmd && !ammo_available {
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                }
                let begin: TargetedBegin = match command {
                    Command::TieCmd => abilities::begin_tie,
                    Command::Untie => abilities::begin_untie,
                    Command::HealCmd => abilities::begin_heal,
                    Command::HitCmd => abilities::begin_hit,
                    Command::StrangleCmd => abilities::begin_strangle,
                    _ => unreachable!(),
                };
                begin(
                    &mut self.world.entities,
                    &mut self.orders.sequence_manager,
                    owner,
                    target,
                    seq_id,
                    elem_idx,
                    &mut self.orders.next_order_id,
                )
            }
            Command::WhistleCmd | Command::EatCmd | Command::ReceivePurse => {
                if command == Command::EatCmd && !ammo_available {
                    self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                }
                let begin: UntargetedBegin = match command {
                    Command::WhistleCmd => abilities::begin_whistle,
                    Command::EatCmd => abilities::begin_eat,
                    Command::ReceivePurse => abilities::begin_receive_purse,
                    _ => unreachable!(),
                };
                begin(
                    &mut self.world.entities,
                    &mut self.orders.sequence_manager,
                    owner,
                    seq_id,
                    elem_idx,
                    &mut self.orders.next_order_id,
                )
            }
            Command::EnterListen => abilities::begin_listen(
                &mut self.world.entities,
                &assets.profile_manager,
                &mut self.orders.sequence_manager,
                owner,
                seq_id,
                elem_idx,
                &mut self.orders.next_order_id,
            ),
            Command::LeaveListen => {
                // EnterListen is NON_INTERRUPTABLE in Original, so production
                // arbitration postpones LeaveListen until the complete
                // entry/listening/exit chain has restored Waiting. Re-dispatch
                // then fails MUST_BE_LISTENING and is Impossible.
                self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                return;
            }
            Command::ThrowNet | Command::ThrowPurse | Command::ThrowWaspNest => {
                let (field, begin): (_, GroundBegin) = match command {
                    Command::ThrowNet => (
                        crate::sequence::Field::NetTarget,
                        abilities::begin_throw_net,
                    ),
                    Command::ThrowPurse => (
                        crate::sequence::Field::PurseTarget,
                        abilities::begin_throw_purse,
                    ),
                    Command::ThrowWaspNest => (
                        crate::sequence::Field::WaspNestTarget,
                        abilities::begin_throw_wasp_nest,
                    ),
                    _ => unreachable!(),
                };
                let Some(target) = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| read_sequence_map_point_property(element, field))
                else {
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                };
                if !ammo_available {
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                }
                begin(
                    &mut self.world.entities,
                    &mut self.orders.sequence_manager,
                    owner,
                    target,
                    seq_id,
                    elem_idx,
                    &mut self.orders.next_order_id,
                )
            }
            Command::ThrowApple | Command::ThrowStone => {
                if !ammo_available {
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                }
                let ground_target = if command == Command::ThrowStone {
                    self.orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .and_then(|element| {
                            match element
                                .get_property(crate::sequence::Field::NoiseDistractionTarget)
                            {
                                Some(crate::sequence::FieldValue::Point3D { x, y, .. }) => {
                                    Some(crate::coordinates::MapPoint::new(*x, *y))
                                }
                                Some(value) => panic!(
                                    "ground ThrowStone has invalid required 3D target {value:?}"
                                ),
                                None => None,
                            }
                        })
                } else {
                    None
                };
                if let Some(target) = ground_target {
                    abilities::begin_throw_stone_at_ground(
                        &mut self.world.entities,
                        &mut self.orders.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        &mut self.orders.next_order_id,
                    )
                } else {
                    let Some(target) = self.sequence_ability_interaction_target(seq_id, elem_idx)
                    else {
                        self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                        return;
                    };
                    let begin: TargetedBegin = match command {
                        Command::ThrowApple => abilities::begin_throw_apple,
                        Command::ThrowStone => abilities::begin_throw_stone,
                        _ => unreachable!(),
                    };
                    begin(
                        &mut self.world.entities,
                        &mut self.orders.sequence_manager,
                        owner,
                        target,
                        seq_id,
                        elem_idx,
                        &mut self.orders.next_order_id,
                    )
                }
            }
            _ => unreachable!("non-direct ability passed to direct ability context"),
        };
        self.sequence_ability_finish_begin(sim, assets, active_scripts, result, seq_id, elem_idx)
    }

    fn sequence_ability_interaction_target(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<EntityId> {
        self.orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
                _ => None,
            })
    }

    fn sequence_ability_finish_begin(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        result: AbilityBeginResult,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match result {
            AbilityBeginResult::Started => {}
            AbilityBeginResult::Impossible => {
                self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx)
            }
        }
    }
}

/// Owner recovery and wake-up animations touch only entity state, sequence
/// state, and the deterministic order-id stream.

impl EngineInner {
    fn dispatch_recovery_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        use crate::order::OrderType;

        match command {
            Command::Fainted => {
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    OrderType::BeingUnconsciousSword,
                    (0.0, 0.0),
                    true,
                );
                self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            }
            Command::Recover | Command::StandUp => {
                let already_queued = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .is_some_and(|element| !element.orders.is_empty());
                if !already_queued {
                    let standing_up = self
                        .world
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
                    OrderEmitter::new(&mut self.orders.next_order_id).push(
                        &mut self.orders.sequence_manager,
                        seq_id,
                        elem_idx,
                        standing_up,
                        (0.0, 0.0),
                        true,
                    );
                }
            }
            Command::WakeUp => {
                let target = self
                    .orders
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
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                };
                let Some(target_position) = self
                    .world
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
                    self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
                    return;
                };
                let owner_position = self
                    .world
                    .entities
                    .expect_entity(owner, format_args!("WakeUp owner before direction setup"))
                    .element_data()
                    .position_map();
                let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                    target_position.x - owner_position.x,
                    target_position.y - owner_position.y,
                );
                self.world
                    .entities
                    .expect_entity_mut(owner, format_args!("WakeUp owner during direction setup"))
                    .element_data_mut()
                    .set_direction_goal(direction);

                // The original game's wake-up command inserts a turning order first,
                // after setting the progressive direction goal toward the
                // target, then appends the non-direction-computing WAKING_UP
                // action.
                let turn_id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
                let turn = crate::order::Order::new(OrderType::Turning, 0.0, 0.0, turn_id);
                self.orders
                    .sequence_manager
                    .push_order_on(seq_id, elem_idx, turn);

                let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
                let mut order = crate::order::Order::new(
                    OrderType::WakingUp,
                    target_position.x,
                    target_position.y,
                    id,
                )
                .with_antagonist(target);
                order.compute_direction = false;
                self.orders
                    .sequence_manager
                    .push_order_on(seq_id, elem_idx, order);
            }
            Command::Knee => {
                OrderEmitter::new(&mut self.orders.next_order_id).push(
                    &mut self.orders.sequence_manager,
                    seq_id,
                    elem_idx,
                    OrderType::FallingBackSword,
                    (0.0, 0.0),
                    true,
                );
                self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
            }
            _ => unreachable!("non-recovery command passed to recovery context"),
        }
    }
}

/// Drink/take translation reads only the interaction pair and object payload,
/// then books one deterministic animation order on the owning element.

impl EngineInner {
    fn dispatch_object_interaction_command(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let owner_is_pc = self.world.entities.get(owner).is_some_and(Entity::is_pc);
        let antagonist = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => antagonist,
                _ => None,
            });
        if let Some(antagonist) = antagonist {
            let object_type = self
                .world
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

        let antagonist_is_net = antagonist
            .and_then(|id| self.world.entities.get(id))
            .is_some_and(|entity| matches!(entity, Entity::Net(_)));
        // Player-character take translation selects the ordinary
        // object's pickup row from the interaction element's stamped
        // post-transition posture.  This is especially important for a Take
        // postponed behind a crouched Seek: the live actor is crouched too,
        // but the authored sequence stamp is authoritative.
        let posture_after_transition = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|element| element.posture_after_transition)
            .unwrap_or(crate::element::Posture::Undefined);
        let order_type = match command {
            Command::DrinkAle => crate::order::OrderType::DrinkingAle,
            Command::Take if antagonist_is_net => crate::order::OrderType::TakingNet,
            Command::Take
                if owner_is_pc && posture_after_transition == crate::element::Posture::Crouched =>
            {
                crate::order::OrderType::TakingCrouched
            }
            Command::Take => crate::order::OrderType::Taking,
            _ => unreachable!(),
        };
        let id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        if let Some(antagonist) = antagonist {
            order = order.with_antagonist(antagonist);
        }
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);
    }
}

/// Immediate mobile controls cannot reach any other world or mission state.

impl EngineInner {
    fn dispatch_mobile_immediate(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let mobile_index = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::as_fx)
            .and_then(|fx| fx.fx.mobile_index)
            .unwrap_or_else(|| panic!("{command:?} owner {owner} is not a mobile child FX"));
        let mobile = self
            .world
            .mobile_elements
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
            self.world
                .entities
                .get_mut(child_id)
                .unwrap_or_else(|| panic!("mobile {mobile_index} child {child_id} is missing"))
                .element_data_mut()
                .active = active;
        }
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }
}

/// Immediate sprite metadata commands are isolated from mobile, AI, camera,
/// script, and mission ownership.

impl EngineInner {
    fn dispatch_sprite_immediate(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match command {
            Command::Unblip => {
                if let Some(entity) = self.world.entities.get_mut(owner)
                    && entity.element_data().blipped
                {
                    entity.reveal_blip();
                }
            }
            Command::ReplaceAnim => {
                let old = self.sequence_sprite_animation_property(
                    seq_id,
                    elem_idx,
                    crate::sequence::Field::OldAnimation,
                );
                let new = self.sequence_sprite_animation_property(
                    seq_id,
                    elem_idx,
                    crate::sequence::Field::NewAnimation,
                );
                if let (Some(old), Some(new), Some(entity)) =
                    (old, new, self.world.entities.get_mut(owner))
                {
                    entity.element_data_mut().sprite.replace_anim(old, new);
                }
            }
            Command::RestoreAnim => {
                let old = self.sequence_sprite_animation_property(
                    seq_id,
                    elem_idx,
                    crate::sequence::Field::OldAnimation,
                );
                if let (Some(old), Some(entity)) = (old, self.world.entities.get_mut(owner)) {
                    entity.element_data_mut().sprite.restore_anim(old);
                }
            }
            _ => unreachable!("non-sprite command passed to sprite context"),
        }
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }

    fn sequence_sprite_animation_property(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        field: crate::sequence::Field,
    ) -> Option<crate::order::OrderType> {
        self.orders
            .sequence_manager
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

impl EngineInner {
    fn timer_immediate_entry(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> TimerEntry {
        let remaining = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(crate::sequence::Field::Timer))
            .and_then(|value| match value {
                // The timer is a signed integer in the original game; the
                // sequence-element property table stores it as a `u32` word.
                crate::sequence::FieldValue::Integer(value) => Some(*value as i32),
                _ => None,
            })
            .unwrap_or(0);
        TimerEntry {
            remaining,
            element_ref: crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
        }
    }
}

/// Map/dialog/popup commands emit presentation outputs and reset input
/// without borrowing host UI state.

impl EngineInner {
    fn dispatch_presentation_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match command {
            Command::DisplayMap => {
                let show = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| element.get_property(crate::sequence::Field::MapDisplay))
                    .and_then(|value| match value {
                        crate::sequence::FieldValue::Bool(value) => Some(*value),
                        _ => None,
                    })
                    .unwrap_or(false);
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(HostEvent::Minimap(MinimapHostEvent::DisplayMap {
                        show,
                        restore_position: false,
                    }));
            }
            Command::PlayDialog => {
                if !self.control.fast_forward {
                    let id = self.sequence_presentation_integer_property(
                        seq_id,
                        elem_idx,
                        crate::sequence::Field::DialogId,
                    );
                    self.feedback.pending_side_effects.extend_dialogues([id]);
                }
                self.sequence_presentation_reset_input(sim, assets);
            }
            Command::DisplayPopupText => {
                if !self.control.fast_forward {
                    let id = self.sequence_presentation_integer_property(
                        seq_id,
                        elem_idx,
                        crate::sequence::Field::PopupTextId,
                    );
                    self.feedback.pending_side_effects.extend_popup_texts([id]);
                }
                self.sequence_presentation_reset_input(sim, assets);
            }
            _ => unreachable!("non-presentation command passed to presentation context"),
        }
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }

    fn sequence_presentation_integer_property(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        field: crate::sequence::Field,
    ) -> i32 {
        self.orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(field))
            .and_then(|value| match value {
                crate::sequence::FieldValue::Integer(value) => Some(*value as i32),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn sequence_presentation_reset_input(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.forward_message(
            sim,
            assets,
            Message::new(MessageType::Simple(SimpleMessage::ResetInput)),
        );
    }
}

impl EngineInner {
    fn dispatch_freeze_immediate(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let frozen = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.get_property(crate::sequence::Field::Freeze))
            .and_then(|value| match value {
                crate::sequence::FieldValue::Bool(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(false);
        self.control.set_actors_frozen(frozen);
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }
}

/// Character availability updates PC metadata and selection synchronously.

impl EngineInner {
    fn dispatch_availability_immediate(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<super::script::ActiveScriptCall>,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let element = self.orders.sequence_manager.get_element(seq_id, elem_idx);
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
                    && let Some(pc) = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(Entity::pc_data_mut)
                {
                    pc.set_playable(available);
                    let message = if available {
                        crate::messenger::PcMessage::EnableCharacter
                    } else {
                        crate::messenger::PcMessage::DisableCharacter
                    };
                    self.forward_message(sim, assets, Message::pc(message, Some(owner)));
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
                    self.forward_message(
                        sim,
                        assets,
                        Message::pc_with_value(message, Some(owner), action_id),
                    );
                }
            }
            _ => unreachable!("non-availability command passed to availability context"),
        }
        self.element_terminated(sim, assets, active_scripts, seq_id, elem_idx);
    }
}

#[cfg(test)]
mod sequence_phase_context_tests {
    use super::*;

    fn shield_pc(action_state: crate::element::ActionState) -> Entity {
        Entity::Pc(crate::element::ActorPc {
            actor: crate::element::ActorData {
                action_state,
                ..Default::default()
            },
            pc: crate::element::PcData {
                life_points: crate::pc_status::LIFEPOINTS_PC,
                ..Default::default()
            },
            ..crate::engine::test_support::actors::unbound_pc(crate::element::Posture::Upright)
        })
    }

    fn object_interaction_soldier(direction_goal: i16) -> Entity {
        let mut soldier =
            crate::engine::test_support::actors::unbound_soldier(crate::element::Posture::Upright);
        soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
        soldier
            .element
            .set_position_map(crate::coordinates::MapPoint::new(863.875, 702.403));
        soldier.element.set_direction_goal(direction_goal);
        Entity::Soldier(soldier)
    }

    fn unconscious_lying_soldier() -> Entity {
        let mut soldier = object_interaction_soldier(0);
        soldier
            .element_data_mut()
            .publish_order_posture(crate::element::Posture::Lying);
        soldier
            .human_data_mut()
            .expect("test soldier is human")
            .unconscious = true;
        soldier
    }

    #[test]
    fn unconscious_human_rejection_precedes_transition_order_allocation() {
        use crate::sequence::{SequenceElement, SequenceState};

        let mut engine = EngineInner::new();
        let mut soldier = unconscious_lying_soldier();
        soldier
            .actor_data_mut()
            .expect("test soldier is an actor")
            .execution_frozen = true;
        let owner = engine.add_test_entity(soldier);
        let sequence = engine.t_launch_element(
            &LevelAssets::default(),
            SequenceElement::new(1, Command::LookRight, Some(owner)),
        );

        engine.t_hourglass_phase_sequences(&LevelAssets::default());

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("rejected element remains inspectable");
        assert_eq!(element.state, SequenceState::Impossible);
        assert!(element.orders.is_empty());
        assert_eq!(
            engine.orders.allocate_order_id().get(),
            1,
            "human instruction rejects before a lying actor can allocate StandingUp"
        );
        assert!(
            !engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .execution_frozen,
            "human instruction still unfreezes the rejected recipient"
        );
    }

    #[test]
    fn postponed_wait_remains_admissible_for_unconscious_human() {
        use crate::sequence::{SequenceElement, SequenceState};

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(unconscious_lying_soldier());
        let sequence = engine.t_launch_element(
            &LevelAssets::default(),
            SequenceElement::new(1, Command::Wait, Some(owner)),
        );
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 0)
            .expect("queued wait")
            .state = SequenceState::Postponed;

        engine.t_hourglass_phase_sequences(&LevelAssets::default());

        assert!(
            !engine.human_instruct_rejects_command(owner, Command::Wait),
            "Wait is explicitly admitted by human instruction while unconscious"
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("admitted wait remains live");
        assert_eq!(element.state, SequenceState::InProgress);
        assert!(!element.orders.is_empty());
    }

    fn interaction_object(object_type: crate::element::ObjectType) -> Entity {
        let mut element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        };
        element.set_position_map(crate::coordinates::MapPoint::new(846.728, 693.890));
        let object = crate::element::ObjectData {
            object_type,
            ..Default::default()
        };
        if object_type == crate::element::ObjectType::Ale {
            element.kind = crate::element::ElementKind::ObjectOther;
            Entity::Bonus(crate::element::ElementBonus { element, object })
        } else {
            Entity::Projectile(crate::element::ElementProjectile {
                element,
                object,
                projectile: crate::element::ProjectileData::default(),
            })
        }
    }

    #[test]
    fn redundant_equip_bow_terminates_even_with_generated_transition_queued() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(shield_pc(crate::element::ActionState::AimingWithBow));
        let mut element = crate::sequence::SequenceElement::new(1, Command::EquipBow, Some(owner));
        element.orders.push_back(crate::order::Order::test_new(
            crate::order::OrderType::TransitionEquipBow,
            0.0,
            0.0,
        ));
        let seq_id = engine.orders.sequence_manager.insert_element(element);
        engine.orders.sequence_manager.start_sequence_level(seq_id);

        engine.dispatch_bow_transition(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            &mut Vec::new(),
            owner,
            Command::EquipBow,
            seq_id,
            0,
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("redundant EquipBow element survives as terminal history");
        assert_eq!(element.state, crate::sequence::SequenceState::Terminated);
        assert_eq!(
            element.orders.len(),
            1,
            "command body must not append orders"
        );
    }

    #[test]
    fn npc_take_and_drink_orders_preserve_authored_direction_goal() {
        for (command, object_type, expected_order) in [
            (
                Command::Take,
                crate::element::ObjectType::Coin,
                crate::order::OrderType::Taking,
            ),
            (
                Command::DrinkAle,
                crate::element::ObjectType::Ale,
                crate::order::OrderType::DrinkingAle,
            ),
        ] {
            let mut engine = EngineInner::new();
            let owner = engine.add_test_entity(object_interaction_soldier(13));
            let antagonist = engine.add_test_entity(interaction_object(object_type));
            let seq_id = engine.orders.sequence_manager.insert_element(
                crate::sequence::SequenceElement::new_interaction(
                    1,
                    command,
                    Some(owner),
                    Some(antagonist),
                ),
            );
            engine.orders.sequence_manager.start_sequence_level(seq_id);

            engine.dispatch_object_interaction_command(owner, command, seq_id, 0);

            assert_eq!(
                engine
                    .get_entity(owner)
                    .expect("interaction owner exists")
                    .position_iface()
                    .get_direction_goal()
                    .as_u8(),
                13,
                "{command:?} must honor Original bComputeDirection=false"
            );
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(seq_id, 0)
                    .expect("interaction remains live")
                    .orders
                    .back()
                    .expect("interaction translated to one order")
                    .order_type,
                expected_order
            );
        }

        // PC Take already preserved its direction; keep that control while
        // removing the NPC-only synthetic pre-set.
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(shield_pc(crate::element::ActionState::Waiting));
        engine.elem_mut(owner).set_direction_goal(7);
        let antagonist =
            engine.add_test_entity(interaction_object(crate::element::ObjectType::Coin));
        let seq_id = engine.orders.sequence_manager.insert_element(
            crate::sequence::SequenceElement::new_interaction(
                1,
                Command::Take,
                Some(owner),
                Some(antagonist),
            ),
        );
        engine.orders.sequence_manager.start_sequence_level(seq_id);
        engine.dispatch_object_interaction_command(owner, Command::Take, seq_id, 0);
        assert_eq!(
            engine
                .get_entity(owner)
                .expect("PC owner exists")
                .position_iface()
                .get_direction_goal()
                .as_u8(),
            7
        );
    }

    #[test]
    fn upright_dead_wait_keeps_base_upright_follow_up_after_emergency_fall() {
        use crate::element::{ActionState, ActorSoldier, Posture};
        use crate::order::OrderType;
        use crate::sequence::SequenceElement;

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
            actor: crate::element::ActorData {
                action_state: ActionState::Moving,
                ..Default::default()
            },
            npc: crate::element::NpcData {
                life_points: 0,
                ..Default::default()
            },
            soldier: crate::element::SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
            ..crate::engine::test_support::actors::unbound_soldier(Posture::Upright)
        }));
        let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
        wait.posture_after_transition = Posture::Upright;
        wait.action_state_after_transition = ActionState::Bored;
        let sequence = engine.orders.sequence_manager.insert_element(wait);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);

        engine.dispatch_wait_command(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            &mut Vec::new(),
            owner,
            Command::Wait,
            sequence,
            0,
        );

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("dead actor wait remains live")
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![
                OrderType::FallingHitHarderUpright,
                OrderType::WaitingUprightBored,
            ]
        );
    }

    #[test]
    fn upright_bored_wait_preserves_human_translator_discarded_order_id() {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::sequence::SequenceElement;

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(shield_pc(ActionState::Bored));
        let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
        wait.posture_after_transition = Posture::Upright;
        wait.action_state_after_transition = ActionState::Bored;
        let sequence = engine.orders.sequence_manager.insert_element(wait);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);

        engine.dispatch_wait_command(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            &mut Vec::new(),
            owner,
            Command::Wait,
            sequence,
            0,
        );

        let order = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("upright bored wait remains live")
            .orders
            .back()
            .expect("upright bored wait translates to a base order");
        assert_eq!(order.order_type, OrderType::WaitingUprightBored);
        assert_eq!(
            order.order_id.get(),
            2,
            "Human allocates and discards ID 1 before base Wait allocates the live order"
        );
        assert_eq!(engine.orders.allocate_order_id().get(), 3);
    }

    #[test]
    fn plain_dead_back_wait_retains_base_actor_direction_computation() {
        use crate::element::{ActionState, ActorSoldier, Posture};
        use crate::order::OrderType;
        use crate::sequence::SequenceElement;

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
            actor: crate::element::ActorData {
                action_state: ActionState::Waiting,
                ..Default::default()
            },
            npc: crate::element::NpcData {
                life_points: 0,
                ..Default::default()
            },
            soldier: crate::element::SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
            ..crate::engine::test_support::actors::unbound_soldier(Posture::DeadBack)
        }));
        let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
        wait.posture_after_transition = Posture::DeadBack;
        wait.action_state_after_transition = ActionState::Waiting;
        let sequence = engine.orders.sequence_manager.insert_element(wait);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);

        engine.dispatch_wait_command(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            &mut Vec::new(),
            owner,
            Command::Wait,
            sequence,
            0,
        );

        let order = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("dead-back wait remains live")
            .orders
            .back()
            .expect("dead-back wait translated to an order");
        assert_eq!(order.order_type, OrderType::BeingDeadFallenBack);
        assert!(
            order.compute_direction,
            "actor translation keeps the order's default direction computation enabled"
        );
    }

    #[test]
    fn live_hourglass_keeps_later_actor_work_observable() {
        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(object_interaction_soldier(0));
        let assets = LevelAssets::new();
        let mut damage = crate::sequence::SequenceElement::new_generic(
            1,
            Command::ReceiveSwordDamage,
            Some(owner),
        );
        damage.priority = crate::sequence::SequencePriority::Injury;
        let damage_sequence = engine.t_launch_element(&assets, damage);

        let mut enter =
            crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
        enter.priority = crate::sequence::SequencePriority::PostponeEverythingButInjuries;
        engine.t_launch_element(&assets, enter);
        assert!(matches!(
            engine.orders.sequence_manager.pop_next_hourglass_action(),
            Some(crate::sequence::SequenceAction::InstructOwner {
                sequence_id,
                element_index: 0,
                ..
            }) if sequence_id == damage_sequence
        ));
        assert!(
            engine
                .orders
                .sequence_manager
                .element_is_about_to_be_launched(owner, Command::EnterSwordfight)
        );
    }

    #[test]
    fn character_availability_updates_selection_before_returning() {
        use crate::sequence::{Field, FieldValue, SequenceElement};

        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_test_entity(shield_pc(crate::element::ActionState::Waiting));
        engine.players.seats[0].selection = vec![owner];
        let mut character =
            SequenceElement::new_generic(1, Command::CharacterAvailable, Some(owner));
        character.set_property(Field::CharacterAvailable, FieldValue::Bool(false));
        engine.t_launch_element(&assets, character);
        assert!(engine.players.seats[0].selection.is_empty());
        let pc = engine.get_entity(owner).and_then(Entity::pc_data).unwrap();
        assert!(!pc.playable);
        assert!(pc.interface_hidden);
    }

    #[test]
    fn making_a_rescue_hero_available_joins_the_player_party() {
        use crate::sequence::{Field, FieldValue, SequenceElement};

        let mut engine = EngineInner::new();
        let mut entity = shield_pc(crate::element::ActionState::Waiting);
        let pc = entity.pc_data_mut().expect("test hero");
        pc.playable = false;
        pc.command_interface = crate::human_control::CommandInterface::None;
        pc.mission_role = crate::human_control::MissionRole::RescueTarget;
        let owner = engine.add_test_entity(entity);
        let mut character =
            SequenceElement::new_generic(1, Command::CharacterAvailable, Some(owner));
        character.set_property(Field::CharacterAvailable, FieldValue::Bool(true));
        let sequence = engine.orders.sequence_manager.insert_element(character);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);

        engine.dispatch_availability_immediate(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            &mut Vec::new(),
            Command::CharacterAvailable,
            sequence,
            0,
        );

        let pc = engine
            .get_entity(owner)
            .and_then(Entity::pc_data)
            .expect("available rescue hero");
        assert!(pc.playable);
        assert_eq!(
            pc.command_interface,
            crate::human_control::CommandInterface::HeroActions
        );
        assert_eq!(
            pc.mission_role,
            crate::human_control::MissionRole::PlayerParty
        );
    }

    #[test]
    fn shield_context_preserves_transition_orders_and_states() {
        use crate::element::ActionState;
        use crate::order::OrderType;
        use crate::sequence::{SequenceElement, SequenceState};

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(shield_pc(ActionState::Waiting));
        let target = engine.add_test_entity(shield_pc(ActionState::Waiting));
        let assets = engine.test_runtime_assets();
        let seq_id =
            engine
                .orders
                .sequence_manager
                .insert_element(SequenceElement::new_interaction(
                    1,
                    Command::RaiseShield,
                    Some(owner),
                    Some(target),
                ));
        engine.orders.sequence_manager.start_sequence_level(seq_id);

        engine.select_sequence_element(owner, None);
        let handled = engine.t_instruct_owner(&assets, owner, seq_id, 0);

        assert!(handled);
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
        let owner_entity = engine.ent(owner);
        assert_eq!(
            owner_entity.element_data().posture(),
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
            .insert_element(SequenceElement::new(
                1,
                Command::RaiseShieldInstantly,
                Some(owner),
            ));
        engine
            .orders
            .sequence_manager
            .start_sequence_level(instant_seq);
        engine.select_sequence_element(owner, None);
        engine.t_instruct_owner(&assets, owner, instant_seq, 0);
        let instant = engine
            .orders
            .sequence_manager
            .get_element(instant_seq, 0)
            .expect("instant raise-shield element remains inspectable");
        assert_eq!(instant.state, SequenceState::InProgress);
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
            .insert_element(SequenceElement::new(1, Command::LowerShield, Some(owner)));
        engine
            .orders
            .sequence_manager
            .start_sequence_level(lower_seq);
        engine.set_action_state_of(owner, ActionState::HoldingShield);
        engine.select_sequence_element(owner, None);
        engine.t_instruct_owner(&assets, owner, lower_seq, 0);
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
            .insert_element(SequenceElement::new(1, Command::ParryShield, Some(owner)));
        engine
            .orders
            .sequence_manager
            .start_sequence_level(parry_seq);
        engine.select_sequence_element(owner, None);
        engine.t_instruct_owner(&assets, owner, parry_seq, 0);
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
            ActionState::HoldingShield
        );
    }

    #[test]
    fn lower_shield_survives_a_completed_stand_up_transition() {
        let assets = LevelAssets::new();
        use crate::element::ActionState;
        use crate::order::{Order, OrderType};
        use crate::sequence::{SequenceElement, SequenceState};

        let mut engine = EngineInner::new();
        // Captured Save035 r001 boundary: ReceiveHitDamage finishes its
        // FallingHitUpright action, then LowerShield resumes after StandingUp
        // has restored the actor to Waiting. Original still translates the
        // authored lowering animation at that point.
        let owner = engine.add_test_entity(shield_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .insert_element(SequenceElement::new(1, Command::LowerShield, Some(owner)));
        engine.orders.sequence_manager.start_sequence_level(seq_id);
        let stand_up = Order::new(
            OrderType::StandingUp,
            0.0,
            0.0,
            engine.orders.allocate_order_id(),
        );
        engine
            .orders
            .sequence_manager
            .push_order_on(seq_id, 0, stand_up);
        engine
            .orders
            .sequence_manager
            .get_element_mut(seq_id, 0)
            .expect("lower-shield element exists")
            .initialize_transition_orders();

        engine.select_sequence_element(owner, None);
        engine.t_instruct_owner(&assets, owner, seq_id, 0);

        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("lower-shield element remains live");
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![OrderType::StandingUp, OrderType::LoweringShield]
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .expect("lower-shield owner exists")
                .actor_data()
                .expect("lower-shield owner has actor data")
                .action_state,
            ActionState::Waiting,
            "translation must not invent a shield action state"
        );
    }

    #[test]
    fn stand_up_and_recover_install_without_changing_lying_posture() {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType;
        use crate::sequence::{SequenceElement, SequenceState};

        for command in [Command::StandUp, Command::Recover] {
            let mut engine = EngineInner::new();
            let owner = engine.add_test_entity(shield_pc(ActionState::Waiting));
            engine.ent_mut(owner).set_posture(Posture::Lying);
            let mut recovery = SequenceElement::new(1, command, Some(owner));
            recovery.priority = crate::sequence::SequencePriority::Normal;
            let seq_id = engine.orders.sequence_manager.insert_element(recovery);
            engine.orders.sequence_manager.start_sequence_level(seq_id);

            engine.select_sequence_element(owner, None);
            engine.t_instruct_owner(&LevelAssets::default(), owner, seq_id, 0);

            let element = engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("recovery element remains live");
            assert_eq!(element.state, SequenceState::InProgress);
            assert_eq!(
                element.orders.front().map(|order| order.order_type),
                Some(OrderType::StandingUp)
            );
            assert_eq!(
                engine
                    .get_entity(owner)
                    .expect("recovery owner exists")
                    .element_data()
                    .posture(),
                Posture::Lying,
                "{command:?} translation must wait for StandingUp MotionState::Start"
            );
        }
    }

    #[test]
    fn raise_shield_from_sword_state_preserves_lowering_transition() {
        use crate::element::ActionState;
        use crate::order::OrderType;
        use crate::sequence::{SequenceElement, SequenceState};

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(shield_pc(ActionState::WaitingSword));
        let assets = engine.test_runtime_assets();
        let seq_id = engine
            .orders
            .sequence_manager
            .insert_element(SequenceElement::new(1, Command::RaiseShield, Some(owner)));
        engine.orders.sequence_manager.start_sequence_level(seq_id);

        engine.select_sequence_element(owner, None);
        let handled = engine.t_instruct_owner(&assets, owner, seq_id, 0);

        assert!(handled);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("raise-shield element remains live");
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![OrderType::TransitionLoweringSword, OrderType::RaisingShield]
        );
    }

    #[test]
    fn shield_refresh_seek_joins_the_current_hourglass_drain() {
        let assets = LevelAssets::new();
        use crate::element::ActionState;
        use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement};

        let mut engine = EngineInner::new();
        let owner = engine.add_test_entity(shield_pc(ActionState::HoldingShield));
        let protected = engine.add_test_entity(shield_pc(ActionState::Waiting));
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
        let raise_seq = engine.t_launch_element(&assets, raise);
        assert!(matches!(
            engine.orders.sequence_manager.pop_next_hourglass_action(),
            Some(crate::sequence::SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 0,
            }) if action_owner == owner && sequence_id == raise_seq
        ));

        let follow_up = engine
            .dispatch_shield_command(
                &crate::sim_rng::test_context(),
                &assets,
                &mut Vec::new(),
                owner,
                Command::RaiseShield,
                raise_seq,
                0,
            )
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
        let follow_up_seq = engine.t_launch_element(&assets, follow_up);

        // The original game's manager tick is a live while-loop. A normal
        // Move/Seek registered from the current instruction callback therefore
        // joins this same drain rather than waiting for the next frame.
        assert!(matches!(
            engine.orders.sequence_manager.pop_next_hourglass_action(),
            Some(crate::sequence::SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 0,
            }) if action_owner == owner && sequence_id == follow_up_seq
        ));
        assert!(
            engine
                .orders
                .sequence_manager
                .pop_next_hourglass_action()
                .is_none()
        );

        let owner_pc = engine.pc(owner);
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
        required_canonical_door(
            &[],
            crate::gate::DoorIndex::new(4).expect("valid door index"),
            "UnlockDoor dispatch",
        );
    }

    #[test]
    #[should_panic(expected = "UnlockDoor completion references missing canonical door 9")]
    fn required_mutable_door_lookup_rejects_stale_completion_target() {
        required_canonical_door_mut(
            &mut [],
            crate::gate::DoorIndex::new(9).expect("valid door index"),
            "UnlockDoor completion",
        );
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

/// The single emission point of command-translation orders. Every translation
/// context owns one over the engine's deterministic order-id counter, so each
/// order allocates its id at the translator's exact emission point.
pub(in crate::engine) struct OrderEmitter<'a> {
    next_order_id: &'a mut u32,
}

impl<'a> OrderEmitter<'a> {
    pub(in crate::engine) fn new(next_order_id: &'a mut u32) -> Self {
        Self { next_order_id }
    }

    /// Allocate the next order id.
    pub(in crate::engine) fn alloc_id(&mut self) -> std::num::NonZeroU32 {
        crate::order::alloc_order_id(self.next_order_id)
    }

    /// Allocate one order and append it to `(seq_id, elem_idx)`. Direction
    /// policy is explicit: recovery orders keep `Order::new`'s ordinary `true`,
    /// whereas posture-local translators disable recomputation.
    pub(in crate::engine) fn push(
        &mut self,
        sequence_manager: &mut crate::sequence::SequenceManager,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
        target: (f32, f32),
        compute_direction: bool,
    ) {
        let id = self.alloc_id();
        let mut order = crate::order::Order::new(order_type, target.0, target.1, id);
        order.compute_direction = compute_direction;
        sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
}
