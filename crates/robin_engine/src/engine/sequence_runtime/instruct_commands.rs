use super::*;

impl EngineInner {
    pub(super) fn instruct_shoot_bow(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        cmd: Command,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        let shoot_once = cmd == Command::ShootBowOnce;
        // Postponed elements are instructed and translated again
        // when their injury successor releases them. A cross-sequence
        // resume refreshes that element to Todo before this dispatch,
        // so the exact execution ownership is the stable discriminator:
        // clear Rust's matching latch so the shot cannot reject its own
        // retranslation as a second concurrent shot.
        bow_shot::clear_matching_retranslated_shot(
            &mut self.world.entities,
            owner,
            seq_id,
            elem_idx,
        );
        let antagonist = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        let target = match antagonist {
            Some(t) => t,
            None => {
                // No target — nothing we can do.
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            }
        };
        // Original rejects zero ammo here only for PCs.
        // Scripted NPC shots remain valid with an empty
        // counter (the release build's later decrement
        // saturates it at zero).
        let ammo_count = self.get_bow_ammo_count(owner);
        let owner_is_pc = self.get_entity(owner).is_some_and(|entity| entity.is_pc());
        if owner_is_pc && ammo_count == 0 {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }

        // Determine shoot mode via
        // `can_shoot_with_bow_at` before
        // beginning the shot.
        let (bow_target, shoot_mode) = self.can_shoot_with_bow_at(assets, owner, target);
        if bow_target != super::input::BowTarget::Valid {
            tracing::debug!(
                ?owner,
                ?target,
                ?bow_target,
                "ShootBow body rejected after preserving its transition prefix"
            );

            // Human instruction handling generates the action
            // transition before command translation checks
            // bow-target validation. An out-of-range or
            // obstructed shot therefore still equips and
            // loads the bow, then completes normally with
            // no shoot-body orders. It is not an
            // Impossible element. This is visible for
            // scripted training shots whose target has
            // moved outside the configured bow range.
            let has_transition_orders = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| !element.orders.is_empty());
            if has_transition_orders {
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);
            } else {
                // Actor instruction stamps
                // mmotionState=IN_PROGRESS before it
                // discovers that Translate produced no
                // current order. The element then
                // terminates, but that acceptance edge
                // remains visible for this frame.
                self.world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::actor_data_mut)
                    .expect("accepted empty ShootBow lost its actor")
                    .continuation
                    .motion_state = crate::sprite::MotionState::InProgress;
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            // Fall through to the shared actor-order
            // publication: the rejected body still leaves
            // an accepted element whose transition prefix
            // becomes the actor's current order this same
            // frame.
        } else {
            match bow_shot::begin_bow_shot(
                &mut self.world.entities,
                &mut self.orders.sequence_manager,
                owner,
                target,
                seq_id,
                elem_idx,
                shoot_once,
                ammo_count,
                Some(shoot_mode),
                &mut self.orders.next_order_id,
            ) {
                BeginShotResult::Started => {
                    self.orders
                        .sequence_manager
                        .element_in_progress(seq_id, elem_idx);
                }
                BeginShotResult::Impossible => {
                    self.orders
                        .sequence_manager
                        .element_impossible(seq_id, elem_idx);
                }
            }
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_change_position(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            layer: _,
            sector,
            direction,
            ..
        } = &elem.data
        {
            let dest = *destination;
            let tgt_sector = *sector;
            let tgt_direction = *direction;

            // Verify actor is in expected sector
            let actor_sector = self
                .get_entity(owner)
                .and_then(|e| e.element_data().sector());

            if tgt_sector.is_some() && actor_sector != tgt_sector {
                self.orders.sequence_manager.element_interrupted(
                    seq_id,
                    elem_idx,
                    crate::sequence::CascadeFlags::NEXT_LEVEL,
                );
                return OwnerActionBarrier::Skip;
            }

            self.finalize_special_move_position(
                assets,
                owner,
                super::special_motion::SpecialMovePosition::Map(dest),
                // The encoded topology is the expected
                // source, not a destination assignment.
                // The original game only changes the map point here.
                None,
                None,
                // Original-game position changes keep the current
                // obstacle/plane and recomputes the 3D
                // position against it. This matters for
                // geometry-less building sectors, whose
                // elevation comes from the plane selected
                // while entering the building.
                None,
                "ChangePosition",
            );
            if let Some(entity) = self.world.entities.get_mut(owner) {
                // instant direction assignment from the
                // element's direction field so a
                // ChangePosition can rotate the
                // actor in the same step.
                entity
                    .element_data_mut()
                    .set_direction_instantly(tgt_direction);
            }
        }
        self.orders
            .sequence_manager
            .element_terminated(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_swordstrike_thrust_a(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        let strike = match elem.command {
            Command::SwordstrikeThrustA => crate::weapons::SwordStrike::A,
            Command::SwordstrikeThrustB => crate::weapons::SwordStrike::B,
            Command::SwordstrikeThrustC => crate::weapons::SwordStrike::C,
            Command::SwordstrikeThrustD => crate::weapons::SwordStrike::D,
            Command::SwordstrikeThrustE => crate::weapons::SwordStrike::E,
            Command::SwordstrikeThrustF => crate::weapons::SwordStrike::F,
            Command::SwordstrikeThrustG => crate::weapons::SwordStrike::G,
            Command::SwordstrikeThrustH => crate::weapons::SwordStrike::H,
            Command::SwordstrikeThrustI => crate::weapons::SwordStrike::I,
            _ => unreachable!(),
        };
        let target = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        // A strike whose translation ends in
        // RHSEQ_IMPOSSIBLE detaches the selected sequence element
        // through removal notification, so Original's
        // Instruction handling returns at its identity-change test
        // and never reaches the
        // motion is marked in progress
        // epilogue.
        let barrier = match target {
            Some(target_id) => {
                self.dispatch_sword_strike(assets, owner, target_id, strike, seq_id, elem_idx)
            }
            None => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                OwnerActionBarrier::Skip
            }
        };
        if barrier == OwnerActionBarrier::Skip {
            return OwnerActionBarrier::Skip;
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_raise_shield(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        cmd: Command,
    ) -> OwnerActionBarrier {
        let follow_up = crate::engine::melee::ShieldCommandContext::new(
            &mut self.world.entities,
            &mut self.orders.sequence_manager,
            &mut self.orders.next_order_id,
        )
        .dispatch(owner, cmd, seq_id, elem_idx);
        if cmd == Command::RaiseShieldInstantly {
            // Human-actor translation performs
            // shield updates immediately after entering
            // HOLDING_SHIELD.
            self.refresh_retained_shield_obstacle(assets, owner);
        }
        if let Some(follow_up) = follow_up {
            // Player-character raise-shield translation
            // launches this SEEK synchronously. Route it
            // through the full owned-element instruction path
            // before the action-loop splice below.
            self.launch_element(follow_up);
        }
        // Human command translation terminates a redundant
        // RAISE_SHIELD synchronously when the actor is
        // already holding it. Actor instruction handling then sees
        // that translation changed the selected sequence element and
        // returns before its accepted-instruct
        // MOTION_IN_PROGRESS epilogue. Preserve the
        // preceding Execute edge (for example LookLeft's
        // START) while still registering the PC's
        // synchronous protectee Seek above.
        let barrier = if cmd == Command::RaiseShield
            && self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_none_or(|element| element.state != crate::sequence::SequenceState::InProgress)
        {
            OwnerActionBarrier::Skip
        } else {
            OwnerActionBarrier::Reach
        };
        if barrier == OwnerActionBarrier::Skip {
            return OwnerActionBarrier::Skip;
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_hide_behind_shield(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        let antagonist = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        let posture_after = elem.posture_after_transition;
        let Some(holder) = antagonist else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let (is_holding, holder_protected) = self
            .get_entity(holder)
            .map(|e| {
                let h = e
                    .actor_data()
                    .map(|a| a.action_state.is_shield())
                    .unwrap_or(false);
                let p = e.pc_data().and_then(|pc| pc.shield_protected);
                (h, p)
            })
            .unwrap_or((false, None));
        if !is_holding || holder_protected.is_some() {
            self.orders.sequence_manager.element_interrupted(
                seq_id,
                elem_idx,
                crate::sequence::CascadeFlags::NEXT_LEVEL,
            );
            return OwnerActionBarrier::Skip;
        }
        if posture_after != crate::element::Posture::Crouched {
            let id = self.orders.allocate_order_id();
            let mut order = crate::order::Order::new(
                crate::order::OrderType::TransitionCrouchingDown,
                0.0,
                0.0,
                id,
            );
            order.compute_direction = false;
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
        }
        let id = self.orders.allocate_order_id();
        let mut order =
            crate::order::Order::new(crate::order::OrderType::HidingBehindShield, 0.0, 0.0, id)
                .with_antagonist(holder);
        order.compute_direction = false;
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_swordstrike_down(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        let antagonist = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        let Some(target) = antagonist else {
            tracing::warn!(?seq_id, elem_idx, "SwordstrikeDown missing antagonist");
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let (tx, ty) = match (self.get_entity(owner), self.get_entity(target)) {
            (Some(_), Some(target_entity)) => {
                let target_pos = target_entity.element_data().position_map();
                (target_pos.x, target_pos.y)
            }
            _ => {
                tracing::warn!(?owner, ?target, "SwordstrikeDown owner or target missing");
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            }
        };
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.clear_path();
        }
        let mut order = crate::order::Order::new(
            crate::order::OrderType::StrikingDownSword,
            tx,
            ty,
            self.orders.allocate_order_id(),
        )
        .with_antagonist(target);
        order.compute_direction = false;
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_get_killed_at_bottom(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        let killer = match elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => antagonist,
            _ => None,
        };
        let already_in_coma = self
            .get_entity(owner)
            .and_then(crate::element::Entity::pc_data)
            .and_then(|pc| self.pc_description_for_pc_data(pc))
            .is_some_and(|description| description.status.in_coma);
        let (damage, raw_life_points_after, died) = {
            let Some(victim) = self.world.entities.get_mut(owner) else {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            };
            let damage = victim
                .human_and_life_points_mut()
                .map(|(_, lp)| (*lp).max(0) as u16);
            let Some(damage) = damage else {
                tracing::warn!(?owner, ?killer, "GetKilledAtBottom owner is not a human");
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            };
            let max_life_points = match victim {
                crate::element::Entity::Pc(_) => crate::combat::LIFEPOINTS_PC,
                crate::element::Entity::Soldier(s) => s.soldier.cached_max_life_points,
                crate::element::Entity::Civilian(_) => 100,
                _ => 100,
            };
            let (_, lp) = victim
                .human_and_life_points_mut()
                .expect("validated GetKilledAtBottom human lost life points");
            // Player-character wounding drops the entire
            // lethal branch when the PC is already in coma.
            // GET_KILLED_AT_BOTTOM still says ouch and
            // terminates its element, but cannot subtract the
            // protected five-point coma floor.
            let died = if !already_in_coma {
                crate::combat::get_wounded(lp, damage, false, max_life_points, false)
            } else {
                false
            };
            (damage, *lp, died)
        };

        // The original game queries wounded state here. A
        // VIP PC therefore establishes its amulet coma
        // inside this command before the posture/death
        // translation continues.
        let coma_saved = self.close_pc_wounded_coma_boundary(
            sim,
            assets,
            owner,
            damage,
            damage as i16,
            raw_life_points_after,
        );
        if died
            && !coma_saved
            && self
                .get_entity(owner)
                .is_some_and(crate::element::Entity::is_dead)
        {
            // The original game's wounded-state query routes through specialized
            // Human life updates, which invoke the complete
            // PC/NPC/Soldier/Human Kill chain before returning.
            self.apply_scripted_virtual_kill(sim, assets, owner, killer);
            if self.get_entity(owner).is_some_and(|victim| {
                victim.is_soldier()
                    && self.camps_are_hostile(victim.camp(), crate::element::Camp::Royalists)
            }) {
                const SCORE_SOLDIER_KILLED_DURING_FIGHT: i32 = 50;
                self.add_campaign_value(
                    crate::campaign::CampaignValue::Score,
                    SCORE_SOLDIER_KILLED_DURING_FIGHT,
                );
            }
        }
        self.say_ouch(sim, assets, owner, Some(damage));

        let victim = self
            .world
            .entities
            .get_mut(owner)
            .expect("GetKilledAtBottom owner vanished after wounding");
        let is_rider = matches!(
            victim,
            crate::element::Entity::Soldier(s) if s.soldier.rider
        );
        if is_rider {
            let anim = victim
                .actor_data()
                .map(|actor| {
                    let action_state = actor.action_state;
                    if action_state.is_sword()
                        || action_state == crate::element::ActionState::Menacing
                    {
                        crate::order::OrderType::DyingSword
                    } else if action_state.is_bow() {
                        crate::order::OrderType::DyingBow
                    } else {
                        crate::order::OrderType::DyingUpright
                    }
                })
                .unwrap_or(crate::order::OrderType::DyingUpright);
            self.push_new_order(seq_id, elem_idx, anim, 0.0, 0.0);
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
        } else {
            if victim.is_dead() {
                victim.set_posture(crate::element::Posture::DeadBack);
            }
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_take_corpse(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        let target = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        match target {
            Some(target_id) => {
                match abilities::begin_carry(
                    &mut self.world.entities,
                    &mut self.orders.sequence_manager,
                    owner,
                    target_id,
                    seq_id,
                    elem_idx,
                    &mut self.orders.next_order_id,
                ) {
                    AbilityBeginResult::Started => {
                        self.orders
                            .sequence_manager
                            .element_in_progress(seq_id, elem_idx);
                        // Freezing the target and
                        // starting its hulk belong to
                        // the pickup order's first
                        // Execute, not to translation:
                        // the carrier's slot for this
                        // frame has already run, so the
                        // body keeps its own selected
                        // order until the carrier
                        // actually begins lifting it.
                    }
                    AbilityBeginResult::Impossible => {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                    }
                }
            }
            None => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_drop_corpse(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        match abilities::begin_drop(
            &mut self.world.entities,
            &mut self.orders.sequence_manager,
            owner,
            seq_id,
            elem_idx,
            &mut self.orders.next_order_id,
        ) {
            AbilityBeginResult::Started => {
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);
                // Drop-transition init twin of
                // the pickup building flash.
                let carried_id = self
                    .get_entity(owner)
                    .and_then(|e| e.pc_data())
                    .and_then(|pc| pc.carried);
                if let Some(cid) = carried_id {
                    // Re-freeze the carried on
                    // drop init.  The victim is
                    // normally already frozen
                    // from the carry, but this
                    // idempotently re-runs the
                    // cascade-interrupt so any
                    // element that slipped onto
                    // the carried (e.g. a
                    // script-driven
                    // `ActionChange`) is
                    // interrupted.
                    self.actor_freeze_execution(cid);
                    self.apply_carry_building_hulk(owner, cid);
                }
            }
            AbilityBeginResult::Impossible => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_hit_cmd(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        cmd: Command,
    ) -> OwnerActionBarrier {
        let Some(target) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Interaction { antagonist, .. } => *antagonist,
                _ => None,
            })
        else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let begin = match cmd {
            Command::HitCmd => abilities::begin_hit(
                &mut self.world.entities,
                &mut self.orders.sequence_manager,
                owner,
                target,
                seq_id,
                elem_idx,
                &mut self.orders.next_order_id,
            ),
            Command::StrangleCmd => abilities::begin_strangle(
                &mut self.world.entities,
                &mut self.orders.sequence_manager,
                owner,
                target,
                seq_id,
                elem_idx,
                &mut self.orders.next_order_id,
            ),
            _ => unreachable!(),
        };
        match begin {
            AbilityBeginResult::Impossible => self
                .orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx),
            AbilityBeginResult::Started => {
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);

                // Human command translation inserts the Hit/Strangle order before
                // a moving antagonist's Think(EVENT_STOP). Think and all
                // of its re-entrant effects finish in this stack frame,
                // before Perform's initialization acquires AILOCK_FREEZE.
                let moving = self
                .get_entity(target)
                .unwrap_or_else(|| {
                    panic!("Hit/Strangle victim {target:?} vanished after translation for {seq_id:?}/{elem_idx}")
                })
                .actor_data()
                .unwrap_or_else(|| {
                    panic!(
                        "Hit/Strangle victim {target:?} lost actor state after translation"
                    )
                })
                .action_state
                .is_moving();
                if moving {
                    self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                        sim,
                        target,
                        assets,
                        crate::ai::Stimulus::new(crate::ai::StimulusType::EventStop),
                    );
                }
            }
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_tie_cmd(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        cmd: Command,
    ) -> OwnerActionBarrier {
        let ammo_available = match cmd {
            Command::HealCmd => self.has_ammo(owner, crate::profiles::Action::Heal),
            Command::EatCmd => self
                .get_entity(owner)
                .and_then(|entity| match entity {
                    Entity::Pc(pc) => self.pc_description_for_pc_data(&pc.pc),
                    _ => None,
                })
                .is_some_and(|description| {
                    description.status.get_ammo(crate::profiles::Action::Eat) > 0
                }),
            Command::ThrowNet => self.has_ammo(owner, crate::profiles::Action::Net),
            Command::ThrowPurse => self.has_ammo(owner, crate::profiles::Action::Purse),
            Command::ThrowWaspNest => self.has_ammo(owner, crate::profiles::Action::WaspNest),
            Command::ThrowApple => self.has_ammo(owner, crate::profiles::Action::Apple),
            Command::ThrowStone => self.has_ammo(owner, crate::profiles::Action::Stone),
            _ => true,
        };
        let barrier = DirectAbilityCommandContext {
            entities: &mut self.world.entities,
            sequence_manager: &mut self.orders.sequence_manager,
            next_order_id: &mut self.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, cmd, ammo_available, seq_id, elem_idx);
        if barrier == OwnerActionBarrier::Skip {
            return OwnerActionBarrier::Skip;
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_climb_up_on_shoulders(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        // Owner is the climber, antagonist is the
        // HelpingToClimb helper.
        let helper = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        let Some(helper_id) = helper else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        // Disjoint-field obstacle list so the headroom
        // ray-cast inside `begin_climb_on_shoulders`
        // can run alongside the `&mut self.world.entities`
        // borrow.
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.environment.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        match abilities::begin_climb_on_shoulders(
            &mut self.world.entities,
            &mut self.orders.sequence_manager,
            owner,
            helper_id,
            seq_id,
            elem_idx,
            &mut self.orders.next_order_id,
            obstacles,
        ) {
            crate::abilities::ClimbResult::Started => {
                self.orders
                    .sequence_manager
                    .element_in_progress(seq_id, elem_idx);
            }
            crate::abilities::ClimbResult::Impossible => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
            crate::abilities::ClimbResult::NoHeadroom { helper_id } => {
                // Low ceiling → helper stands
                // back up (LeaveHelpingClimb) and
                // the climber's element is
                // Impossible.
                let leave_elem = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::LeaveHelpingClimb,
                    Some(helper_id),
                );
                self.launch_element(leave_elem);
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_pay(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        // Validate campaign has enough ransom.
        // The original aborts with the post-walk
        // validity check if ransom dropped
        // mid-sequence.  We pre-check on launch;
        // a race where ransom becomes
        // insufficient between the click and the
        // animation is acceptable (next frame's
        // completion handler would just not
        // deduct — see PayDone branch).
        let beggar = match &elem.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
            _ => None,
        };
        match beggar {
            Some(beggar_id) => {
                match abilities::begin_pay(
                    &mut self.world.entities,
                    &mut self.orders.sequence_manager,
                    owner,
                    beggar_id,
                    seq_id,
                    elem_idx,
                    &mut self.orders.next_order_id,
                ) {
                    AbilityBeginResult::Started => {
                        self.orders
                            .sequence_manager
                            .element_in_progress(seq_id, elem_idx);
                    }
                    AbilityBeginResult::Impossible => {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                    }
                }
            }
            None => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
        }
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_drop_ammo(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("instructed sequence element disappeared");
        // Decrement the PC's ammo for the action,
        // then either merge into an adjacent
        // just-dropped bonus (same action,
        // combined quantity ≤ 5) or spawn a fresh
        // `ElementBonus` at the PC's position.
        // We skip the TAKING animation frames
        // (the original plays a taking animation
        // during the drop) and apply the effect
        // in one step — the observable result is
        // the same: ammo goes down, a bonus
        // appears.
        //
        // Merge gate: when the PC hasn't moved or
        // turned since its last drop AND the
        // previous bonus is still active AND same
        // action AND combined quantity ≤
        // `MAX_AMMO_PER_PILE`, the existing pile's
        // quantity is bumped; otherwise a fresh
        // bonus spawns. When the previous bonus is
        // still active but the merge cap is reached
        // (or it's a different action), the PC's
        // facing rotates +1 sector so the next
        // drop's "same direction" check fails and a
        // fresh pile spawns again.
        const MAX_AMMO_PER_PILE: u16 = 5;
        let (action_id, amount) = match &elem.data {
            crate::sequence::SequenceElementData::Generic { properties } => {
                let a = properties
                    .get(&crate::sequence::Field::ActionId)
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Integer(n) => Some(*n),
                        _ => None,
                    });
                let q = properties
                    .get(&crate::sequence::Field::Amount)
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Integer(n) => Some(*n),
                        _ => None,
                    });
                (a, q)
            }
            _ => (None, None),
        };
        let Some(action_id) = action_id else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let requested = amount.unwrap_or(1) as u16;
        let action = crate::profiles::Action::from_u32(action_id);
        // `get_ammo` returns `u16::MAX` (0xFFFF)
        // for actions without an ammo counter
        // (pc_status.rs:368-386), so
        // `!action_uses_ammo` is the equivalent
        // sentinel test.  Treat this as terminate,
        // not impossible.
        if !crate::inventory::action_uses_ammo(action) {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }
        // Refuse the drop when no walkable cell
        // exists near the PC's hand: skip the
        // `DROPPING_AMMO[_CROUCHED]` order and
        // terminate.
        if self.try_get_drop_position(owner).is_none() {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }
        // Capture PC
        // position/layer/sector/obstacle for the
        // spawned bonus.
        let pc_snap = self.get_entity(owner).map(|e| {
            let el = e.element_data();
            (
                el.position_map(),
                el.layer(),
                el.sector(),
                el.obstacle_index(),
                el.direction(),
                el.material(),
            )
        });
        let Some((pos, layer, sector, obstacle, direction, material)) = pc_snap else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        // Decrement PC ammo, clamped to current
        // count.
        let status_idx = self.get_entity(owner).and_then(|e| match e {
            crate::element::Entity::Pc(pc) => self.pc_description_index_for_pc_data(&pc.pc),
            _ => None,
        });
        let Some(status_idx) = status_idx else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let dropped = if let Some(campaign) = Some(&mut self.mission_domain.campaign)
            && let Some(pc_desc) = campaign.characters.get_mut(status_idx)
        {
            let current = pc_desc.status.get_ammo(action);
            let take = requested.min(current);
            pc_desc.status.decrease_ammo(action, take);
            take
        } else {
            0
        };
        if dropped == 0 {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }
        // Auto-disable the action slot when ammo
        // reaches 0.  `dropped` was clamped to the
        // available amount so "now empty" is
        // detectable by re-reading.
        let now_empty = Some(&self.mission_domain.campaign)
            .and_then(|c| c.characters.get(status_idx))
            .map(|d| d.status.get_ammo(action) == 0)
            .unwrap_or(false);
        if now_empty {
            self.disable_pc_action(assets, owner, action);
        }

        // Merge into the previously-dropped pile if
        // PC hasn't moved/turned and the previous
        // bonus is still alive and accepts more.
        let prev = self.get_entity(owner).and_then(|e| match e {
            crate::element::Entity::Pc(pc) => Some((
                pc.pc.last_dropped_ammo,
                pc.pc.last_ammo_dropping_position,
                pc.pc.last_dropping_direction,
            )),
            _ => None,
        });
        let same_position_and_direction = prev
            .map(|(_, last_pos, last_dir)| {
                last_pos.x == pos.x && last_pos.y == pos.y && last_dir as i16 == direction
            })
            .unwrap_or(false);
        // `prev_bonus_state`: Some((id, current_quantity, action))
        // if a previous pile is still active.
        let prev_bonus_state = prev.and_then(|(last, _, _)| last).and_then(|last_id| {
            self.get_entity(last_id).and_then(|e| match e {
                crate::element::Entity::Bonus(b) if b.element.active => {
                    Some((last_id, b.object.quantity, b.object.associated_action))
                }
                _ => None,
            })
        });
        let merged = if same_position_and_direction
            && let Some((last_id, prev_qty, prev_action)) = prev_bonus_state
            && prev_action == action
            && prev_qty + dropped <= MAX_AMMO_PER_PILE
        {
            if let Some(crate::element::Entity::Bonus(b)) = self.world.entities.get_mut(last_id) {
                b.object.quantity = prev_qty + dropped;
            }
            tracing::debug!(
                pc = ?owner,
                ?action,
                dropped,
                bonus = ?last_id,
                new_qty = prev_qty + dropped,
                "DropAmmo: merged into previous bonus"
            );
            true
        } else {
            false
        };

        // When the previous bonus is still alive
        // but we couldn't merge into it (cap reached
        // or different action), rotate the PC by
        // +1 sector so the next drop spawns fresh.
        // Only fires if the PC hadn't moved/turned
        // — otherwise the merge gate would already
        // have rejected next time.
        let bumped_direction =
            if !merged && same_position_and_direction && prev_bonus_state.is_some() {
                let new_dir = (direction + 1).rem_euclid(16);
                if let Some(entity) = self.world.entities.get_mut(owner) {
                    entity.element_data_mut().set_direction_instantly(new_dir);
                }
                new_dir
            } else {
                direction
            };

        let spawned_id = if !merged {
            // Spawn a fresh bonus at the PC's
            // position, refined via
            // `find_authorized_position` to nudge
            // it onto a walkable cell.
            let spawn_pos = {
                let mut b = crate::coordinates::MapBBox::new();
                b.expand_point(pos);
                if self
                    .world
                    .fast_grid
                    .find_authorized_position_toward(&mut b, pos, layer)
                {
                    b.center()
                } else {
                    pos
                }
            };
            let object_type = crate::inventory::action_to_object_type(action);
            let mut bonus_element = {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ObjectBonus;
                initial_element.active = true;
                // Bonus default: blipped iff this
                // isn't a forest level.
                initial_element.blipped = !self.world.weather.is_forest_level;
                initial_element
            };
            bonus_element.sprite.apply_placement(
                spawn_pos,
                layer,
                sector,
                bumped_direction,
                material,
                obstacle,
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    obstacle,
                    assets.environment.static_sight_obstacles.as_slice(),
                ),
            );
            let bonus = crate::element::Entity::Bonus(crate::element::ElementBonus {
                element: bonus_element,
                object: crate::element::ObjectData {
                    quantity: dropped,
                    object_type,
                    associated_action: action,
                    ..Default::default()
                },
            });
            let bonus_id = self.add_entity(bonus);
            tracing::debug!(
                pc = ?owner,
                ?action,
                dropped,
                ?bonus_id,
                "DropAmmo: decremented PC ammo and spawned bonus"
            );
            Some(bonus_id)
        } else {
            None
        };

        // Stamp the per-PC drop trackers so the
        // next drop's merge gate evaluates against
        // this drop.
        if let Some(crate::element::Entity::Pc(pc)) = self.world.entities.get_mut(owner) {
            pc.pc.last_ammo_dropping_position = pos;
            pc.pc.last_dropping_direction = bumped_direction as u8;
            if let Some(new_id) = spawned_id {
                pc.pc.last_dropped_ammo = Some(new_id);
            }
        }

        self.orders
            .sequence_manager
            .element_terminated(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    pub(super) fn instruct_unlock_door(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let id = required_unlock_door_id(
            self.orders.sequence_manager.get_element(seq_id, elem_idx),
            seq_id,
            elem_idx,
        );
        // Pick UnlockingDoor vs UnlockingTrap
        // by door type.
        let anim_type = match required_canonical_door(
            &self.script_domains.interactables.doors,
            id,
            "UnlockDoor dispatch",
        )
        .door_type
        {
            crate::gate::DoorType::BuildingTrap => crate::order::OrderType::UnlockingTrap,
            _ => crate::order::OrderType::UnlockingDoor,
        };
        tracing::debug!(
            door_id = %id,
            entity = ?owner,
            ?anim_type,
            "UnlockDoor: starting lockpick animation"
        );
        let order = crate::order::Order::new(anim_type, 0.0, 0.0, self.orders.allocate_order_id())
            .with_completion(crate::order::OrderCompletion::UnlockDoor { door_id: id });
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }
}
