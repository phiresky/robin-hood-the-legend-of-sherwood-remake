//! Soldier-specific helpers used by the engine, scripts, and cheats.
//!
//! Small soldier-actor methods that don't fit naturally into any of
//! the larger modules — attentive-mode transitions, drunken-step
//! perturbation, etc.

use super::movement::GoalShape;
use super::{EngineInner, LevelAssets};
use crate::ai::{DoorCombatInfo, Position, Stimulus, StimulusType};
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{Command, Entity, EntityId, Posture};
use crate::order::OrderType;
use crate::sequence::{PendingCondolation, SequenceElement, SequenceId};

impl EngineInner {
    /// Request a soldier enter or leave attentive mode, launching the
    /// appropriate transition-animation sequence element.
    ///
    /// Early-exits when the target matches `will_be_attentive`,
    /// otherwise picks `EnterAttentiveMode` / `LeaveAttentiveMode` /
    /// `LeaveAttentiveModeOfficer` and launches it as a
    /// `SequenceElement`.  The officer variant is only chosen when
    /// `fast_variant` is set *and* the sprite actually has the officer
    /// transition animation.
    pub(crate) fn set_soldier_attentive_mode(
        &mut self,
        entity_id: EntityId,
        target: bool,
        fast_variant: bool,
    ) {
        let (will_be_attentive, has_officer_anim) = match self.world.entities.get(entity_id) {
            Some(Entity::Soldier(s)) => (
                s.npc
                    .ai_brain
                    .enemy()
                    .map(|e| e.will_be_attentive)
                    .unwrap_or(false),
                s.element
                    .sprite
                    .has_animation(OrderType::TransitionWaitingAlertedWaitingUprightOfficer),
            ),
            _ => return,
        };

        if target == will_be_attentive {
            return;
        }

        let command = if target {
            Command::EnterAttentiveMode
        } else if fast_variant && has_officer_anim {
            Command::LeaveAttentiveModeOfficer
        } else {
            Command::LeaveAttentiveMode
        };

        self.launch_element(SequenceElement::new(1, command, Some(entity_id)));

        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(entity_id)
            && let Some(enemy) = s.npc.ai_brain.enemy_mut()
        {
            enemy.will_be_attentive = target;
        }
    }

    /// Translate the body of an attentive-mode transition command during
    /// sequence dispatch.  Returns `true` when the transition animation
    /// was queued (caller should mark the sequence element in-progress
    /// and let the animation-DONE handler terminate it); returns
    /// `false` when the soldier snapped to the target flag (caller
    /// should terminate the sequence element immediately).
    ///
    /// When the sequence's `posture_after_transition` is Upright and
    /// the target flag differs from the current, start the transition
    /// animation via `active_ai_anim`.  Otherwise snap the flag
    /// immediately ("Consider as done").
    pub(super) fn dispatch_attentive_transition(
        &mut self,
        owner: EntityId,
        command: Command,
        posture_after_transition: Posture,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> bool {
        let target_attentive = matches!(command, Command::EnterAttentiveMode);
        let anim = match command {
            Command::EnterAttentiveMode => OrderType::TransitionWaitingUprightWaitingAlerted,
            Command::LeaveAttentiveMode => OrderType::TransitionWaitingAlertedWaitingUpright,
            Command::LeaveAttentiveModeOfficer => {
                OrderType::TransitionWaitingAlertedWaitingUprightOfficer
            }
            _ => return false,
        };

        // LeaveAttentiveModeOfficer has no else branch — the officer
        // salute-and-drop animation is pushed unconditionally.
        if matches!(command, Command::LeaveAttentiveModeOfficer) {
            self.push_new_order(seq_id, elem_idx, anim, 0.0, 0.0);
            return true;
        }

        // Gate on the posture the actor will hold when the sequence completes.
        let posture_upright_after = posture_after_transition == Posture::Upright;
        let (currently_attentive, idle) = {
            let Some(entity) = self.world.entities.get(owner) else {
                return false;
            };
            let cur = entity.enemy_ai().is_some_and(|e| e.attentive);
            // The soldier is considered idle when their current in-progress
            // element is the low-priority wait element (no active action
            // order driving the sprite).
            let idle = self
                .sequence_manager
                .current_element_for_actor(owner)
                .and_then(|(s, e)| self.sequence_manager.get_element(s, e))
                .map(|el| el.command == Command::Wait)
                .unwrap_or(true);
            (cur, idle)
        };

        let needs_change = currently_attentive != target_attentive;
        let can_play_transition = posture_upright_after && idle && needs_change;

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
            "dispatch_attentive_transition"
        );

        if can_play_transition {
            // Push the transition animation onto the attentive-mode
            // sequence element so the animation driver picks it up via
            // `current_order_for_actor`; the default `AdvanceElement`
            // completion chains through `do_next_order` when the sprite
            // finishes.
            self.push_new_order(seq_id, elem_idx, anim, 0.0, 0.0);
            true
        } else {
            if let Some(entity) = self.world.entities.get_mut(owner)
                && let Some(enemy) = entity.enemy_ai_mut()
            {
                enemy.attentive = target_attentive;
            }
            false
        }
    }

