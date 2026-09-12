use super::*;

impl EngineInner {
    pub(super) fn dispatch_sequence_phase_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        action: crate::sequence::SequenceAction,
        accepted_instruct_owners: &mut Vec<EntityId>,
    ) {
        'action: {
            match action {
                crate::sequence::SequenceAction::InstructOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    let Some(PreparedOwnerInstruction {
                        owner,
                        cmd,
                        trace_path_owner,
                        satisfied_enter_swordfight_order,
                    }) = self.prepare_owner_instruction(sim, assets, owner, seq_id, elem_idx)
                    else {
                        break 'action;
                    };
                    // Re-borrow element for data access.
                    let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                        Some(e) => e,
                        None => break 'action,
                    };
                    // Do not run a generic human validity check here.
                    // Original-game human instruction handling delegates directly to
                    // base actor handling after its dead/unconscious and repeated
                    // PC bow-shot guards. Commands that require live
                    // revalidation do so in their specific Execute
                    // initialization arm; WakeUp, for example, deliberately
                    // has no position-validity check during instruction.
                    match cmd {
                        Command::Move | Command::Seek => {
                            let barrier = self.dispatch_ordered_move_seek_instruct(
                                sim, assets, owner, seq_id, elem_idx,
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        Command::ShootBow | Command::ShootBowOnce => {
                            if self.instruct_shoot_bow(assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
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
                                break 'action;
                            }
                        }
                        // ── CHANGE_POSITION ────────────────────────
                        // Instant teleport to a new position.
                        Command::ChangePosition => {
                            if self.instruct_change_position(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // ── ASSERT_POSITION ────────────────────────
                        // Check actor is at expected position/sector.
                        Command::AssertPosition => {
                            let barrier = PositionAssertionContext {
                                entities: &self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                            }
                            .dispatch(owner, seq_id, elem_idx);
                            debug_assert_eq!(barrier, OwnerActionBarrier::Skip);
                            break 'action;
                        }
                        // ── WAIT_FREE_LIFT ──────────────────────
                        // Translation is identical to WAIT: book the
                        // stationary actor order and enter InProgress. The
                        // live actor-slot coordinator rechecks/reserves the
                        // lift after each actual Execute, matching
                        // actor updating rather than this one-shot
                        // instruction boundary.
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
                            if self.instruct_swordstrike_thrust_a(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Swordfight enter/quit ───────────────
                        Command::EnterSwordfight | Command::PrepareSwordfight => {
                            let opponent = match elem.get_property(crate::sequence::Field::Opponent)
                            {
                                Some(crate::sequence::FieldValue::Element(id)) => Some(*id),
                                _ => None,
                            };
                            let barrier = self.dispatch_enter_swordfight(
                                sim, assets, owner, opponent, seq_id, elem_idx,
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                self.dispatch_condolations(sim, assets);
                                if let Some(retained_order) = satisfied_enter_swordfight_order {
                                    let entity = self
                                        .get_entity_mut(owner)
                                        .expect("satisfied EnterSwordfight owner disappeared");
                                    let actor = entity.actor_data_mut().unwrap();
                                    actor.installed_order = Some(retained_order);
                                    actor.retained_waiting_sword_order_id =
                                        Some(retained_order.order_id);
                                }
                                break 'action;
                            }
                        }
                        Command::QuitSwordfight => {
                            if self.dispatch_quit_swordfight(sim, assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Parry commands ──────────────────────
                        Command::ParrySword => {
                            if self.dispatch_parry_sword(owner, false, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::ParrySwordLow => {
                            if self.dispatch_parry_sword(owner, true, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::StopParrySword => {
                            if self.dispatch_stop_parry(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Damage reception commands ───────────
                        Command::ReceiveSwordDamage
                        | Command::ReceiveDamage
                        | Command::ReceiveArrowDamage
                        | Command::ReceiveStoneDamage
                        | Command::ReceiveHitDamage
                        | Command::ReceiveMobileDamage
                        | Command::ReceiveNet => {
                            if self.dispatch_receive_damage(sim, assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
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
                            self.trace_attentive_owner_handoff(
                                "translate_before",
                                owner,
                                Some((seq_id, elem_idx)),
                                format_args!("before attentive translator"),
                            );
                            let barrier = NpcAttentionCommandContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(owner, cmd, seq_id, elem_idx);
                            self.trace_attentive_owner_handoff(
                                "translate_after",
                                owner,
                                Some((seq_id, elem_idx)),
                                format_args!(
                                    "{}",
                                    match barrier {
                                        OwnerActionBarrier::Reach => {
                                            "attentive translator queued transition"
                                        }
                                        OwnerActionBarrier::Skip => {
                                            "attentive translator terminalized inline"
                                        }
                                    }
                                ),
                            );
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
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
                        | Command::EnterCloak
                        | Command::LeaveSpy
                        | Command::LeaveTree => {
                            if cmd == Command::EnterBeggar {
                                // "To avoid beggar & run bug": the beggar
                                // entry stops the actor from inside its own
                                // translation, so the stop runs after this
                                // element has already taken over and pushed
                                // whatever it replaced into its postponed
                                // slot. Walking that slot is the point — a
                                // move the beggar entry displaced is
                                // interrupted here and never resumes. The
                                // element is not the actor's selection yet on
                                // this side, so root the stop at it directly.
                                let resolver = Self::priority_resolver(&self.world.entities);
                                self.orders.sequence_manager.stop_owner_from_root(
                                    owner,
                                    Some((seq_id, elem_idx)),
                                    crate::sequence::SequencePriority::Normal,
                                    &resolver,
                                );
                            }
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
                            if self.instruct_raise_shield(assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // ── Bow equip / raise / lower ───────────
                        //
                        // The original game's actor translation appends
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
                                break 'action;
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
                            if self.instruct_hide_behind_shield(seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Other sword-related commands ────────
                        Command::SwordstrikeDown => {
                            if self.instruct_swordstrike_down(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::GetKilledAtBottom => {
                            if self
                                .instruct_get_killed_at_bottom(sim, assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
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
                            self.dispatch_provoke(sim, assets, owner, seq_id, elem_idx);
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
                                break 'action;
                            }
                        }

                        // ── Ability commands ─────────────────────
                        Command::TakeCorpse => {
                            if self.instruct_take_corpse(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::DropCorpse => {
                            if self.instruct_drop_corpse(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::HitCmd | Command::StrangleCmd => {
                            if self.instruct_hit_cmd(sim, assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::TieCmd
                        | Command::Untie
                        | Command::HealCmd
                        | Command::WhistleCmd
                        | Command::EatCmd
                        | Command::ReceivePurse
                        | Command::EnterListen
                        | Command::LeaveListen
                        | Command::ThrowNet
                        | Command::ThrowPurse
                        | Command::ThrowWaspNest
                        | Command::ThrowApple
                        | Command::ThrowStone => {
                            if self.instruct_tie_cmd(assets, owner, seq_id, elem_idx, cmd)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
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
                            if self.instruct_climb_up_on_shoulders(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::Pay => {
                            if self.instruct_pay(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        Command::DropAmmo => {
                            if self.instruct_drop_ammo(assets, owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }
                        // ── Drop ale bottle ───────────────────────
                        Command::DropAle => {
                            let order_type = match self.get_entity(owner) {
                                Some(entity)
                                    if entity.element_data().posture()
                                        == crate::element::Posture::Crouched =>
                                {
                                    crate::order::OrderType::DroppingAleCrouched
                                }
                                Some(_) => crate::order::OrderType::DroppingAle,
                                None => {
                                    self.orders
                                        .sequence_manager
                                        .element_impossible(seq_id, elem_idx);
                                    break 'action;
                                }
                            };
                            self.push_new_order(seq_id, elem_idx, order_type, 0.0, 0.0);
                            self.orders
                                .sequence_manager
                                .element_in_progress(seq_id, elem_idx);
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
                                break 'action;
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
                                break 'action;
                            }
                        }

                        // Owner-ful Freeze pushes a `Freezing` order
                        // onto the element.  The engine-side
                        // immediate engine-execution arm at the bottom
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
                                break 'action;
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
                                break 'action;
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
                                break 'action;
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
                        // Instruction admission calls `generate_transition`
                        // before this command body is reached. For these NPC
                        // commands the transition flags match legacy behavior,
                        // so any needed leave-action/posture orders have
                        // already been queued ahead of the command's own
                        // animation.
                        Command::SitDown | Command::BeggarShowFace | Command::EnterLeisure => {
                            let barrier = NpcStateCommandContext {
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                            }
                            .dispatch(cmd, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
                            }
                        }
                        // ── Menace / Sleep transitions ─────────
                        // Each pushes a fixed sequence of transition
                        // orders with `compute_direction = false`.
                        // The animation system's DONE/TERMINATED
                        // hooks in `animation.rs` flip posture /
                        // action_state appropriately when each order
                        // finishes. The sequence element remains selected
                        // and InProgress until its final order completes.
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
                                break 'action;
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
                            if self.instruct_unlock_door(owner, seq_id, elem_idx)
                                == OwnerActionBarrier::Skip
                            {
                                break 'action;
                            }
                        }

                        // ── Jump ────────────────────────────────
                        // Build a step list covering the run-up,
                        // airborne trajectory, and landing
                        // transitions, then drive the actor through
                        // them via `tick_active_jump_for`.  If the jump
                        // can't be installed (missing data) the
                        // element is terminated so the sequence
                        // doesn't stall.
                        Command::JumpCmd => {
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
                        // PlayAnimFreeze / PlayAnimFrozen. The original game translates these to
                        // PLAY_CUSTOM non-animations for actors, which
                        // then drive the stored animation identifier.
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
                            let preserve_trigger_visual = self.control.sim_config.reversible_background_patches
                                && self.script_domains.interactables.patches.iter().any(|patch| {
                                    patch.repeat_activation.as_ref().is_some_and(|(handle, _)| {
                                        *handle == crate::natives::ScriptHandleCodec::actor_handle(owner)
                                    })
                                });
                            let barrier = TargetAnimationContext {
                                entities: &mut self.world.entities,
                                sequence_manager: &mut self.orders.sequence_manager,
                                next_order_id: &mut self.orders.next_order_id,
                                preserve_trigger_visual,
                            }
                            .dispatch_play_animation(owner, cmd, animation, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
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
                            .dispatch(owner, cmd, target, seq_id, elem_idx);
                            if barrier == OwnerActionBarrier::Skip {
                                break 'action;
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
                                // run. Original-game actor instruction handling nevertheless
                                // writes mmotionState=IN_PROGRESS immediately
                                // after Translate returns, before discovering
                                // that the current order is null and
                                // terminating the accepted element. Preserve
                                // that otherwise-invisible acceptance edge
                                // before the state change clears the selected element.
                                self.world
                                    .entities
                                    .get_mut(owner)
                                    .and_then(Entity::actor_data_mut)
                                    .expect("accepted empty Generic lost its actor")
                                    .continuation
                                    .motion_state = crate::sprite::MotionState::InProgress;
                                self.orders.sequence_manager.set_translating_element(None);
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
                            self.orders.sequence_manager.set_translating_element(None);
                            self.orders
                                .sequence_manager
                                .element_terminated(seq_id, elem_idx);
                        }
                    }
                    // Accepted actor instruction handling publishes the translated
                    // current order through the actor-order field. Keep this write at the
                    // dispatch boundary rather than inferring it later from
                    // whichever element happens to be selected.
                    if self
                        .world
                        .entities
                        .get(owner)
                        .is_some_and(|entity| entity.actor_data().is_some())
                    {
                        self.publish_instructed_order_as_installed(owner, seq_id, elem_idx);
                    }
                    if trace_path_owner {
                        self.trace_path_owner_lifecycle(
                            "after_instruct_translation",
                            owner,
                            Some((seq_id, elem_idx)),
                        );
                    }
                    // The original game returns immediately when translation completed the
                    // element re-entrantly and changed the selected sequence element;
                    // that path deliberately does not overwrite the
                    // actor's preceding motion latch with IN_PROGRESS.
                    // Only an element that survived translation and was
                    // promoted to INPROGRESS reaches the common write
                    // below. Command-specific empty-order paths publish
                    // their required edge at the dispatch site above.
                    if self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .is_some_and(|element| {
                            element.state == crate::sequence::SequenceState::InProgress
                        })
                        && self
                            .world
                            .entities
                            .get(owner)
                            .is_some_and(|entity| entity.actor_data().is_some())
                    {
                        accepted_instruct_owners.push(owner);
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
                    if let Some((msg, arg1, arg2)) =
                        self.dispatch_engine_or_execute_immediate(sim, assets, seq_id, elem_idx)
                    {
                        self.dispatch_sequence_messages(sim, assets, &[], &[(msg, arg1, arg2)]);
                        self.orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
                    }
                }
            }
        }
    }
}
