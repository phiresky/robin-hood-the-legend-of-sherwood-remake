//! Actor execution operations. Each call finishes before its Execute arm continues.

use super::*;
use crate::engine::sequence_runtime::required_canonical_door_mut;

impl EngineInner {
    pub(in crate::engine) fn execute_waiting_upright(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        let owner = effect;
        let soldier = self.expect_entity(owner, "WaitingUpright owner");
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
            // As in the symmetric
            // alerted-waiting repair below, launch the sequence element
            // directly: enabling attentive mode would suppress the repair
            // precisely because desired attentiveness is already true.
            self.launch_element(
                sim,
                assets,
                crate::sequence::SequenceElement::new(1, Command::EnterAttentiveMode, Some(owner)),
            );
        }
    }

    pub(in crate::engine) fn execute_waiting_alerted(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        let owner = effect;
        let soldier = self.expect_entity(owner, "WaitingAlerted owner");
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
            // This is deliberately not
            // disabling attentive mode: that helper suppresses a request when
            // desired attentiveness is already false, while this corrective
            // Execute arm exists specifically for that inconsistent state.
            self.launch_element(
                sim,
                assets,
                crate::sequence::SequenceElement::new(1, Command::LeaveAttentiveMode, Some(owner)),
            );
        }

        // A soldier playing the
        // non-sword WAITING_ALERTED animation must not still be linked
        // into a swordfight. The shipped game unconditionally tears
        // the relationship down.
        let still_swordfighting = !self
            .expect_entity(owner, "WaitingAlerted owner")
            .human_data()
            .unwrap_or_else(|| panic!("WaitingAlerted soldier {owner:?} is not human"))
            .opponents
            .is_empty();
        if still_swordfighting {
            self.quit_swordfight(sim, assets, owner);
        }
    }

    pub(in crate::engine) fn execute_non_interruptable_lifts(
        &mut self,
        effect: (crate::sequence::SequenceId, usize),
    ) {
        let (seq_id, elem_idx) = effect;
        self.orders.sequence_manager.set_element_priority(
            seq_id,
            elem_idx,
            crate::sequence::SequencePriority::NonInterruptable,
        );
    }

    pub(in crate::engine) fn execute_corpse_drop_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        // Player-character execution drops the corpse from inside the terminal
        // transition branch, before returning TERMINATED to the actor update and
        // therefore before order advancement exposes a following command.
        let carrier_id = effect;
        let selected_order = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, carrier_id)
            .map(|(_, _, order)| order.order_type)
            .unwrap_or_else(|| {
                panic!(
                    "corpse-drop transition owner {carrier_id:?} lost its selected terminal order"
                )
            });
        assert_eq!(
            selected_order,
            crate::order::OrderType::TransitionCarryingCorpseWaitingUpright,
            "corpse-drop side effect must run before order advancement exposes a successor"
        );
        crate::abilities::sync_terminal_corpse_drop_animation(
            &mut self.world.entities,
            &assets.profile_manager,
            carrier_id,
        );
        let (target_id, drop_posture, carrier_pos, carrier_direction) = {
            let carrier = self.expect_entity(carrier_id, "corpse-drop transition owner");
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
            sim,
            assets,
            carrier_id,
            target_id,
            drop_posture,
            carrier_pos,
            carrier_direction,
        );
    }

    pub(in crate::engine) fn execute_seq_advance(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (crate::sequence::SequenceId, usize),
    ) {
        let (seq_id, elem_idx) = effect;
        // `do_next_order` semantics: pop the just-completed
        // order; advance to the next if one exists, otherwise
        // terminate the element.
        self.do_next_order(sim, assets, seq_id, elem_idx);
    }

    pub(in crate::engine) fn execute_wasp_next_cycle(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (crate::sequence::SequenceId, usize, u16),
    ) {
        // Wasp struggle-cycle refill: push a fresh `GettingFreeFromWasp`
        // order with the decremented counter, then pop the current one
        // via `do_next_order` so the new order takes over cleanly.
        let (seq_id, elem_idx, cycles_remaining) = effect;
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
        self.do_next_order(sim, assets, seq_id, elem_idx);
    }

    pub(in crate::engine) fn execute_seq_terminate(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (crate::sequence::SequenceId, usize),
    ) {
        let (seq_id, elem_idx) = effect;
        self.element_terminated(sim, assets, &mut Vec::new(), seq_id, elem_idx);
    }

    pub(in crate::engine) fn execute_play_anim_frozen(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, u16, crate::order::OrderType),
    ) {
        let (actor, command_level, anim) = effect;
        let mut elem = crate::sequence::SequenceElement::new_generic(
            command_level,
            crate::element::Command::PlayAnimFrozen,
            Some(actor),
        );
        elem.set_property(
            crate::sequence::Field::AnimationId,
            crate::sequence::FieldValue::Animation(anim),
        );
        self.launch_element(sim, assets, elem);
    }

    pub(in crate::engine) fn execute_seq_impossible(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (crate::sequence::SequenceId, usize),
    ) {
        let (seq_id, elem_idx) = effect;
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
            // Push-damage translation accidentally authors an
            // end-of-animation stand-up order for a conscious,
            // crouched-family stunning push. Base actor action execution returns
            // ABORTED for that unknown action; the release build then
            // sets even this NonInterruptable injury Impossible and
            // synchronously releases its postponed successor.
            self.element_impossible_from_execute(sim, assets, &mut Vec::new(), seq_id, elem_idx);
        } else {
            self.element_impossible(sim, assets, &mut Vec::new(), seq_id, elem_idx);
        }
    }

    pub(in crate::engine) fn execute_unlock_door_done(&mut self, effect: crate::gate::DoorIndex) {
        let door_id = effect;
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

    pub(in crate::engine) fn execute_next_jump_step(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        let entity_id = effect;
        if let Some((new_layer, new_sector, projection_point)) =
            self.advance_jump_step(sim, assets, entity_id)
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

    pub(in crate::engine) fn execute_select_hulk(&mut self, effect: (EntityId, f32)) {
        let (entity_id, speed) = effect;
        self.apply_select_hulk(entity_id, speed);
    }

    pub(in crate::engine) fn execute_resume_door_pass(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        use super::movement::DoorPassAdvance;

        let entity_id = effect;
        let Some(action) = self
            .get_entity(entity_id)
            .and_then(|entity| entity.actor_data())
            .and_then(|actor| actor.active_door_pass.as_ref())
            .map(|pass| pass.current_action)
        else {
            return;
        };
        self.apply_door_pass_transition_completion_side_effects(assets, entity_id, action);
        // Materialize one successor. PassingDoor and Select callbacks
        // run only when that successor reaches its own Execute slot.
        let (advance, arrived_movement, completed_pass) = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                return;
            };
            let transition_destination = entity.element_data().position_map();
            let Some(actor) = entity.actor_data_mut() else {
                return;
            };
            let adv = Self::advance_door_pass(
                actor,
                entity_id,
                transition_destination,
                &mut self.orders.next_order_id,
            );
            // If the door pass is done (no more steps), mirror the
            // arrival teardown performed by the movement tick.
            let arrived = if let DoorPassAdvance::Done { completed } = &adv {
                let selected = actor.selected_sequence_element;
                actor.action_state = if actor.action_state.is_sword() {
                    crate::element::ActionState::WaitingSword
                } else {
                    crate::element::ActionState::Waiting
                };
                actor.active_door_pass = None;
                Some((
                    selected.expect("completed door pass has a selected instruction"),
                    *completed,
                ))
            } else {
                None
            };
            let (arrived, completed) = match arrived {
                Some((am, completed)) => (Some(am), completed),
                None => (None, None),
            };
            (adv, arrived, completed)
        };

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
        if let Some((seq_id, elem_idx)) = self.world.entities.current_element_for_actor(entity_id) {
            match advance.clone() {
                DoorPassAdvance::Continue {
                    order_id,
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
                        order_id,
                        destination,
                        action,
                        reverse,
                        compute_direction,
                        tolerance,
                        None,
                        "PassDoor resumed walk",
                    );
                    self.do_next_order(sim, assets, seq_id, elem_idx);
                }
                DoorPassAdvance::Paused { transition_order } => {
                    self.orders
                        .sequence_manager
                        .push_order_on(seq_id, elem_idx, transition_order);
                    self.do_next_order(sim, assets, seq_id, elem_idx);
                }
                DoorPassAdvance::ActionPoint { order } => {
                    self.orders
                        .sequence_manager
                        .push_order_on(seq_id, elem_idx, order);
                    self.do_next_order(sim, assets, seq_id, elem_idx);
                }
                DoorPassAdvance::NoActive => {
                    tracing::warn!(
                        entity = ?entity_id,
                        "DoorPass: resume callback had no active pass"
                    );
                    self.do_next_order(sim, assets, seq_id, elem_idx);
                }
                DoorPassAdvance::Done { .. } => {}
            }
        }

        // If the door pass completed, notify the sequence manager. Its
        // owner-local condolence drain runs immediately after these
        // outcomes and is the sole EVENT_REACHPOINT owner: in particular,
        // The last-real-action check suppresses the event while AssertPosition /
        // Move followers remain. Dispatching it manually here bypassed
        // that completion gate for translated door routes.
        if let Some(selected) = arrived_movement {
            self.element_terminated(
                sim,
                assets,
                &mut Vec::new(),
                selected.sequence_id,
                selected.element_index,
            );
        }

        let _ = advance;
    }

    pub(in crate::engine) fn execute_drop_ale_done(
        &mut self,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        let pc_id = effect;
        let action = crate::profiles::Action::Ale;
        let (position, layer, sector, obstacle, direction, material, status_idx) = {
            let pc = self.expect_entity(pc_id, "DropAle DONE PC");
            let position = pc.current_gameplay_point_map().unwrap_or_else(|| {
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

        // Ale position copying copies the actor placement
        // exactly; unlike cursor authorization, the action point does not
        // search for a nearby walkable position.
        let mut ale_element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectOther;
            initial_element.active = true;
            // Ale-element creation constructs its object without minimap display.
            initial_element.blipped = false;
            initial_element
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
                assets.environment.static_sight_obstacles.as_slice(),
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
        if self
            .world
            .entities
            .get(pc_id)
            .and_then(Entity::pc_data)
            .is_some_and(|pc| pc.kind == Some(crate::character_kind::CharacterKind::FriarTuck))
        {
            self.mission_domain.achievements.record_tuck_beer(ale_id);
        }
        // Ale creation clones the ACCESSORIES_Ale master before
        // lying-object animation assignment, whose forced restart resets the
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
            // Decreasing player-character ammo uses its default
            // speech enabled here: after disabling the emptied action it
            // queues HERO_OUT_OF_AMMO, except on the Sherwood hub map.
            // Keep this after bottle creation/detection and inventory
            // consumption, matching the ale-dropping animation's completed arm.
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

    pub(in crate::engine) fn execute_pc_bow_equip_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        let pc_id = effect;
        // Human-actor execution forwards this synchronously from
        // the TransitionEquipBow START arm after setting AimingWithBow.
        // An unselected PC only restores its remembered action; a
        // selected PC also restores the messenger-global action.
        self.set_pc_action_from_message(sim, assets, 0, pc_id, crate::profiles::Action::Bow);
    }

    pub(in crate::engine) fn execute_pc_bow_unequip_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, bool),
    ) {
        let (pc_id, script_driven) = effect;
        // the human actor's unequip-bow transition start arm
        // (PC branch): an empty quiver disables the Bow action outright
        // (regardless of the script flag); otherwise non-script elements
        // forward MSG_UNSELECT_ACTION(BOW).
        if self.get_pc_ammo_count(pc_id, crate::profiles::Action::Bow) == 0 {
            self.disable_pc_action(assets, pc_id, crate::profiles::Action::Bow);
        } else if !script_driven {
            // Messenger preprocessing for MSG_UNSELECT_ACTION drops the
            // message unless the unselected action is the messenger's
            // currently selected action; it then clears that selection
            // and action deselection clears the PC's remembered
            // action (the freshly-Waiting action state means no further
            // cleanup sequence is launched).
            if self.players.seats[0].selected_action == crate::profiles::Action::Bow {
                self.players.seats[0].selected_action = crate::profiles::Action::NoAction;
                self.unselect_action(sim, assets, pc_id);
            }
        }
    }

    pub(in crate::engine) fn execute_pc_helping_climb_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        let pc_id = effect;
        // Player-character execution forwards the help-to-climb selection message
        // straight after state assignment on the DONE edge of the helping-climb
        // entry transition. HelpToClimb is already the current action, but
        // a selected PC still goes through the action-reselection Stop at
        // Normal priority, which interrupts whatever the entry transition
        // postponed behind itself — the move the player queued while the
        // PC was kneeling down never resumes.
        self.set_pc_action_from_message(
            sim,
            assets,
            0,
            pc_id,
            crate::profiles::Action::HelpToClimb,
        );
    }

    pub(in crate::engine) fn execute_hidden_titbit_removals(&mut self, effect: EntityId) {
        let entity_id = effect;
        self.feedback.titbit_manager.remove_titbit(
            crate::titbit::TitbitKind::Hidden,
            crate::titbit::ElementHandle(entity_id.index()),
        );
    }

    pub(in crate::engine) fn execute_beggar_wait_handoffs(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, bool),
    ) {
        let (pc_id, entering) = effect;
        // Wait registers against the still-executing transition. The following
        // action selection can synchronously stop that postponed wait.
        self.actor_wait(sim, assets, pc_id);
        if entering {
            self.set_pc_action_from_message(sim, assets, 0, pc_id, crate::profiles::Action::Beggar);
        } else if self.players.seats[0].selection.contains(&pc_id) {
            // Leaving forwards MSG_UNSELECT_ACTION(BEGGAR) for a
            // selected PC. The messenger drops the message unless Beggar
            // is still its selected action. A newer action (for example
            // Net selected while the exit transition was running) must
            // survive both the transition and this callback.
            if self.players.seats[0].selected_action == crate::profiles::Action::Beggar {
                self.players.seats[0].selected_action = crate::profiles::Action::NoAction;
                self.unselect_action(sim, assets, pc_id);
            }
        } else if let Some(pc) = self
            .get_entity_mut(pc_id)
            .and_then(|entity| entity.pc_data_mut())
        {
            pc.current_action = crate::profiles::Action::NoAction;
        }
    }

    pub(in crate::engine) fn execute_beggar_coin_flags(
        &mut self,
        assets: &LevelAssets,
        effect: (EntityId, bool),
    ) {
        let (pc_id, enabled) = effect;
        super::beggar::set_flags_of_near_coins_on_ground(&mut self.world.entities, pc_id, enabled);
        if enabled {
            super::beggar::add_beggar_for_all_intelligent_seeking_soldiers(
                &mut self.world.entities,
                &assets.profile_manager,
                &self.mission_domain.diplomacy,
                pc_id,
                self.control.sim_config.difficulty,
            );
        }
    }

    pub(in crate::engine) fn execute_smalltalk_strikes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, EntityId, crate::weapons::SwordStrike),
    ) {
        let (actor_id, target_id, strike) = effect;
        let wound_target = {
            let attacker = self.expect_entity(actor_id, "smalltalk attacker");
            let target = self.expect_entity(target_id, "smalltalk antagonist");
            // Original builds this relative vector from
            // ground position, i.e. the stored world X/Y pair.
            // Projected map Y differs by elevation, and using it here can
            // flip the back-hit half-plane test when the fighters stand
            // at different heights.
            let attacker_pos = attacker.ground_position();
            let target_pos = target.ground_position();
            // Element direction-vector lookup returns a vector in the
            // isometric map plane.  Smalltalk's "striking in the back"
            // dot product therefore needs the aspect-scaled Y component;
            // the ordinary unit-circle helper can flip this half-plane
            // test and suppress the ensuing sword-damage RNG draws.
            let [dx, dy] =
                crate::position_interface::sector_to_vector_iso(target.element_data().direction());
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
            return;
        }

        let (position, weapon1) = {
            let entity = self.expect_entity(actor_id, "smalltalk attacker");
            let target_mutual = self
                .get_entity(target_id)
                .and_then(|e| e.human_data())
                .and_then(|h| h.opponents.first().copied())
                .map(|id| id == actor_id)
                .unwrap_or(false);
            if !target_mutual {
                return;
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

    pub(in crate::engine) fn execute_killed_at_bottom(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, EntityId),
    ) {
        let (victim_id, killer_id) = effect;
        let mut elem = crate::sequence::SequenceElement::new_interaction(
            1,
            crate::element::Command::GetKilledAtBottom,
            Some(victim_id),
            Some(killer_id),
        );
        elem.priority = crate::sequence::SequencePriority::Lethal;
        self.launch_element(sim, assets, elem);
    }

    pub(in crate::engine) fn execute_deactivate_entities(&mut self, effect: EntityId) {
        // DRINKING_ALE DONE — deactivate the antagonist to hide
        // the ale bottle.
        let antag = effect;
        if let Some(entity) = self.world.entities.get_mut(antag) {
            entity.element_data_mut().active = false;
        }
    }

    pub(in crate::engine) fn execute_pc_target_activations(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, EntityId, Command),
    ) {
        let (pc, target, activation_cmd) = effect;
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
            return;
        }
        let mut activation = crate::sequence::SequenceElement::new(1, activation_cmd, Some(target));
        activation.data = crate::sequence::SequenceElementData::Interaction {
            antagonist: Some(pc),
        };
        self.launch_element(sim, assets, activation);
    }

    pub(in crate::engine) fn execute_waking_up_done(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, EntityId),
    ) {
        let (rescuer, target) = effect;
        let target_entity = self.world.entities.expect_entity(
            target,
            format_args!("WakingUp DONE from rescuer {rescuer:?} required target"),
        );
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
            // Updating human posture calls
            // corpse-intersection updates synchronously. Keep this
            // cross-owner WAKING_UP write distinct from the target's
            // later actor slot: a recovering target can enter Lying here
            // and leave it again on StandingUp's START edge in that slot.
            // Deferring both writes to the owner-tail sampler would see
            // only Upright -> Upright and lose both corpse callbacks.
            self.process_corpse_intersection_update_for(target);
            self.apply_concussion(sim, assets, target, 0, false);
            // Concussion handling synchronously sends FITAGAIN from
            // the WakingUp DONE stack. This AI consequence is immediate
            // even when the target's creation-ordered actor slot has
            // already passed; only its next animation Execute is delayed.

            // Original-game completed waking makes the target wait
            // unconditionally. That launches a fresh priority-Wait
            // element even while the old unconscious Wait is live, so
            // ordinary equal-priority arbitration replaces and
            // retranslates it immediately as StandingUp.
            self.actor_wait(sim, assets, target);
        }

        if target_is_pc {
            self.hero_speaking(assets, target, crate::engine::melee::HERO_RECOVER);
        }
    }

    pub(in crate::engine) fn execute_taking_net_ticks(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: crate::engine::animation::TakingNetTick,
    ) {
        use crate::coordinates::MapVec;
        use crate::element::Animation;
        use crate::order::OrderType;

        let tick = effect;
        if tick.action_done {
            let (seq_id, elem_idx) = self
                .world
                .entities
                .current_element_for_actor(tick.taker)
                .unwrap_or_else(|| {
                    panic!("TakingNet taker {:?} lost its live element", tick.taker)
                });
            let valid = {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .expect("TakingNet live element disappeared");
                self.check_sequence_element_validity(assets, tick.taker, element, true)
            };
            if !valid {
                self.get_entity_mut(tick.taker)
                    .and_then(Entity::actor_data_mut)
                    .expect("TakingNet invalidation lost taker actor data")
                    .continuation
                    .motion_state = crate::sprite::MotionState::Aborted;
                self.element_impossible_from_execute(
                    sim,
                    assets,
                    &mut Vec::new(),
                    seq_id,
                    elem_idx,
                );
                return;
            }
            let taker_point = self
                .expect_entity(tick.taker, "TakingNet taker")
                .current_gameplay_point_map()
                .unwrap_or_else(|| {
                    panic!(
                        "TakingNet taker {:?} has no current action point",
                        tick.taker
                    )
                });
            let (net_position, crumpled, duration) = {
                let net = self.expect_entity(tick.net, "TakingNet antagonist");
                let Entity::Net(net) = net else {
                    panic!("TakingNet antagonist {:?} is not a net", tick.net);
                };
                let animation = if net.net.crumpled {
                    OrderType::NetBeingTakenCrumpled
                } else {
                    OrderType::NetBeingTaken
                };
                (
                    net.element.position_map(),
                    net.net.crumpled,
                    net.element.sprite.gameplay_time_for_anim(animation),
                )
            };
            assert!(
                duration != 0,
                "TakingNet antagonist {:?} has no pickup animation",
                tick.net
            );
            let dx = taker_point.x - net_position.x;
            let dy = taker_point.y - net_position.y;
            let length = (dx * dx + dy * dy).sqrt();
            let radius = if crumpled { 7.0 } else { 40.0 };
            let increment = if length == 0.0 {
                MapVec::ZERO
            } else {
                let scale = radius / f32::from(duration) / length;
                MapVec::new(dx * scale, dy * scale)
            };
            let Entity::Net(net) = self
                .get_entity_mut(tick.net)
                .expect("validated TakingNet antagonist disappeared")
            else {
                unreachable!()
            };
            let animation = if crumpled {
                Animation::NetBeingTakenCrumpled
            } else {
                Animation::NetBeingTaken
            };
            net.object.animation = animation;
            // Updating object animation immediately forces the sprite
            // animation, so the first snapshot on the
            // DONE edge already exposes frame zero of the pickup row.
            net.element.sprite.force_animation(animation, 0);
            net.element
                .sprite
                .position_iface
                .set_map_increment(increment);
            let actor = self
                .get_entity_mut(tick.taker)
                .and_then(Entity::actor_data_mut)
                .expect("validated TakingNet taker lost actor data");
            actor.wait_time = 8;
            actor.seek_refresh_wait = 8;
        }

        // Actor action execution observes the order's done flag before the enclosing
        // hourglass marks the order done. Consequently the DONE edge only
        // initializes the tail; pulling begins on the following owner
        // slot, and removal happens one slot after the counter reaches
        // zero.
        if tick.order_was_done {
            let wait = self
                .get_entity(tick.taker)
                .and_then(Entity::actor_data)
                .expect("TakingNet taker lost actor data")
                .wait_time;
            if wait == 0 {
                let taker_is_pc = self
                    .get_entity(tick.taker)
                    .expect("TakingNet taker disappeared before removal")
                    .is_pc();
                // Removing a net unlinks it from active element lists
                // but deliberately keeps the object alive because
                // orders may still reference it. Preserve
                // that stable entity slot and only deactivate it here.
                self.get_entity_mut(tick.net)
                    .expect("TakingNet net disappeared during deactivation")
                    .element_data_mut()
                    .active = false;
                self.unapply_net_effect(sim, assets, tick.net);
                if taker_is_pc {
                    self.increase_ammo_and_enable(
                        assets,
                        tick.taker,
                        crate::profiles::Action::Net,
                        1,
                    );
                }
                let actor = self
                    .get_entity_mut(tick.taker)
                    .and_then(Entity::actor_data_mut)
                    .expect("TakingNet taker disappeared after removal");
                actor.wait_time = u32::from(u16::MAX);
                actor.seek_refresh_wait = actor.wait_time;
            } else {
                let remaining = wait - 1;
                let actor = self
                    .get_entity_mut(tick.taker)
                    .and_then(Entity::actor_data_mut)
                    .expect("TakingNet taker disappeared during pull");
                actor.wait_time = remaining;
                actor.seek_refresh_wait = remaining;
                let net = self.expect_entity_mut(tick.net, "TakingNet net during pull");
                net.position_iface_mut().update_position_map_scaled(1.0);
            }
        }
    }

    pub(in crate::engine) fn execute_pickups(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: (EntityId, EntityId),
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
        //   the scroll script's `IsTaken` callback.
        let (taker, object) = effect;
        let Some(taker_entity) = self.world.entities.get(taker) else {
            tracing::warn!(
                ?taker,
                ?object,
                "dropping deferred pickup because its taker no longer exists"
            );
            return;
        };
        let taker_is_pc = taker_entity.is_pc();
        if !taker_is_pc && !taker_entity.is_npc() {
            tracing::warn!(
                ?taker,
                ?object,
                "dropping deferred pickup because its taker is not an actor"
            );
            return;
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
            return;
        };
        let is_scroll = matches!(object_entity, crate::element::Entity::Scroll(_));
        if is_scroll {
            self.scroll_is_taken(sim, assets, object, taker);
            return;
        }

        let object_type = object_entity.object_data().map(|o| o.object_type);
        // Original's special net pickup arm is selected by the
        // net element. An inventory bonus element whose object
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
            return;
        }

        match object_type {
            Some(_) if is_landed_net => {
                self.unapply_net_effect(sim, assets, object);
                if taker_is_pc {
                    self.increase_ammo_and_enable(assets, taker, crate::profiles::Action::Net, 1);
                }
                self.remove_entity(object);
            }
            // Scroll — PC click-to-take path.  Flips `taken`,
            // sets status to Opened, forces the BonusThree
            // sprite row, then (when a script is bound) invokes
            // `IsTaken(pc)` on the bound script class.
            // When the script returns non-zero the status
            // advances to Taken; otherwise it rests at Opened.
            Some(crate::element::ObjectType::Scroll) => {
                self.take_scroll(sim, assets, taker, object);
            }
            Some(obj_type) if taker_is_pc => {
                // Snapshot the object's position/layer/quantity/
                // associated-action before mutating the engine.
                let Some(obj_entity) = self.get_entity(object) else {
                    return;
                };
                let obj_data = obj_entity.object_data();
                let (quantity, assoc_action) = match obj_data {
                    Some(o) => (o.quantity, o.associated_action),
                    None => return,
                };
                let elem = obj_entity.element_data();
                let (bx, by, blayer) = (elem.position_map().x, elem.position_map().y, elem.layer());
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
            Some(crate::element::ObjectType::Purse) | Some(crate::element::ObjectType::Coin) => {
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

    pub(in crate::engine) fn execute_drink_done(
        &mut self,
        assets: &LevelAssets,
        effect: (EntityId, Option<EntityId>),
    ) {
        // DRINKING_ALE TERMINATED — add the profile's beer value
        // to the soldier's blood alcohol (clamped to 100).
        // `blood_alcohol` lives on the `AiController` attached to
        // the soldier's NPC data via `ai_brain`; `profile.beer` is
        // the per-profile increment (see profiles.rs).
        let (soldier, bottle) = effect;
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
            return;
        };
        let Some(profile) = assets.profile_manager.get_soldier(profile_idx) else {
            tracing::warn!(
                ?soldier,
                ?profile_idx,
                "dropping deferred DrinkAle completion because its soldier profile is missing"
            );
            return;
        };
        let beer = crate::gameplay_config::effective_ale_potency(
            profile.beer,
            profile.vip,
            self.control
                .sim_config
                .item_gameplay
                .ale_reliable_distraction,
        );
        if beer == 0 {
            return;
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
            return;
        };
        let new_val = (base.blood_alcohol as u16 + beer).min(100);
        base.blood_alcohol = new_val as u8;
        if let Some(bottle) = bottle {
            let hostile = self
                .world
                .entities
                .get(soldier)
                .and_then(Entity::soldier_data)
                .is_some_and(|s| self.is_hostile_to_player_camp(s.cached_camp));
            if hostile {
                self.mission_domain
                    .achievements
                    .record_beer_drunk(soldier, bottle);
            }
        }
    }

    pub(in crate::engine) fn execute_pickpockets(&mut self, effect: (EntityId, EntityId)) {
        // SEARCHING DONE — NPC-on-NPC pickpocket money transfer:
        // thief.money += victim.money; victim.money = 0.
        let (thief, victim) = effect;
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
            return;
        };
        if stolen == 0 {
            return;
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
            return;
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

    pub(in crate::engine) fn execute_wasp_sting_remark(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        // GETTING_FREE_FROM_WASP START — `Say(REMARK_WASP_STING)`.
        let speaker = effect;
        self.mission_domain
            .achievements
            .complete_wasp_sting(speaker);
        if self
            .world
            .entities
            .get(speaker)
            .and_then(Entity::ai_controller)
            .is_some()
        {
            self.execute_ai_speech(
                sim,
                assets,
                speaker,
                crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::WaspSting,
                    flags: 0,
                },
            );
        }
    }

    pub(in crate::engine) fn execute_special_remark(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        // Shield bearers speak unconditionally. Other soldiers draw only
        // while silent, before entering the synchronous speech operation.
        let speaker = effect;
        if self
            .world
            .entities
            .get(speaker)
            .and_then(Entity::enemy_ai)
            .is_none()
        {
            return;
        }
        let flags = if self.live_ai_is_shield_bearer(assets, speaker) {
            crate::ai::SpeechFlags::ALWAYS.bits()
        } else {
            let silent = self
                .world
                .entities
                .expect_ai_controller(speaker, format_args!("special action speaker"))
                .current_remark
                == crate::ai::Remark::TheSoundOfSilence;
            if !silent
                || crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SpecialActionRemark, 0..3) != 0
            {
                return;
            }
            0
        };
        self.execute_ai_speech(
            sim,
            assets,
            speaker,
            crate::ai::AiSpeechAttempt {
                remark: crate::ai::Remark::SpecialAction,
                flags,
            },
        );
    }

    pub(in crate::engine) fn execute_cry_for_help_under_net(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        effect: EntityId,
    ) {
        // LYING_STUCK_UNDER_NET 1/31 cycle — NPCs say
        // `UnderNet` (soldier) or `CivUnderNet` (civilian) plus a
        // HEEELP noise at the entity's 2D position (volume
        // `NOISE_VOLUME_HEEELP`, = 200).
        let speaker = effect;
        let (remark, origin, layer) = {
            let Some(entity) = self.world.entities.get(speaker) else {
                return;
            };
            let is_soldier = matches!(entity, Entity::Soldier(_));
            let remark = if is_soldier {
                crate::ai::Remark::UnderNet
            } else {
                crate::ai::Remark::CivUnderNet
            };
            let elem = entity.element_data();
            (remark, elem.position_map(), elem.layer())
        };
        if self
            .world
            .entities
            .get(speaker)
            .and_then(Entity::ai_controller)
            .is_some()
        {
            self.execute_ai_speech(
                sim,
                assets,
                speaker,
                crate::ai::AiSpeechAttempt { remark, flags: 0 },
            );
        }
        // The origin is captured before speaking; elevation is read
        // after speech, which can synchronously run a rejection callback.
        let elevation = self
            .world
            .entities
            .expect_entity(speaker, format_args!("under-net noise speaker"))
            .element_data()
            .position()
            .z
            .max(0.0) as u16;
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Heeelp,
            origin,
            crate::position_interface::Layer::new(layer),
            crate::parameters_ai::NOISE_VOLUME_HEEELP as u16,
            elevation,
            Some(speaker),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorSoldier, ElementData, ElementKind, ElementNet, NetData, ObjectData, ObjectType,
        Posture, ProjectileData, SoldierData,
    };

    fn test_soldier() -> Entity {
        Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: crate::element::SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
        })
    }

    #[test]
    fn deferred_pickpocket_keeps_victim_money_when_thief_disappears() {
        let mut engine = EngineInner::new();
        let thief = engine.add_test_entity(test_soldier());
        let victim = engine.add_test_entity(test_soldier());
        engine
            .get_entity_mut(victim)
            .and_then(Entity::npc_data_mut)
            .expect("test victim NPC")
            .money = 125;
        engine.remove_entity(thief);

        engine.execute_pickpockets((thief, victim));

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

    #[test]
    fn reliable_zero_beer_completion_uses_minimum_potency_and_classic_uses_zero() {
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .soldiers
            .push(crate::profiles::SoldierProfile {
                beer: 0,
                vip: false,
                ..Default::default()
            });
        let mut engine = EngineInner::new();
        let mut soldier = test_soldier();
        let Entity::Soldier(actor) = &mut soldier else {
            unreachable!()
        };
        actor.soldier = SoldierData {
            soldier_profile_index: crate::profiles::SoldierProfileIdx(0),
            cached_camp: crate::element::Camp::Lacklandists,
            ..Default::default()
        };
        actor.npc.ai.ai_brain =
            crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi::new(0)));
        let soldier_id = engine.add_test_entity(soldier);

        engine.execute_drink_done(&assets, (soldier_id, None));
        assert_eq!(
            engine
                .get_entity(soldier_id)
                .and_then(Entity::npc_data)
                .and_then(|npc| npc.ai_brain.base())
                .expect("test soldier AI")
                .blood_alcohol,
            0
        );

        engine
            .control
            .sim_config
            .item_gameplay
            .ale_reliable_distraction = true;
        engine.execute_drink_done(&assets, (soldier_id, None));
        assert_eq!(
            engine
                .get_entity(soldier_id)
                .and_then(Entity::npc_data)
                .and_then(|npc| npc.ai_brain.base())
                .expect("test soldier AI")
                .blood_alcohol,
            crate::gameplay_config::REBALANCED_ALE_MIN_POTENCY as u8
        );
    }

    #[test]
    fn taking_net_tail_pulls_eight_ticks_then_removes_on_ninth() {
        let sim_context = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let taker = engine.add_test_entity(test_soldier());
        let mut net_element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectNet;
            initial_element.active = true;
            initial_element
        };
        net_element.set_position_map(crate::coordinates::MapPoint::new(0.0, 0.0));
        net_element
            .sprite
            .position_iface
            .set_map_increment(crate::coordinates::MapVec::new(1.0, 0.0));
        let net = engine.add_test_entity(Entity::Net(ElementNet {
            element: net_element,
            object: ObjectData {
                object_type: ObjectType::Net,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
            net: NetData::default(),
        }));
        let actor = engine
            .get_entity_mut(taker)
            .and_then(Entity::actor_data_mut)
            .expect("test taker actor data");
        actor.wait_time = 8;
        actor.seek_refresh_wait = 8;

        let tick = crate::engine::animation::TakingNetTick {
            taker,
            net,
            action_done: false,
            order_was_done: true,
        };
        let assets = LevelAssets::new();
        for expected_remaining in (0..8).rev() {
            engine.execute_taking_net_ticks(&sim_context, &assets, tick);
            assert!(engine.get_entity(net).is_some());
            assert_eq!(
                engine
                    .get_entity(taker)
                    .and_then(Entity::actor_data)
                    .unwrap()
                    .wait_time,
                expected_remaining
            );
        }
        assert_eq!(
            engine
                .get_entity(net)
                .unwrap()
                .element_data()
                .position_map()
                .x,
            8.0
        );

        engine.execute_taking_net_ticks(&sim_context, &assets, tick);
        assert!(
            !engine
                .get_entity(net)
                .expect("removed net allocation remains referenceable")
                .is_active()
        );
        assert_eq!(
            engine
                .get_entity(taker)
                .and_then(Entity::actor_data)
                .unwrap()
                .wait_time,
            u32::from(u16::MAX)
        );
    }
}