    /// Translate `ReceiveWaspSting` during sequence dispatch.  Books
    /// the `GettingFreeFromWasp` turning animation with a random
    /// rotation offset (-8..=+8), says the wasp-sting remark, and
    /// fires the `EventWasp` AI stimulus.
    ///
    /// The original queues `bee_time` struggle orders, doubled while
    /// smelling apple. The order-completion side channel below re-arms
    /// each cycle and keeps the sequence element in progress until the
    /// last cycle finishes.
    pub(super) fn dispatch_receive_wasp_sting(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        let Some(entity) = self.world.entities.get(owner) else {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        };
        if !entity.is_soldier() {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        }

        // Random rotation offset, applied to direction_goal so the
        // soldier rotates during the animation.  Range is `rand(0..17) - 8`.
        let rotation =
            crate::sim_rng::i32(crate::sim_rng::RngSite::SoldierFreedRotation, 0..17) - 8;
        let new_goal = {
            let current_goal = i16::from(entity.position_iface().get_direction_goal());
            (current_goal + rotation as i16).rem_euclid(16)
        };

        // On-ladder soldiers fall off the ladder before the struggle begins.
        let on_ladder = entity.element_data().posture == crate::element::Posture::OnLadder;

        // bee_time struggle cycles, doubled when apple-smelling.  Each
        // cycle is one `GettingFreeFromWasp` animation; the sequence
        // element stays in-progress until the last one finishes.
        let bee_time = entity
            .soldier_data()
            .and_then(|s| assets.profile_manager.get_soldier(s.soldier_profile_index))
            .map(|p| {
                let base = p.bee_time.max(1) as u32;
                if entity
                    .soldier_data()
                    .map(|s| s.apple_smell > 0)
                    .unwrap_or(false)
                {
                    base * 2
                } else {
                    base
                }
            })
            .unwrap_or(1)
            .min(u16::MAX as u32) as u16;

        // Drop the immutable entity borrow before any mutable self calls.
        let _ = entity;

        // Process the ladder fall before the wasp-struggle orders are
        // queued; the soldier needs to leave the lift first and the
        // animation queue would otherwise be clobbered.
        if on_ladder {
            self.translate_ladder_wall_fall(owner, (seq_id, elem_idx));
        }

        // Book the first struggle animation onto the wasp-sting
        // sequence element so `current_order_for_actor` picks it up.
        // Each cycle's completion hook (`WaspStruggleCycle`) decides
        // whether to re-push the next cycle or terminate the element.
        if let Some(entity) = self.world.entities.get_mut(owner) {
            if entity.actor_data().is_some() {
                entity.position_iface_mut().set_direction(
                    crate::position_interface::Direction::from_raw(new_goal as i32),
                );
            }
            if let Some(npc) = entity.npc_data_mut() {
                npc.wasp_victim = true;
            }
        }
        let order = crate::order::Order::new(
            OrderType::GettingFreeFromWasp,
            0.0,
            0.0,
            self.alloc_order_id(),
        )
        .with_completion(crate::order::OrderCompletion::WaspStruggleCycle {
            cycles_remaining: bee_time,
        });
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);

        // Queue the EventWasp AI stimulus.
        self.dispatch_ai_stimulus(owner, Stimulus::new(StimulusType::EventWasp));

