//! Direct actor update, including execution, completion, and the derived tail.

use super::*;
use crate::engine::TickCtx;

impl EngineInner {
    pub(crate) fn tick_one_actor_animation_action_change_slot(
        &mut self,
        tcx: TickCtx<'_>,
        entity_id: EntityId,
    ) {
        use crate::sprite::MotionState;
        self.debug_patrol_turn_lifecycle("actor_slot_before_prelude", entity_id);
        self.tick_actor_prelude(tcx, entity_id);
        self.debug_patrol_turn_lifecycle("actor_slot_after_prelude", entity_id);
        // Derived updates finish their callbacks before entering the base
        // actor update. Only abortion retains this entry selection.
        let aborted_element = self.world.entities.current_element_for_actor(entity_id);
        self.apply_delayed_actor_position(tcx, entity_id);
        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::BaseActor(entity_id));

        let frozen_without_order = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
            && self
                .orders
                .sequence_manager
                .current_order_for_actor(&self.world.entities, entity_id)
                .is_none();
        if frozen_without_order {
            // The actor update refreshes the order after applying
            // the delayed position. With no selected order it
            // clears the pointer, then an execution freeze returns
            // before lazy Wait and the second movement snapshot.
            self.install_actor_order(entity_id, None);
            self.debug_refresh_view_lifecycle(
                "derived_tail_frozen_without_order",
                entity_id,
                Some(crate::order::OrderType::NonanimationEnd),
            );
            self.tick_actor_derived_tail(tcx, entity_id, crate::order::OrderType::NonanimationEnd);
            return;
        }

        // The engine tick updates every element regardless of
        // whether it is active. The actor update
        // then installs Wait whenever its order is empty. Active
        // controls world presence/rendering, not sequence time.
        self.ensure_wait_element(tcx, entity_id);
        // Sequence launch through element dispatch to instruction is
        // synchronous. A command registered for later manager or
        // deferred processing cannot suppress this transient
        // Execute: Wait may publish its START sprite row before
        // that later command interrupts it in the same frame.
        observe_actor_animation_boundary(ActorAnimationBoundaryPhase::WaitReady(entity_id));

        // The actor update starts a move after lazy Wait
        // installation and immediately before it samples the
        // current order ID and enters Execute. The delayed-position
        // branches begin an earlier movement step for their crossing
        // segment, then reach this second snapshot as well. Keep
        // PositionInterface's old-position latch frame-local;
        // movement and combat helpers query movement status later in
        // this same owner slot.
        self.world
            .entities
            .get_mut(entity_id)
            .expect("actor disappeared before new movement update")
            .position_iface_mut()
            .new_move();

