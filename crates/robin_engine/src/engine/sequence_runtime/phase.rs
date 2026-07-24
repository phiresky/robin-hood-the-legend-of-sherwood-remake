use super::*;

impl EngineInner {
    /// Translate one Move/Seek at the exact `RHSequenceManager::Hourglass`
    /// FIFO position where its `Go()` action was emitted.
    fn dispatch_ordered_move_seek_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
    ) {
        let Some((command, stored_destination, target_element, action, flags, tolerance)) = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement {
                    destination,
                    element: target,
                    action,
                    flags,
                    tolerance,
                    ..
                } if matches!(element.command, Command::Move | Command::Seek) => Some((
                    element.command,
                    *destination,
                    *target,
                    *action,
                    *flags,
                    *tolerance,
                )),
                _ => None,
            })
        else {
            tracing::warn!(
                ?sequence_id,
                element_index,
                "Move/Seek action has invalid sequence-element data"
            );
            self.orders
                .sequence_manager
                .element_impossible(sequence_id, element_index);
            return;
        };

        let is_anonymous_archer_pc = self.get_entity(owner).is_some_and(|entity| {
            entity.is_pc()
                && entity.element_data().posture == crate::element_kinds::Posture::AnonymousArcher
        });
        if is_anonymous_archer_pc {
            self.hero_speaking(
                assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            self.orders
                .sequence_manager
                .element_impossible(sequence_id, element_index);
            return;
        }

        let is_seek = command == Command::Seek;
        let destination = if is_seek {
            let post_seek = self
                .orders
                .sequence_manager
                .get_element_mut(sequence_id, element_index)
                .and_then(|element| match &mut element.data {
                    crate::sequence::SequenceElementData::Movement {
                        post_seek_sequence, ..
                    } => post_seek_sequence.take(),
                    _ => None,
                });
            if let Some(post_seek) = post_seek
                && let Some(actor) = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
            {
                actor.post_seek_sequence = Some(post_seek);
            }

            match target_element {
                Some(target) => {
                    if target == owner {
                        self.orders
                            .sequence_manager
                            .element_terminated(sequence_id, element_index);
                        self.start_post_seek_sequence(owner, None);
                        return;
                    }
                    if self.try_handle_same_sector_actor_seek_wait(
                        owner,
                        sequence_id,
                        element_index,
                        target,
                        flags,
                    ) {
                        return;
                    }
                    let seek_distance = tolerance.max(4.0);
                    if self.try_dispatch_cross_sector_entity_seek(
                        sim,
                        assets,
                        owner,
                        sequence_id,
                        element_index,
                        target,
                        action,
                        flags,
                        seek_distance,
                    ) {
                        return;
                    }
                    let Some(resolved) =
                        self.resolve_entity_seek(owner, target, flags, seek_distance)
                    else {
                        self.orders
                            .sequence_manager
                            .element_impossible(sequence_id, element_index);
                        return;
                    };
                    if let Some(crate::sequence::SequenceElementData::Movement {
                        destination,
                        tolerance,
                        speed_factor,
                        ..
                    }) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(sequence_id, element_index)
                        .map(|element| &mut element.data)
                    {
                        *destination = resolved.destination;
                        *tolerance = resolved.tolerance;
                        *speed_factor = resolved.speed_factor;
                    }
                    if let Some(actor) = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(|entity| entity.actor_data_mut())
                    {
                        actor.wait_time = 0;
                        actor.seek_refresh_wait = 25;
                    }
                    if resolved.stop_npc {
                        self.send_seek_stop_to_npc(target);
                    }
                    resolved.destination
                }
                None => {
                    if let Some(actor) = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(|entity| entity.actor_data_mut())
                    {
                        actor.seek_target = None;
                        actor.last_seek_target_position = stored_destination;
                        actor.wait_time = 0;
                        actor.seek_refresh_wait = 25;
                    }
                    stored_destination
                }
            }
        } else {
            stored_destination
        };

        let owner_sector = self
            .get_entity(owner)
            .and_then(|entity| entity.element_data().sector());
        let owner_in_building = self.sector_is_building(owner_sector);
        let is_last_of_sequence = self
            .orders
            .sequence_manager
            .get_sequence(sequence_id)
            .map(|sequence| element_index + 1 >= sequence.elements.len())
            .unwrap_or(false);
        if owner_in_building && (!is_seek || !is_last_of_sequence) {
            self.finalize_special_move_position(
                assets,
                owner,
                super::special_motion::SpecialMovePosition::Map(destination),
                None,
                None,
                Some(destination),
                "building interior move",
            );
            self.orders
                .sequence_manager
                .element_terminated(sequence_id, element_index);
            return;
        }

        self.dispatch_prepared_move_instruction(
            sim,
            assets,
            owner,
            sequence_id,
            element_index,
            destination,
            action,
        );
    }

    /// Launch and dispatch sequence elements after the ported base entity and
    /// actor-Hourglass work, including inline immediate-action cascades and
    /// message/target callbacks at their exact owner-dispatch positions.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3726-3727` calls
    /// `RHSequenceManager::Hourglass` after the entity loop; its FIFO `Go()`
    /// drain is in `original-code/RHsequencemanager.cpp:931-943`.
    pub(in crate::engine) fn hourglass_phase_sequences(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) {
        // An actor order can terminate during the preceding entity phase.
        // Original `SetState` closes its `SendCondolationCard` / `Ready`
        // stack immediately, so a postponed successor is registered before
        // `RHSequenceManager::Hourglass` starts and is instructed by that
        // same drain. Rust defers the callback to avoid re-entrant borrows;
        // close any such pre-existing stacks before collecting manager work.
        //
        // This deliberately does not process paths. A resumed Move/Seek is
        // translated below, after this frame's path phase, and its request
        // remains queued for the next frame just as in the Original.
        self.dispatch_condolations(sim, assets);

        // AI Think reached from an entity/NPC slot can launch GoTo after the
        // pre-entity order drain. Original registers that Move immediately,
        // so the sequence-manager Hourglass below still instructs it in this
        // frame. It reaches pathfinding only at next frame's earlier Paths
        // phase and therefore remains MoveWaiting meanwhile.
        self.drain_pending_move_requests();

        // ── Sequence manager dispatch ────────────────────────────
        // Process pending sequence elements in the manager's emitted order.
        let mut phase = SequencePhase::begin(&mut self.orders);

        // Dispatch each action at its exact FIFO position. In particular,
        // Move/Seek translation must not leap ahead of an earlier script
        // callback in this same batch.
        //
        // Pop actions one at a time and drain any synchronous
        // immediate-dispatch follow-ups produced by cascades inside
        // each action (e.g. an `element_terminated` whose
        // `signal_ready` re-registers the next element which happens
        // to be Speak / Teleport / etc.).  Successors land at the
        // front of the action queue, so they fire before the next
        // non-immediate action in the batch rather than waiting for
        // the next `Hourglass()`.
        while let Some(action) = phase.pop_action() {
            match action {
                crate::sequence::SequenceAction::InstructOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    // Sword damage produced by an actor strike is launched
                    // through the ordinary manager queue so it reaches its
                    // native Instruct boundary only after every actor's
                    // frame. Stamp that deferred element from the victim's
                    // state as it exists now, matching RHElementActor::Instruct.
                    //
                    // Other deferred element families retain their existing
                    // admission behavior; broadening this requires auditing
                    // their distinct native launch paths.
                    let needs_transition = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .is_some_and(|element| {
                            matches!(
                                element.state,
                                crate::sequence::SequenceState::Todo
                                    | crate::sequence::SequenceState::Postponed
                            ) && element.posture_after_transition
                                == crate::element::Posture::Undefined
                        });
                    let is_deferred_sword_damage = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .is_some_and(|element| {
                            element.command == crate::element::Command::ReceiveSwordDamage
                        });
                    if needs_transition && is_deferred_sword_damage {
                        self.stamp_element_transition_state(owner, seq_id, elem_idx);
                    }
                    if !self.arbitrate_instruct(seq_id, elem_idx) {
                        continue;
                    }
                    if needs_transition && !self.generate_transition(owner, seq_id, elem_idx) {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                        continue;
                    }
                    // Skip elements whose state moved to terminal /
                    // interrupted while an earlier action in this batch
                    // arbitrated against them. Without this, the loop
                    // would try to dispatch a non-live element
                    // and hit `set_element_state: Terminated from
                    // illegal state Interrupted`.
                    let cmd = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                        Some(e) => {
                            use crate::sequence::SequenceState;
                            if !matches!(e.state, SequenceState::Todo | SequenceState::Postponed) {
                                continue;
                            }
                            e.command
                        }
                        None => continue,
                    };
                    // Beggar-command filter: reject anything other
                    // than RECEIVE_PURSE / BEGGAR_SHOW_FACE / WAIT on
                    // beggar civilians.
                    if self.beggar_rejects_command(owner, cmd) {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                        continue;
                    }
                    // Posture transitions (leave-disguise, stand-up, …)
                    // are handled before command dispatch: owned
                    // single-element launches do it in
                    // `launch_element_for_owner`, and prebuilt
                    // `launch_sequence` elements do it at this ordered
                    // InstructOwner admission boundary.
                    //
                    // Re-borrow element for data access.
                    let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                        Some(e) => e,
                        None => continue,
                    };
                    // Pre-flight re-validation, humans only — non-
                    // human owners (e.g. script-driven objects) skip
                    // the gate because the validity check only
                    // applies to humans.  Passes `check_position =
                    // true` to match the default at all call sites.
                    let owner_is_human = self
                        .get_entity(owner)
                        .map(|e| e.is_human())
                        .unwrap_or(false);
                    if owner_is_human
                        && !self.check_sequence_element_validity(assets, owner, elem, true)
                    {
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                        continue;
                    }
                    match cmd {
                        Command::Move | Command::Seek => {
                            self.dispatch_ordered_move_seek_instruct(
                                sim, assets, owner, seq_id, elem_idx,
                            );
                        }
                        Command::ShootBow | Command::ShootBowOnce => {
                            let shoot_once = cmd == Command::ShootBowOnce;
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let target = match antagonist {
                                Some(t) => t,
                                None => {
                                    // No target — nothing we can do.
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    continue;
                                }
                            };
                            // Check ammo before starting the shot
                            // (PCs only).  Zero bow ammo → impossible.
                            // Soldiers have unlimited ammo (no ammo
                            // counter).
                            let ammo_count = self.get_bow_ammo_count(owner);
                            if ammo_count == 0 {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            }

                            // Determine shoot mode via
                            // `can_shoot_with_bow_at` before
                            // beginning the shot.
                            let (bow_target, shoot_mode) =
                                self.can_shoot_with_bow_at(assets, owner, target);
                            if bow_target != super::input::BowTarget::Valid {
                                tracing::debug!(
                                    ?owner,
                                    ?target,
                                    ?bow_target,
                                    "ShootBow command rejected during dispatch"
                                );
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            }

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
                        Command::PassDoor => {
                            let barrier = crate::engine::door_pass::PassDoorLaunchContext::new(
                                self.script_domains.interactables.doors.as_slice(),
                                &mut self.world.entities,
                                &self.world.fast_grid,
                                &mut self.orders.sequence_manager,
                                &mut self.orders.next_order_id,
                            )
                            .dispatch(owner, seq_id, elem_idx);
                            if barrier
                                == crate::engine::door_pass::PassDoorLaunchBarrier::SkipSplice
                            {
                                continue;
                            }
                        }
                        // ── CHANGE_POSITION ────────────────────────
                        // Instant teleport to a new position.
                        Command::ChangePosition => {
                            if let crate::sequence::SequenceElementData::Movement {
                                destination,
                                layer,
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
                                    continue;
                                }

                                self.finalize_special_move_position(
                                    assets,
                                    owner,
                                    super::special_motion::SpecialMovePosition::Map(dest),
                                    Some(*layer),
                                    tgt_sector.map(u16::from),
                                    Some(dest),
                                    "ChangePosition",
                                );
                                if let Some(entity) = self.world.entities.get_mut(owner) {
                                    // `SetDirectionInstantly` from the
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
                        }
                        // ── ASSERT_POSITION ────────────────────────
                        // Check actor is at expected position/sector.
                        Command::AssertPosition => {
                            PositionAssertionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                            }
                            .dispatch(owner, seq_id, elem_idx);
                        }
                        // ── WAIT_FREE_LIFT ──────────────────────
                        // Translation is identical to WAIT: book the
                        // stationary actor order and enter InProgress. The
                        // live actor-slot coordinator rechecks/reserves the
                        // lift after each actual Execute, matching
                        // RHElementActor::Hourglass rather than this one-shot
                        // Instruct boundary.
                        Command::WaitFreeLift => {
                            WaitCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(
                                owner,
                                Command::WaitFreeLift,
                                seq_id,
                                elem_idx,
                            );
                        }
                        // ── Sword strike commands ────────────────
                        Command::SwordstrikeThrustA
                        | Command::SwordstrikeThrustB
                        | Command::SwordstrikeThrustC
                        | Command::SwordstrikeThrustD
                        | Command::SwordstrikeThrustE
                        | Command::SwordstrikeThrustF
                        | Command::SwordstrikeThrustG
                        | Command::SwordstrikeThrustH
                        | Command::SwordstrikeThrustI => {
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
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    self.dispatch_sword_strike(
                                        sim, assets, owner, target_id, strike, seq_id, elem_idx,
                                    );
                                }
                                None => {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                }
                            }
                        }

                        // ── Swordfight enter/quit ───────────────
                        Command::EnterSwordfight | Command::PrepareSwordfight => {
                            let opponent = match elem.get_property(crate::sequence::Field::Opponent)
                            {
                                Some(crate::sequence::FieldValue::Element(id)) => Some(*id),
                                _ => None,
                            };
                            self.dispatch_enter_swordfight(
                                sim, assets, owner, opponent, seq_id, elem_idx,
                            );
                        }
                        Command::QuitSwordfight => {
                            self.dispatch_quit_swordfight(sim, assets, owner, seq_id, elem_idx);
                        }

                        // ── Parry commands ──────────────────────
                        Command::ParrySword => {
                            self.dispatch_parry_sword(owner, false, seq_id, elem_idx);
                        }
                        Command::ParrySwordLow => {
                            self.dispatch_parry_sword(owner, true, seq_id, elem_idx);
                        }
                        Command::StopParrySword => {
                            self.dispatch_stop_parry(owner, seq_id, elem_idx);
                        }

                        // ── Damage reception commands ───────────
                        Command::ReceiveSwordDamage
                        | Command::ReceiveDamage
                        | Command::ReceiveArrowDamage
                        | Command::ReceiveStoneDamage
                        | Command::ReceiveHitDamage
                        | Command::ReceiveMobileDamage
                        | Command::ReceiveNet => {
                            self.dispatch_receive_damage(sim, assets, owner, seq_id, elem_idx);
                        }

                        // ── Shoulder-fall sub-sequence ──────────
                        // Launched by `translate_shoulder_damage` on
                        // the carrier/carried partner when shoulder-
                        // damage lands on the other side of the carry.
                        Command::Fall => {
                            self.dispatch_fall(owner, seq_id, elem_idx);
                        }

                        // ── NPC head-turn / lean-out commands ────
                        // Insert a Looking{Left,Right}[Alerted] or
                        // TransitionWaitingAlertedLeaningOut order on
                        // the actor's queue, then stay in-progress
                        // until the sprite reaches DONE.  Terminating
                        // the element immediately (as the code did
                        // before) let `LOOK_LEFT_RIGHT` sequences
                        // advance to the second command before the
                        // first animation ran, so the second booking
                        // overwrote the first and only one of the
                        // two head turns played.
                        Command::LookLeft | Command::LookRight | Command::LeanOut => {
                            let barrier = NpcAttentionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Reach);
                        }

                        // ── Attentive-mode transitions ───────────
                        Command::EnterAttentiveMode
                        | Command::LeaveAttentiveMode
                        | Command::LeaveAttentiveModeOfficer => {
                            let barrier = NpcAttentionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Reach);
                        }

                        // ── Wasp sting ─────────────────────────
                        Command::ReceiveWaspSting => {
                            self.dispatch_receive_wasp_sting(sim, assets, owner, seq_id, elem_idx);
                        }

                        // ── Stealth posture commands ────────────
                        Command::CrouchDown
                        | Command::CrouchUp
                        | Command::EnterBeggar
                        | Command::LeaveBeggar
                        | Command::EnterHelpingClimb
                        | Command::LeaveHelpingClimb
                        | Command::LeaveSpy
                        | Command::LeaveTree => {
                            let barrier = StealthCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                titbit_manager: &mut self.feedback.titbit_manager,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Reach);
                        }

                        // ── Shield commands ─────────────────────
                        Command::RaiseShield
                        | Command::RaiseShieldInstantly
                        | Command::LowerShield
                        | Command::ParryShield => {
                            let follow_up = crate::engine::melee::ShieldCommandContext::new(
                                &mut self.world.entities,
                                &mut self.orders.sequence_manager,
                                &mut self.orders.next_order_id,
                            )
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if let Some(follow_up) = follow_up {
                                // `RHElementActorPC::Translate(RAISE_SHIELD)`
                                // launches this SEEK synchronously. Route it
                                // through the full owned-element Instruct path
                                // before the action-loop splice below.
                                self.launch_element(follow_up);
                            }
                        }
                        // ── Bow equip / raise / lower ───────────
                        //
                        // C++ RHElementActorHuman::Translate appends
                        // these bow animation orders from the command
                        // body itself. Some command profiles may have
                        // already queued transition orders before
                        // translate; when they have not, push the
                        // command's own orders here.
                        Command::EquipBow
                        | Command::EquipBowDown
                        | Command::UnequipBow
                        | Command::RaiseBow
                        | Command::LowerBow => {
                            let barrier = BowTransitionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }
                        // ── Hide behind shield ──────────────────
                        //
                        // 1. Holder must be holding-shield (HOLDING/
                        //    MOVING/PARRYING) AND not currently
                        //    protecting anyone.  Otherwise → INTERRUPTED
                        //    (note: this is stricter than the
                        //    validity gate, which permits
                        //    `holder.shield_protected == self`).
                        // 2. If the element's posture-after-transition is
                        //    not Crouched, prepend a TRANSITION_CROUCHING_DOWN
                        //    order so the actor crouches before hiding.
                        // 3. Push the HIDING_BEHIND_SHIELD non-animation
                        //    order with the shield holder as antagonist.
                        Command::HideBehindShield => {
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let posture_after = elem.posture_after_transition;
                            let Some(holder) = antagonist else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
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
                                continue;
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
                            let mut order = crate::order::Order::new(
                                crate::order::OrderType::HidingBehindShield,
                                0.0,
                                0.0,
                                id,
                            )
                            .with_antagonist(holder);
                            order.compute_direction = false;
                            self.orders
                                .sequence_manager
                                .push_order_on(seq_id, elem_idx, order);
                            self.orders
                                .sequence_manager
                                .element_in_progress(seq_id, elem_idx);
                        }

                        // ── Other sword-related commands ────────
                        Command::SwordstrikeDown => {
                            let antagonist = self
                                .orders
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|elem| match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                });
                            let Some(target) = antagonist else {
                                tracing::warn!(
                                    ?seq_id,
                                    elem_idx,
                                    "SwordstrikeDown missing antagonist"
                                );
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let (tx, ty, dir) =
                                match (self.get_entity(owner), self.get_entity(target)) {
                                    (Some(owner_entity), Some(target_entity)) => {
                                        let owner_pos = owner_entity.element_data().position_map();
                                        let target_pos =
                                            target_entity.element_data().position_map();
                                        let dir =
                                            crate::position_interface::vector_to_sector_0_to_15(
                                                target_pos.x - owner_pos.x,
                                                target_pos.y - owner_pos.y,
                                            );
                                        (target_pos.x, target_pos.y, dir)
                                    }
                                    _ => {
                                        tracing::warn!(
                                            ?owner,
                                            ?target,
                                            "SwordstrikeDown owner or target missing"
                                        );
                                        self.orders
                                            .sequence_manager
                                            .element_impossible(seq_id, elem_idx);
                                        continue;
                                    }
                                };
                            if let Some(entity) = self.world.entities.get_mut(owner) {
                                entity.element_data_mut().set_direction_instantly(dir);
                                if let Some(actor) = entity.actor_data_mut() {
                                    actor.clear_path();
                                }
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
                        }
                        Command::GetKilledAtBottom => {
                            let killer = self
                                .orders
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|elem| match elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => antagonist,
                                    _ => None,
                                });
                            let Some(victim) = self.world.entities.get_mut(owner) else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let damage = victim
                                .human_and_life_points_mut()
                                .map(|(_, lp)| (*lp).max(0) as u16);
                            let Some(damage) = damage else {
                                tracing::warn!(
                                    ?owner,
                                    ?killer,
                                    "GetKilledAtBottom owner is not a human"
                                );
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let max_life_points = match victim {
                                crate::element::Entity::Pc(_) => crate::combat::LIFEPOINTS_PC,
                                crate::element::Entity::Soldier(s) => {
                                    s.soldier.cached_max_life_points
                                }
                                crate::element::Entity::Civilian(_) => 100,
                                _ => 100,
                            };
                            if let Some((_, lp)) = victim.human_and_life_points_mut() {
                                crate::combat::get_wounded(
                                    lp,
                                    damage,
                                    false,
                                    max_life_points,
                                    false,
                                );
                            }
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
                        }
                        // SwordstrikeTired pushes a `BeingWeakSword`
                        // animation order; the order is consumed by
                        // `do_next_order` and (on a soldier)
                        // `apply_combat_injury_side_effect`
                        // dispatches `EventAfterCombatInjury` so the
                        // AI can resume the fight.
                        Command::SwordstrikeTired => {
                            if self.get_entity(owner).is_some() {
                                self.push_new_order(
                                    seq_id,
                                    elem_idx,
                                    crate::order::OrderType::BeingWeakSword,
                                    0.0,
                                    0.0,
                                );
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            } else {
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            }
                        }
                        // ── Smalltalk strikes / parries (Wait priority) ─
                        // WAIT-priority launch is synchronous. Use the same
                        // narrow translator from both the normal sequence
                        // phase and owner-local WaitingSword callbacks.
                        Command::SwordstrikeSmalltalkLeft
                        | Command::SwordstrikeSmalltalkRight
                        | Command::ParrySmalltalkLeft
                        | Command::ParrySmalltalkRight => {
                            SmalltalkCommandContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                        }
                        // ── Provoke (taunt) ─────────────────────
                        // Say `ProvokesCombat` and queue a `Provoking`
                        // animation order (with `compute_direction =
                        // false`).  The animation is consumed via
                        // `active_ai_anim` tied to the sequence
                        // element; its START hook in
                        // `melee::process_pc_combat_anim_speech`
                        // fires `HERO_PROVOKE_OPPONENT` for PCs.
                        Command::Provoke => {
                            if let Some(entity) = self.world.entities.get_mut(owner)
                                && let Some(ai) = entity.ai_controller_mut()
                            {
                                ai.say(crate::ai::Remark::ProvokesCombat);
                            }
                            self.drain_ai_owner_work_for(sim, assets, owner);
                            // Append the order to the sequence
                            // element's queue.
                            let mut order = crate::order::Order::new(
                                crate::order::OrderType::Provoking,
                                0.0,
                                0.0,
                                self.orders.allocate_order_id(),
                            );
                            order.compute_direction = false;
                            self.orders
                                .sequence_manager
                                .push_order_on(seq_id, elem_idx, order);
                            self.orders
                                .sequence_manager
                                .element_in_progress(seq_id, elem_idx);
                        }
                        Command::Fainted
                        | Command::Recover
                        | Command::StandUp
                        | Command::WakeUp
                        | Command::Knee => {
                            let barrier = RecoveryCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // ── Ability commands ─────────────────────
                        Command::TakeCorpse => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
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
                                            // Freeze the target's
                                            // execution, cascading
                                            // the interrupt on its
                                            // current sequence
                                            // element so a postponed
                                            // successor resumes
                                            // cleanly after the carry
                                            // ends.
                                            self.actor_freeze_execution(target_id);
                                            // Inside a building,
                                            // re-select + start hulk
                                            // on the carried target
                                            // flashes the body
                                            // through walls.
                                            self.apply_carry_building_hulk(owner, target_id);
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
                        }
                        Command::DropCorpse => {
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
                        }
                        Command::StrangleCmd => {
                            let Some(target) = self
                                .orders
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|element| match &element.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                        ..
                                    } => *antagonist,
                                    _ => None,
                                })
                            else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            match abilities::begin_strangle(
                                &mut self.world.entities,
                                &mut self.orders.sequence_manager,
                                owner,
                                target,
                                seq_id,
                                elem_idx,
                                &mut self.orders.next_order_id,
                            ) {
                                AbilityBeginResult::Impossible => self
                                    .orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx),
                                AbilityBeginResult::Started => {
                                    self.orders
                                        .sequence_manager
                                        .element_in_progress(seq_id, elem_idx);

                                    // Translate inserts the Strangling order before calling
                                    // a moving antagonist's Think(EVENT_STOP). Think and all
                                    // of its re-entrant effects finish in this stack frame,
                                    // before Perform's initialization acquires AILOCK_FREEZE.
                                    let moving = self
                                        .get_entity(target)
                                        .unwrap_or_else(|| {
                                            panic!(
                                                "strangle victim {target:?} vanished after translation for {seq_id:?}/{elem_idx}"
                                            )
                                        })
                                        .actor_data()
                                        .unwrap_or_else(|| {
                                            panic!(
                                                "strangle victim {target:?} lost actor state after translation"
                                            )
                                        })
                                        .action_state
                                        .is_moving();
                                    if moving {
                                        self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                                            sim,
                                            target,
                                            assets,
                                            crate::ai::Stimulus::new(
                                                crate::ai::StimulusType::EventStop,
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        Command::TieCmd
                        | Command::HealCmd
                        | Command::WhistleCmd
                        | Command::EatCmd
                        | Command::HitCmd
                        | Command::ReceivePurse
                        | Command::EnterListen
                        | Command::LeaveListen
                        | Command::ThrowNet
                        | Command::ThrowPurse
                        | Command::ThrowWaspNest
                        | Command::ThrowApple
                        | Command::ThrowStone => {
                            let ammo_available = match cmd {
                                Command::HealCmd => {
                                    self.has_ammo(owner, crate::profiles::Action::Heal)
                                }
                                Command::EatCmd => self
                                    .get_entity(owner)
                                    .and_then(|entity| match entity {
                                        Entity::Pc(pc) => self.pc_description_for_pc_data(&pc.pc),
                                        _ => None,
                                    })
                                    .is_some_and(|description| {
                                        description.status.get_ammo(crate::profiles::Action::Eat)
                                            > 0
                                    }),
                                Command::ThrowNet => {
                                    self.has_ammo(owner, crate::profiles::Action::Net)
                                }
                                Command::ThrowPurse => {
                                    self.has_ammo(owner, crate::profiles::Action::Purse)
                                }
                                Command::ThrowWaspNest => {
                                    self.has_ammo(owner, crate::profiles::Action::WaspNest)
                                }
                                Command::ThrowApple => {
                                    self.has_ammo(owner, crate::profiles::Action::Apple)
                                }
                                Command::ThrowStone => {
                                    self.has_ammo(owner, crate::profiles::Action::Stone)
                                }
                                _ => true,
                            };
                            let barrier = DirectAbilityCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(
                                owner,
                                cmd,
                                ammo_available,
                                seq_id,
                                elem_idx,
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }
                        Command::ClimbDownFromShoulders => {
                            // Owner is the climber; the carrier
                            // (helper) is read from the climber's
                            // `human.carrier` back-reference latched
                            // at climb-up time.
                            let carrier_id = self
                                .get_entity(owner)
                                .and_then(|e| e.human_data())
                                .and_then(|h| h.carrier);
                            match abilities::begin_climb_down_from_shoulders(
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
                                    // Helper is frozen for the
                                    // duration of the climb-down so
                                    // it can't acquire a fresh
                                    // sequence element while playing
                                    // the sync'd
                                    // TRANSITION_HELPING_CLIMBING_DOWN.
                                    if let Some(helper_id) = carrier_id {
                                        self.actor_freeze_execution(helper_id);
                                    }
                                }
                                AbilityBeginResult::Impossible => {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ClimbUpOnShoulders => {
                            // Owner is the climber, antagonist is the
                            // HelpingToClimb helper.
                            let helper = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let Some(helper_id) = helper else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            // Disjoint-field obstacle list so the headroom
                            // ray-cast inside `begin_climb_on_shoulders`
                            // can run alongside the `&mut self.world.entities`
                            // borrow.
                            let obstacles = crate::sight_obstacle::ObstacleList {
                                static_obstacles: assets.static_sight_obstacles.as_slice(),
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
                                    // Helper is frozen for the
                                    // duration of the climb.
                                    self.actor_freeze_execution(helper_id);
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
                        }
                        Command::Pay => {
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
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
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
                                            // HERO_GIVE_MONEY speech
                                            // cue.
                                            self.hero_speaking(
                                                assets,
                                                owner,
                                                crate::engine::melee::HERO_GIVE_MONEY,
                                            );
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
                        }
                        Command::DropAmmo => {
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
                                continue;
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
                                continue;
                            }
                            // Refuse the drop when no walkable cell
                            // exists near the PC's hand: skip the
                            // `DROPPING_AMMO[_CROUCHED]` order and
                            // terminate.
                            if self.try_get_drop_position(owner).is_none() {
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                                continue;
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
                            let Some((pos, layer, sector, obstacle, direction, material)) = pc_snap
                            else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            // Decrement PC ammo, clamped to current
                            // count.
                            let status_idx = self.get_entity(owner).and_then(|e| match e {
                                crate::element::Entity::Pc(pc) => {
                                    self.pc_description_index_for_pc_data(&pc.pc)
                                }
                                _ => None,
                            });
                            let Some(status_idx) = status_idx else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let dropped = if let Some(campaign) =
                                Some(&mut self.mission_domain.campaign)
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
                                continue;
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
                                    last_pos.x == pos.x
                                        && last_pos.y == pos.y
                                        && last_dir as i16 == direction
                                })
                                .unwrap_or(false);
                            // `prev_bonus_state`: Some((id, current_quantity, action))
                            // if a previous pile is still active.
                            let prev_bonus_state =
                                prev.and_then(|(last, _, _)| last).and_then(|last_id| {
                                    self.get_entity(last_id).and_then(|e| match e {
                                        crate::element::Entity::Bonus(b) if b.element.active => {
                                            Some((
                                                last_id,
                                                b.object.quantity,
                                                b.object.associated_action,
                                            ))
                                        }
                                        _ => None,
                                    })
                                });
                            let merged = if same_position_and_direction
                                && let Some((last_id, prev_qty, prev_action)) = prev_bonus_state
                                && prev_action == action
                                && prev_qty + dropped <= MAX_AMMO_PER_PILE
                            {
                                if let Some(crate::element::Entity::Bonus(b)) =
                                    self.world.entities.get_mut(last_id)
                                {
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
                            let bumped_direction = if !merged
                                && same_position_and_direction
                                && prev_bonus_state.is_some()
                            {
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
                                let mut bonus_element = crate::element::ElementData {
                                    kind: crate::element::ElementKind::ObjectBonus,
                                    active: true,
                                    // Bonus default: blipped iff this
                                    // isn't a forest level.
                                    blipped: !self.world.weather.is_forest_level,
                                    ..Default::default()
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
                                        assets.static_sight_obstacles.as_slice(),
                                    ),
                                );
                                let bonus =
                                    crate::element::Entity::Bonus(crate::element::ElementBonus {
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
                            if let Some(crate::element::Entity::Pc(pc)) =
                                self.world.entities.get_mut(owner)
                            {
                                pc.pc.last_ammo_dropping_position = pos;
                                pc.pc.last_dropping_direction = bumped_direction as u8;
                                if let Some(new_id) = spawned_id {
                                    pc.pc.last_dropped_ammo = Some(new_id);
                                }
                            }

                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }
                        // ── Drop ale bottle ───────────────────────
                        // Spawn a fresh ale at the PC's position,
                        // mark it detectable for all NPCs, and
                        // decrement ale ammo.  We collapse the
                        // animation into an immediate state change
                        // (no DROPPING_ALE order frames) — the
                        // observable result is the same: ammo ticks
                        // down and an ale bottle appears at the PC's
                        // feet.
                        //
                        // The Rust model represents the same dropped accessory
                        // bottle as `Entity::Bonus` + `ObjectType::Ale`.
                        // `spawn_dropped_ale` clones the `ACCESSORIES_Ale`
                        // sprite and forces `OBJECT_LYING`, so no
                        // dedicated enum variant is needed for parity.
                        Command::DropAle => {
                            let action = crate::profiles::Action::Ale;

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
                            let Some((pos, layer, sector, obstacle, direction, material)) = pc_snap
                            else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };

                            // Decrement ammo (clamped to current count).
                            let status_idx = self.get_entity(owner).and_then(|e| match e {
                                crate::element::Entity::Pc(pc) => {
                                    self.pc_description_index_for_pc_data(&pc.pc)
                                }
                                _ => None,
                            });
                            let Some(status_idx) = status_idx else {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let dropped = if let Some(campaign) =
                                Some(&mut self.mission_domain.campaign)
                                && let Some(pc_desc) = campaign.characters.get_mut(status_idx)
                            {
                                let current = pc_desc.status.get_ammo(action);
                                let take = 1u16.min(current);
                                pc_desc.status.decrease_ammo(action, take);
                                take
                            } else {
                                0
                            };
                            if dropped == 0 {
                                self.orders
                                    .sequence_manager
                                    .element_impossible(seq_id, elem_idx);
                                continue;
                            }
                            // Auto-disable when empty.
                            let now_empty = Some(&self.mission_domain.campaign)
                                .and_then(|c| c.characters.get(status_idx))
                                .map(|d| d.status.get_ammo(action) == 0)
                                .unwrap_or(false);
                            if now_empty {
                                self.disable_pc_action(assets, owner, action);
                            }

                            // Spawn an ale bottle at the PC's
                            // position, nudged onto a walkable cell
                            // when possible (same authorized-position
                            // handoff as generic DropAmmo above).
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

                            // Spawn the concrete RHElementAle-equivalent at
                            // the resolved position. Rust shares the
                            // `Entity::Bonus` payload, but the Original Ale
                            // constructor inherits RHElementObject's
                            // OBJECT_OTHERS category, not OBJECT_BONUS.
                            let mut ale_element = crate::element::ElementData {
                                kind: crate::element::ElementKind::ObjectOther,
                                active: true,
                                blipped: !self.world.weather.is_forest_level,
                                ..Default::default()
                            };
                            ale_element.sprite.apply_placement(
                                spawn_pos,
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
                                    ..Default::default()
                                },
                            });
                            let ale_id = self.add_entity(ale);
                            tracing::debug!(
                                pc = ?owner,
                                ?ale_id,
                                "DropAle: decremented PC ale ammo and spawned ale bottle"
                            );
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }
                        // ── Turn ───────────────────────────────
                        // Rotate the actor to face the `CameraPoint`
                        // property (or `Direction` property if no
                        // point), then push a single `Turning` order.
                        // The element terminates when the animation's
                        // sprite reports completion.  TURN and
                        // TURN_FAST share an identical body — both
                        // read CameraPoint / Direction from the
                        // element and push Turning onto the order
                        // queue; only Upright posture is legal.
                        Command::Turn | Command::TurnFast => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // Face the element's antagonist, then push
                        // Turning.  Carried by
                        // `SequenceElementData::Interaction`.
                        Command::TurnElement => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // Owner-ful Freeze pushes a `Freezing` order
                        // onto the element.  The engine-side
                        // `ExecuteImmediateEngine` arm at the bottom
                        // of this file handles non-owner Freeze
                        // (which collapses into FreezeAll).
                        Command::Freeze => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // ── Point / GatherSoldiers ─────────────
                        // Each pushes a single one-shot animation
                        // order (`Pointing` / `GatheringSoldiers`)
                        // with `compute_direction = false`.  Point
                        // reads `Direction` and sets the actor's
                        // facing before the anim; GatherSoldiers has
                        // no direction.  Both terminate the sequence
                        // element on animation completion, wired via
                        // `AiAnimCompletion::SequenceElement`.
                        Command::Point | Command::GatherSoldiers => {
                            let barrier = TurnCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // ── Wait (soldier-specific override) ───
                        //   - attentive + upright + waiting + alive →
                        //     WAITING_ALERTED
                        //   - leaning out with AimingWithBow{,Down} →
                        //     AIMING_WITH_BOW_LEANING_OUT
                        //   - leaning out otherwise → LEANING_OUT
                        //   - anything else → fall through to NPC
                        //     base (not dispatched here — terminates,
                        //     which matches the existing catch-all).
                        // WAIT_TIMER additionally records `wait_time`
                        // from the element's Timer property.
                        // WAIT_FREE_LIFT is translated by the identical
                        // stationary-order path above, then rechecked by its
                        // owner after Execute.
                        Command::Wait | Command::WaitTimer => {
                            let barrier = WaitCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                profiles: &assets.profile_manager,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }
                        // ── NPC-specific one-shot anims ────────
                        // Each command appends one animation order
                        // with `compute_direction = false`, so we
                        // book it through `active_ai_anim` and bind
                        // sequence termination to its DONE — matching
                        // the existing `Point` arm above.  Posture
                        // flips (Upright→Sitting / Upright→Leisure)
                        // are handled by the animation-completion
                        // side effects in `animation.rs`.
                        //
                        // `launch_element_for_owner`/single-order launch
                        // call `generate_transition` before this command
                        // body is reached.  For these NPC commands the
                        // transition flags match legacy behavior, so any
                        // needed leave-action/posture orders have already been
                        // queued ahead of the command's own animation.
                        Command::SitDown | Command::BeggarShowFace | Command::EnterLeisure => {
                            let barrier = NpcStateCommandContext {
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }
                        // ── Menace / Sleep transitions ─────────
                        // Each pushes a fixed sequence of transition
                        // orders with `compute_direction = false`.
                        // The animation system's DONE/TERMINATED
                        // hooks in `animation.rs` flip posture /
                        // action_state appropriately when each order
                        // finishes, so the sequence element itself
                        // can terminate immediately — the visual
                        // transition plays off the actor's order
                        // queue.
                        Command::StartMenace
                        | Command::StopMenace
                        | Command::StopSleep
                        | Command::LowerBowLeanOut
                        | Command::RaiseBowLeanOut => {
                            let barrier = NpcStateCommandContext {
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }
                        // ── DrinkAle / Take ────────────────────
                        // DrinkAle / Take push a single interaction
                        // order whose animation (DRINKING_ALE /
                        // TAKING) references the antagonist (bottle /
                        // purse / coin).  The corresponding Execute
                        // handlers hide / remove the antagonist on
                        // DONE and bump money / blood-alcohol on
                        // TERMINATED.  Book through `active_ai_anim`
                        // with the antagonist threaded along so the
                        // `apply_soldier_execute_side_effects`
                        // handler picks up the target.
                        Command::DrinkAle | Command::Take => {
                            ObjectInteractionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                        }

                        // ── UnlockDoor ─────────────────────────
                        // The PC pushes a single `UnlockingDoor`
                        // order (or `UnlockingTrap` when the door is
                        // a building-trap) and the door's `locked_pc`
                        // flag flips off when the lockpick animation
                        // finishes.  We book the anim via
                        // `active_ai_anim` + `UnlockDoor` completion
                        // so the flag flip + element termination
                        // happen on animation end.  Target door is
                        // read from the `Field::Door` property set
                        // by `build_gate_movement_sequence`.
                        Command::UnlockDoor => {
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
                                crate::gate::DoorType::BuildingTrap => {
                                    crate::order::OrderType::UnlockingTrap
                                }
                                _ => crate::order::OrderType::UnlockingDoor,
                            };
                            tracing::debug!(
                                door_id = %id,
                                entity = ?owner,
                                ?anim_type,
                                "UnlockDoor: starting lockpick animation"
                            );
                            let order = crate::order::Order::new(
                                anim_type,
                                0.0,
                                0.0,
                                self.orders.allocate_order_id(),
                            )
                            .with_completion(
                                crate::order::OrderCompletion::UnlockDoor { door_id: id },
                            );
                            self.orders
                                .sequence_manager
                                .push_order_on(seq_id, elem_idx, order);
                            self.orders
                                .sequence_manager
                                .element_in_progress(seq_id, elem_idx);
                        }

                        // ── Jump ────────────────────────────────
                        // Build a step list covering the run-up,
                        // airborne trajectory, and landing
                        // transitions, then drive the actor through
                        // them via `tick_active_jumps`.  If the jump
                        // can't be installed (missing data) the
                        // element is terminated so the sequence
                        // doesn't stall.
                        Command::Jump => {
                            if self.start_jump(sim, assets, owner, seq_id, elem_idx) {
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            } else {
                                tracing::warn!(
                                    entity = ?owner,
                                    seq = ?seq_id,
                                    elem = elem_idx,
                                    "Jump: failed to install ActiveJump — terminating element"
                                );
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            }
                        }

                        Command::ActivateApple
                        | Command::ActivateArrow
                        | Command::ActivateHandle
                        | Command::ActivateHeal
                        | Command::ActivateLever
                        | Command::ActivateMoney
                        | Command::ActivateSearch
                        | Command::ActivateStone
                        | Command::ActivateSword => {
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let (target_handle, pc_handle, method) = TargetActivationContext {
                                entities: &self.world.entities,
                            }
                            .dispatch(owner, cmd, antagonist);
                            let key = crate::engine::ScriptVmKey::Target(target_handle);
                            let is_instantiated = self
                                .scripts
                                .mission
                                .as_ref()
                                .is_some_and(|script| script.has_script_vm(key));
                            if is_instantiated
                                && let Err(error) = self.call_script_vm(
                                    sim,
                                    assets,
                                    key,
                                    method,
                                    &[pc_handle],
                                    crate::natives::ScriptCallFrame::actor(target_handle),
                                )
                            {
                                tracing::warn!("{method} (target {target_handle}): {error}");
                            }
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }

                        // Script-recorded PlayAnim / PlayAnimLoop /
                        // PlayAnimFreeze / PlayAnimFrozen.  C++ translates these to
                        // PLAY_CUSTOM non-animations for actors, which
                        // then drive the stored RHFIELD_ANIMATION_ID.
                        // FX targets instead force the target sprite
                        // animation/progression immediately.
                        Command::PlayAnim
                        | Command::PlayAnimLoop
                        | Command::PlayAnimFreeze
                        | Command::PlayAnimFrozen => {
                            let animation = match elem
                                .get_property(crate::sequence::Field::AnimationId)
                            {
                                Some(crate::sequence::FieldValue::Animation(anim)) => Some(*anim),
                                Some(crate::sequence::FieldValue::Integer(v)) => {
                                    crate::order::OrderType::try_from(*v).ok()
                                }
                                _ => None,
                            };
                            let barrier = TargetAnimationContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch_play_animation(owner, cmd, animation, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // PC-side target interaction commands.  Each
                        // enqueues a per-command animation order on
                        // the PC (USING_LEVER / HITTING_TARGET /
                        // HANDLING_TARGET / TAKING_TARGET /
                        // SEARCHING), and on DONE the engine launches
                        // the corresponding `Activate*` interaction
                        // element on the target antagonist.
                        //
                        // The order driver plays the PC order first;
                        // `apply_pc_target_interaction_side_effect`
                        // launches the target activation when that
                        // order reports `MotionState::Done`.
                        Command::HitTarget
                        | Command::HandleTarget
                        | Command::UseLever
                        | Command::TakeTarget
                        | Command::SearchCmd => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let barrier = TargetInteractionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(cmd, target, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                continue;
                            }
                        }

                        // Internal carrier for a pre-built animation order.
                        // `launch_single_order_sequence_stamped` normally
                        // promotes these synchronously, but a postponed
                        // carrier returns here as Todo when its blocker
                        // completes.  The order is already attached; keep the
                        // element alive so the actor animation driver can
                        // consume it instead of dropping the visible action.
                        Command::Generic => {
                            if elem.orders.is_empty() {
                                // The carrier's animation may have completed
                                // while this element was postponed behind a
                                // higher-priority command.  There is no
                                // command-specific Translate body left to
                                // run, so completion is the correct resumed
                                // state.
                                self.orders
                                    .sequence_manager
                                    .element_terminated(seq_id, elem_idx);
                            } else {
                                self.orders
                                    .sequence_manager
                                    .element_in_progress(seq_id, elem_idx);
                            }
                        }

                        _ => {
                            // Dispatch for remaining owner-instructed
                            // commands will be added per-command;
                            // marking terminated here keeps the
                            // sequence ticking.  Warn so unhandled
                            // commands don't silently vanish (the
                            // Seek-vs-Move bug hid here for months
                            // because the element just terminated
                            // without any log — Seek needed dispatch
                            // through the Move path and this default
                            // arm swallowed it).
                            tracing::warn!(
                                ?cmd,
                                ?owner,
                                ?seq_id,
                                elem_idx,
                                "InstructOwner: no dispatch for command; terminating element"
                            );
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }
                    }
                }
                crate::sequence::SequenceAction::ExecuteImmediateOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    if let Some((handle, msg, arg1, arg2)) =
                        self.dispatch_execute_immediate_owner(sim, assets, owner, seq_id, elem_idx)
                    {
                        self.dispatch_sequence_messages(
                            sim,
                            assets,
                            &[(handle, msg, arg1, arg2)],
                            &[],
                        );
                        self.orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
                    }
                }
                crate::sequence::SequenceAction::EngineCommand {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                }
                | crate::sequence::SequenceAction::ExecuteImmediateEngine {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    if let Some((msg, arg1, arg2)) = self.dispatch_engine_or_execute_immediate(
                        sim, display, assets, seq_id, elem_idx,
                    ) {
                        self.dispatch_sequence_messages(sim, assets, &[], &[(msg, arg1, arg2)]);
                        self.orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
                    }
                }
            }

            // `SetState` calls the owner's SendCondolationCard and resumes at
            // `Ready()` before returning to this action loop. Closing that
            // boundary here lets an immediate next-level successor preempt
            // older actions already detached into `SequencePhase`.
            self.dispatch_condolations(sim, assets);

            // After-action drain: callbacks can synchronously register an
            // immediate command or complete a level whose successor is WAIT.
            // Splice that ordered registration stream onto the FRONT so the
            // re-entrant work fires before the next older action in the batch.
            phase.splice_synchronous_actions(&mut self.orders);
        }
    }
}