        self.sequence_manager.element_in_progress(seq_id, elem_idx);
    }

    /// Drain and dispatch `SendCondolationCard` notifications queued by
    /// the sequence manager since the last call.  Invoked by the tick
    /// loop after `hourglass` so every sequence-element terminal state
    /// change (Terminated / Interrupted / Impossible) fires its
    /// per-entity cleanup in a single pass.
    ///
    /// Notifications are queued (rather than fired inline) to avoid
    /// re-entrant borrows; this method drains the queue.
    pub(super) fn dispatch_condolations(&mut self, assets: &LevelAssets) {
        let mut pending: std::collections::VecDeque<_> =
            self.sequence_manager.drain_pending_condolations().into();
        while let Some(dispatch) = pending.pop_front() {
            let owner = dispatch.card.owner;
            self.send_condolation_card(dispatch.card, assets);
            self.drain_self_stimuli_for_npc(owner, assets);
            self.sequence_manager.finish_pending_condolation(dispatch);

            for nested in self
                .sequence_manager
                .drain_pending_condolations()
                .into_iter()
                .rev()
            {
                pending.push_front(nested);
            }
        }
    }

    /// Complete pending condolence stack frames after an NPC Think call.
    /// Cascading `SetState` calls can cross owners, so filtering by the
    /// originating NPC would strand a nested `SendCondolationCard` until
    /// later in the frame.  Once this synchronous boundary is entered, all
    /// queued cards must therefore drain depth-first.
    pub(super) fn dispatch_condolations_for_npc(
        &mut self,
        _npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.dispatch_condolations(assets);
    }

    /// Dispatch a single `SendCondolationCard` to the owner entity.
    ///
    /// When a sequence element reaches a terminal state, walk the
    /// remaining chain via `is_last_real_action`; if nothing
    /// meaningful follows (and the AI isn't inside a `Halt()` call),
    /// dispatch a command-specific stimulus back to the owner so the
    /// AI state machine can advance.  That dispatch is what unsticks
    /// substates like `DefaultOnPostLookingSidewards`, whose only exit
    /// is an `EventDone` stimulus after the `LookLeft` / `LookRight`
    /// sequence completes.
    fn send_condolation_card(&mut self, card: PendingCondolation, assets: &LevelAssets) {
        use crate::sequence::SequenceState;
        let PendingCondolation {
            owner,
            command,
            terminal_state,
            seq_id,
            elem_idx,
            from_halt,
            postponed_successor_pending,
        } = card;

        // Orders live on `SequenceElement.orders` and disappear when
        // the element is cancelled / destroyed (see `Sequence::set_element_state`
        // clearing `orders` in the `Interrupted` / `Impossible` arms).
        // That automatically invalidates anything the animation driver
        // reads via `current_order_for_actor`, so no per-actor cleanup
        // is needed here.

        // Snapshot the owner's posture for the `is_very_very_busy` check
        // below without holding a mutable borrow on `self.world.entities` — so
        // the Impossible arm can read `self.sequence_manager` without a
        // split-borrow conflict, and the final state mutation below can
        // use a fresh `get_mut`.
        if !self.world.entities.get(owner).is_some() {
            return;
        }

        // PC override.  Runs before the human-base chain, so its
        // STRANGLE / TAKE_CORPSE antagonist-side effects fire
        // regardless of the `is_last_real_action` / `from_halt` gates
        // below (those gate the per-owner stimulus dispatch, which is
        // NPC-only — the PC has no `ai_controller` for
        // `fire_self_stimulus` to land on).
        if self.world.entities.get(owner).is_some_and(|e| e.is_pc()) {
            self.send_condolation_card_pc(owner, command, seq_id, elem_idx, assets);
        }

        // Soldier override: ReceiveWaspSting termination clears the
        // wasp-victim flag.  The human-base cleanup runs before the
        // halt-method guard, so it fires whether or not `from_halt`
        // is set.
        if matches!(command, Command::ReceiveWaspSting)
            && let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(npc) = entity.npc_data_mut()
        {
            npc.wasp_victim = false;
        }

        // When a movement element with the MAP flag terminates, toggle
        // `active` / `in_honolulu` / anti-collision based on whether
        // the actor's current map position is inside the playable map
        // box.  Without this the reinforcement-door flow (PASS_DOOR |
        // MAP) leaves the actor sitting on the wrong side of the
        // activate/honolulu split — script-driven `SetActorLocation`
        // is the only other path that cleans it up.  Runs unconditionally,
        // regardless of `from_halt` / the `is_last_real_action` Think
        // gate below.
        let map_flag_terminated = self
            .sequence_manager
            .get_element(seq_id, elem_idx as usize)
            .map(|e| {
                matches!(
                    e.data,
                    crate::sequence::SequenceElementData::Movement { flags, .. }
                        if flags.contains(crate::sequence::MoveFlags::MAP)
                )
            })
            .unwrap_or(false);
        if map_flag_terminated && let Some(entity) = self.world.entities.get_mut(owner) {
            let pos = entity.element_data().position_map();
            let inside_map = self.world.fast_grid.is_inside_grid_point(pos);
            // Re-borrow mutably (the read above released the borrow).
            if let Some(entity) = self.world.entities.get_mut(owner) {
                let ed = entity.element_data_mut();
                ed.active = inside_map;
                ed.in_honolulu = !inside_map;
                if inside_map {
                    ed.sprite.position_iface.set_anti_collision_on(true);
                }
            }
        }

        // Skip the per-command Think dispatch when further real
        // actions follow in the chain, or when we're tearing the
        // sequence down from inside a `Halt()` call.
        if from_halt
            || postponed_successor_pending
            || !self
                .sequence_manager
                .is_last_real_action(seq_id, elem_idx as usize)
        {
            return;
        }

        // Decide which stimulus the owner should see based on the
        // command and terminal state.
        let stimulus = match command {
            // Motion commands — Terminated → ReachPoint, Impossible →
            // CouldntReachPoint.  Nothing for Interrupted (motion
            // dispatch has no Interrupted stimulus).
            Command::PassDoor | Command::Move | Command::MoveOk | Command::SitDown => {
                match terminal_state {
                    SequenceState::Terminated => Some(StimulusType::EventReachPoint),
                    SequenceState::Impossible => {
                        // Flip `was_busy` + take AILOCK_BUSY when the
                        // actor got stuck because it's "very very busy"
                        // — posture Flying / OnLadder / OnWall, or the
                        // active sequence element is a PassDoor / Fall.
                        // Mid-transition postures can't accept a fresh
                        // move, so the AI holds on AILOCK_BUSY until
                        // the posture clears.  The per-tick edge
                        // detector (`tick_npc_busy_edge_detect`)
                        // handles the symmetric unlock when the busy
                        // state clears.
                        if self.is_very_very_busy(owner)
                            && let Some(ent) = self.world.entities.get_mut(owner)
                            && let Some(ai) = ent.ai_controller_mut()
                            && !ai.was_busy
                        {
                            ai.non_script_lock(crate::ai::AiLockFlags::BUSY);
                            ai.was_busy = true;
                        }
                        Some(StimulusType::EventCouldntReachPoint)
                    }
                    _ => None,
                }
            }

            // Generic action commands — Terminated or Interrupted →
            // EventDone, any other terminal state → EventImpossible.
            // Covers the long case list including LookLeft / LookRight
            // / LeanOut which is what the idle "look around" cascade
            // waits on.
            Command::DrinkWhisky
            | Command::KickLow
            | Command::Point
            | Command::FlyDoor
            | Command::Untie
            | Command::Fainted
            | Command::Recover
            | Command::Knee
            | Command::Turn
            | Command::TurnElement
            | Command::TurnFast
            | Command::SwordstrikeThrustA
            | Command::SwordstrikeThrustB
            | Command::SwordstrikeThrustC
            | Command::SwordstrikeThrustD
            | Command::SwordstrikeThrustE
            | Command::SwordstrikeThrustF
            | Command::SwordstrikeThrustG
            | Command::SwordstrikeThrustH
            | Command::SwordstrikeThrustI
            | Command::SwordstrikeTired
            | Command::StandUp
            | Command::SwordstrikeDown
            | Command::StopParrySword
            | Command::LookLeft
            | Command::LookRight
            | Command::LeanOut
            | Command::WakeUp
            | Command::ReceiveSwordDamage
            | Command::ReceiveHitDamage
            | Command::EquipBow
            | Command::EquipBowUp
            | Command::EquipBowDown
            | Command::RaiseBow
            | Command::ShootBow
            | Command::Take
            | Command::HitCmd
            | Command::SearchCmd
            | Command::DrinkAle
            | Command::LowerShield
            | Command::StartMenace
            | Command::EnterSwordfight => match terminal_state {
                SequenceState::Terminated | SequenceState::Interrupted => {
                    Some(StimulusType::EventDone)
                }
                _ => Some(StimulusType::EventImpossible),
            },

            // Smalltalk-combat commands — only Terminated / Interrupted
            // fire, no Impossible fallback.
            Command::SwordstrikeSmalltalkLeft
            | Command::SwordstrikeSmalltalkRight
            | Command::ParrySmalltalkLeft
            | Command::ParrySmalltalkRight => match terminal_state {
                SequenceState::Terminated | SequenceState::Interrupted => {
                    Some(StimulusType::EventDone)
                }
                _ => None,
            },

            // Plain Wait — EventDone only on clean termination.
            Command::Wait => match terminal_state {
                SequenceState::Terminated => Some(StimulusType::EventDone),
                _ => None,
            },

            // Wasp sting — fires EventWaspAway on every terminal state.
            Command::ReceiveWaspSting => match terminal_state {
                SequenceState::Terminated
                | SequenceState::Impossible
                | SequenceState::Interrupted => Some(StimulusType::EventWaspAway),
                _ => None,
            },

            _ => None,
        };

        if let Some(st) = stimulus
            && let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            tracing::trace!(
                owner = owner.index(),
                ?command,
                ?terminal_state,
                stimulus = ?st,
                "send_condolation_card: fire EventDone/EventReachPoint to owner"
            );
            ai.fire_self_stimulus(st);
        }
    }

    /// PC-specific arms of `send_condolation_card`.
    ///
    /// Two arms with antagonist side-effects:
    ///
    /// - **Strangle**: when a strangle element terminates, reset the
    ///   victim's AI/eye state.  Fires for both the success path
    ///   (victim about to be killed via ReceiveDamage — the Wait /
    ///   Unlock / SetView calls are dead-on-corpse but kept for
    ///   symmetry) and the abort/non-stranglable path (victim alive —
    ///   needs the AI un-wedged).
    ///
    /// - **TakeCorpse**: if the carrier was mid-grab (`pc.carried` set
    ///   but the carried's posture isn't yet `Carried`), force-drop
    ///   the partially-grabbed body.  `pc.carried` is set by
    ///   `begin_carry` at TakeCorpse init.
    fn send_condolation_card_pc(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: SequenceId,
        _elem_idx: u16,
        _assets: &LevelAssets,
    ) {
        match command {
            Command::StrangleCmd => {
                // Look up the antagonist NPC stored on the strangle
                // element's `Interaction` data.  Accept any non-PC
                // living human.
                let Some(elem) = self
                    .sequence_manager
                    .get_element(seq_id, _elem_idx as usize)
                else {
                    return;
                };
                let Some(victim_id) = (match &elem.data {
                    crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
                    _ => None,
                }) else {
                    return;
                };

                // Low-priority Wait element to re-park the victim's AI
                // in the default loop.
                self.actor_wait(victim_id);

                // Release the AILOCK_FREEZE acquired in `begin_strangle`.
                if let Some(victim) = self.world.entities.get_mut(victim_id) {
                    if let Some(ai) = victim.ai_controller_mut() {
                        ai.non_script_unlock(crate::ai::AiLockFlags::FREEZE);
                    }
                    // Alert the victim's AI that the PC was the
                    // aggressor.  Today `dispatch_ai_stimulus` only
                    // delivers to soldiers; civilians' friendly AI
                    // doesn't accept EventGotHit yet, so a civilian
                    // victim is a no-op here (matches the existing
                    // not-stranglable arm in `combat.rs::StrangleDone`).
                    let _ = victim;
                }
                let stim = crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventGotHit,
                    owner.index(),
                );
                self.dispatch_ai_stimulus(victim_id, stim);

                // Reset the victim's gaze (cosmetic; dead-on-corpse on
                // the kill path, observable on abort).
                if let Some(victim) = self.world.entities.get_mut(victim_id)
                    && let Some(npc) = victim.npc_data_mut()
                {
                    crate::ai_vision::set_view_status(npc, crate::element::EyeStatus::LookForward);
                }

                tracing::debug!(
                    pc = owner.index(),
                    victim = victim_id.index(),
                    "send_condolation_card_pc: strangle terminated → wait + unlock(freeze) + event_got_hit + look_forward"
                );
            }

            Command::TakeCorpse => {
                // If mid-grab cancel (`pc.carried.is_some()` but the
                // carried's posture isn't yet `Carried`), drop the
                // partially-grabbed corpse instantly.
                let (carried_id, posture_is_carried) = {
                    let Some(carrier) = self.world.entities.get(owner) else {
                        return;
                    };
                    let Some(carried_id) = carrier.pc_data().and_then(|pc| pc.carried) else {
                        return;
                    };
                    let posture_is_carried = self
                        .world
                        .entities
                        .get(carried_id)
                        .map(|e| e.element_data().posture == crate::element::Posture::Carried)
                        .unwrap_or(false);
                    (carried_id, posture_is_carried)
                };

                if posture_is_carried {
                    // Pickup completed cleanly — nothing to drop.
                    return;
                }

                // Instantaneous drop (already ported as
                // `force_drop_carried_corpse_instant` in
                // `engine/melee.rs`).  Also clear the actor's
                // `active_ability` so a stale Carry slot can't drive a
                // bogus `CarryDone` after the element is gone.
                self.force_drop_carried_corpse_instant(owner);
                if let Some(carrier) = self.world.entities.get_mut(owner)
                    && let Some(actor) = carrier.actor_data_mut()
                    && actor.active_ability.kind == Some(crate::movement::AbilityKind::Carry)
                {
                    actor.active_ability.clear();
                }

                tracing::debug!(
                    pc = owner.index(),
                    carried = carried_id.index(),
                    "send_condolation_card_pc: take_corpse mid-grab cancel → instant drop"
                );
            }

            _ => {}
        }
    }

    /// Orchestrate a door-battle after `EnemyInHouseAlert`: pick the
    /// nearest unlocked building door, layout defender/attacker
    /// positions around it, and fan out
    /// [`EngineInner::send_before_door_to_fight`] to each occupant.
    ///
    /// `fleeing` is the outnumbered side (leaves the house first);
    /// `pursuing` is the stronger side (follows and provokes for
    /// duel).  The `pursuing.len() >= fleeing.len()` invariant is
    /// enforced by the caller.
    pub(crate) fn init_battle_before_door(
        &mut self,
        door_indices: &[u32],
        fleeing: &[EntityId],
        pursuing: &[EntityId],
    ) {
        if fleeing.is_empty() || pursuing.is_empty() {
            return;
        }
        // Use the first fleeing guy as the distance anchor for the
        // nearest-door search.
        let first_fleeing = fleeing[0];
        let Some(entity) = self.get_entity(first_fleeing) else {
            return;
        };
        let first_pos = entity.element_data().position_map();
        let move_box = *entity.position_iface().get_move_box();

        // Pick the unlocked door nearest to first_pos by MaxNorm of
        // (door.point_in - first_pos).
        let (point_in, point_out, point_mid, out_layer) = {
            let Some(host) = self.mission_script.as_ref().and_then(|s| s.game_host()) else {
                return;
            };
            let mut best: Option<(u32, f32)> = None;
            for &di in door_indices {
                let Some(door) = host.doors.get(di as usize) else {
                    continue;
                };
                if door.is_locked_pc() || door.is_locked_npc_villain() {
                    continue;
                }
                let dist = (door.point_in.x - first_pos.x)
                    .abs()
                    .max((door.point_in.y - first_pos.y).abs());
                if best.map(|(_, d)| dist < d).unwrap_or(true) {
                    best = Some((di, dist));
                }
            }
            let Some((best_idx, _)) = best else {
                return;
            };
            let Some(door) = host.doors.get(best_idx as usize) else {
                return;
            };
            (
                door.point_in,
                door.point_out,
                door.point_mid,
                door.layer_out,
            )
        };
        let _ = point_in;

        // Battle center = door.point_out; facing = sector(point_out -
        // point_mid) via `vector_to_sector_0_to_15_iso` on the door
        // vector.
        let center = point_out;
        let dir_vec = point_out - point_mid;
        let base_direction =
            crate::position_interface::vector_to_sector_0_to_15_iso(dir_vec.x, dir_vec.y);

        // pursuing >= fleeing is asserted by the caller.
        let num_pursuing = pursuing.len();
        let num_fleeing = fleeing.len();
        debug_assert!(num_pursuing >= num_fleeing);

        for i in 0..num_pursuing {
            // Try 10 random dispersion vectors.  The dispersed
            // direction is `(base + rand(0..7) - 3) & 15`, and the
            // vector magnitude is `30 + rand(0..64)`.  The defender
            // goes to `center + vec`; the attacker to
            // `center + 0.5 * vec`.
            let mut found = false;
            let mut dispersed_direction: i16 = base_direction;
            let mut defender = center;
            let mut attacker = center;
            for _ in 0..10 {
                let jitter =
                    crate::sim_rng::i32(crate::sim_rng::RngSite::DoorFightDispersion, 0..7) - 3;
                let dd = (base_direction + jitter as i16).rem_euclid(16);
                let magnitude = 30.0
                    + crate::sim_rng::u32(crate::sim_rng::RngSite::DoorFightDispersion, 0..64)
                        as f32;
                // Apply the isometric aspect ratio to the Y component
                // — `direction_vector_16` returns a pure unit vector,
                // but the door-battle dispersion expects an
                // iso-compressed offset.
                let (dx, dy_raw) = crate::element_kinds::direction_vector_16(dd);
                let dy = dy_raw * crate::position_interface::ASPECT_RATIO;
                let offset = MapVec::new(dx * magnitude, dy * magnitude);
                let cand = center + offset;
                if self
                    .world
                    .fast_grid
                    .is_straight_movement_authorized(center, cand, out_layer, &move_box)
                {
                    dispersed_direction = dd;
                    defender = cand;
                    attacker = center + offset.scale(0.5);
                    found = true;
                    break;
                }
            }
            if !found {
                // Emergency fallback: collapse both positions to the
                // battle center.
                dispersed_direction = base_direction;
                defender = center;
                attacker = center;
            }

            let defender_pos = Position {
                x: defender.x,
                y: defender.y,
                sector: None,
                level: out_layer,
            };
            let attacker_pos = Position {
                x: attacker.x,
                y: attacker.y,
                sector: None,
                level: out_layer,
            };

            if i < num_fleeing {
                // Fleeing guy exits and faces back
                // (`dispersed_direction ^ 8`).  Pursuer follows and
                // provokes.
                self.send_before_door_to_fight(
                    fleeing[i],
                    defender_pos,
                    (dispersed_direction ^ 8) as u16,
                    (i as u16) * 10 + 10,
                    None,
                );
                self.send_before_door_to_fight(
                    pursuing[i],
                    attacker_pos,
                    dispersed_direction as u16,
                    ((i + num_fleeing) as u16) * 10 + 20,
                    Some(fleeing[i]),
                );
            } else {
                // Extra pursuers pick a random fleeing guy as their
                // duel target.
                let target_idx = crate::sim_rng::u32(
                    crate::sim_rng::RngSite::DoorFightTarget,
                    0..num_fleeing as u32,
                ) as usize;
                self.send_before_door_to_fight(
                    pursuing[i],
                    attacker_pos,
                    dispersed_direction as u16,
                    ((i + num_fleeing) as u16) * 10 + 20,
                    Some(fleeing[target_idx]),
                );
            }
        }
    }

    /// Send a human actor before a door to fight.  Dispatches on the
    /// actor's concrete type:
    ///
    /// * **Soldier** — builds a `DoorCombatInfo` and queues an
    ///   `EventDoorCombat` stimulus on the soldier's own AI.  The
    ///   stimulus lands on `pending_stimuli` so the next think tick
    ///   picks it up (see the `EventDoorCombat` arm in `ai_enemy.rs`);
    ///   the AI then transitions into
    ///   `Substate::AttackingDoorFightDelay`.
    /// * **PC** — synthesises a four-element sequence (WaitTimer →
    ///   Move → Turn → optional EnterSwordfight) and launches it
    ///   directly.  PCs aren't driven by an enemy-AI brain so the
    ///   stimulus path doesn't apply.
    ///
    /// Any other entity kind reaching this helper is a wiring bug.
    pub(crate) fn send_before_door_to_fight(
        &mut self,
        actor_id: EntityId,
        goal: Position,
        direction: u16,
        delay: u16,
        adversary: Option<EntityId>,
    ) {
        let kind = self
            .world
            .entities
            .get(actor_id)
            .map(|e| (e.is_pc(), e.is_soldier()));
        match kind {
            Some((true, _)) => {
                self.send_before_door_to_fight_pc(actor_id, goal, direction, delay, adversary);
            }
            Some((_, true)) => {
                let info = DoorCombatInfo {
                    delay,
                    goal,
                    direction,
                    adversary: adversary.map(|id| id.index()).unwrap_or(0),
                };
                self.dispatch_ai_stimulus(
                    actor_id,
                    Stimulus::with_door_combat(StimulusType::EventDoorCombat, info),
                );
            }
            _ => {
                tracing::warn!(
                    actor = actor_id.index(),
                    "send_before_door_to_fight: actor is neither PC nor Soldier"
                );
            }
        }
    }

    /// PC override of [`Self::send_before_door_to_fight`].  Builds a
    /// synthetic `WaitTimer → Move → Turn → [EnterSwordfight]`
    /// sequence and launches it on the PC.
    ///
    /// Cross-sector moves use the shared gate-routing builder with
    /// this method's wait element as a prefix and the turn / optional
    /// swordfight elements as a tail.
    ///
    /// **Enemy-VIP filter:** the swordfight gate is
    /// `enemy.is_some() && !(!is_robin && enemy.is_vip)` — i.e.
    /// non-Robin PCs refuse to engage VIP enemies.  Soldiers use the
    /// cached enemy-AI VIP flag; civilians use the cached civilian type.
    fn send_before_door_to_fight_pc(
        &mut self,
        pc_id: EntityId,
        goal: Position,
        direction: u16,
        delay: u16,
        enemy_to_attack: Option<EntityId>,
    ) {
        use crate::sequence::{Field, FieldValue, MoveFlags, Sequence, SequenceElementData};

        let is_robin = self
            .world
            .entities
            .get(pc_id)
            .and_then(|e| e.pc_data())
            .map(|pc| pc.robin)
            .unwrap_or(false);

        // `WaitTimer` element with `Field::Timer = delay`.
        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(pc_id));
        wait.set_property(Field::Timer, FieldValue::Integer(delay as u32));

        let mut tail_elements: Vec<SequenceElement> = Vec::new();
        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(pc_id));
        turn.set_property(Field::Direction, FieldValue::Integer(direction as u32));
        tail_elements.push(turn);

        // Optional `EnterSwordfight`, gated by
        // `enemy_to_attack.is_some() && (is_robin || !enemy_is_vip)`.
        if let Some(enemy_id) = enemy_to_attack {
            let enemy_is_vip = self
                .world
                .entities
                .get(enemy_id)
                .map(door_combat_enemy_is_vip)
                .unwrap_or(false);
            if is_robin || !enemy_is_vip {
                let mut enter =
                    SequenceElement::new_generic(1, Command::EnterSwordfight, Some(pc_id));
                enter.set_property(Field::Opponent, FieldValue::Element(enemy_id));
                // No jumpline for this swordfight (door-battle is sector-local).
                enter.set_property(Field::JumplineDestination, FieldValue::Integer(0));
                tail_elements.push(enter);
            }
        }

        let source = self.get_entity(pc_id).map(|e| {
            let elem = e.element_data();
            (
                elem.position_map(),
                elem.sector(),
                elem.layer(),
                e.actor_auth_info(),
            )
        });

        if let Some((source_pos, Some(source_sector), _source_layer, auth)) = source
            && let Some(goal_sector) = goal.sector
            && source_sector != goal_sector
        {
            let path = {
                let level = self.world.fast_grid.level.clone();
                let game_host = self.mission_script.as_ref().and_then(|s| s.game_host());
                game_host.and_then(|h| {
                    crate::gate::find_path_gates(
                        &h.doors,
                        (source_pos.x, source_pos.y),
                        source_sector.get(),
                        (goal.x, goal.y),
                        goal_sector.get(),
                        Some(&auth),
                        false,
                        &|sector| {
                            level
                                .sectors
                                .iter()
                                .find(|candidate| candidate.sector_number == sector)
                                .and_then(|candidate| candidate.lift_type)
                        },
                    )
                })
            };
            if let Some(path) = path
                && !path.is_empty()
            {
                self.build_gate_movement_sequence(
                    pc_id,
                    path,
                    GoalShape::Point(MapPoint::new(goal.x, goal.y)),
                    goal.level,
                    crate::order::OrderType::RunningUpright,
                    true,
                    1.0,
                    MoveFlags::empty(),
                    vec![wait],
                    tail_elements,
                    false,
                    false,
                );
                return;
            }
        }

        let mut sequence = Sequence::new();
        let mut level: u16 = 1;

        sequence.append_element(wait);
        level += 1;

        // Single-MOVE same-sector port — see method doc for the
        // cross-sector caveat.  Uses the `RunningUpright` animation.
        let mut move_elem = SequenceElement::new_movement(
            level,
            Command::Move,
            Some(pc_id),
            OrderType::RunningUpright,
        );
        if let SequenceElementData::Movement {
            destination,
            layer,
            sector,
            tolerance,
            flags,
            ..
        } = &mut move_elem.data
        {
            *destination = crate::coordinates::MapPoint {
                x: goal.x,
                y: goal.y,
            };
            *layer = goal.level;
            *sector = goal.sector;
            *tolerance = 0.0;
            *flags = MoveFlags::empty();
        }
        sequence.append_element(move_elem);
        level += 1;

        for mut elem in tail_elements {
            elem.command_level = level;
            sequence.append_element(elem);
            level += 1;
        }

        self.launch_sequence(sequence);
    }
}

