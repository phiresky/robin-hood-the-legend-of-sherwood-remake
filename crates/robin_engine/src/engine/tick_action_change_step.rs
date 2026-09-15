//! Direct actor update, including execution, completion, and the derived tail.

use super::*;

impl EngineInner {
    pub(super) fn tick_one_actor_animation_action_change_slot<ExecuteMotion>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        before_actor: &mut impl FnMut(&mut Self, EntityId),
        execute_owner_arm: &mut impl FnMut(
            &mut Self,
            EntityId,
            Option<super::movement::MovementOwnerSelection>,
            Option<MeleeOwnerSelection>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)>,
            Option<std::num::NonZeroU32>,
        ) -> ExecuteMotion,
        after_slot: &mut impl FnMut(&mut Self, EntityId, crate::order::OrderType),
    ) where
        ExecuteMotion: IntoExplicitExecuteMotion,
    {
        // The actor update consumes one queued base
        // position update before it inspects the current
        // sequence/order.
        self.apply_delayed_actor_position(sim, assets, entity_id);
        self.debug_patrol_turn_lifecycle("actor_slot_before_prelude", entity_id);
        before_actor(self, entity_id);
        self.debug_patrol_turn_lifecycle("actor_slot_after_prelude", entity_id);
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
            self.world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("frozen actor disappeared before mpOrder clear")
                .installed_order = None;
            self.debug_refresh_view_lifecycle(
                "derived_tail_frozen_without_order",
                entity_id,
                Some(crate::order::OrderType::NonanimationEnd),
            );
            after_slot(self, entity_id, crate::order::OrderType::NonanimationEnd);
            return;
        }

        // The engine tick updates every element regardless of
        // whether it is active. The actor update
        // then installs Wait whenever its order is empty. Active
        // controls world presence/rendering, not sequence time.
        self.ensure_wait_element(sim, assets, entity_id);
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
        let selected_order_compute_direction = entry.map(|(_, _, order)| order.compute_direction);
        let selected_owner_family = selected_order_type
            .and_then(|order_type| classify_live_actor_execute_arm(entity_id, order_type));
        let installed_at_entry = entry.map(|(_, _, order)| crate::element::InstalledActorOrder {
            order_id: order.order_id,
            order_type: order.order_type,
        });
        {
            let actor = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("actor disappeared before installing its update order");
            actor.installed_order = installed_at_entry;
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
        let validity_short_circuited = !enter_swordfight_corpse_exit
            && self.pre_tick_human_execute_validity_for(sim, assets, entity_id);
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
        let movement_entity_target_seek = movement_selection.is_some_and(|selection| {
            self.orders
                .sequence_manager
                .get_element(selection.seq_id, selection.elem_idx)
                .is_some_and(|element| {
                    let crate::sequence::SequenceElementData::Movement {
                        element: target,
                        flags,
                        ..
                    } = &element.data
                    else {
                        return false;
                    };
                    // The seek wrapper is chosen per animation
                    // arm, not per element: wall and ladder
                    // orders keep the SEEK flag while their
                    // Execute arms drive the sprite directly
                    // and hand the raw START edge back.
                    flags.contains(crate::sequence::MoveFlags::SEEK)
                        && target.is_some()
                        && element.current_order().is_some_and(|order| {
                            super::movement::perform_seek_calls_per_execute(order.order_type) > 0
                        })
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
        let explicit_execute = execute_owner_arm(
            self,
            entity_id,
            movement_selection,
            melee_selection,
            bow_selection,
            ability_selection,
            beggar_selection,
        )
        .into_explicit_execute_motion();
        let explicit_execute_motion = explicit_execute.initial;
        let post_completion_execute_override = explicit_execute.post_completion_override;
        if let Some(entity) = self.world.entities.get(entity_id) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                entity_id,
                self.control.frame_counter,
                "owner_post_execute",
            );
        }
        let mut specialized_execute_motion = explicit_execute_motion.or_else(|| {
            (!validity_short_circuited)
                .then_some(selected_owner_family)
                .flatten()
                .filter(|family| specialized_execute_uses_sprite_motion(*family))
                .and_then(|_| {
                    specialized_execute_motion(
                        self.world
                            .entities
                            .get(entity_id)
                            .and_then(|entity| entity.element_data().sprite.last_motion_state),
                        beggar_selection.is_some(),
                        movement_entity_target_seek,
                    )
                })
        });
        let mut specialized_wait_modifier_terminated = false;
        if let (Some(motion), Some((entry_seq_id, entry_elem_idx, _))) =
            (specialized_execute_motion.as_mut(), selected_order)
        {
            // The actor update applies WAIT_TIMER / WAIT_FREE_LIFT
            // after complete action execution. Specialized
            // movement, combat, ability, and beggar arms therefore
            // pass through the same base modifier as generic sprite
            // animation, exactly once.
            let motion_before_modifier = *motion;
            self.apply_actor_post_execute_wait_modifier_to_motion(
                entity_id,
                entry_seq_id,
                entry_elem_idx,
                motion,
            );
            specialized_wait_modifier_terminated = motion_before_modifier
                != crate::sprite::MotionState::Terminated
                && *motion == crate::sprite::MotionState::Terminated;
        }
        let explicit_execute_in_progress = matches!(
            explicit_execute_motion,
            Some(crate::sprite::MotionState::InProgress)
        );
        let explicit_execute_terminated = matches!(
            explicit_execute_motion,
            Some(crate::sprite::MotionState::Terminated)
        );
        if let Some(motion) = specialized_execute_motion {
            // Movement/combat/ability owners are derived Execute
            // arms just like the generic animation switch below.
            // Their sprite result is therefore the initial value
            // assigned to the actor's motion state before the update
            // applies completion and order-advancement handling.
            self.world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("specialized Execute owner disappeared before motion-state latch")
                .continuation
                .motion_state = motion;
        }
        if !validity_short_circuited
        && explicit_execute_motion.is_none()
        // Generic animation owns its completion through
        // `tick_actor_animation_for` below.  In
        // particular TURNING deliberately ignores the
        // visual sprite's Done edge while `Turn()` still
        // reports that the body rotated this frame.  A
        // stale Done retained by the looping alerted-turn
        // sprite must therefore not complete the order
        // ahead of that authoritative Execute result.
        && selected_owner_family
            .is_some_and(specialized_execute_uses_sprite_motion)
        && self.world.entities.get(entity_id).is_some_and(|entity| {
            entity.element_data().sprite.last_motion_state
                == Some(crate::sprite::MotionState::Done)
        }) {
            let (entry_seq_id, entry_elem_idx, entry_order_id) =
        selected_order.unwrap_or_else(|| {
            panic!(
                "specialized actor owner {entity_id:?} recorded Done without an entry-latched order"
            )
        });
            self.mark_entry_order_done(entity_id, entry_seq_id, entry_elem_idx, entry_order_id);
        }
        observe_actor_animation_boundary(ActorAnimationBoundaryPhase::GenericExecute(entity_id));
        let mut execute_result = if validity_short_circuited
            || movement_selection.is_some()
            || melee_selection.is_some()
            || bow_selection.is_some()
            || ability_selection.is_some()
            || beggar_selection.is_some()
        {
            None
        } else if enter_swordfight_corpse_exit {
            let (seq_id, elem_idx, _) = selected_order.unwrap_or_else(|| {
                panic!("ENTER_SWORDFIGHT corpse-exit Execute lost its entry order")
            });
            self.world
                .entities
                .get(entity_id)
                .and_then(Entity::pc_data)
                .and_then(|pc| pc.carried)
                .unwrap_or_else(|| {
                    panic!(
                        "ENTER_SWORDFIGHT corpse-exit Execute owner {entity_id:?} has no carried body"
                    )
                });
            self.force_drop_carried_corpse_instant(sim, assets, entity_id);
            Some(super::animation::ActorExecuteResult {
                order_type: crate::order::OrderType::TransitionCarryingCorpseWaitingUpright,
                entry_seq_id: seq_id,
                entry_elem_idx: elem_idx,
                motion: crate::sprite::MotionState::Terminated,
            })
        } else if selected_order_type == Some(crate::order::OrderType::Rolling) {
            self.tick_rolling_owner(sim, assets, entity_id)
        } else {
            self.tick_actor_animation_for(sim, assets, entity_id)
        };
        // Falling-hit/pushed/lift flight is part of this
        // actor's selected Execute arm. Advance it
        // before the derived NPC tail so later creation slots
        // observe the committed flight position.
        let flight_motion = self.tick_push_flight_for_owner(sim, assets, entity_id);
        if let (Some(result), Some(motion)) = (execute_result.as_mut(), flight_motion) {
            // FallingLadderWall returns Terminated directly
            // from Execute when its countdown reaches zero.
            // The split flight tail owns that terminal edge,
            // so replace the earlier sprite Start result before
            // the actor update latches it.
            result.motion = motion;
        }
        if execute_result
            .as_ref()
            .is_some_and(|result| result.motion == crate::sprite::MotionState::Start)
            && self
                .world
                .entities
                .get(entity_id)
                .is_some_and(Entity::is_pc)
        {
            // Player-character execution owns eventual strike /
            // execution remarks. Their 50% RNG draw and speech
            // side effects occur synchronously before the next
            // element's update slot.
            self.tick_pc_combat_anim_speech_for_owner(sim, assets, entity_id);
        }
        // Clear the sequence-started flag immediately
        // after Execute returns. It means "the selected element
        // has not had its first owner slot yet", not "this
        // element has ever started". In particular, a Move issued
        // while an already-running non-interruptable PassDoor is
        // postponed; only a PassDoor newly installed since the
        // actor's last slot rejects that Move as impossible.
        if let Some(actor) = self
            .world
            .entities
            .get_mut(entity_id)
            .and_then(Entity::actor_data_mut)
        {
            actor.sequence_element_started = false;
        }
        // Human-actor execution performs this work inside
        // the sword-waiting arm, after action processing and before
        // returning its motion result to the actor update. Keep
        // launches and cross-actor mutations live so later slots
        // observe them and earlier slots do not.
        // This is part of human action execution's selected
        // WAITING_SWORD arm, not an animation-completion
        // callback.  In particular, the arm still runs when
        // the generic sprite helper has no completion record
        // for this slot. Key it to the actor update's
        // entry-latched order while keeping
        // the two Execute entry exits above intact.
        let execution_frozen = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen);
        if waiting_sword_execute_reaches_evaluation(
            selected_order_type,
            validity_short_circuited,
            execution_frozen,
        ) {
            self.tick_waiting_sword_execute_for(sim, assets, entity_id);
        }

        // Human-actor execution decrements the parry hold
        // counter and queues a parry stop before this actor yields
        // its legacy slot. Preserve that ordering relative to
        // sword hits performed by later-created actors.
        if let Some(result) = execute_result.as_mut() {
            self.tick_parry_counter_for_execute(sim, assets, entity_id, result);
        }

        // The actor update modifies the just-produced
        // Execute result for WAIT_TIMER / WAIT_FREE_LIFT before
        // completion or order advancement. Sampling the current element
        // here is intentional: WaitingSword callbacks above may
        // have synchronously replaced it.
        if let Some(result) = execute_result.as_mut() {
            self.apply_actor_post_execute_wait_modifier(entity_id, result);
        }
        // The base actor update calls line-crossing detection
        // after the complete execution chain and its wait
        // modifier, but before interpreting the motion result.
        // Movement owners and Rolling close this boundary in
        // their specialized executors; generic animation
        // (including death-place selection and flight) reaches it here.
        if selected_owner_family != Some(ExecuteOwnerFamily::Movement)
            && selected_order_type != Some(crate::order::OrderType::Rolling)
        {
            self.dispatch_actor_post_execute_line_crossing(
                sim,
                assets,
                entity_id,
                selected_order_compute_direction,
            );
        }
        if specialized_wait_modifier_terminated {
            let (entry_seq_id, entry_elem_idx, entry_order_id) =
                selected_order.expect("specialized wait modifier lost its entry order");
            self.finish_actor_execute_completion(
                sim,
                assets,
                entity_id,
                Some(entry_order_id),
                super::animation::ActorExecuteResult {
                    order_type: selected_order_type
                        .expect("specialized wait modifier lost its entry order type"),
                    entry_seq_id,
                    entry_elem_idx,
                    motion: crate::sprite::MotionState::Terminated,
                },
            );
        }
        if explicit_execute_terminated {
            let (seq_id, elem_idx, _) = selected_order.unwrap_or_else(|| {
            panic!(
                "actor {entity_id:?} returned explicit Terminated without an entry-latched order"
            )
        });
            self.execute_seq_advance(sim, assets, (seq_id, elem_idx));
        }
        if let Some(result) = execute_result.take() {
            // The actor update stores every execution
            // return in serialized `mmotionState` before it
            // handles Done/Terminated/Aborted. Keeping only the
            // transient Sprite result leaves the save-loaded
            // value frozen forever and makes the very first
            // post-load frame diverge whenever an animation
            // crosses a motion boundary.
            self.world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::actor_data_mut)
                .expect("Execute owner disappeared before motion-state latch")
                .continuation
                .motion_state = result.motion;
            self.finish_actor_execute_completion(
                sim,
                assets,
                entity_id,
                selected_order.map(|(_, _, order_id)| order_id),
                result,
            );
        }
        // The soldier update runs AI before returning
        // Terminated to the base actor update. Only after that
        // synchronous decision tick finishes may order advancement/completion
        // promote the actor's successor order.

        let selected_element_state = selected_order.and_then(|(entry_seq, entry_idx, _)| {
            self.orders
                .sequence_manager
                .get_element(entry_seq, entry_idx)
                .map(|element| element.state)
        });
        let selected_element_retired = selected_order.is_some()
            && selected_element_state.is_none_or(|state| {
                !matches!(
                    state,
                    crate::sequence::SequenceState::Todo
                        | crate::sequence::SequenceState::InProgress
                        | crate::sequence::SequenceState::Postponed
                )
            });
        let selected_element_interrupted =
            selected_element_state == Some(crate::sequence::SequenceState::Interrupted);
        let selected_element_impossible =
            selected_element_state == Some(crate::sequence::SequenceState::Impossible);
        let live_entry_element_order = selected_order.and_then(|(entry_seq, entry_idx, _)| {
            self.orders
                .sequence_manager
                .current_order_for_actor(&self.world.entities, entity_id)
                .filter(|(live_seq, live_idx, _)| *live_seq == entry_seq && *live_idx == entry_idx)
                .map(|(_, _, order)| order)
        });
        let selected_order_rewritten_by_stop = specialized_execute_motion
            .zip(selected_order_type)
            .is_some_and(|(motion, entry_order_type)| {
                selected_order.is_some_and(|(_, _, entry_order_id)| {
                    live_entry_element_order.is_some_and(|live_order| {
                        live_order.order_id != entry_order_id
                            && is_start_stop_movement_rewrite(
                                entry_order_id,
                                entry_order_type,
                                live_order.order_id,
                                live_order.order_type,
                                motion,
                            )
                    })
                })
            });
        let selected_entry_order_still_current =
            selected_order.is_some_and(|(_, _, entry_order)| {
                live_entry_element_order.is_some_and(|order| order.order_id == entry_order)
            });
        let selected_specialized_order_advanced = !explicit_execute_in_progress
            && specialized_order_advanced_after_execute(
                specialized_execute_motion,
                selected_order_rewritten_by_stop,
                selected_element_retired,
                selected_element_interrupted,
                selected_entry_order_still_current,
            );
        // Terminal sequence elements retain their allocated
        // orders for diagnostics/save parity. Do not mistake
        // that same retired entry order for a successor, while
        // still accepting a distinct order installed by a
        // synchronous condolence-card callback.
        // Order advancement changes the retained execution result to
        // IN_PROGRESS only when Proceed returns a non-null
        // actor order. Manager residency is not sufficient: queue
        // exhaustion can terminate the selected element while
        // leaving a fallback Wait discoverable in the manager,
        // yet the actor's order remains empty until its next
        // update entry. `installed_order` is the explicit
        // mirror updated by order advancement and accepted instruction.
        let installed_order = self
            .world
            .entities
            .get(entity_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| actor.installed_order);
        let installed_successor_exists = installed_order.is_some_and(|installed| {
            !selected_order.is_some_and(|(_, _, entry_order)| {
                selected_element_retired && installed.order_id == entry_order
            })
        });
        let motion_latch_debug = motion_latch_debug_config().filter(|config| {
            config.frame == self.control.frame_counter
                && config.creation_order == self.world.original_creation_order(entity_id)
        });
        if let Some(actor) = self
            .world
            .entities
            .get_mut(entity_id)
            .and_then(Entity::actor_data_mut)
        {
            // Advancing to the next actor order overwrites a TERMINATED result
            // result with IN_PROGRESS when Proceed exposes another
            // order. Instruction handling does the same when terminating the
            // old element synchronously installs a successor.
            // Specialized owners retire their order internally,
            // so an entry-identity change is their equivalent of
            // the base update's TERMINATED branch even when the
            // last raw sprite edge was START/DONE/IN_PROGRESS.
            // ABORTED is tied to the sequence element captured
            // on actor-update entry. Its synchronous
            // Impossible condolence may install Wait and
            // overwrite the sprite's last edge, but it cannot
            // rewrite the Execute return already held by Actor.
            let motion_before_projection = actor.continuation.motion_state;
            actor.continuation.motion_state = project_post_completion_motion(
                motion_before_projection,
                selected_element_impossible && !explicit_execute_in_progress,
                installed_successor_exists,
                selected_specialized_order_advanced,
            );
            actor.continuation.motion_state = apply_post_completion_execute_override(
                actor.continuation.motion_state,
                post_completion_execute_override,
                selected_element_interrupted,
                installed_successor_exists,
            );
            if let Some(config) = motion_latch_debug {
                eprintln!(
                    "[MOTION_LATCH frame={} co={} owner={} entry_order={:?} entry_state={:?} specialized_motion={:?} explicit_in_progress={} retired={} interrupted={} impossible={} specialized_advanced={} installed_order={:?} installed_successor={} motion_before={:?} motion_after={:?}]",
                    config.frame,
                    config.creation_order,
                    entity_id.index(),
                    selected_order,
                    selected_element_state,
                    specialized_execute_motion,
                    explicit_execute_in_progress,
                    selected_element_retired,
                    selected_element_interrupted,
                    selected_element_impossible,
                    selected_specialized_order_advanced,
                    installed_order,
                    installed_successor_exists,
                    motion_before_projection,
                    actor.continuation.motion_state,
                );
            }
            tracing::trace!(
                target: "parity_motion_state",
                entity = ?entity_id,
                family = ?selected_owner_family,
                entry_order = ?selected_order,
                specialized_motion = ?specialized_execute_motion,
                element_retired = selected_element_retired,
                element_interrupted = selected_element_interrupted,
                specialized_advanced = selected_specialized_order_advanced,
                installed_successor = installed_successor_exists,
                motion_state = ?actor.continuation.motion_state,
                "actor motion-state latch",
            );
        }
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
        self.dispatch_actor_action_change_for(sim, assets, entity_id);
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
            .map(|order| order.order_type)
            .unwrap_or(crate::order::OrderType::NonanimationEnd);
        self.debug_refresh_view_lifecycle(
            "derived_tail_normal",
            entity_id,
            Some(installed_tail_order_type),
        );
        after_slot(self, entity_id, installed_tail_order_type);
        if let Some(entity) = self.world.entities.get(entity_id) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                entity_id,
                self.control.frame_counter,
                "owner_tail_after_derived",
            );
        }

        if let Some(actor) = self
            .world
            .entities
            .get_mut(entity_id)
            .and_then(Entity::actor_data_mut)
        {
            actor.execute_order_initialising = false;
        }
        self.debug_drop_owner_boundary("execute_latch_cleared", entity_id, selected_order);

        // Human posture changes update intersecting-corpse state
        // synchronously. Close the owner-local
        // boundary before the next creation slot samples this
        // actor for anti-collision.
        self.process_corpse_intersection_update_for(entity_id);
    }
}
