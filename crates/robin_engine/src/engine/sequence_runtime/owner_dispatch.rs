use super::*;
use crate::engine::TickCtx;

impl EngineInner {
    /// Instruct an actor at the caller's scheduling boundary. The result
    /// tells retained callers whether the instruction was handled, including
    /// postponement and completion during translation.
    pub(in crate::engine) fn instruct_owner(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> bool {
        let prepared = self.prepare_owner_instruction(tcx, active_scripts, owner, seq_id, elem_idx);
        let PreparedOwnerInstruction {
            owner,
            cmd,
            trace_path_owner,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(handled) => return handled,
        };
        self.translate_instructed_command(tcx, active_scripts, owner, cmd, seq_id, elem_idx);
        // Translation can synchronously finish this element or instruct another
        // one. Only the instruction still selected by the actor owns acceptance.
        let Some(actor) = self
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::actor_data_mut)
        else {
            return true;
        };
        if actor.selected_sequence_element
            != Some(crate::sequence::SequenceElementRef::new(seq_id, elem_idx))
        {
            return true;
        }
        actor.continuation.motion_state = crate::sprite::MotionState::InProgress;
        self.element_in_progress(tcx, active_scripts, seq_id, elem_idx);
        self.world
            .entities
            .expect_actor_data_mut(owner, format_args!("accepted instruction owner"))
            .sequence_element_started = true;

        self.publish_selected_order_as_installed(owner);
        let has_order = self
            .world
            .entities
            .expect_actor_data(owner, format_args!("accepted instruction owner"))
            .installed_order
            .is_some();
        if !has_order {
            // Clear selection before completion callbacks: this accepted empty
            // instruction no longer owns the actor's goal or any nested order.
            self.select_sequence_element(owner, None);
            self.element_terminated(tcx, active_scripts, seq_id, elem_idx);
        } else {
            self.world
                .entities
                .expect_actor_data_mut(owner, format_args!("accepted instruction owner"))
                .execute_order_initialising = true;
        }
        if trace_path_owner {
            self.trace_path_owner_lifecycle(
                "after_instruct_translation",
                owner,
                Some((seq_id, elem_idx)),
            );
        }
        true
    }