fn door_combat_enemy_is_vip(entity: &Entity) -> bool {
    match entity {
        Entity::Soldier(s) => s.npc.ai_brain.enemy().map(|en| en.is_vip).unwrap_or(false),
        Entity::Civilian(c) => {
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Vip
        }
        Entity::Pc(_) => {
            panic!("door combat PC enemy VIP lookup requires LevelAssets profile data")
        }
        _ => panic!("door combat enemy is not a human actor"),
    }
}

/// Pick the slow-vs-very-slow turning speed for an ale-intoxicated soldier.
///
/// Angles 14/15/0/1/2 of `(direction - goal) & 15` (the soldier is
/// nearly aligned with the goal) return `true` (→ `TurnVerySlow`);
/// every other delta returns `false` (→ `TurnSlow`).  `direction`
/// and `goal` are 16-sector compass values (0..=15).
///
/// Wired into [`crate::engine::movement::EngineInner::tick_move`] via the
/// drunken-turn pre-pass: each tick while a drunken soldier has an
/// active walk path, the pre-pass advances the soldier's facing one
/// step toward the movement-vector direction using this function to
/// pick the step size.
pub fn turn_drunken_is_very_slow(direction: u16, goal: u16) -> bool {
    let delta = direction.wrapping_sub(goal) & 15;
    matches!(delta, 14 | 15 | 0 | 1 | 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_drunken_very_slow_window() {
        // Facing close to goal → TurnVerySlow.
        for delta in [14u16, 15, 0, 1, 2] {
            assert!(turn_drunken_is_very_slow(delta, 0));
        }
        // Further off → TurnSlow.
        for delta in [3u16, 5, 8, 10, 13] {
            assert!(!turn_drunken_is_very_slow(delta, 0));
        }
    }

    #[test]
    fn turn_drunken_wraps_around() {
        // (direction - goal) must wrap: direction=0, goal=1 → delta=15 → very slow.
        assert!(turn_drunken_is_very_slow(0, 1));
        // direction=0, goal=3 → delta=13 → slow.
        assert!(!turn_drunken_is_very_slow(0, 3));
    }
}