        // Latch only the order facts that survive callbacks during execution.
        let entry = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id);
        let selected_order =
            entry.map(|(seq_id, elem_idx, order)| (seq_id, elem_idx, order.order_id));
        let selected_order_type = entry.map(|(_, _, order)| order.order_type);
        let selected_command = entry.map(|(sequence, element, _)| {
            self.orders
                .sequence_manager
                .get_element(sequence, element)
                .expect("selected order lost its element")
                .command
        });
        let selected_owner_family = selected_order_type
            .and_then(|order_type| classify_live_actor_execute_arm(entity_id, order_type));
        let installed_at_entry = entry.map(|(sequence_id, element_index, order)| {
            crate::element::InstalledActorOrder::new(
                crate::sequence::SequenceElementRef::new(sequence_id, element_index),
                order,
            )
        });
        self.install_actor_order(entity_id, installed_at_entry);
        {
            let actor = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("actor disappeared before installing its update order");
            if let Some((_, _, order_id)) = selected_order {
                actor.select_execute_order(order_id);
            }
        }
        self.debug_drop_owner_boundary("execute_latch_published", entity_id, selected_order);
        // Player action execution handles the carrying-corpse exit for an
        // ENTER_SWORDFIGHT before the default validity arm: on
        // the transition's first Execute it drops immediately
        // and returns TERMINATED. Translation still has to register the
        // transition, so key this to the entry-latched order
        // rather than consuming it during transition generation.
        let enter_swordfight_corpse_exit = selected_order_type
            == Some(crate::order::OrderType::TransitionCarryingCorpseWaitingUpright)
            && self.world.entities.get(entity_id).is_some_and(|entity| {
                entity.is_pc()
                    && entity.actor_data().is_some_and(|actor| {
                        actor.execute_order_initialising && !actor.execution_frozen
                    })
            })
            && selected_order.is_some_and(|(seq_id, elem_idx, _)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .is_some_and(|element| {
                        element.command == crate::element::Command::EnterSwordfight
                    })
            });
        // Human/PC validity belongs to the live Execute entry,
        // after the actor update has established the new-order flag for
        // this exact selected order. Earlier actor callbacks may
        // replace the selected order in the same owner walk, so
        // sampling in a global pre-pass would validate stale work.
        let validity_motion = (!enter_swordfight_corpse_exit)
            .then(|| self.pre_tick_human_execute_validity_for(tcx, entity_id))
            .flatten();
        let validity_short_circuited = validity_motion.is_some();
        if !validity_short_circuited
            && !enter_swordfight_corpse_exit
            && selected_order_type
                == Some(crate::order::OrderType::TransitionCarryingCorpseWaitingUpright)
            && self
                .world
                .entities
                .get(entity_id)
                .and_then(Entity::actor_data)
                .is_some_and(|actor| actor.execute_order_initialising)
        {
            // Player-character execution owns this initialization,
            // not the DROP_CORPSE command builder. Posture
            // transitions inserted for another PC command enter
            // the same animation arm. Align the carried actor after validity
            // and before starting the action.
            let (carried, carried_direction) = {
                let carrier = self.world.entities.get(entity_id).unwrap_or_else(|| {
                    panic!(
                        "corpse-exit transition owner {entity_id:?} vanished before initialization"
                    )
                });
                let carried = carrier
                    .pc_data()
                    .unwrap_or_else(|| {
                        panic!("corpse-exit transition owner {entity_id:?} is not a PC")
                    })
                    .carried
                    .unwrap_or_else(|| {
                        panic!("corpse-exit transition owner {entity_id:?} has no carried body")
                    });
                (
                    carried,
                    carrier.element_data().direction().wrapping_sub(4) & 15,
                )
            };
            self.world
                .entities
                .get_mut(carried)
                .unwrap_or_else(|| {
                    panic!(
                        "corpse-exit transition target {carried:?} vanished before initialization"
                    )
                })
                .element_data_mut()
                .set_direction_instantly(carried_direction);
        }
        let movement_selection = (!validity_short_circuited)
            .then_some(selected_order)
            .flatten()
            .and_then(|(seq_id, elem_idx, order_id)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .filter(|element| {
                        selected_owner_family == Some(ExecuteOwnerFamily::Movement)
                            && element.data.is_movement()
                            && !matches!(
                                element.command,
                                crate::element::Command::WaitTimer
                                    | crate::element::Command::WaitFreeLift
                            )
                    })
                    .map(|_| super::movement::MovementOwnerSelection {
                        seq_id,
                        elem_idx,
                        order_id,
                    })
            });
        let melee_selection = (!validity_short_circuited)
            .then_some(selected_order)
            .flatten()
            .and_then(|(seq_id, elem_idx, order_id)| {
                let order_type = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| element.current_order())
                    .map(|order| order.order_type)?;
                (selected_owner_family == Some(ExecuteOwnerFamily::Melee)
                    && MELEE_ORDERS.contains(&order_type))
                .then_some(MeleeOwnerSelection {
                    seq_id,
                    elem_idx,
                    order_id,
                })
            });
        // Bow belongs to the same entry-latched Execute choice as
        // movement and melee. If its terminal callback exposes a
        // successor order, that successor must wait until the
        // actor's next update rather than entering generic
        // Execute later in this same slot.
        let bow_selection = (!validity_short_circuited
            && selected_owner_family == Some(ExecuteOwnerFamily::Bow))
        .then(|| self.selected_bow_order(entity_id))
        .flatten();
        let ability_selection = selected_order.filter(|(seq, elem, order_id)| {
            !validity_short_circuited
                && !enter_swordfight_corpse_exit
                && selected_owner_family == Some(ExecuteOwnerFamily::Ability)
                && crate::abilities::selected_ability(
                    &self.world.entities,
                    &self.orders.sequence_manager,
                    entity_id,
                )
                .is_some_and(|ability| {
                    ability.sequence_id == *seq
                        && ability.element_index == *elem
                        && ability.order_id == *order_id
                })
        });
        let beggar_selection = selected_order.and_then(|(seq, elem, order_id)| {
            if validity_short_circuited || selected_owner_family != Some(ExecuteOwnerFamily::Beggar)
            {
                return None;
            }
            self.orders
                .sequence_manager
                .get_element(seq, elem)
                .and_then(|element| element.current_order())
                .and_then(|order| {
                    (order.order_id == order_id
                        && order.order_type == crate::order::OrderType::SimulatingBeggar)
                        .then_some(order_id)
                })
        });
        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::MovementExecute(entity_id));
        self.debug_drop_owner_boundary("tick_ability_entry", entity_id, selected_order);
        if let Some(entity) = self.world.entities.get(entity_id) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                entity_id,
                self.control.frame_counter,
                "owner_execute_entry",
            );
        }
        observe_actor_animation_boundary(ActorAnimationBoundaryPhase::GenericExecute(entity_id));
        let execution_frozen = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen);
        let mut motion = 'execute: {
            let _detail = entity_system_detail_guard(EntitySystemDetail::OwnerExecute);
            if execution_frozen {
                break 'execute MotionState::InProgress;
            }
            if let Some(motion) = validity_motion {
                break 'execute motion;
            }
            if let Some(selection) = movement_selection {
                if self.abort_orphaned_sword_movement(tcx, entity_id, selection) {
                    break 'execute MotionState::Aborted;
                }
                if super::refresh_seek::perform_seek_lost_actor_target(self, entity_id, selection) {
                    break 'execute MotionState::Terminated;
                }
                if let Some(motion) = self.tick_refreshing_seek_for_owner(tcx, entity_id) {
                    break 'execute motion;
                }
                if self.selected_seek_refresh_decision(entity_id).is_some() {
                    self.apply_pre_perform_seek_facing_prologue(entity_id);
                }
                if self.tick_refresh_seek_for_owner(tcx, entity_id) {
                    break 'execute MotionState::InProgress;
                }
                break 'execute self
                    .tick_entity_movement_owner(tcx, entity_id, Some(selection))
                    .expect("selected movement owner did not execute");
            }
            if let Some(selection) = melee_selection {
                break 'execute self
                    .tick_selected_melee_owner(tcx, entity_id, selection)
                    .expect("selected melee owner did not execute");
            }
            if let Some((_, _, order_id)) = bow_selection {
                break 'execute self
                    .tick_bow_shot_for(tcx, entity_id, order_id)
                    .expect("selected bow owner did not execute");
            }
            if ability_selection.is_some() {
                let listen = crate::abilities::selected_ability(
                    &self.world.entities,
                    &self.orders.sequence_manager,
                    entity_id,
                )
                .is_some_and(|ability| ability.kind == crate::movement::AbilityKind::Listen);
                if listen {
                    if let Some(motion) =
                        self.tick_enemy_ai_blip_detection_for_owner(tcx, entity_id)
                    {
                        break 'execute motion;
                    }
                }
                break 'execute self
                    .tick_selected_ability(tcx, entity_id, self.actors_frozen())
                    .expect("selected ability owner did not execute");
            }
            if let Some(order_id) = beggar_selection {
                self.tick_beggar_bid_for(tcx, entity_id, order_id);
                break 'execute MotionState::InProgress;
            }
            if enter_swordfight_corpse_exit {
                self.force_drop_carried_corpse_instant(tcx, entity_id);
                break 'execute MotionState::Terminated;
            }
            let result = if selected_order_type == Some(crate::order::OrderType::Rolling) {
                self.tick_rolling_owner(tcx, entity_id)
            } else {
                self.tick_actor_animation_for(tcx, entity_id)
            };
            result.unwrap_or(MotionState::InProgress)
        };
        if let Some(entity) = self.world.entities.get(entity_id) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                entity_id,
                self.control.frame_counter,
                "owner_post_execute",
            );
        }
        if !validity_short_circuited
            && !execution_frozen
            && self
                .world
                .entities
                .get(entity_id)
                .is_some_and(Entity::is_pc)
        {
            self.tick_pc_combat_anim_speech_for_owner(
                tcx,
                entity_id,
                selected_order_type,
                selected_command,
                motion,
            );
        }
        if waiting_sword_execute_reaches_evaluation(
            selected_order_type,
            validity_short_circuited,
            execution_frozen,
        ) {
            self.tick_waiting_sword_execute_for(tcx, entity_id);
        }
        if let Some(order_type) = selected_order_type {
            self.tick_parry_counter_for_execute(tcx, entity_id, order_type, &mut motion);
        }
        {
            let actor = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("Execute owner disappeared before storing its result");
            actor.continuation.motion_state = motion;
            actor.sequence_element_started = false;
        }
        self.apply_actor_post_execute_wait_modifier_to_motion(entity_id, &mut motion);
        {
            let actor = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("Execute owner disappeared before line crossing");
            actor.continuation.motion_state = motion;
            actor.execute_order_initialising = false;
        }
        self.debug_drop_owner_boundary("execute_latch_cleared", entity_id, selected_order);
        self.dispatch_actor_post_execute_line_crossing(tcx, entity_id);
        // Crossing callbacks can synchronously instruct a replacement and
        // publish its motion state before completion inspects this actor.
        let completion_motion = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .expect("actor disappeared during line crossing")
            .continuation
            .motion_state;
        self.finish_actor_execute_completion(tcx, entity_id, aborted_element, completion_motion);

        // Order advancement may synchronously expose a real postponed
        // successor through state changes and readiness. If it does not,
        // the actor order stays empty for the rest of this
        // actor update. The fallback Wait is created
        // only by the null-order guard at the start of the next
        // actor frame, so ActionChange observes NONANIMATION_END
        // on this completion frame.
        observe_actor_animation_boundary(ActorAnimationBoundaryPhase::CompletionEffects(entity_id));

        // Release every animation/completion borrow before the VM:
        // ActionChange can synchronously replace this or a later
        // actor's order and the next slot must sample that live.
        observe_actor_animation_boundary(ActorAnimationBoundaryPhase::ActionChange(entity_id));
        self.dispatch_actor_action_change_for(tcx, entity_id);
        // Do not derive the actor order from the manager at the tail. The
        // exact identity was published at update entry and is
        // subsequently changed only by order advancement, selected
        // element cleanup, or a synchronous accepted instruction.
        let installed_tail_order_type = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| actor.installed_order)
            .map(|handle| handle.resolve(&self.orders.sequence_manager).order_type)
            .unwrap_or(crate::order::OrderType::NonanimationEnd);
        self.debug_refresh_view_lifecycle(
            "derived_tail_normal",
            entity_id,
            Some(installed_tail_order_type),
        );
        self.tick_actor_derived_tail(tcx, entity_id, installed_tail_order_type);
        if let Some(entity) = self.world.entities.get(entity_id) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                entity_id,
                self.control.frame_counter,
                "owner_tail_after_derived",
            );
        }

        // Human posture changes update intersecting-corpse state
        // synchronously. Close the owner-local
        // boundary before the next creation slot samples this
        // actor for anti-collision.
        self.process_corpse_intersection_update_for(entity_id);
    }
}