    pub(in crate::engine) fn dispatch_sequence_phase_action(
        &mut self,
        tcx: TickCtx<'_>,
        action: crate::sequence::SequenceAction,
    ) {
        {
            match action {
                crate::sequence::SequenceAction::InstructOwner {
                    owner,
                    sequence_id,
                    element_index,
                } => {
                    self.instruct_owner(tcx, &mut Vec::new(), owner, sequence_id, element_index);
                }
                crate::sequence::SequenceAction::ExecuteImmediateOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    self.dispatch_script_synchronous_action(
                        tcx,
                        crate::sequence::SequenceAction::ExecuteImmediateOwner {
                            owner,
                            sequence_id: seq_id,
                            element_index: elem_idx,
                        },
                        &mut Vec::new(),
                    )
                    .unwrap_or_else(|error| panic!("immediate owner dispatch failed: {error:?}"));
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
                        self.dispatch_engine_or_execute_immediate(tcx, seq_id, elem_idx)
                    {
                        self.dispatch_sequence_messages(tcx, &[], &[(msg, arg1, arg2)]);
                        self.element_terminated(tcx, &mut Vec::new(), seq_id, elem_idx);
                    }
                }
            }
        }
    }

    /// Command translation for an admitted owner instruction: movement,
    /// combat, posture and recovery commands. Every other command falls
    /// through to [`Self::translate_instructed_ability_command`]; the two
    /// matches together form one match over disjoint command patterns.
    fn translate_instructed_command(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        cmd: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match cmd {
            Command::Move | Command::Seek => self.dispatch_ordered_move_seek_instruct(
                tcx,
                active_scripts,
                owner,
                seq_id,
                elem_idx,
            ),
            Command::ShootBow | Command::ShootBowOnce => {
                self.instruct_shoot_bow(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }
            Command::PassDoor => {
                self.instruct_pass_door(tcx, active_scripts, owner, seq_id, elem_idx);
            }
            // ── CHANGE_POSITION ────────────────────────
            // Instant teleport to a new position.
            Command::ChangePosition => {
                self.instruct_change_position(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            // ── ASSERT_POSITION ────────────────────────
            // Check actor is at expected position/sector.
            Command::AssertPosition => {
                self.dispatch_position_assertion(tcx, active_scripts, owner, seq_id, elem_idx);
            }
            // ── WAIT_FREE_LIFT ──────────────────────
            // Translation is identical to WAIT: book the
            // stationary actor order and enter InProgress. The
            // live actor-slot coordinator rechecks/reserves the
            // lift after each actual Execute, matching
            // actor updating rather than this one-shot
            // instruction boundary.
            Command::WaitFreeLift => {
                self.dispatch_wait_command(
                    tcx,
                    active_scripts,
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
                self.instruct_swordstrike_thrust_a(tcx, active_scripts, owner, seq_id, elem_idx)
            }

            // ── Swordfight enter/quit ───────────────
            Command::EnterSwordfight | Command::PrepareSwordfight => {
                self.instruct_enter_swordfight(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            Command::QuitSwordfight => {
                self.dispatch_quit_swordfight(tcx, active_scripts, owner, seq_id, elem_idx)
            }

            // ── Parry commands ──────────────────────
            Command::ParrySword => {
                self.dispatch_parry_sword(tcx, active_scripts, owner, false, seq_id, elem_idx)
            }
            Command::ParrySwordLow => {
                self.dispatch_parry_sword(tcx, active_scripts, owner, true, seq_id, elem_idx)
            }
            Command::StopParrySword => {
                self.dispatch_stop_parry(tcx, active_scripts, owner, seq_id, elem_idx)
            }

            // ── Damage reception commands ───────────
            Command::ReceiveSwordDamage
            | Command::ReceiveDamage
            | Command::ReceiveArrowDamage
            | Command::ReceiveStoneDamage
            | Command::ReceiveHitDamage
            | Command::ReceiveMobileDamage
            | Command::ReceiveNet => {
                self.dispatch_receive_damage(tcx, active_scripts, owner, seq_id, elem_idx)
            }

            // ── Shoulder-fall sub-sequence ──────────
            // Launched by `translate_shoulder_damage` on
            // the carrier/carried partner when shoulder-
            // damage lands on the other side of the carry.
            Command::Fall => {
                self.dispatch_fall(tcx, owner, seq_id, elem_idx);
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
                self.dispatch_npc_attention_command(
                    tcx,
                    active_scripts,
                    owner,
                    cmd,
                    seq_id,
                    elem_idx,
                );
            }

            // ── Attentive-mode transitions ───────────
            Command::EnterAttentiveMode
            | Command::LeaveAttentiveMode
            | Command::LeaveAttentiveModeOfficer => {
                self.instruct_attentive_mode(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }

            // ── Wasp sting ─────────────────────────
            Command::ReceiveWaspSting => {
                self.dispatch_receive_wasp_sting(tcx, owner, seq_id, elem_idx);
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
                self.instruct_stealth_posture(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }

            // ── Shield commands ─────────────────────
            Command::RaiseShield
            | Command::RaiseShieldInstantly
            | Command::LowerShield
            | Command::ParryShield => {
                self.instruct_raise_shield(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
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
                self.dispatch_bow_transition(tcx, active_scripts, owner, cmd, seq_id, elem_idx)
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
                self.instruct_hide_behind_shield(tcx, active_scripts, seq_id, elem_idx)
            }

            // ── Other sword-related commands ────────
            Command::SwordstrikeDown => {
                self.instruct_swordstrike_down(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            Command::GetKilledAtBottom => {
                self.instruct_get_killed_at_bottom(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            Command::SwordstrikeTired => {
                self.instruct_swordstrike_tired(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            // ── Smalltalk strikes / parries (Wait priority) ─
            // WAIT-priority launch is synchronous. Use the same
            // narrow translator from both the normal sequence
            // phase and owner-local WaitingSword callbacks.
            Command::SwordstrikeSmalltalkLeft
            | Command::SwordstrikeSmalltalkRight
            | Command::ParrySmalltalkLeft
            | Command::ParrySmalltalkRight => {
                self.dispatch_smalltalk_command(tcx, active_scripts, owner, cmd, seq_id, elem_idx);
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
                self.dispatch_provoke(tcx, owner, seq_id, elem_idx);
            }
            Command::Fainted
            | Command::Recover
            | Command::StandUp
            | Command::WakeUp
            | Command::Knee => {
                self.dispatch_recovery_command(tcx, active_scripts, owner, cmd, seq_id, elem_idx)
            }

            _ => self.translate_instructed_ability_command(
                tcx,
                active_scripts,
                owner,
                cmd,
                seq_id,
                elem_idx,
            ),
        }
    }

    /// Second half of [`Self::translate_instructed_command`]: ability,
    /// animation, target-interaction and internal carrier commands, plus the
    /// catch-all for commands without an owner translation.
    fn translate_instructed_ability_command(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        cmd: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        match cmd {
            // ── Ability commands ─────────────────────
            Command::TakeCorpse => {
                self.instruct_take_corpse(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            Command::DropCorpse => {
                self.instruct_drop_corpse(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            Command::HitCmd | Command::StrangleCmd => {
                self.instruct_hit_cmd(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
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
                self.instruct_tie_cmd(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }
            Command::ClimbDownFromShoulders => self.instruct_climb_down_from_shoulders(
                tcx,
                active_scripts,
                owner,
                seq_id,
                elem_idx,
            ),
            Command::ClimbUpOnShoulders => {
                self.instruct_climb_up_on_shoulders(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            Command::Pay => self.instruct_pay(tcx, active_scripts, owner, seq_id, elem_idx),
            Command::DropAmmo => {
                self.instruct_drop_ammo(tcx, active_scripts, owner, seq_id, elem_idx)
            }
            // ── Drop ale bottle ───────────────────────
            Command::DropAle => {
                self.instruct_drop_ale(tcx, active_scripts, owner, seq_id, elem_idx)
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
                self.dispatch_turn_command(owner, cmd, seq_id, elem_idx)
            }

            // Face the element's antagonist, then push
            // Turning.  Carried by
            // `SequenceElementData::Interaction`.
            Command::TurnElement => self.dispatch_turn_command(owner, cmd, seq_id, elem_idx),

            // Owner-ful Freeze pushes a `Freezing` order
            // onto the element.  The engine-side
            // immediate engine-execution arm
            // (`dispatch_engine_or_execute_immediate`)
            // handles non-owner Freeze
            // (which collapses into FreezeAll).
            Command::Freeze => self.dispatch_turn_command(owner, cmd, seq_id, elem_idx),

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
                self.dispatch_turn_command(owner, cmd, seq_id, elem_idx)
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
            // stationary-order path in
            // `translate_instructed_command`, then rechecked by its
            // owner after Execute.
            Command::Wait | Command::WaitTimer => {
                self.dispatch_wait_command(tcx, active_scripts, owner, cmd, seq_id, elem_idx)
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
                self.dispatch_npc_state_command(cmd, seq_id, elem_idx)
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
            | Command::RaiseBowLeanOut => self.dispatch_npc_state_command(cmd, seq_id, elem_idx),
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
                self.dispatch_object_interaction_command(owner, cmd, seq_id, elem_idx);
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
            // by `launch_gate_movement_sequence`.
            Command::UnlockDoor => self.instruct_unlock_door(owner, seq_id, elem_idx),

            // ── Jump ────────────────────────────────
            Command::JumpCmd => self.instruct_jump(tcx, active_scripts, owner, seq_id, elem_idx),

            Command::ActivateApple
            | Command::ActivateArrow
            | Command::ActivateHandle
            | Command::ActivateHeal
            | Command::ActivateLever
            | Command::ActivateMoney
            | Command::ActivateSearch
            | Command::ActivateStone
            | Command::ActivateSword => {
                self.instruct_activate_target(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }

            Command::PlayAnim
            | Command::PlayAnimLoop
            | Command::PlayAnimFreeze
            | Command::PlayAnimFrozen => {
                self.instruct_play_anim(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }

            Command::HitTarget
            | Command::HandleTarget
            | Command::UseLever
            | Command::TakeTarget
            | Command::SearchCmd => {
                self.instruct_target_interaction(tcx, active_scripts, owner, seq_id, elem_idx, cmd)
            }

            Command::Generic => {}

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
            }
        }
    }
}
