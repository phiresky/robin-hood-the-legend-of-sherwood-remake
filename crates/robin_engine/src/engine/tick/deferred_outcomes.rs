//! Deferred completion outcomes collected during the ordered entity walk.
//!
//! These handlers preserve the Original callback boundary: actor animation
//! execution records cross-entity work, then the hourglass owner pass applies
//! it after the mutable actor borrow has ended.

use super::*;

impl EngineInner {
    pub(in crate::engine) fn drain_waiting_upright(&mut self, owners: Vec<EntityId>) {
        for owner in owners {
            let soldier = self
                .world
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("WaitingUpright owner {owner:?} disappeared"));
            let enemy = match soldier {
                Entity::Soldier(soldier) => soldier.npc.ai_brain.enemy().unwrap_or_else(|| {
                    panic!("WaitingUpright soldier {owner:?} has no enemy AI state")
                }),
                _ => panic!("WaitingUpright candidate {owner:?} is not a soldier"),
            };
            let needs_enter = enemy.will_be_attentive
                && !self
                    .orders
                    .sequence_manager
                    .element_is_about_to_be_launched(owner, Command::EnterAttentiveMode);

            if needs_enter {
                // RHelementactorsoldier.cpp:753-758. As in the symmetric
                // WaitingAlerted repair below, call LaunchSequenceElement
                // directly: SetAttentiveMode(true) would suppress the repair
                // precisely because mbWillBeAttentive is already true.
                self.launch_element(crate::sequence::SequenceElement::new(
                    1,
                    Command::EnterAttentiveMode,
                    Some(owner),
                ));
            }
        }
    }

    pub(in crate::engine) fn drain_waiting_alerted(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owners: Vec<EntityId>,
    ) {
        for owner in owners {
            let soldier = self
                .world
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("WaitingAlerted owner {owner:?} disappeared"));
            let enemy = match soldier {
                Entity::Soldier(soldier) => soldier.npc.ai_brain.enemy().unwrap_or_else(|| {
                    panic!("WaitingAlerted soldier {owner:?} has no enemy AI state")
                }),
                _ => panic!("WaitingAlerted candidate {owner:?} is not a soldier"),
            };
            let needs_leave = !enemy.will_be_attentive
                && !self
                    .orders
                    .sequence_manager
                    .element_is_about_to_be_launched(owner, Command::LeaveAttentiveMode);

            if needs_leave {
                // RHelementactorsoldier.cpp:736-740. This is deliberately not
                // SetAttentiveMode(false): that helper suppresses a request when
                // mbWillBeAttentive is already false, while this corrective
                // Execute arm exists specifically for that inconsistent state.
                self.launch_element(crate::sequence::SequenceElement::new(
                    1,
                    Command::LeaveAttentiveMode,
                    Some(owner),
                ));
            }

            // RHelementactorsoldier.cpp:742-747: a soldier playing the
            // non-sword WAITING_ALERTED animation must not still be linked
            // into a swordfight. The Original asserts in debug builds and
            // unconditionally tears the relationship down in release.
            let still_swordfighting = !self
                .world
                .entities
                .get(owner)
                .unwrap_or_else(|| panic!("WaitingAlerted owner {owner:?} disappeared"))
                .human_data()
                .unwrap_or_else(|| panic!("WaitingAlerted soldier {owner:?} is not human"))
                .opponents
                .is_empty();
            if still_swordfighting {
                self.quit_swordfight(sim, assets, owner);
            }
        }
    }

    pub(super) fn drain_non_interruptable_lifts(
        &mut self,
        non_interruptable_lifts: Vec<(crate::sequence::SequenceId, usize)>,
    ) {
        for (seq_id, elem_idx) in non_interruptable_lifts {
            self.orders.sequence_manager.set_element_priority(
                seq_id,
                elem_idx,
                crate::sequence::SequencePriority::NonInterruptable,
            );
        }
    }

    pub(super) fn drain_corpse_drop_done(
        &mut self,
        assets: &LevelAssets,
        corpse_drop_done: Vec<EntityId>,
    ) {
        // RHElementActorPC::Execute calls DropCorpse from inside the terminal
        // transition arm, before returning TERMINATED to Actor::Hourglass and
        // therefore before DoNextOrder exposes a following command.
        for carrier_id in corpse_drop_done {
            let selected_order = self
                .orders
                .sequence_manager
                .current_order_for_actor(carrier_id)
                .map(|(_, _, order)| order.order_type)
                .unwrap_or_else(|| {
                    panic!(
                        "corpse-drop transition owner {carrier_id:?} lost its selected terminal order"
                    )
                });
            assert_eq!(
                selected_order,
                crate::order::OrderType::TransitionCarryingCorpseWaitingUpright,
                "corpse-drop side effect must run before DoNextOrder exposes a successor"
            );
            crate::abilities::sync_terminal_corpse_drop_animation(
                &mut self.world.entities,
                &assets.profile_manager,
                carrier_id,
            );
            let (target_id, drop_posture, carrier_pos, carrier_direction) = {
                let carrier = self.get_entity(carrier_id).unwrap_or_else(|| {
                    panic!("corpse-drop transition owner {carrier_id:?} disappeared")
                });
                let pc = carrier.pc_data().unwrap_or_else(|| {
                    panic!("corpse-drop transition owner {carrier_id:?} is not a PC")
                });
                let target_id = pc.carried.unwrap_or_else(|| {
                    panic!("corpse-drop transition owner {carrier_id:?} has no carried body")
                });
                let direction =
                    u16::try_from(carrier.element_data().direction()).unwrap_or_else(|_| {
                        panic!("corpse-drop transition owner {carrier_id:?} has invalid direction")
                    });
                (
                    target_id,
                    pc.live_carried_posture(),
                    carrier.element_data().position_map(),
                    direction,
                )
            };
            self.apply_completed_corpse_drop(
                carrier_id,
                target_id,
                drop_posture,
                carrier_pos,
                carrier_direction,
            );
        }
    }

    pub(super) fn drain_seq_advance(
        &mut self,
        seq_advance: Vec<(crate::sequence::SequenceId, usize)>,
    ) {
        for (seq_id, elem_idx) in seq_advance {
            // `do_next_order` semantics: pop the just-completed
            // order; advance to the next if one exists, otherwise
            // terminate the element.
            self.do_next_order(seq_id, elem_idx);
        }
    }

    pub(super) fn drain_wasp_next_cycle(
        &mut self,
        wasp_next_cycle: Vec<(crate::sequence::SequenceId, usize, u16)>,
    ) {
        // Wasp struggle-cycle refill: push a fresh `GettingFreeFromWasp`
        // order with the decremented counter, then pop the current one
        // via `do_next_order` so the new order takes over cleanly.
        for (seq_id, elem_idx, cycles_remaining) in wasp_next_cycle {
            let order = crate::order::Order::new(
                crate::order::OrderType::GettingFreeFromWasp,
                0.0,
                0.0,
                self.orders.allocate_order_id(),
            )
            .with_completion(crate::order::OrderCompletion::WaspStruggleCycle { cycles_remaining });
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
            self.do_next_order(seq_id, elem_idx);
        }
    }

    pub(super) fn drain_seq_terminate(
        &mut self,
        seq_terminate: Vec<(crate::sequence::SequenceId, usize)>,
    ) {
        for (seq_id, elem_idx) in seq_terminate {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }
    }

    pub(super) fn drain_play_anim_frozen(
        &mut self,
        play_anim_frozen: Vec<(EntityId, u16, crate::order::OrderType)>,
    ) {
        for (actor, command_level, anim) in play_anim_frozen {
            let mut elem = crate::sequence::SequenceElement::new_generic(
                command_level,
                crate::element::Command::PlayAnimFrozen,
                Some(actor),
            );
            elem.set_property(
                crate::sequence::Field::AnimationId,
                crate::sequence::FieldValue::Animation(anim),
            );
            self.orders.sequence_manager.launch_element(elem);
        }
    }

    pub(super) fn drain_seq_impossible(
        &mut self,
        seq_impossible: Vec<(crate::sequence::SequenceId, usize)>,
    ) {
        for (seq_id, elem_idx) in seq_impossible {
            let original_push_sentinel = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| {
                    element.command == crate::element::Command::ReceiveSwordDamage
                        && element.current_order().is_some_and(|order| {
                            order.order_type == crate::order::OrderType::NonanimationEnd
                        })
                });

            if original_push_sentinel {
                // TranslatePushDamage accidentally authors a
                // RHNONANIMATION_END stand-up order for a conscious,
                // crouched-family stunning push. Base Actor::Execute returns
                // ABORTED for that unknown action; the release build then
                // sets even this NonInterruptable injury Impossible and
                // synchronously releases its postponed successor.
                self.orders
                    .sequence_manager
                    .element_impossible_from_execute(seq_id, elem_idx);
            } else {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
        }
    }

    pub(super) fn drain_unlock_door_done(&mut self, unlock_door_done: Vec<crate::gate::DoorIndex>) {
        for door_id in unlock_door_done {
            let door = required_canonical_door_mut(
                &mut self.script_domains.interactables.doors,
                door_id,
                "UnlockDoor action-point callback",
            );
            door.locked_pc = false;
            door.locked_npc_civilian = false;
            door.locked_npc_villain = false;
            door.unlockable = false;
            tracing::debug!(
                door_id = %door_id,
                "UnlockDoor: action point cleared all live door locks"
            );
        }
    }

    pub(super) fn drain_next_jump_step(
        &mut self,
        assets: &LevelAssets,
        next_jump_step: Vec<EntityId>,
    ) {
        for entity_id in next_jump_step {
            if let Some((new_layer, new_sector, projection_point)) =
                self.advance_jump_step(entity_id)
            {
                self.finalize_airborne_jump_landing(
                    assets,
                    entity_id,
                    new_layer,
                    new_sector,
                    projection_point,
                );
            }
        }
    }

    pub(super) fn drain_select_hulk(&mut self, select_hulk: Vec<(EntityId, f32)>) {
        for (entity_id, speed) in select_hulk {
            self.apply_select_hulk(entity_id, speed);
        }
    }

    pub(super) fn drain_resume_door_pass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        resume_door_pass: Vec<EntityId>,
    ) {
        use super::movement::DoorPassAdvance;

        for entity_id in resume_door_pass {
            let Some(action) = self
                .get_entity(entity_id)
                .and_then(|entity| entity.actor_data())
                .and_then(|actor| actor.active_door_pass.as_ref())
                .map(|pass| pass.current_action)
            else {
                continue;
            };
            self.apply_door_pass_transition_completion_side_effects(assets, entity_id, action);
            // Advance through Transition / PassingDoor / Walk steps.
            // PassingDoor triggers fired here need to run through
            // `execute_pass_door` with `&mut self`, so we collect them
            // and drain after the borrow on the actor ends.
            let mut door_triggers: Vec<(EntityId, crate::gate::DoorIndex, bool, u8)> = Vec::new();
            let mut select_triggers: Vec<(EntityId, f32)> = Vec::new();
            let (advance, arrived_movement, completed_pass) = {
                let Some(entity) = self.world.entities.get_mut(entity_id) else {
                    continue;
                };
                let transition_destination = entity.element_data().position_map();
                let Some(actor) = entity.actor_data_mut() else {
                    continue;
                };
                let adv = Self::advance_door_pass(
                    actor,
                    entity_id,
                    transition_destination,
                    &mut door_triggers,
                    &mut select_triggers,
                    &mut self.orders.next_order_id,
                );
                // If the door pass is done (no more steps), mirror the
                // arrival teardown performed by the movement tick.
                let arrived = if let DoorPassAdvance::Done { completed } = &adv {
                    let am = actor.active_movement;
                    actor.clear_path();
                    actor.action_state = if actor.action_state.is_sword() {
                        crate::element::ActionState::WaitingSword
                    } else {
                        crate::element::ActionState::Waiting
                    };
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    Some((am, *completed))
                } else {
                    None
                };
                let (arrived, completed) = match arrived {
                    Some((am, completed)) => (Some(am), completed),
                    None => (None, None),
                };
                (adv, arrived, completed)
            };

            // Fire any PassingDoor triggers that came up during this resume.
            for (eid, door_index, direct, trigger_num) in door_triggers {
                self.execute_pass_door(sim, assets, eid, door_index, direct, trigger_num);
            }
            for (eid, speed) in select_triggers {
                self.apply_select_hulk(eid, speed);
            }
            if let Some((door_index, direct)) = completed_pass {
                tracing::debug!(
                    entity = ?entity_id,
                    door = %door_index,
                    direct,
                    "DoorPass: completed after transition resume"
                );
                self.commit_completed_door_pass_position(assets, entity_id, door_index, direct);
                self.apply_completed_door_pass_lift_entry_state(entity_id, door_index, direct);
            }
            // If the advance yielded another Walk or Transition step,
            // append it behind the completed transition order, then pop
            // that completed transition so the new order becomes the
            // front order.  This mirrors the movement-tick door-pass
            // path, where `transition_pushes` are drained before
            // `order_pops`.
            if let Some((seq_id, elem_idx)) = self
                .orders
                .sequence_manager
                .current_element_for_actor(entity_id)
            {
                match advance.clone() {
                    DoorPassAdvance::Continue {
                        destination,
                        action,
                        reverse,
                        compute_direction,
                        tolerance,
                    } => {
                        tracing::debug!(
                            entity = ?entity_id,
                            ?action,
                            target_x = destination.x,
                            target_y = destination.y,
                            "DoorPass: resumed with movement order after transition"
                        );
                        self.install_special_walk_order(
                            entity_id,
                            seq_id,
                            elem_idx,
                            destination,
                            action,
                            reverse,
                            compute_direction,
                            tolerance,
                            None,
                            "PassDoor resumed walk",
                        );
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::Paused { transition_order } => {
                        self.orders.sequence_manager.push_order_on(
                            seq_id,
                            elem_idx,
                            transition_order,
                        );
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::ActionPoint { order } => {
                        self.orders
                            .sequence_manager
                            .push_order_on(seq_id, elem_idx, order);
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::NoActive => {
                        tracing::warn!(
                            entity = ?entity_id,
                            "DoorPass: resume callback had no active pass"
                        );
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::Done { .. } => {}
                }
            }

            // If the door pass completed, notify the sequence manager. Its
            // owner-local condolence drain runs immediately after these
            // outcomes and is the sole EVENT_REACHPOINT owner: in particular,
            // IsLastRealAction suppresses the event while AssertPosition /
            // Move followers remain. Dispatching it manually here bypassed
            // that source gate for translated door routes.
            if let Some(am) = arrived_movement {
                let seq_id = am.sequence_id.unwrap_or_else(|| {
                    panic!(
                        "completed transition-resumed PassDoor for {entity_id:?} has no sequence identity"
                    )
                });
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, am.element_index);
            }

            let _ = advance;
        }
    }

    pub(super) fn drain_drop_ale_done(
        &mut self,
        assets: &LevelAssets,
        drop_ale_done: Vec<EntityId>,
    ) {
        for pc_id in drop_ale_done {
            let action = crate::profiles::Action::Ale;
            let (position, layer, sector, obstacle, direction, material, status_idx) = {
                let pc = self
                    .get_entity(pc_id)
                    .unwrap_or_else(|| panic!("DropAle DONE references missing PC {pc_id:?}"));
                let position = pc.cxx_current_point_map().unwrap_or_else(|| {
                    panic!("DropAle DONE PC {pc_id:?} has no current sprite action point")
                });
                let crate::element::Entity::Pc(pc) = pc else {
                    panic!("DropAle DONE owner {pc_id:?} is not a PC");
                };
                let element = &pc.element;
                (
                    position,
                    element.layer(),
                    element.sector(),
                    element.obstacle_index(),
                    element.direction(),
                    element.material(),
                    self.pc_description_index_for_pc_data(&pc.pc),
                )
            };
            let status_idx = status_idx.unwrap_or_else(|| {
                panic!("DropAle DONE PC {pc_id:?} has no campaign character status")
            });

            // RHElementAle::CopyPositionMapEtc copies the actor placement
            // exactly; unlike cursor authorization, the action point does not
            // search for a nearby walkable position.
            let mut ale_element = crate::element::ElementData {
                kind: crate::element::ElementKind::ObjectOther,
                active: true,
                // RHElementAle constructs RHElementObject with bBlipped=false.
                blipped: false,
                ..Default::default()
            };
            ale_element.sprite.apply_placement(
                position,
                layer,
                sector,
                direction,
                material,
                obstacle,
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    obstacle,
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            let ale = crate::element::Entity::Bonus(crate::element::ElementBonus {
                element: ale_element,
                object: crate::element::ObjectData {
                    quantity: 1,
                    object_type: crate::element::ObjectType::Ale,
                    associated_action: action,
                    animation: crate::element::Animation::ObjectLying,
                    ..Default::default()
                },
            });
            let ale_id = self.add_entity(ale);
            // RHElementAle::Create clones the ACCESSORIES_Ale master before
            // SetAnimation(OBJECT_LYING), whose ForceAnimation resets the
            // new sprite to frame/count zero.
            self.attach_accessory_sprite(assets, ale_id);
            let ale_sprite = &mut self
                .get_entity_mut(ale_id)
                .expect("newly-added ale must still exist")
                .element_data_mut()
                .sprite;
            assert!(
                ale_sprite.has_animation(crate::order::OrderType::ObjectLying),
                "DropAle requires the preloaded ACCESSORIES_Ale ObjectLying animation"
            );
            ale_sprite.force_animation(crate::order::OrderType::ObjectLying, 0);
            self.add_detectable_for_all_npc(ale_id, crate::element::DetectableType::Object);

            // Original consumes the inventory item only after the new ale is
            // in the engine and visible to every NPC's detection list.
            let status = &mut self.mission_domain.campaign.characters[status_idx].status;
            let removed = status.decrease_ammo(action, 1);
            assert_eq!(removed, 1, "DropAle DONE PC {pc_id:?} had no ale ammo");
            let now_empty = status.get_ammo(action) == 0;
            if now_empty {
                self.disable_pc_action(assets, pc_id, action);
                // RHElementActorPC::DecreaseAmmoAmount uses its default
                // bSpeak=true here: after disabling the emptied action it
                // queues HERO_OUT_OF_AMMO, except on the Sherwood hub map.
                // Keep this after bottle creation/detection and inventory
                // consumption, matching RHANIMATION_DROPPING_ALE's DONE arm.
                if !self.is_sherwood(&assets.profile_manager) {
                    self.hero_speaking(assets, pc_id, crate::engine::melee::HERO_OUT_OF_AMMO);
                }
            }
            tracing::debug!(
                pc = ?pc_id,
                ?ale_id,
                "DropAle DONE: decremented ale ammo and spawned bottle"
            );
        }
    }

    pub(super) fn drain_pc_bow_equip_action(
        &mut self,
        assets: &LevelAssets,
        pc_bow_equip_action: Vec<EntityId>,
    ) {
        for pc_id in pc_bow_equip_action {
            // RHElementActorHuman::Execute forwards this synchronously from
            // the TransitionEquipBow START arm after setting AimingWithBow.
            // An unselected PC only restores its remembered action; a
            // selected PC also restores the messenger-global action.
            self.set_pc_action_from_message(assets, 0, pc_id, crate::profiles::Action::Bow);
        }
    }

    pub(super) fn drain_pc_bow_unequip_action(
        &mut self,
        assets: &LevelAssets,
        pc_bow_unequip_action: Vec<(EntityId, bool)>,
    ) {
        for (pc_id, script_driven) in pc_bow_unequip_action {
            // RHElementActorHuman::Execute, TRANSITION_UNEQUIP_BOW START arm
            // (PC branch): an empty quiver disables the Bow action outright
            // (regardless of the script flag); otherwise non-script elements
            // forward MSG_UNSELECT_ACTION(BOW).
            if self.get_pc_ammo_count(pc_id, crate::profiles::Action::Bow) == 0 {
                self.disable_pc_action(assets, pc_id, crate::profiles::Action::Bow);
            } else if !script_driven {
                // RHMessenger's MSG_UNSELECT_ACTION pre-process drops the
                // message unless the unselected action is the messenger's
                // currently selected action; it then clears that selection
                // and RHEngine::UnSelectAction clears the PC's remembered
                // action (the freshly-Waiting action state means no further
                // cleanup sequence is launched).
                if self.players.seats[0].selected_action == crate::profiles::Action::Bow {
                    self.players.seats[0].selected_action = crate::profiles::Action::NoAction;
                    self.unselect_action(pc_id);
                }
            }
        }
    }

    pub(super) fn drain_pc_helping_climb_action(
        &mut self,
        assets: &LevelAssets,
        pc_helping_climb_action: Vec<EntityId>,
    ) {
        for pc_id in pc_helping_climb_action {
            // RHElementActorPC::Execute forwards MSG_SELECT_ACTION(HELP_TO_CLIMB)
            // straight after SetStates on the DONE edge of the helping-climb
            // entry transition. HelpToClimb is already the current action, but
            // a selected PC still goes through the action-reselection Stop at
            // Normal priority, which interrupts whatever the entry transition
            // postponed behind itself — the move the player queued while the
            // PC was kneeling down never resumes.
            self.set_pc_action_from_message(assets, 0, pc_id, crate::profiles::Action::HelpToClimb);
        }
    }

    pub(super) fn drain_stature_change_end(&mut self, stature_change_end: Vec<EntityId>) {
        for _pc_id in stature_change_end {
            self.orders.messenger.send(crate::messenger::Message::new(
                crate::messenger::MessageType::Simple(
                    crate::messenger::SimpleMessage::StatureChangeEnd,
                ),
            ));
        }
    }

    pub(super) fn drain_weak_stunned_start(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        weak_stunned_start: Vec<(
            EntityId,
            crate::order::OrderType,
            crate::element::ActionState,
        )>,
    ) {
        for (entity_id, anim_type, action_state_before_perform) in weak_stunned_start {
            // RHElementActorHuman::Execute notifies every adversary before
            // calling PerformAction. Rust has to defer the cross-entity Think
            // until the animation borrow is released, by which point the
            // START edge may already have installed WaitingSword. Temporarily
            // expose the captured pre-PerformAction action state so nested
            // ReconsiderSwordfight applies Original's honour/action gate.
            let action_state_after_perform = {
                let actor = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "weak/stunned callback owner {} disappeared before drain",
                            entity_id.index()
                        )
                    })
                    .actor_data_mut()
                    .unwrap_or_else(|| {
                        panic!(
                            "weak/stunned callback owner {} is not an actor",
                            entity_id.index()
                        )
                    });
                std::mem::replace(&mut actor.action_state, action_state_before_perform)
            };
            self.add_weak_stunned_combat(
                sim,
                assets,
                entity_id,
                anim_type == crate::order::OrderType::BeingWeakSword,
            );
            self.world
                .entities
                .get_mut(entity_id)
                .unwrap_or_else(|| {
                    panic!(
                        "weak/stunned callback owner {} disappeared during drain",
                        entity_id.index()
                    )
                })
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "weak/stunned callback owner {} lost actor data during drain",
                        entity_id.index()
                    )
                })
                .action_state = action_state_after_perform;
        }
    }

    pub(super) fn drain_hidden_titbit_removals(&mut self, hidden_titbit_removals: Vec<EntityId>) {
        for entity_id in hidden_titbit_removals {
            self.feedback.titbit_manager.remove_titbit(
                crate::titbit::TitbitKind::Hidden,
                crate::titbit::ElementHandle(entity_id.index()),
            );
        }
    }

    pub(in crate::engine) fn drain_beggar_wait_handoffs(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        beggar_wait_handoffs: Vec<(EntityId, bool)>,
    ) {
        for (pc_id, entering) in beggar_wait_handoffs {
            // RHElementActorPC::Execute calls Wait() inside both beggar
            // transition DONE arms. This occurs in the actor's live legacy
            // slot, before base Actor completion and the later
            // SequenceManager::Hourglass drain. It is normally a fresh
            // launch: ensure_wait_element intentionally suppresses a wait
            // while the finishing transition is still current. The selected
            // entry exception below represents the fresh Wait that Original's
            // immediately-following Stop discards before it can translate.
            let wait_sequence = self.actor_wait(pc_id);
            let selected_entry = entering && self.players.seats[0].selection.contains(&pc_id);
            if selected_entry {
                self.orders
                    .sequence_manager
                    .interrupt_just_registered_wait_before_instruct(pc_id, wait_sequence);
            }
            // The next Original statement forwards MSG_SELECT_ACTION(BEGGAR).
            // When this PC is selected, RHEngine::SelectAction calls Stop at
            // Normal priority even if Beggar was already the current action;
            // that can discard the just-postponed low-priority Wait. For an
            // unselected PC, SelectAction only stores the action and the Wait
            // survives. Preserve that general message behavior and ordering.
            // WAIT-priority `RHSequenceElement::Go()` is synchronous in
            // Original. During entry it is postponed behind the still-live
            // noninterruptible EnterBeggar element, then the immediately
            // forwarded selected-PC SelectAction calls Stop(Normal) and
            // discards that postponed Wait (RHsequenceelement.cpp:509-558,
            // RHengine.cpp:13017-13075). Rust drains this execute side effect
            // after retiring the transition, so translating the Wait first
            // would select it and Actor::Stop's default-Wait exclusion would
            // preserve it. Suppress exactly that selected-entry Wait: the
            // next actor Hourglass installs the ordinary beggar idle, after
            // the same transient null-order frame as Original. Unselected
            // entry and both leave paths still execute Wait synchronously.
            if !selected_entry {
                self.drain_script_registration_inline_actions(sim, assets, &mut Vec::new())
                    .unwrap_or_else(|error| {
                        panic!(
                            "beggar transition Wait registration failed before action message for {pc_id:?}: {error:?}"
                        )
                    });
            }
            if entering {
                self.set_pc_action_from_message(assets, 0, pc_id, crate::profiles::Action::Beggar);
            } else if self.players.seats[0].selection.contains(&pc_id) {
                // Leaving forwards MSG_UNSELECT_ACTION(BEGGAR) for a
                // selected PC; after SetStates has made it Upright, the
                // UnSelectAction cleanup only clears the stored action.
                self.unselect_action(pc_id);
            } else if let Some(pc) = self
                .get_entity_mut(pc_id)
                .and_then(|entity| entity.pc_data_mut())
            {
                pc.current_action = crate::profiles::Action::NoAction;
            }
        }
    }

    pub(super) fn drain_beggar_coin_flags(&mut self, beggar_coin_flags: Vec<(EntityId, bool)>) {
        for (pc_id, enabled) in beggar_coin_flags {
            super::beggar::set_flags_of_near_coins_on_ground(
                &mut self.world.entities,
                pc_id,
                enabled,
            );
            if enabled {
                super::beggar::add_beggar_for_all_intelligent_seeking_soldiers(
                    &mut self.world.entities,
                    pc_id,
                    self.control.sim_config.difficulty,
                );
            }
        }
    }

    pub(super) fn drain_smalltalk_strikes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        smalltalk_strikes: Vec<(EntityId, EntityId, crate::weapons::SwordStrike)>,
    ) {
        for (actor_id, target_id, strike) in smalltalk_strikes {
            let wound_target = {
                let attacker = self
                    .get_entity(actor_id)
                    .unwrap_or_else(|| panic!("smalltalk attacker {actor_id:?} disappeared"));
                let target = self
                    .get_entity(target_id)
                    .unwrap_or_else(|| panic!("smalltalk antagonist {target_id:?} disappeared"));
                // Original builds this relative vector from
                // `GetPositionGround()`, i.e. the stored world X/Y pair.
                // Projected map Y differs by elevation, and using it here can
                // flip the back-hit half-plane test when the fighters stand
                // at different heights.
                let attacker_pos = attacker.ground_position();
                let target_pos = target.ground_position();
                // RHElement::GetDirectionVector returns a vector in the
                // isometric map plane.  Smalltalk's "striking in the back"
                // dot product therefore needs the aspect-scaled Y component;
                // the ordinary unit-circle helper can flip this half-plane
                // test and suppress the ensuing sword-damage RNG draws.
                let [dx, dy] = crate::position_interface::sector_to_vector_iso(
                    target.element_data().direction(),
                );
                let relative_x = target_pos.x - attacker_pos.x;
                let relative_y = target_pos.y - attacker_pos.y;
                target
                    .actor_data()
                    .is_some_and(|actor| actor.action_state.is_sword())
                    && dx * relative_x + dy * relative_y > 0.0
            };
            if wound_target {
                let profile_idx = self
                    .get_entity(actor_id)
                    .and_then(|entity| {
                        super::melee::get_hth_weapon_id_full(entity, &assets.profile_manager)
                    })
                    .unwrap_or_else(|| {
                        panic!("smalltalk attacker {actor_id:?} has no HtH weapon profile")
                    });
                self.queue_sword_damage(sim, assets, target_id, actor_id, strike, profile_idx);
                continue;
            }

            let (position, weapon1) = {
                let entity = self
                    .get_entity(actor_id)
                    .unwrap_or_else(|| panic!("smalltalk attacker {actor_id:?} disappeared"));
                let target_mutual = self
                    .get_entity(target_id)
                    .and_then(|e| e.human_data())
                    .and_then(|h| h.opponents.first().copied())
                    .map(|id| id == actor_id)
                    .unwrap_or(false);
                if !target_mutual {
                    continue;
                }
                let pos = entity.element_data().position_map();
                let weapon1 =
                    super::melee::weapon_material_from_profile(entity, &assets.profile_manager);
                (pos, weapon1)
            };
            let weapon2 = self
                .get_entity(target_id)
                .map(|e| super::melee::weapon_material_from_profile(e, &assets.profile_manager))
                .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood);
            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::StrikeFx {
                    strike_kind: crate::sound::StrikeKind::Swipe,
                    weapon1,
                    weapon2,
                    position,
                });
        }
    }

    pub(super) fn drain_killed_at_bottom(&mut self, killed_at_bottom: Vec<(EntityId, EntityId)>) {
        for (victim_id, killer_id) in killed_at_bottom {
            let mut elem = crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::GetKilledAtBottom,
                Some(victim_id),
                Some(killer_id),
            );
            elem.priority = crate::sequence::SequencePriority::Lethal;
            self.launch_element(elem);
        }
    }

    pub(super) fn drain_deactivate_entities(&mut self, deactivate_entities: Vec<EntityId>) {
        // DRINKING_ALE DONE — deactivate the antagonist to hide
        // the ale bottle.
        for antag in deactivate_entities {
            if let Some(entity) = self.world.entities.get_mut(antag) {
                entity.element_data_mut().active = false;
            }
        }
    }

    pub(super) fn drain_pc_target_activations(
        &mut self,
        pc_target_activations: Vec<(EntityId, EntityId, Command)>,
    ) {
        for (pc, target, activation_cmd) in pc_target_activations {
            let target_is_fx = self
                .get_entity(target)
                .is_some_and(|e| e.kind().is_fx_target());
            if !target_is_fx {
                tracing::warn!(
                    ?pc,
                    ?target,
                    ?activation_cmd,
                    "PC target animation DONE but antagonist is not an FX target"
                );
                continue;
            }
            let mut activation =
                crate::sequence::SequenceElement::new(1, activation_cmd, Some(target));
            activation.data = crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(pc),
            };
            self.launch_element(activation);
        }
    }

    pub(super) fn drain_waking_up_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        waking_up_done: Vec<(EntityId, EntityId)>,
    ) {
        for (rescuer, target) in waking_up_done {
            let target_entity = self.get_entity(target).unwrap_or_else(|| {
                panic!(
                    "WakingUp DONE from rescuer {rescuer:?} references missing required target {target:?}"
                )
            });
            if !target_entity.is_human() {
                panic!(
                    "WakingUp DONE from rescuer {rescuer:?} requires human target {target:?}, found {:?}",
                    target_entity.kind()
                );
            }

            let target_is_dead = target_entity.is_dead();
            let target_is_pc = target_entity.is_pc();
            if !target_is_dead {
                if let Some(target_entity) = self.get_entity_mut(target) {
                    target_entity.set_posture(crate::element::Posture::Lying);
                }
                let wake_outcome = self.apply_concussion(sim, assets, target, 0, false);
                // SetConcussionOfTheBrain synchronously sends FITAGAIN from
                // the WakingUp DONE stack. This AI consequence is immediate
                // even when the target's creation-ordered actor slot has
                // already passed; only its next animation Execute is delayed.
                self.drain_pending_concussion_side_effects(sim, assets);
                if !target_is_pc && matches!(wake_outcome, crate::combat::ConcussionOutcome::WokeUp)
                {
                    assert!(
                        self.dispatch_pending_fit_again_for_npc(sim, target, assets),
                        "WakingUp DONE for NPC {target:?} cleared concussion without queueing the required EVENT_FITAGAIN"
                    );
                    // These are inline consequences of the NPC's FITAGAIN
                    // Think call in Original, not work for its next actor slot.
                    self.tick_ai_pending_resurrection_and_eyes_for_npc(target);
                    self.apply_wake_redetection_blinks(target);
                }
                // Original WAKING_UP DONE calls target->Wait()
                // unconditionally. That launches a fresh priority-Wait
                // element even while the old unconscious Wait is live, so
                // ordinary equal-priority arbitration replaces and
                // retranslates it immediately as StandingUp.
                self.actor_wait(target);
            }

            if target_is_pc {
                self.hero_speaking(assets, target, crate::engine::melee::HERO_RECOVER);
            }
        }
    }

    pub(super) fn drain_pickups(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pickups: Vec<(EntityId, EntityId)>,
    ) {
        // TAKING DONE — dispatches by taker + object_type.
        //
        // * PC takers route through `apply_pc_take_object` which
        //   covers amulet, purse, coin, ransom, relics, and the
        //   default ammo-bonus fall-through.
        //
        // * Net takers (PC or NPC) hit the shared net-release path.
        //
        // * NPC soldiers picking up Coin/Purse use the short
        //   money-bump path.
        //
        // * Scrolls route through `take_scroll` which fires
        //   `IScrollScript::IsTaken`.
        for (taker, object) in pickups {
            let Some(taker_entity) = self.world.entities.get(taker) else {
                tracing::warn!(
                    ?taker,
                    ?object,
                    "dropping deferred pickup because its taker no longer exists"
                );
                continue;
            };
            let taker_is_pc = taker_entity.is_pc();
            if !taker_is_pc && !taker_entity.is_npc() {
                tracing::warn!(
                    ?taker,
                    ?object,
                    "dropping deferred pickup because its taker is not an actor"
                );
                continue;
            }

            // Scrolls are not ObjectData carriers — they have their
            // own Entity::Scroll variant and a script-driven
            // `IsTaken` dispatch.
            let Some(object_entity) = self.world.entities.get(object) else {
                tracing::warn!(
                    ?taker,
                    ?object,
                    "dropping deferred pickup because its object no longer exists"
                );
                continue;
            };
            let is_scroll = matches!(object_entity, crate::element::Entity::Scroll(_));
            if is_scroll {
                self.scroll_is_taken(sim, assets, object, taker);
                continue;
            }

            let object_type = object_entity.object_data().map(|o| o.object_type);
            // Original's special net pickup arm is selected by the
            // RHElementNet class. An inventory RHElementBonus whose object
            // type is BONUS_NET follows ordinary TakeObject instead: it uses
            // the usual capacity/quantity split and is only deactivated when
            // the bonus is fully consumed.
            let is_landed_net = matches!(object_entity, Entity::Net(_));

            if object_type.is_none() {
                tracing::warn!(
                    ?taker,
                    ?object,
                    "dropping deferred pickup because the object has no object data"
                );
                continue;
            }

            match object_type {
                Some(_) if is_landed_net => {
                    self.unapply_net_effect(object);
                    if taker_is_pc {
                        self.increase_ammo_and_enable(
                            assets,
                            taker,
                            crate::profiles::Action::Net,
                            1,
                        );
                    }
                    self.remove_entity(object);
                }
                // Scroll — PC click-to-take path.  Flips `taken`,
                // sets status to Opened, forces the BonusThree
                // sprite row, then (when a script is bound) invokes
                // `IScrollScript::IsTaken(pc)` on the bound class.
                // When the script returns non-zero the status
                // advances to Taken; otherwise it rests at Opened.
                Some(crate::element::ObjectType::Scroll) => {
                    self.take_scroll(sim, assets, taker, object);
                }
                Some(obj_type) if taker_is_pc => {
                    // Snapshot the object's position/layer/quantity/
                    // associated-action before mutating the engine.
                    let Some(obj_entity) = self.get_entity(object) else {
                        continue;
                    };
                    let obj_data = obj_entity.object_data();
                    let (quantity, assoc_action) = match obj_data {
                        Some(o) => (o.quantity, o.associated_action),
                        None => continue,
                    };
                    let elem = obj_entity.element_data();
                    let (bx, by, blayer) =
                        (elem.position_map().x, elem.position_map().y, elem.layer());
                    self.apply_pc_take_object(
                        assets,
                        taker,
                        object,
                        obj_type,
                        assoc_action,
                        quantity,
                        bx,
                        by,
                        blayer,
                    );
                }
                Some(crate::element::ObjectType::Purse)
                | Some(crate::element::ObjectType::Coin) => {
                    // NPC soldier picking up a dropped purse/coin:
                    // add the money to the soldier's purse and
                    // remove the element.  PCs went through the
                    // branch above.
                    let value = match object_type {
                        Some(crate::element::ObjectType::Purse) => {
                            crate::inventory::COINS_PER_PURSE as u32 * crate::inventory::COIN_VALUE
                        }
                        Some(crate::element::ObjectType::Coin) => crate::inventory::COIN_VALUE,
                        _ => 0,
                    };
                    if value > 0 {
                        if let Some(entity) = self.world.entities.get_mut(taker)
                            && let Some(npc) = entity.npc_data_mut()
                        {
                            npc.money = npc.money.saturating_add(value);
                        }
                        // Deactivate the object (clearing `active`
                        // is our equivalent of unlinking from the
                        // engine's active-element list).
                        if let Some(entity) = self.world.entities.get_mut(object) {
                            entity.element_data_mut().active = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn drain_drink_done(&mut self, assets: &LevelAssets, drink_done: Vec<EntityId>) {
        // DRINKING_ALE TERMINATED — add the profile's beer value
        // to the soldier's blood alcohol (clamped to 100).
        // `blood_alcohol` lives on the `AiController` attached to
        // the soldier's NPC data via `ai_brain`; `profile.beer` is
        // the per-profile increment (see profiles.rs).
        for soldier in drink_done {
            let Some(profile_idx) = self
                .world
                .entities
                .get(soldier)
                .and_then(Entity::soldier_data)
                .map(|soldier| soldier.soldier_profile_index)
            else {
                tracing::warn!(
                    ?soldier,
                    "dropping deferred DrinkAle completion because its owner is missing or is not a soldier"
                );
                continue;
            };
            let Some(profile) = assets.profile_manager.get_soldier(profile_idx) else {
                tracing::warn!(
                    ?soldier,
                    ?profile_idx,
                    "dropping deferred DrinkAle completion because its soldier profile is missing"
                );
                continue;
            };
            let beer = profile.beer;
            if beer == 0 {
                continue;
            }
            let Some(base) = self
                .world
                .entities
                .get_mut(soldier)
                .and_then(Entity::npc_data_mut)
                .and_then(|npc| npc.ai_brain.base_mut())
            else {
                tracing::warn!(
                    ?soldier,
                    "dropping deferred DrinkAle completion because its soldier has no AI controller"
                );
                continue;
            };
            let new_val = (base.blood_alcohol as u16 + beer).min(100);
            base.blood_alcohol = new_val as u8;
        }
    }

    pub(super) fn drain_pickpockets(&mut self, pickpockets: Vec<(EntityId, EntityId)>) {
        // SEARCHING DONE — NPC-on-NPC pickpocket money transfer:
        // thief.money += victim.money; victim.money = 0.
        for (thief, victim) in pickpockets {
            let Some(stolen) = self
                .world
                .entities
                .get(victim)
                .and_then(|e| e.npc_data())
                .map(|n| n.money)
            else {
                tracing::warn!(
                    ?thief,
                    ?victim,
                    "dropping deferred pickpocket because its victim is missing or is not an NPC"
                );
                continue;
            };
            if stolen == 0 {
                continue;
            }

            if self
                .world
                .entities
                .get(thief)
                .and_then(|entity| entity.npc_data())
                .is_none()
            {
                tracing::warn!(
                    ?thief,
                    ?victim,
                    "dropping deferred pickpocket because its thief is missing or is not an NPC"
                );
                continue;
            }

            self.world
                .entities
                .get_mut(victim)
                .and_then(Entity::npc_data_mut)
                .expect("validated deferred pickpocket victim disappeared")
                .money = 0;
            let thief_money = &mut self
                .world
                .entities
                .get_mut(thief)
                .and_then(Entity::npc_data_mut)
                .expect("validated deferred pickpocket thief disappeared")
                .money;
            *thief_money = thief_money.saturating_add(stolen);
        }
    }

    pub(super) fn drain_wasp_sting_remark(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        wasp_sting_remark: Vec<EntityId>,
    ) {
        // GETTING_FREE_FROM_WASP START — `Say(REMARK_WASP_STING)`.
        // Plain `say` on the AI base.
        for speaker in wasp_sting_remark {
            if let Some(entity) = self.world.entities.get_mut(speaker)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                base.say(crate::ai::Remark::WaspSting);
            }
            self.drain_ai_owner_work_for(sim, assets, speaker);
        }
    }

    pub(super) fn drain_special_remark(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        special_remark: Vec<EntityId>,
    ) {
        // SPECIAL START — `make_special_action_remark`.  Branches
        // on `IsShieldBearer`: shield-bearers always speak,
        // everyone else only speaks at 1-in-3 odds and only when
        // currently silent.  `IsShieldBearer` = sword is a shield
        // weapon AND the sprite has the `WaitingShield` animation —
        // the same two-gate check used by the per-tick
        // FighterSnapshot build (engine/ai/snapshots.rs:619-632).
        for speaker in special_remark {
            // Two-step: read weapon/sprite info immutably, then
            // dispatch the remark mutably.  Splitting avoids holding
            // an immutable borrow on `self.world.entities` across the
            // mutable `npc.ai_brain.enemy_mut()` call.
            let is_shield_bearer = self
                .world
                .entities
                .get(speaker)
                .map(|entity| {
                    let hth_weapon_id = entity
                        .npc_data()
                        .and_then(|npc| npc.ai_brain.enemy())
                        .map(|e| e.hth_weapon_id)
                        .unwrap_or(0);
                    let weapon_is_shield = assets
                        .profile_manager
                        .get_hth_weapon(hth_weapon_id)
                        .map(|w| w.shield)
                        .unwrap_or(false);
                    let has_shield_anim = entity
                        .element_data()
                        .sprite
                        .has_animation(crate::order::OrderType::WaitingShield);
                    weapon_is_shield && has_shield_anim
                })
                .unwrap_or(false);
            if let Some(entity) = self.world.entities.get_mut(speaker)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(enemy) = npc.ai_brain.enemy_mut()
            {
                enemy.make_special_action_remark(sim, is_shield_bearer);
            }
            self.drain_ai_owner_work_for(sim, assets, speaker);
        }
    }

    pub(super) fn drain_cry_for_help_under_net(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        cry_for_help_under_net: Vec<EntityId>,
    ) {
        // LYING_STUCK_UNDER_NET 1/31 cycle — NPCs say
        // `UnderNet` (soldier) or `CivUnderNet` (civilian) plus a
        // HEEELP noise at the entity's 2D position (volume
        // `NOISE_VOLUME_HEEELP`, = 200).
        for speaker in cry_for_help_under_net {
            let (remark, origin, layer, elevation) = {
                let Some(entity) = self.world.entities.get(speaker) else {
                    continue;
                };
                let is_soldier = matches!(entity, Entity::Soldier(_));
                let remark = if is_soldier {
                    crate::ai::Remark::UnderNet
                } else {
                    crate::ai::Remark::CivUnderNet
                };
                let elem = entity.element_data();
                let pos3d = elem.position();
                (
                    remark,
                    elem.position_map(),
                    elem.layer(),
                    pos3d.z.max(0.0) as u16,
                )
            };
            if let Some(entity) = self.world.entities.get_mut(speaker)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                base.say(remark);
            }
            self.drain_ai_owner_work_for(sim, assets, speaker);
            self.broadcast_noise_synchronously(
                sim,
                assets,
                crate::ai::NoiseType::Heeelp,
                origin,
                layer,
                crate::parameters_ai::NOISE_VOLUME_HEEELP as u16,
                elevation,
                Some(speaker),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ActorSoldier, ElementData, ElementKind, Posture};

    fn test_soldier() -> Entity {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                posture: Posture::Upright,
                ..Default::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    }

    #[test]
    fn deferred_pickpocket_keeps_victim_money_when_thief_disappears() {
        let mut engine = EngineInner::new();
        let thief = engine.add_entity(test_soldier());
        let victim = engine.add_entity(test_soldier());
        engine
            .get_entity_mut(victim)
            .and_then(Entity::npc_data_mut)
            .expect("test victim NPC")
            .money = 125;
        engine.remove_entity(thief);

        engine.drain_pickpockets(vec![(thief, victim)]);

        assert_eq!(
            engine
                .get_entity(victim)
                .and_then(Entity::npc_data)
                .expect("test victim NPC")
                .money,
            125,
            "a stale deferred thief must not destroy the victim's money"
        );
    }
}
