//! Per-frame strike ticks (melee, sweep, push, rider, enemy AI) and concussion healing.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId, EyeStatus, Posture};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

fn sweep_rotation_complete(sweep: &crate::movement::SweepState) -> bool {
    match sweep.direction {
        crate::profiles::WeaponThrustDirection::LeftToRight => {
            sweep.current_angle >= sweep.final_angle
        }
        _ => sweep.current_angle <= sweep.final_angle,
    }
}

fn true_sweep_still_rotating(sweep: &crate::movement::SweepState) -> bool {
    matches!(
        sweep.strike_kind,
        WeaponThrustKind::TrueCircle | WeaponThrustKind::TrueHalfCircle
    ) && !sweep_rotation_complete(sweep)
}

impl EngineInner {
    // ─── Per-frame melee tick ───────────────────────────────────────

    /// Per-frame melee combat tick.
    ///
    /// Processes two categories of melee combat:
    /// 1. **Sequence-driven strikes**: actors with `ActiveMelee` set by
    ///    `dispatch_sword_strike` — count down timers, apply damage at
    ///    the hit frame, clean up on completion.
    /// 2. **Enemy AI strikes**: soldiers in `AttackingSwordfight` substate
    ///    whose cooldown has expired — check distance, apply damage directly.
    ///
    /// Also ticks concussion healing for all humans.
    pub(crate) fn tick_melee_combat(&mut self, assets: &LevelAssets) {
        if self.freeze_all {
            return;
        }

        // Periodic combat state dump (every 64 frames)
        if self.frame_counter.is_multiple_of(64) {
            for (entity_id, entity) in self.entities_iter_with_id() {
                let Some(human) = entity.human_data() else {
                    continue;
                };
                if human.opponents.is_empty() {
                    continue;
                }
                let action = entity.actor_data().map(|a| a.action_state);
                let substate = match entity {
                    Entity::Soldier(s) => Some(s.npc.ai_substate()),
                    _ => None,
                };
                tracing::debug!(
                    entity = ?entity_id,
                    kind = ?entity.kind(),
                    opponents = ?human.opponents,
                    action_state = ?action,
                    ai_substate = ?substate,
                    "COMBAT STATE"
                );
            }
        }

        self.tick_melee_strikes(assets);
        self.tick_sweep_strikes(assets);
        self.tick_parry_counters();
        self.tick_push_flights(assets);
        self.tick_rider_charges(assets);
        self.tick_enemy_sword_attacks(assets);
        let consumed_smalltalk_hint_actors = self.tick_evaluate_swordfight(assets);
        self.tick_smalltalk(assets, &consumed_smalltalk_hint_actors);
        self.tick_concussion_healing(assets);
        self.tick_tiredness(assets);
        self.tick_refresh_hero_mouth();
        self.tick_pc_combat_anim_speech(assets);
        self.tick_refresh_purse_disable(assets);
    }

    /// Decrement parry counters and end the parry stance when the timer
    /// runs out.
    ///
    /// Each frame `parry_counter` counts down; when it hits 0 a
    /// `StopParrySword` sequence is launched which returns the actor
    /// to `WaitingSword`.  `ParrySwordLow` instead terminates its
    /// own sequence element directly (no explicit stop-parry step).
    pub(super) fn tick_parry_counters(&mut self) {
        let mut launch_stop_parry: Vec<EntityId> = Vec::new();
        let mut terminate_low_parry: Vec<(crate::sequence::SequenceId, usize, EntityId)> =
            Vec::new();

        for (entity_id, entity) in self.entities.occupied_mut() {
            let is_parrying = entity
                .actor_data()
                .map(|a| {
                    matches!(
                        a.action_state,
                        ActionState::ParryingSword | ActionState::ParryingSwordLow
                    )
                })
                .unwrap_or(false);
            if !is_parrying {
                // Reset the counter when not parrying so a fresh parry
                // always starts from the full duration.
                if let Some(human) = entity.human_data_mut() {
                    human.parry_counter = 0;
                }
                continue;
            }
            let current_parry_hold = self
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, order)| order.order_type)
                .filter(|order_type| {
                    matches!(
                        order_type,
                        crate::order::OrderType::ParryingSword
                            | crate::order::OrderType::ParryingLowSword
                    )
                });
            if current_parry_hold.is_none() {
                continue;
            }
            let state = entity.actor_data().map(|a| a.action_state);
            let counter = entity.human_data().map(|h| h.parry_counter).unwrap_or(0);
            if counter == 0 {
                if matches!(state, Some(ActionState::ParryingSword)) {
                    launch_stop_parry.push(entity_id);
                } else if matches!(state, Some(ActionState::ParryingSwordLow))
                    && let Some(elem_ref) = self
                        .sequence_manager
                        .in_progress_element_for_actor_matching(entity_id, |elem| {
                            elem.command == Command::ParrySwordLow
                        })
                {
                    terminate_low_parry.push((elem_ref.0, elem_ref.1, entity_id));
                }
                tracing::trace!(
                    "tick_parry_counters: entity={} timer already 0",
                    entity_id.index(),
                );
                continue;
            }
            let new_counter = counter - 1;
            if let Some(human) = entity.human_data_mut() {
                human.parry_counter = new_counter;
            }
            if new_counter == 0 {
                match state {
                    Some(ActionState::ParryingSword) => {
                        launch_stop_parry.push(entity_id);
                    }
                    Some(ActionState::ParryingSwordLow) => {
                        if let Some(elem_ref) = self
                            .sequence_manager
                            .in_progress_element_for_actor_matching(entity_id, |elem| {
                                elem.command == Command::ParrySwordLow
                            })
                        {
                            terminate_low_parry.push((elem_ref.0, elem_ref.1, entity_id));
                        }
                    }
                    _ => {}
                }
                tracing::trace!(
                    "tick_parry_counters: entity={} state={:?} parry timer expired",
                    entity_id.index(),
                    state,
                );
            }
        }

        for owner in launch_stop_parry {
            if self
                .sequence_manager
                .has_unpostponed_element_for_actor_matching(owner, |cmd| {
                    cmd == Command::StopParrySword
                })
            {
                continue;
            }
            let elem =
                crate::sequence::SequenceElement::new(1, Command::StopParrySword, Some(owner));
            self.launch_element(elem);
        }

        for (seq_id, elem_idx, owner) in terminate_low_parry {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            if let Some(Some(entity)) = self.entities.get_mut(owner.index() as usize)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.action_state = ActionState::WaitingSword;
            }
        }
    }

    /// True when the actor's animation is `WaitingSword` for purposes
    /// of running `EvaluateSwordfight`.
    ///
    /// Some sequence-driven combat commands keep `action_state` at
    /// `WaitingSword` until the sprite pipeline consumes their order;
    /// the gate here works on the current animation, so those
    /// in-flight commands must not run the spacing/smalltalk
    /// evaluator in the same window.
    pub(super) fn is_waiting_sword_idle_for_evaluate(&self, entity_id: EntityId) -> bool {
        use crate::element::Command;

        let Some(actor) = self.get_entity(entity_id).and_then(|e| e.actor_data()) else {
            return false;
        };
        if actor.active_melee.is_active() {
            return false;
        }
        if self
            .sequence_manager
            .has_live_element_for_actor_matching(entity_id, |command| {
                matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft
                        | Command::SwordstrikeSmalltalkRight
                        | Command::ParrySmalltalkLeft
                        | Command::ParrySmalltalkRight
                )
            })
        {
            return false;
        }

        let principal_opponent = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());

        !self
            .sequence_manager
            .has_unpostponed_element_for_actor_matching_element(entity_id, |elem| {
                let blocks_waiting_sword = matches!(
                    elem.command,
                    Command::SwordstrikeThrustA
                        | Command::SwordstrikeThrustB
                        | Command::SwordstrikeThrustC
                        | Command::SwordstrikeThrustD
                        | Command::SwordstrikeThrustE
                        | Command::SwordstrikeThrustF
                        | Command::SwordstrikeThrustG
                        | Command::SwordstrikeThrustH
                        | Command::SwordstrikeThrustI
                        | Command::SwordstrikeTired
                        | Command::ParrySword
                        | Command::ParrySwordLow
                );
                if !blocks_waiting_sword {
                    return false;
                }

                let crate::sequence::SequenceElementData::Interaction {
                    antagonist: Some(antagonist),
                } = elem.data
                else {
                    return true;
                };

                Some(antagonist) == principal_opponent
                    && self
                        .get_entity(antagonist)
                        .map(|e| e.is_human() && !e.is_dead())
                        .unwrap_or(false)
            })
    }

    /// Per-frame smalltalk paired strike/parry system.
    ///
    /// For every entity that is the principal opponent of its primary
    /// opponent, choose a smalltalk strike/parry. This drives the
    /// choreographed back-and-forth visible in classic swordfights.
    ///
    pub(crate) fn tick_smalltalk(&mut self, assets: &LevelAssets, suppressed_actors: &[EntityId]) {
        let mut pending_smalltalk_strikes: Vec<(EntityId, EntityId, bool)> = Vec::new();
        let mut pending_initiative_transfers: Vec<EntityId> = Vec::new();
        let mut pending_step_backs: Vec<(EntityId, crate::coordinates::MapPoint)> = Vec::new();

        // Collect entities with smalltalk_initiative who are principal opponents.
        // The follow-up decision path mutates engine state, so keep this
        // first pass read-only and stage typed ids.
        let mut smalltalk_candidates = Vec::new();
        for (entity_id, entity) in self.entities_iter_with_id() {
            let (has_initiative, opponents_empty, principal_id, action_ok, observing) = {
                if !entity.is_human() || entity.is_dead() {
                    continue;
                }
                let Some(human) = entity.human_data() else {
                    continue;
                };
                if human.unconscious {
                    continue;
                }
                let action = entity
                    .actor_data()
                    .map(|a| a.action_state)
                    .unwrap_or_default();
                let action_ok = action == ActionState::WaitingSword
                    && self.is_waiting_sword_idle_for_evaluate(entity_id);
                let principal = human.opponents.first().copied();
                // EvaluateSwordfight (and the smalltalk-strike pick)
                // is gated on `is_soldier_observing_swordfight() ==
                // false`.  The base human always returns false, so
                // only soldiers can short-circuit here.
                let observing = match entity {
                    Entity::Soldier(s) => s.is_soldier_observing_swordfight(),
                    _ => false,
                };
                (
                    human.smalltalk_initiative,
                    human.opponents.is_empty(),
                    principal,
                    action_ok,
                    observing,
                )
            };
            if opponents_empty || !action_ok || observing {
                continue;
            }
            let Some(principal_id) = principal_id else {
                continue;
            };

            if suppressed_actors.contains(&entity_id) {
                continue;
            }

            // Verify mutual principal opponents
            let is_mutual = self
                .entities
                .get(principal_id.index() as usize)
                .and_then(|s| s.as_ref())
                .and_then(|e| e.human_data())
                .and_then(|h| h.opponents.first().copied())
                .map(|opp| opp == entity_id)
                .unwrap_or(false);
            if !is_mutual {
                continue;
            }
            smalltalk_candidates.push((entity_id, principal_id, has_initiative));
        }

        for (entity_id, principal_id, has_initiative) in smalltalk_candidates {
            if self.evaluate_smalltalk_hint(entity_id) {
                continue;
            }

            if !has_initiative {
                // No initiative: the legacy implementation waiting-sword path updates
                // swordfight distance and returns without picking a
                // smalltalk strike.
                continue;
            }

            // Initiative-exchange block:
            //   - If `received_smalltalk_initiative` just flipped on,
            //     consume it once and proceed to strike.
            //   - Otherwise roll `rand%100 <= relative_fighting_ability`,
            //     or check `can_he_kill_me_but_me_not`: if either is
            //     true, transfer initiative back to the opponent and
            //     skip this strike.
            let received = self
                .get_entity(entity_id)
                .and_then(|e| e.human_data())
                .map(|h| h.received_smalltalk_initiative)
                .unwrap_or(false);
            if received {
                if let Some(entity) = self.get_entity_mut(entity_id)
                    && let Some(human) = entity.human_data_mut()
                {
                    human.received_smalltalk_initiative = false;
                }
            } else {
                let rfa = self
                    .get_entity(entity_id)
                    .and_then(|e| e.human_data())
                    .map(|h| h.relative_fighting_ability)
                    .unwrap_or(50);
                let roll_loses = crate::sim_rng::u32(0..100) <= rfa as u32;
                let out_of_range = self.can_he_kill_me_but_me_not(entity_id, principal_id, assets);
                if roll_loses || out_of_range {
                    // Lose initiative and hand it to the opponent; skip
                    // striking this frame.
                    pending_initiative_transfers.push(principal_id);
                    if let Some(entity) = self.get_entity_mut(entity_id)
                        && let Some(human) = entity.human_data_mut()
                    {
                        human.smalltalk_initiative = false;
                    }
                    continue;
                }
            }

            // Step-back check.  The step-back arm fires between the
            // "we're in range" check and the left/right smalltalk
            // strike pick, and suppresses the strike for this frame
            // when the actor wants to break the encirclement.
            if let Some(dest) = self.is_step_back_needed(entity_id, assets) {
                pending_step_backs.push((entity_id, dest));
                continue;
            }

            // Pick left or right smalltalk strike
            let is_left = crate::sim_rng::bool();
            pending_smalltalk_strikes.push((entity_id, principal_id, is_left));
        }

        for new_owner in pending_initiative_transfers {
            self.take_smalltalk_initiative(new_owner);
        }

        for (actor_id, dest) in pending_step_backs {
            // Face the principal opponent before stepping back so the
            // chosen animation lines up with the iso direction
            // picker.
            let principal_id = self
                .get_entity(actor_id)
                .and_then(|e| e.human_data())
                .and_then(|h| h.opponents.first().copied());
            if let Some(pid) = principal_id {
                let dir = direction_to(&self.entities, actor_id, pid);
                if let Some(Some(entity)) = self.entities.get_mut(actor_id.index() as usize) {
                    entity.element_data_mut().set_direction_instantly(dir);
                }
            }
            if let Some(Some(entity)) = self.entities.get_mut(actor_id.index() as usize)
                && let Some(human) = entity.human_data_mut()
            {
                human.last_motion_was_step_back_in_combat = true;
            }
            let mut elem = crate::sequence::SequenceElement::new_movement(
                1,
                crate::element::Command::Move,
                Some(actor_id),
                crate::order::OrderType::WalkingUpright,
            );
            elem.data = crate::sequence::SequenceElementData::Movement {
                destination: crate::coordinates::MapPoint {
                    x: dest.x,
                    y: dest.y,
                },
                layer: self
                    .get_entity(actor_id)
                    .map(|e| e.element_data().layer())
                    .unwrap_or(0),
                sector: None,
                gate_id: None,
                line_id: None,
                element: None,
                flags: crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT,
                tolerance: 0.0,
                direction: 0,
                action: crate::order::OrderType::WalkingUpright,
                speed_factor: 1.0,
                post_seek_sequence: None,
            };
            self.launch_element(elem);
            tracing::debug!(
                actor = ?actor_id,
                destination = ?dest,
                "Step-back in combat launched"
            );
        }

        for (attacker_id, target_id, is_left) in pending_smalltalk_strikes {
            // Smalltalk strike / parry commands are Wait-priority
            // sequence elements, so any real action (Preference,
            // Normal, Injury, etc.) will interrupt them via
            // `DecidePriorities(Wait, _) → InterruptCurrent`.  Launch
            // them as real sequence elements here so the arbitration +
            // animation-completion pipelines run the same way as any
            // other per-element command — rather than poking
            // `actor.combat_anim` directly, which bypassed priority
            // handling and caused smalltalk parries to stick on the
            // sprite for seconds at a time.
            let strike_cmd = if is_left {
                crate::element::Command::SwordstrikeSmalltalkLeft
            } else {
                crate::element::Command::SwordstrikeSmalltalkRight
            };

            // Face the target and set yellow outline for smalltalk
            let dir = direction_to(&self.entities, attacker_id, target_id);
            if let Some(Some(entity)) = self.entities.get_mut(attacker_id.index() as usize) {
                entity.element_data_mut().set_direction_instantly(dir);
                entity.element_data_mut().current_outline =
                    crate::element::OutlineColorName::Striking;
            }
            let strike_elem = crate::sequence::SequenceElement::new_interaction(
                1,
                strike_cmd,
                Some(attacker_id),
                Some(target_id),
            );
            self.launch_element(strike_elem);

            self.receive_smalltalk_hint(attacker_id, target_id, is_left);

            tracing::debug!(
                attacker = ?attacker_id,
                target = ?target_id,
                ?is_left,
                "Smalltalk paired strike"
            );
        }
    }

    fn receive_smalltalk_hint(
        &mut self,
        attacker_id: EntityId,
        target_id: EntityId,
        is_left: bool,
    ) {
        let is_principal = self
            .get_entity(target_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied())
            .map(|principal| principal == attacker_id)
            .unwrap_or(false);
        if !is_principal {
            return;
        }
        if let Some(Some(target)) = self.entities.get_mut(target_id.index() as usize)
            && let Some(human) = target.human_data_mut()
        {
            human.smalltalk_hint = if is_left {
                crate::element::SmalltalkHint::Left
            } else {
                crate::element::SmalltalkHint::Right
            };
            human.smalltalk_hint_opponent = Some(attacker_id);
        }
    }

    pub(super) fn evaluate_smalltalk_hint(&mut self, entity_id: EntityId) -> bool {
        let (hint, hint_opponent) = {
            let Some(human) = self.get_entity(entity_id).and_then(|e| e.human_data()) else {
                return false;
            };
            (human.smalltalk_hint, human.smalltalk_hint_opponent)
        };

        let parry_cmd = match hint {
            crate::element::SmalltalkHint::Left => crate::element::Command::ParrySmalltalkLeft,
            crate::element::SmalltalkHint::Right => crate::element::Command::ParrySmalltalkRight,
            crate::element::SmalltalkHint::Legs => crate::element::Command::ParrySwordLow,
            crate::element::SmalltalkHint::None => return false,
        };

        if let Some(Some(entity)) = self.entities.get_mut(entity_id.index() as usize)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_hint = crate::element::SmalltalkHint::None;
            human.smalltalk_hint_opponent = None;
        }

        let Some(opponent_id) = hint_opponent else {
            return false;
        };

        let elem = crate::sequence::SequenceElement::new_interaction(
            1,
            parry_cmd,
            Some(entity_id),
            Some(opponent_id),
        );
        self.launch_element(elem);
        true
    }

    /// Advance sequence-driven melee strikes (ActiveMelee timers).
    pub(super) fn tick_melee_strikes(&mut self, assets: &LevelAssets) {
        // Collect strike results to avoid borrow conflicts
        struct StrikeHit {
            attacker_id: EntityId,
            victim_id: EntityId,
            strike: SwordStrike,
            attacker_profile_idx: Option<u32>,
        }
        struct CompletedStrike {
            actor_id: EntityId,
            sequence_id: Option<crate::sequence::SequenceId>,
            element_index: usize,
            strike: SwordStrike,
            profile_idx: Option<u32>,
        }

        let mut hits: Vec<StrikeHit> = Vec::new();
        let mut completed: Vec<CompletedStrike> = Vec::new();

        // Pre-pass: face-track straight strikes toward their targets.
        // Done in a separate pass because the main iter_mut loop
        // can't access two entities simultaneously.
        let mut pending_directions = Vec::new();
        for (entity_id, entity) in self.entities_iter_with_id() {
            let (strike, target_id) = {
                let actor = match entity.actor_data() {
                    Some(a) => a,
                    None => continue,
                };
                if !actor.active_melee.is_active() {
                    continue;
                }
                let s = actor.active_melee.strike;
                let t = actor.active_melee.target;
                (s, t)
            };
            let is_straight = {
                let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);
                profile_idx
                    .and_then(|pi| assets.profile_manager.get_hth_weapon(pi))
                    .map(|p| {
                        matches!(
                            p.thrusts[strike as usize].kind,
                            WeaponThrustKind::Straight | WeaponThrustKind::Assault
                        )
                    })
                    .unwrap_or(true)
            };
            if is_straight && let Some(tid) = target_id {
                let dir = direction_to(&self.entities, entity_id, tid);
                pending_directions.push((entity_id, dir));
            }
        }
        for (entity_id, dir) in pending_directions {
            if let Some(e) = self.get_entity_mut(entity_id) {
                e.element_data_mut().set_direction_instantly(dir);
            }
        }

        // Phase 1: advance timers and collect hits
        for (entity_id, entity) in self.entities.occupied_mut() {
            // Read weapon profile ID before taking mutable actor borrow
            let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);

            // Drive the strike animation through the sprite (like bow_shot).
            // This makes the character visually swing the sword.
            let direction = entity.element_data().direction() as u16;
            {
                let actor = match entity.actor_data() {
                    Some(a) => a,
                    None => continue,
                };
                if !actor.active_melee.is_active() {
                    continue;
                }
                let order_id = actor.active_melee.order_id;
                let strike = actor.active_melee.strike;
                let hold_true_sweep = actor.active_melee.hit_applied
                    && actor
                        .sweep_state
                        .as_ref()
                        .is_some_and(true_sweep_still_rotating);
                if let Some(order_id) = order_id {
                    if hold_true_sweep {
                        tracing::trace!(
                            "tick_melee_strikes: entity={} order_id={} strike={:?} holding true-circle sweep",
                            entity_id.index(),
                            order_id,
                            strike
                        );
                    } else {
                        let anim = strike_to_animation(strike);
                        let elem = entity.element_data_mut();
                        let sprite = &mut elem.sprite;
                        let motion = sprite.perform_action(
                            Some(order_id),
                            anim,
                            direction,
                            crate::sprite::FrameProgression::Default,
                            false,
                        );
                        tracing::trace!(
                            "tick_melee_strikes: entity={} order_id={} strike={:?} anim={:?} dir={} motion={:?}",
                            entity_id.index(),
                            order_id,
                            strike,
                            anim,
                            direction,
                            motion
                        );
                        // Mark sprite as driving hit timing on first
                        // non-Error frame.  When sprite-driven, the
                        // natural frames_remaining countdown is frozen —
                        // hit timing comes from MotionState::Done (damage)
                        // and cleanup from MotionState::Terminated (end
                        // animation).
                        if !matches!(motion, crate::sprite::MotionState::Error) {
                            let actor = entity.actor_data_mut().unwrap();
                            actor.active_melee.sprite_driving_hit = true;
                        }
                        // Done = the action-done frame.  Jump
                        // frames_remaining to the hit threshold so
                        // is_hit_frame fires.
                        if matches!(motion, crate::sprite::MotionState::Done) {
                            let actor = entity.actor_data_mut().unwrap();
                            if !actor.active_melee.hit_applied {
                                actor.active_melee.frames_remaining =
                                    crate::movement::MELEE_STRIKE_DURATION
                                        - crate::movement::MELEE_HIT_FRAME;
                            }
                        }
                        // Terminated/Aborted = animation fully finished.
                        // Trigger cleanup.
                        if matches!(
                            motion,
                            crate::sprite::MotionState::Terminated
                                | crate::sprite::MotionState::Aborted
                        ) {
                            let actor = entity.actor_data_mut().unwrap();
                            if !actor.active_melee.hit_applied {
                                // Edge case: Done was skipped (action_done
                                // frame == last frame).  Trigger hit first.
                                actor.active_melee.frames_remaining =
                                    crate::movement::MELEE_STRIKE_DURATION
                                        - crate::movement::MELEE_HIT_FRAME;
                            } else {
                                actor.active_melee.frames_remaining = 0;
                            }
                        }
                    }
                }
            }

            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => continue,
            };

            // Read melee state before mutating
            let melee = actor.active_melee;
            let attacker_id = entity_id;

            // Advance frame timer.  When sprite_driving_hit, the natural
            // countdown is frozen — only the sprite handler above moves
            // frames_remaining (Done → hit threshold, Terminated → 0).
            if !melee.sprite_driving_hit {
                actor.active_melee.frames_remaining = melee.frames_remaining.saturating_sub(1);
            }

            if melee.is_hit_frame() && !melee.hit_applied {
                actor.active_melee.hit_applied = true;
                let target = melee.target.unwrap();

                hits.push(StrikeHit {
                    attacker_id,
                    victim_id: target,
                    strike: melee.strike,
                    attacker_profile_idx: profile_idx,
                });
            }

            if actor.active_melee.frames_remaining == 0 {
                // If the sweep hasn't completed its full rotation
                // yet, keep going instead of terminating.
                let sweep_still_active = actor
                    .sweep_state
                    .as_ref()
                    .is_some_and(true_sweep_still_rotating);
                if sweep_still_active {
                    // Extend by 1 frame — tick_sweep_strikes will advance
                    // the angle and eventually reach final_angle.
                    actor.active_melee.frames_remaining = 1;
                } else {
                    let seq_id = melee.sequence_id;
                    let elem_idx = melee.element_index;
                    actor.active_melee.clear();
                    completed.push(CompletedStrike {
                        actor_id: attacker_id,
                        sequence_id: seq_id,
                        element_index: elem_idx,
                        strike: melee.strike,
                        profile_idx,
                    });
                }
            }
        }

        // Phase 2: apply hits — check strike kind for multi-target
        for hit in hits {
            // Determine the strike kind
            let strike_kind = hit
                .attacker_profile_idx
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|profile| profile.thrusts[hit.strike as usize].kind)
                .unwrap_or(WeaponThrustKind::Straight);

            let is_sweep = matches!(
                strike_kind,
                WeaponThrustKind::Lateral
                    | WeaponThrustKind::TrueHalfCircle
                    | WeaponThrustKind::FalseHalfCircle
                    | WeaponThrustKind::TrueCircle
                    | WeaponThrustKind::FalseCircle
            );
            let is_push = matches!(strike_kind, WeaponThrustKind::PushAside);

            if is_sweep {
                // Sweep strike: collect victims but apply damage per-frame
                // as the arc passes their position.  MOTION_DONE phase —
                // no AI warn tolerance.
                let victims = self.execute_multi_target_strike(
                    assets,
                    hit.attacker_id,
                    hit.strike,
                    hit.attacker_profile_idx,
                    false,
                );
                let mut all_victims = victims;
                if !all_victims.contains(&hit.victim_id) {
                    let obstacles = crate::sight_obstacle::ObstacleList {
                        static_obstacles: assets.static_sight_obstacles.as_slice(),
                        dynamic_obstacles: &self.dynamic_sight_obstacles,
                        static_active: &self.static_sight_obstacle_active,
                    };
                    let distance = entity_distance(&self.entities, hit.attacker_id, hit.victim_id);
                    let in_range = hit
                        .attacker_profile_idx
                        .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                        .map(|p| combat::is_strike_in_range(p, hit.strike, distance))
                        .unwrap_or(false);
                    if in_range
                        && is_possible_sword_strike_victim_id(
                            &self.entities,
                            hit.attacker_id,
                            hit.victim_id,
                            &assets.profile_manager,
                            &self.fast_grid,
                            obstacles,
                        )
                    {
                        all_victims.push(hit.victim_id);
                    }
                }
                self.initialize_sweep(
                    assets,
                    hit.attacker_id,
                    hit.strike,
                    hit.attacker_profile_idx,
                    strike_kind,
                    all_victims,
                );
            } else if is_push {
                // Push strike: apply damage to all victims at the
                // hit frame (no AI warn tolerance), but defer the
                // EnterSwordfight command to the strike's completion
                // by stashing victim IDs on the actor.
                let victims = self.execute_multi_target_strike(
                    assets,
                    hit.attacker_id,
                    hit.strike,
                    hit.attacker_profile_idx,
                    false,
                );
                let mut all_victims = victims;
                if !all_victims.contains(&hit.victim_id) {
                    let obstacles = crate::sight_obstacle::ObstacleList {
                        static_obstacles: assets.static_sight_obstacles.as_slice(),
                        dynamic_obstacles: &self.dynamic_sight_obstacles,
                        static_active: &self.static_sight_obstacle_active,
                    };
                    let distance = entity_distance(&self.entities, hit.attacker_id, hit.victim_id);
                    let in_range = hit
                        .attacker_profile_idx
                        .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                        .map(|p| combat::is_strike_in_range(p, hit.strike, distance))
                        .unwrap_or(false);
                    if in_range
                        && is_possible_sword_strike_victim_id(
                            &self.entities,
                            hit.attacker_id,
                            hit.victim_id,
                            &assets.profile_manager,
                            &self.fast_grid,
                            obstacles,
                        )
                    {
                        all_victims.push(hit.victim_id);
                    }
                }
                for victim_id in &all_victims {
                    if let Some(profile_idx) = hit.attacker_profile_idx {
                        self.launch_sword_damage_now(
                            assets,
                            *victim_id,
                            hit.attacker_id,
                            hit.strike,
                            profile_idx,
                        );
                    }
                }
                if let Some(Some(entity)) = self.entities.get_mut(hit.attacker_id.index() as usize)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.pending_push_swordfight = all_victims;
                }
            } else {
                // Single-target straight strike: distance check only
                let distance = entity_distance(&self.entities, hit.attacker_id, hit.victim_id);
                let in_range = hit
                    .attacker_profile_idx
                    .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                    .map(|profile| combat::is_strike_in_range(profile, hit.strike, distance))
                    .unwrap_or(distance <= 50.0);
                let obstacles = crate::sight_obstacle::ObstacleList {
                    static_obstacles: assets.static_sight_obstacles.as_slice(),
                    dynamic_obstacles: &self.dynamic_sight_obstacles,
                    static_active: &self.static_sight_obstacle_active,
                };

                if in_range
                    && is_possible_sword_strike_victim_id(
                        &self.entities,
                        hit.attacker_id,
                        hit.victim_id,
                        &assets.profile_manager,
                        &self.fast_grid,
                        obstacles,
                    )
                {
                    let dir = direction_to(&self.entities, hit.attacker_id, hit.victim_id);
                    if let Some(Some(entity)) =
                        self.entities.get_mut(hit.attacker_id.index() as usize)
                    {
                        entity.element_data_mut().set_direction_instantly(dir);
                    }
                    if let Some(profile_idx) = hit.attacker_profile_idx {
                        self.launch_sword_damage_now(
                            assets,
                            hit.victim_id,
                            hit.attacker_id,
                            hit.strike,
                            profile_idx,
                        );
                    }
                    self.enter_swordfight(assets, hit.victim_id, hit.attacker_id, true);
                } else {
                    tracing::debug!(
                        attacker = ?hit.attacker_id,
                        victim = ?hit.victim_id,
                        distance,
                        "Sword strike missed — out of range"
                    );
                }
            }
        }

        // Phase 3: notify sequence manager for completed strikes and clear sweep state
        for completed_strike in completed {
            let actor_id = completed_strike.actor_id;
            // Drain any deferred push-strike swordfight entries — fire
            // EnterSwordfight per victim at MOTION_TERMINATED, after
            // the damage sequences have already resolved (possibly
            // killing / knocking out victims who then get filtered by
            // `enter_swordfight`).
            let pending_sf = if let Some(Some(entity)) =
                self.entities.get_mut(actor_id.index() as usize)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.sweep_state = None;
                std::mem::take(&mut actor.pending_push_swordfight)
            } else {
                Vec::new()
            };
            for victim_id in pending_sf {
                self.enter_swordfight(assets, victim_id, actor_id, true);
            }
            match completed_strike
                .profile_idx
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            {
                Some(profile) => {
                    let energy = combat::strike_energy_cost(profile, completed_strike.strike);
                    if let Some(Some(entity)) = self.entities.get_mut(actor_id.index() as usize)
                        && let Some(human) = entity.human_data_mut()
                    {
                        human.tiredness = human.tiredness.saturating_add(energy);
                    }
                }
                None => {
                    tracing::warn!(
                        ?actor_id,
                        ?completed_strike.strike,
                        ?completed_strike.profile_idx,
                        "completed sword strike has no attacker weapon profile; tiredness unchanged"
                    );
                }
            }
            if let Some(sid) = completed_strike.sequence_id {
                if let Some(elem) = self
                    .sequence_manager
                    .get_element(sid, completed_strike.element_index)
                {
                    use crate::sequence::SequenceState;
                    if matches!(
                        elem.state,
                        SequenceState::Interrupted
                            | SequenceState::Impossible
                            | SequenceState::Terminated
                            | SequenceState::Done
                    ) {
                        tracing::debug!(
                            ?sid,
                            elem_idx = completed_strike.element_index,
                            state = ?elem.state,
                            actor = ?actor_id,
                            "tick_melee_strikes: skipping stale completed strike callback"
                        );
                        continue;
                    }
                }
                self.sequence_manager
                    .element_terminated(sid, completed_strike.element_index);
            }
        }
    }

    /// Initialize a per-frame sweep for a lateral/circle sword strike.
    ///
    /// Collects potential victims and computes the sweep angles so that
    /// `tick_sweep_strikes` can advance the arc each frame and hit victims
    /// as the sweep passes their position.
    ///
    pub(super) fn initialize_sweep(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        strike: SwordStrike,
        profile_idx: Option<u32>,
        strike_kind: WeaponThrustKind,
        victims: Vec<EntityId>,
    ) {
        let profile = match profile_idx.and_then(|idx| assets.profile_manager.get_hth_weapon(idx)) {
            Some(p) => p,
            None => return,
        };
        let thrust = &profile.thrusts[strike as usize];
        let direction = thrust.direction;
        let initial_angle_deg = thrust.initial_angle as f32;
        let final_angle_deg = thrust.final_angle as f32;
        let rotation_angle_deg = thrust.rotation_angle as f32;

        let attacker_dir = self
            .get_entity(attacker_id)
            .map(|e| e.element_data().direction())
            .unwrap_or(0);
        let dir_angle = sector_to_angle(attacker_dir);

        let deg_to_rad = std::f32::consts::PI / 180.0;
        let rotation_per_frame = rotation_angle_deg * deg_to_rad;

        use crate::profiles::WeaponThrustDirection;

        let (initial, final_a, signed_rotation) = match strike_kind {
            WeaponThrustKind::Lateral => match direction {
                WeaponThrustDirection::RightToLeft => {
                    let init = dir_angle + initial_angle_deg * deg_to_rad;
                    let fin = dir_angle - final_angle_deg * deg_to_rad;
                    (init, fin, -rotation_per_frame)
                }
                _ => {
                    let init = dir_angle - initial_angle_deg * deg_to_rad;
                    let fin = dir_angle + final_angle_deg * deg_to_rad;
                    (init, fin, rotation_per_frame)
                }
            },
            WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => match direction
            {
                WeaponThrustDirection::RightToLeft => {
                    let init = dir_angle + initial_angle_deg * deg_to_rad;
                    let fin = init - std::f32::consts::PI;
                    (init, fin, -rotation_per_frame)
                }
                _ => {
                    let init = dir_angle - initial_angle_deg * deg_to_rad;
                    let fin = init + std::f32::consts::PI;
                    (init, fin, rotation_per_frame)
                }
            },
            WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => match direction {
                WeaponThrustDirection::RightToLeft => {
                    let init = dir_angle + initial_angle_deg * deg_to_rad;
                    let fin = dir_angle - 2.0 * std::f32::consts::PI;
                    (init, fin, -rotation_per_frame)
                }
                _ => {
                    let init = dir_angle - initial_angle_deg * deg_to_rad;
                    let fin = dir_angle + 2.0 * std::f32::consts::PI;
                    (init, fin, rotation_per_frame)
                }
            },
            _ => return, // not a sweep type
        };

        let num_victims = victims.len();
        let sweep = crate::movement::SweepState {
            pending_victims: victims,
            initial_angle: initial,
            current_angle: dir_angle,
            final_angle: final_a,
            rotation_per_frame: signed_rotation,
            direction,
            strike,
            attacker_profile_idx: profile_idx,
            strike_kind,
        };

        if let Some(Some(entity)) = self.entities.get_mut(attacker_id.index() as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.sweep_state = Some(sweep);
        }

        tracing::debug!(
            attacker = ?attacker_id,
            ?strike_kind,
            num_victims,
            "Sweep strike initialized"
        );
    }

    /// Per-frame tick for active sweep strikes.
    ///
    /// For each entity with an active sweep, advances the current angle and
    /// applies damage to any pending victims whose direction from the attacker
    /// now falls within the swept arc.
    ///
    pub(super) fn tick_sweep_strikes(&mut self, assets: &LevelAssets) {
        use crate::profiles::WeaponThrustDirection;

        // Phase 1: collect active sweeps (clone to avoid borrow conflicts)
        struct ActiveSweep {
            attacker_id: EntityId,
            attacker_pos: (f32, f32),
            sweep: crate::movement::SweepState,
        }
        let mut sweeps: Vec<ActiveSweep> = Vec::new();

        for (entity_id, entity) in self.entities_iter_with_id() {
            let actor = match entity.actor_data() {
                Some(a) => a,
                None => continue,
            };
            if let Some(sweep) = &actor.sweep_state {
                let pos = entity.element_data().position_map();
                sweeps.push(ActiveSweep {
                    attacker_id: entity_id,
                    attacker_pos: (pos.x, pos.y),
                    sweep: sweep.clone(),
                });
            }
        }

        // Phase 2: advance angles, rotate attacker, and check victims
        for active in &mut sweeps {
            // Advance the sweep angle — clamp to final.
            // Three-way: use candidate if not-past, else if the
            // candidate and final angles land in the same sector let
            // the overshoot stand (keeps the sector stable at the
            // tail of the sweep), else clamp.
            let candidate = active.sweep.current_angle + active.sweep.rotation_per_frame;
            let past_final = match active.sweep.direction {
                WeaponThrustDirection::LeftToRight => candidate >= active.sweep.final_angle,
                _ => candidate <= active.sweep.final_angle,
            };
            if !past_final
                || angle_to_sector(candidate) == angle_to_sector(active.sweep.final_angle)
            {
                active.sweep.current_angle = candidate;
            } else {
                active.sweep.current_angle = active.sweep.final_angle;
            }

            // Rotate the attacker's sprite direction to follow the
            // sweep each frame.  Only the TRUE variants (TrueHalfCircle,
            // TrueCircle) rotate the sprite; FALSE variants do not.
            if matches!(
                active.sweep.strike_kind,
                crate::profiles::WeaponThrustKind::TrueCircle
                    | crate::profiles::WeaponThrustKind::TrueHalfCircle
            ) {
                let new_dir = angle_to_sector(active.sweep.current_angle);
                if let Some(Some(entity)) =
                    self.entities.get_mut(active.attacker_id.index() as usize)
                {
                    let elem = entity.element_data_mut();
                    elem.set_direction_instantly(new_dir as i16);
                    elem.sprite.force_action_direction(
                        strike_to_animation(active.sweep.strike),
                        new_dir.into(),
                    );
                }
            }

            let initial_sector = angle_to_sector(active.sweep.initial_angle);
            let current_sector = angle_to_sector(active.sweep.current_angle);

            let mut hit_indices = Vec::new();

            for (i, &victim_id) in active.sweep.pending_victims.iter().enumerate() {
                let obstacles = crate::sight_obstacle::ObstacleList {
                    static_obstacles: assets.static_sight_obstacles.as_slice(),
                    dynamic_obstacles: &self.dynamic_sight_obstacles,
                    static_active: &self.static_sight_obstacle_active,
                };
                if !is_possible_sword_strike_victim_id(
                    &self.entities,
                    active.attacker_id,
                    victim_id,
                    &assets.profile_manager,
                    &self.fast_grid,
                    obstacles,
                ) {
                    hit_indices.push(i);
                    continue;
                }
                let victim_pos = match self.get_entity(victim_id) {
                    Some(e) => e.element_data().position_map(),
                    None => {
                        hit_indices.push(i); // remove dead/gone victims
                        continue;
                    }
                };
                let dx = victim_pos.x - active.attacker_pos.0;
                let dy = (victim_pos.y - active.attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
                let victim_sector =
                    crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;

                // Check if victim is in the swept arc
                let is_hit = match active.sweep.direction {
                    WeaponThrustDirection::LeftToRight => {
                        is_sector_between(victim_sector, initial_sector, current_sector)
                    }
                    _ => is_sector_between(victim_sector, current_sector, initial_sector),
                };

                if is_hit {
                    hit_indices.push(i);
                }
            }

            // Apply damage to hit victims (separate pass to avoid borrow issues)
            let hit_victim_ids: Vec<EntityId> = hit_indices
                .iter()
                .filter_map(|&i| {
                    let vid = active.sweep.pending_victims[i];
                    // Only apply damage if the entity still exists
                    if self.get_entity(vid).is_some() {
                        Some(vid)
                    } else {
                        None
                    }
                })
                .collect();

            for victim_id in hit_victim_ids {
                if let Some(profile_idx) = active.sweep.attacker_profile_idx {
                    self.launch_sword_damage_now(
                        assets,
                        victim_id,
                        active.attacker_id,
                        active.sweep.strike,
                        profile_idx,
                    );
                }
                let should_enter = match (
                    self.get_entity(active.attacker_id),
                    self.get_entity(victim_id),
                ) {
                    (Some(a), Some(v)) => {
                        should_enter_swordfight_after_strike(a, v, &assets.profile_manager)
                    }
                    _ => false,
                };
                if should_enter {
                    self.enter_swordfight(assets, victim_id, active.attacker_id, true);
                }
            }

            // Remove hit victims (reverse to preserve indices)
            for &i in hit_indices.iter().rev() {
                active.sweep.pending_victims.remove(i);
            }
        }

        // Phase 3: write back updated sweep states
        for active in sweeps {
            if let Some(Some(entity)) = self.entities.get_mut(active.attacker_id.index() as usize)
                && let Some(actor) = entity.actor_data_mut()
            {
                let rotation_complete = sweep_rotation_complete(&active.sweep);
                let keep_for_rotation = true_sweep_still_rotating(&active.sweep);
                if rotation_complete
                    || (active.sweep.pending_victims.is_empty() && !keep_for_rotation)
                {
                    actor.sweep_state = None;
                } else {
                    actor.sweep_state = Some(active.sweep);
                }
            }
        }
    }

    // ─── Push flight tick ─────────────────────────────────────────

    /// Per-frame push flight advancement.
    ///
    /// For each entity with an `active_flight`, advance position by
    /// the stored increment.  On the final frame, snap to the goal
    /// position.
    ///
    /// Combat-driven flights also propagate the "Bud-Spencer-style"
    /// domino effect: after the position update, sweep nearby upright
    /// actors in the flight direction and queue a `ReceiveHitDamage`
    /// element citing the original hitter — fired per frame at the
    /// tail of `ExecuteFallingHit` / `ExecuteFallingPushed`.
    pub(super) fn tick_push_flights(&mut self, assets: &LevelAssets) {
        // Domino sweeps fire after positions have advanced, so collect
        // (flyer, hitter, post-advance increment) here and dispatch in a
        // second pass — `apply_domino_effect` reads many entities and
        // launches sequence elements, which would conflict with the
        // single-entity mutable borrow below.
        let mut domino_sweeps: Vec<(EntityId, EntityId, f32, f32)> = Vec::new();
        // Landing-resolution side effects deferred to after the loop so
        // we can call `set_obstacle_and_material` (which needs `&mut
        // self`) without conflicting with the per-entity mutable borrow.
        // Apply the goal obstacle / layer / sector at flight
        // termination.
        let mut landings: Vec<(EntityId, Option<u16>)> = Vec::new();

        for (entity_id, entity) in self.entities.occupied_mut() {
            // Read flight state without holding a mutable borrow.
            let flight_info = entity.actor_data().and_then(|a| a.active_flight);

            let flight = match flight_info {
                Some(f) => f,
                None => continue,
            };

            // Capture the domino-sweep request *before* clearing the
            // flight on the final frame.  An exact zero increment
            // skips frames where the sprite isn't actually moving.
            let is_moving = flight.increment_x != 0.0 || flight.increment_y != 0.0;
            if is_moving && let Some(hitter) = flight.antagonist {
                domino_sweeps.push((entity_id, hitter, flight.increment_x, flight.increment_y));
            }

            // Z (elevation) is tracked explicitly only when the flight
            // setup populated a non-default z component (push flights
            // landing on a slope; see `apply_push_effect`). For
            // rolling / ladder-wall / hit fall (`increment_z == 0` &&
            // `obstacle == None`) we keep the original 2D integrator
            // path so the actor's existing elevation (which may be
            // driven by a plane) is preserved exactly as before.
            let tracks_z = flight.increment_z != 0.0 || flight.obstacle.is_some();

            if flight.frames_remaining <= 1 {
                // Final frame — snap to goal position / layer /
                // sector on landing.
                if tracks_z {
                    entity
                        .position_iface_mut()
                        .set_position(crate::coordinates::WorldPoint3D {
                            x: flight.goal_x,
                            y: flight.goal_y + flight.goal_z,
                            z: flight.goal_z,
                        });
                    entity.element_data_mut().set_layer(flight.goal_layer);
                    entity.element_data_mut().set_sector(flight.goal_sector);
                    landings.push((entity_id, flight.obstacle.map(|h| h.get())));
                } else {
                    entity
                        .element_data_mut()
                        .set_position_map(crate::coordinates::MapPoint {
                            x: flight.goal_x,
                            y: flight.goal_y,
                        });
                }
                entity.actor_data_mut().unwrap().active_flight = None;
            } else {
                // Advance by increment.  The per-frame increment in
                // 3D is `(goal - position) / frames_of_flight`, so
                // the z advance is linear from start_z to goal_z.
                let mut m = entity.element_data().position_map();
                m.x += flight.increment_x;
                m.y += flight.increment_y;
                if tracks_z {
                    let cur_z = entity.position_iface().get_elevation();
                    let new_z = cur_z + flight.increment_z;
                    entity
                        .position_iface_mut()
                        .set_position(crate::coordinates::WorldPoint3D {
                            x: m.x,
                            y: m.y + new_z,
                            z: new_z,
                        });
                } else {
                    entity.element_data_mut().set_position_map(m);
                }
                entity
                    .actor_data_mut()
                    .unwrap()
                    .active_flight
                    .as_mut()
                    .unwrap()
                    .frames_remaining -= 1;
            }
        }

        // Landing-resolution second pass: apply the goal obstacle
        // (and its plane + footstep material) via the shared helper.
        // We keep the obstacle on the flight struct and apply it on
        // landing — the per-frame integrator drives z explicitly via
        // `increment_z`, so this is equivalent to applying it
        // up-front for sloped goals.
        for (flyer_id, obstacle) in landings {
            self.set_obstacle_and_material(assets, flyer_id, obstacle);
        }

        for (flyer_id, hitter_id, inc_x, inc_y) in domino_sweeps {
            self.apply_domino_effect(flyer_id, hitter_id, inc_x, inc_y);
        }
    }

    /// Bud-Spencer-style domino punch propagation.
    ///
    /// Called once per flight frame from
    /// [`Self::tick_push_flights`] for every actor whose
    /// `active_flight.antagonist` is `Some` and whose per-frame
    /// increment is non-zero.
    ///
    /// Sweeps every NPC and PC and queues a `RECEIVE_HIT_DAMAGE`
    /// sequence element (citing `hitter_id` as the origin, not the
    /// flying actor) for any candidate that:
    /// 1. Isn't the original hitter,
    /// 2. Has `Posture::Upright`,
    /// 3. Shares the flyer's sector,
    /// 4. Is active and outside any building sector,
    /// 5. Sits within `DOMINO_DISTANCE` (Chebyshev *and* Euclidean),
    /// 6. Is in front of the flight vector (positive dot product with
    ///    the per-frame increment).
    ///
    /// Damage payload is `damage = 0`, `concussion = DOMINO_DAMAGE`,
    /// `is_harder_hit = false`.  The concussion-only payload routes
    /// through the same hit pipeline the original strike used, so
    /// victims also get knocked down and can themselves trigger
    /// further domino cascades.
    pub(super) fn apply_domino_effect(
        &mut self,
        flyer_id: EntityId,
        hitter_id: EntityId,
        inc_x: f32,
        inc_y: f32,
    ) {
        // Read flyer position + sector.
        let (flyer_pos, flyer_sector) = match self.get_entity(flyer_id) {
            Some(e) => {
                let elem = e.element_data();
                (elem.position_map(), elem.sector())
            }
            None => return,
        };

        // The flyer's `is_active_and_outside_building` test is
        // implicit (an actor in flight is by construction active),
        // but we need the flyer's sector index for the per-candidate
        // sector match.  No early return on a building-sector
        // flyer: the per-candidate `is_active_and_outside_building`
        // check covers that case below.

        // Collect candidate victims first to avoid holding the entity
        // borrow while launching sequence elements.  We iterate NPCs
        // (soldiers / civilians) then PCs.  Animals live in a
        // separate list and are excluded.
        let candidate_ids: Vec<EntityId> = self
            .entities
            .npc_ids()
            .chain(self.pc_ids.iter().copied())
            .collect();
        let mut victims: Vec<EntityId> = Vec::new();
        for candidate_id in candidate_ids {
            let candidate = match self.get_entity(candidate_id) {
                Some(e) => e,
                None => continue,
            };

            // Only exclude the original hitter.  The flyer itself is
            // left in the iteration; its zero distance makes the
            // dot-product filter below reject it implicitly.
            if candidate_id == hitter_id {
                continue;
            }

            let elem = candidate.element_data();

            // Only upright postures qualify.
            if elem.posture != Posture::Upright {
                continue;
            }

            // Same-sector test (compared by index, including both
            // being None).
            if elem.sector() != flyer_sector {
                continue;
            }

            // is_active_and_outside_building =
            // active && (sector == 0 || !sector.is_building()).
            if !candidate.is_active() {
                continue;
            }
            if is_in_building_sector(elem.sector(), &self.fast_grid) {
                continue;
            }

            // me-to-guy = guy.position_ground - flyer.position_ground.
            let dx = elem.position_map().x - flyer_pos.x;
            let dy = elem.position_map().y - flyer_pos.y;

            // Chebyshev pre-filter (max(|dx|,|dy|) < DOMINO_DISTANCE).
            if dx.abs() >= DOMINO_DISTANCE || dy.abs() >= DOMINO_DISTANCE {
                continue;
            }
            // True Euclidean test.
            if dx * dx + dy * dy >= DOMINO_DISTANCE * DOMINO_DISTANCE {
                continue;
            }
            // Dot product > 0: candidate sits in front of the flyer
            // along its motion vector.
            if inc_x * dx + inc_y * dy <= 0.0 {
                continue;
            }

            victims.push(candidate_id);
        }

        // Launch one ReceiveHitDamage element per victim, citing the
        // original hitter as origin.  Damage stays 0; the
        // DOMINO_DAMAGE value lands in concussion; `is_harder_hit`
        // stays false (HIT, not HIT_HARD).
        for victim_id in victims {
            let elem = crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveHitDamage,
                Some(victim_id),
                Some(hitter_id),
                0,             // damage stays 0
                DOMINO_DAMAGE, // concussion
            );
            self.launch_element(elem);
            tracing::trace!(
                ?flyer_id,
                ?hitter_id,
                ?victim_id,
                "ApplyDominoEffect: queued domino hit"
            );
        }
    }

    // ─── Roll update on elevation-line crossing ───────────────────

    /// Per-entity re-validation of a Rolling animation after the
    /// actor crosses an elevation line.  Called when the obstacle
    /// pointer swaps to a new sight obstacle — at which point the
    /// roll-direction derivation needs to re-run against the new
    /// slope.
    ///
    /// If the new obstacle isn't steep enough to roll, or the
    /// recomputed roll direction opposes the entity's current
    /// movement increment, we snap the active flight to the current
    /// position.  Otherwise we update the flight target to the new
    /// destination, re-sizing the per-frame increment over the
    /// remaining frames.
    ///
    /// Early-outs if the entity is not currently in a Rolling combat
    /// animation.
    pub(crate) fn update_roll_after_crossing(&mut self, assets: &LevelAssets, entity_id: EntityId) {
        // Cheap early-out: only act while the actor is rolling.
        let is_rolling = self
            .sequence_manager
            .current_order_for_actor(entity_id)
            .map(|(_, _, o)| o.order_type == OrderType::Rolling)
            .unwrap_or(false);
        if !is_rolling {
            return;
        }

        // Recompute using the new obstacle's normal.
        let normal = self.get_roll_normal(assets, entity_id);
        let new_dest = normal.and_then(|n| self.find_roll_point(entity_id, n, true));

        if let Some(Some(entity)) = self.entities.get_mut(entity_id.index() as usize) {
            let pos = entity.element_data().position_map();
            // Compute the new facing up front — we may need to update
            // the entity's direction before re-borrowing actor data.
            let new_facing = match new_dest {
                Some(dest) => {
                    let dx = dest.x - pos.x;
                    let dy = dest.y - pos.y;
                    if dx.abs() > 0.01 || dy.abs() > 0.01 {
                        Some(crate::position_interface::vector_to_sector_0_to_15(dx, dy))
                    } else {
                        None
                    }
                }
                None => None,
            };

            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => return,
            };
            let flight = match actor.active_flight.as_mut() {
                Some(f) => f,
                None => return,
            };

            match new_dest {
                Some(dest) => {
                    // Retarget the flight to the new roll point.
                    let frames = flight.frames_remaining.max(1);
                    flight.goal_x = dest.x;
                    flight.goal_y = dest.y;
                    flight.increment_x = (dest.x - pos.x) / frames as f32;
                    flight.increment_y = (dest.y - pos.y) / frames as f32;
                }
                None => {
                    // Halt the flight at the current position: the
                    // next push-flight tick will snap and clear it.
                    flight.goal_x = pos.x;
                    flight.goal_y = pos.y;
                    flight.increment_x = 0.0;
                    flight.increment_y = 0.0;
                    flight.frames_remaining = 1;
                }
            }

            // The rolling animation calls `turn()` every frame to
            // rotate the entity's facing toward its current movement
            // direction.  When this helper redirects the flight, we
            // also rotate the sprite so the rolling animation faces
            // the new slope direction.
            if let Some(facing) = new_facing {
                entity.element_data_mut().set_direction_instantly(facing);
            }
        }
    }

    // ─── Rider charge tick ────────────────────────────────────────

    /// Per-frame rider charge damage tick.
    ///
    /// For each entity with an `active_rider_charge`:
    /// 1. On the first frame (initialization): collect all potential victims
    ///    in a large polygon hit zone (20 units back, 180 forward, 80 sidewards).
    /// 2. Each subsequent frame: build a per-frame hit zone (expanding backward
    ///    from the rider's current position) and check each pending victim.
    /// 3. Victims inside the hit zone take `SwordStrike::Charge` damage and
    ///    are removed from the candidate list.
    /// 4. When the rider finishes its path, clear the charge state.
    pub(super) fn tick_rider_charges(&mut self, assets: &LevelAssets) {
        use crate::element::{ActiveRiderCharge, EntityId};

        // Phase 1: Initialize any new rider charges.
        // Entities with active_rider_charge == None that are moving with
        // a RiderCharging action need to be initialized.
        {
            // Disjoint-field obstacle list so we can keep `entities` mutably
            // borrowed below without locking out `self.dynamic_sight_obstacles`
            // / `self.static_sight_obstacle_active` (which `sight_obstacles`
            // would do via a whole-`self` immutable borrow).
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.dynamic_sight_obstacles,
                static_active: &self.static_sight_obstacle_active,
            };
            let mut pending_charge_inits = Vec::new();
            for (attacker_id, entity) in self.entities_iter_with_id() {
                let actor = match entity.actor_data() {
                    Some(a) => a,
                    None => continue,
                };
                // Skip already-initialized charges and entities with no
                // active Move element (the charge path is queued as
                // orders on the Move element).
                if actor.active_rider_charge.is_some() {
                    continue;
                }
                let Some(move_seq_id) = actor.active_movement.sequence_id else {
                    continue;
                };
                let move_elem_idx = actor.active_movement.element_index;
                // Read the live `MoveFlags` from the sequence element to
                // gate on `RIDER_CHARGE`.  The RIDER_CHARGE_HIT flag
                // sets the order to RiderCharging, which puts the
                // entity in MovingFast with active path.
                let has_charge = self
                    .sequence_manager
                    .get_element(move_seq_id, move_elem_idx)
                    .and_then(|e| match &e.data {
                        crate::sequence::SequenceElementData::Movement { flags, .. } => {
                            Some(flags.contains(crate::sequence::MoveFlags::RIDER_CHARGE))
                        }
                        _ => None,
                    })
                    .unwrap_or(false);
                if !has_charge {
                    continue;
                }

                let elem = entity.element_data();
                let dir = elem.direction();
                let pos = elem.position_map();
                let layer = elem.layer();

                // Compute direction vectors.
                let forward = sector_to_vector_iso(dir as u16, ASPECT_RATIO);
                let sidewards = sector_to_vector_iso(((dir + 4) & 15) as u16, ASPECT_RATIO);

                // Initial large hit-zone polygon corners:
                // ptMyPoint - 20*forward - 20*sidewards
                // ptMyPoint + 180*forward - 20*sidewards
                // ptMyPoint + 180*forward + 80*sidewards
                // ptMyPoint - 20*forward + 80*sidewards
                let p0 = (
                    pos.x - 20.0 * forward.0 - 20.0 * sidewards.0,
                    pos.y - 20.0 * forward.1 - 20.0 * sidewards.1,
                );
                let p1 = (
                    pos.x + 180.0 * forward.0 - 20.0 * sidewards.0,
                    pos.y + 180.0 * forward.1 - 20.0 * sidewards.1,
                );
                let p2 = (
                    pos.x + 180.0 * forward.0 + 80.0 * sidewards.0,
                    pos.y + 180.0 * forward.1 + 80.0 * sidewards.1,
                );
                let p3 = (
                    pos.x - 20.0 * forward.0 + 80.0 * sidewards.0,
                    pos.y - 20.0 * forward.1 + 80.0 * sidewards.1,
                );

                // Collect potential victims inside the initial polygon.
                let mut pending_victims = Vec::new();
                for (victim_id, victim) in self.entities_iter_with_id() {
                    if victim_id == attacker_id {
                        continue;
                    }
                    if !is_possible_sword_strike_victim(
                        &self.entities,
                        attacker_id,
                        victim,
                        victim_id,
                        &assets.profile_manager,
                        &self.fast_grid,
                        obstacles,
                    ) {
                        continue;
                    }
                    let velem = victim.element_data();
                    if velem.layer() != layer {
                        continue;
                    }
                    let vpos = velem.position_map();
                    if point_in_quad(vpos.x, vpos.y, p0, p1, p2, p3) {
                        pending_victims.push(victim_id);
                    }
                }

                // Frame count comes from the TransitionCharging anim.
                let total_frames = {
                    let n = entity
                        .sprite()
                        .num_frames_for_anim(crate::order::OrderType::TransitionCharging);
                    if n > 1 { n } else { 14u16 }
                };

                pending_charge_inits.push((
                    attacker_id,
                    ActiveRiderCharge {
                        forward,
                        sidewards,
                        origin: pos,
                        layer,
                        pending_victims,
                        current_frame: 0,
                        total_frames,
                        initialized: true,
                    },
                ));
            }

            for (attacker_id, charge) in pending_charge_inits {
                if let Some(entity) = self.get_entity_mut(attacker_id)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.active_rider_charge = Some(charge);
                    tracing::debug!(entity = ?attacker_id, "Rider charge initialized");
                }
            }
        }

        // Phase 2: Per-frame hit zone check and damage.
        struct ChargeHit {
            attacker_id: EntityId,
            victim_id: EntityId,
            attacker_profile_idx: Option<u32>,
            attacker_pos: crate::coordinates::MapPoint,
            forward: (f32, f32),
            sidewards: (f32, f32),
            current_frame: u16,
            total_frames: u16,
            charge_layer: u16,
        }
        let mut hits: Vec<ChargeHit> = Vec::new();
        let mut finished_charges: Vec<EntityId> = Vec::new();

        for (entity_id, entity) in self.entities.occupied_mut() {
            let attacker_id = entity_id;
            let (elem_pos, _elem_layer, attacker_profile_idx) = {
                let elem = entity.element_data();
                let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);
                (elem.position_map(), elem.layer(), profile_idx)
            };
            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => continue,
            };
            let charge = match actor.active_rider_charge.as_mut() {
                Some(c) => c,
                None => continue,
            };

            charge.current_frame += 1;
            // Last frame = frame counter reached total OR the Move
            // element has drained its order queue (no more walking
            // order to drive the charge).
            let active_move_over = actor
                .active_movement
                .sequence_id
                .map(|s| {
                    self.sequence_manager
                        .get_element(s, actor.active_movement.element_index)
                        .map(|e| e.orders.is_empty())
                        .unwrap_or(true)
                })
                .unwrap_or(true);
            let is_last_frame = charge.current_frame >= charge.total_frames || active_move_over;

            // Collect all pending victims for deferred position checking.
            // We can't look up victim positions here (borrow conflict with
            // self.entities iter_mut), so we defer to phase 3.
            let forward = charge.forward;
            let sidewards = charge.sidewards;
            let current_frame = charge.current_frame;
            let total_frames = charge.total_frames;
            let charge_layer = charge.layer;

            for &vid in &charge.pending_victims {
                hits.push(ChargeHit {
                    attacker_id,
                    victim_id: vid,
                    attacker_profile_idx,
                    attacker_pos: elem_pos,
                    forward,
                    sidewards,
                    current_frame,
                    total_frames,
                    charge_layer,
                });
            }

            if is_last_frame {
                finished_charges.push(attacker_id);
            }
        }

        // Phase 3: Check victim positions and apply damage.
        // (Deferred to avoid borrow conflicts with self.entities.)
        for hit in &hits {
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.dynamic_sight_obstacles,
                static_active: &self.static_sight_obstacle_active,
            };
            if !is_possible_sword_strike_victim_id(
                &self.entities,
                hit.attacker_id,
                hit.victim_id,
                &assets.profile_manager,
                &self.fast_grid,
                obstacles,
            ) {
                if let Some(attacker) = self.entities[hit.attacker_id.index() as usize].as_mut()
                    && let Some(actor) = attacker.actor_data_mut()
                    && let Some(charge) = actor.active_rider_charge.as_mut()
                {
                    charge.pending_victims.retain(|&v| v != hit.victim_id);
                }
                continue;
            }

            let (victim_pos, victim_layer) = match self.get_entity(hit.victim_id) {
                Some(e) => (e.element_data().position_map(), e.element_data().layer()),
                None => continue,
            };

            if victim_layer != hit.charge_layer {
                continue;
            }

            // Build per-frame hit zone from stored charge data.
            let back_len = (5.0 * hit.current_frame as f32).min(50.0);
            let back = (-back_len * hit.forward.0, -back_len * hit.forward.1);
            let pos = hit.attacker_pos;
            let is_last = hit.current_frame >= hit.total_frames;

            let hz = if !is_last {
                [
                    (pos.x + back.0, pos.y + back.1),
                    (pos.x, pos.y),
                    (
                        pos.x + 60.0 * hit.sidewards.0,
                        pos.y + 60.0 * hit.sidewards.1,
                    ),
                    (
                        pos.x + back.0 + 60.0 * hit.sidewards.0,
                        pos.y + back.1 + 60.0 * hit.sidewards.1,
                    ),
                ]
            } else {
                [
                    (pos.x + back.0, pos.y + back.1),
                    (pos.x + 15.0 * hit.forward.0, pos.y + 15.0 * hit.forward.1),
                    (
                        pos.x + 15.0 * hit.forward.0 + 60.0 * hit.sidewards.0,
                        pos.y + 15.0 * hit.forward.1 + 60.0 * hit.sidewards.1,
                    ),
                    (
                        pos.x + back.0 + 60.0 * hit.sidewards.0,
                        pos.y + back.1 + 60.0 * hit.sidewards.1,
                    ),
                ]
            };

            if point_in_quad(victim_pos.x, victim_pos.y, hz[0], hz[1], hz[2], hz[3]) {
                // Hit! Remove victim from pending list and apply damage.
                if let Some(attacker) = self.entities[hit.attacker_id.index() as usize].as_mut()
                    && let Some(actor) = attacker.actor_data_mut()
                    && let Some(charge) = actor.active_rider_charge.as_mut()
                {
                    charge.pending_victims.retain(|&v| v != hit.victim_id);
                }

                // Launch a SwordstrikeCharge damage element.
                if let Some(profile_idx) = hit.attacker_profile_idx {
                    self.launch_sword_damage_now(
                        assets,
                        hit.victim_id,
                        hit.attacker_id,
                        SwordStrike::Charge,
                        profile_idx,
                    );
                }

                tracing::debug!(
                    attacker = ?hit.attacker_id,
                    victim = ?hit.victim_id,
                    "Rider charge hit"
                );
            }
        }

        // Phase 4: Clean up finished charges.
        for entity_id in finished_charges {
            if let Some(entity) = self.entities[entity_id.index() as usize].as_mut()
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.active_rider_charge = None;
                // Transition back to running after charge animation ends.
                actor.action_state = crate::element::ActionState::MovingFast;
                tracing::debug!(
                    entity = ?entity_id,
                    "Rider charge finished, returning to running"
                );
            }
        }
    }

    /// When a soldier's attack cooldown expires and its target is in sword
    /// range, apply a sword strike directly (bypassing the sequence system).
    ///
    /// Simplified version of the engine-level combat loop where the AI
    /// launches individual `SwordstrikeThrust*` sequence elements.
    pub(super) fn tick_enemy_sword_attacks(&mut self, assets: &LevelAssets) {
        let current_frame = self.frame_counter;

        // Per-tick reconciliation for `EnemyAi::pending_special_strike`.
        // Single chokepoint: if a soldier is flagged
        // mid-special-strike but the sequence manager has no live
        // preparation timer or sword-strike element for them, the
        // sequence has ended (any reason — natural completion,
        // terminate_sequence, stop_owner, friday_evening_cleanup) so
        // we clear the flag and relaunch the 20-frame swordfight
        // heartbeat.  Drift is bounded to one tick, and the
        // cancellation paths that don't fire an EventDone all fall
        // through here.
        //
        // Collecting flagged-npcs first so we can query the sequence
        // manager and then mutate the AI without aliasing `self`.
        let mut flagged: Vec<EntityId> = Vec::new();
        for (npc_id, soldier) in self.entities.soldiers() {
            let npc_id = EntityId::from(npc_id);
            if let crate::element::AiBrain::Enemy(ref ai) = soldier.npc.ai_brain
                && ai.pending_special_strike
            {
                flagged.push(npc_id);
            }
        }
        for npc_id in flagged {
            let has_active = self
                .sequence_manager
                .has_live_element_for_actor_matching(npc_id, |cmd| {
                    cmd.is_swordstrike() || cmd == crate::element::Command::WaitTimer
                });
            if let Some(Some(Entity::Soldier(soldier))) =
                self.entities.get_mut(npc_id.index() as usize)
                && let crate::element::AiBrain::Enemy(ref mut ai) = soldier.npc.ai_brain
            {
                ai.reconcile_special_strike(has_active, current_frame);
            }
        }

        // Collect pending attacks
        struct PendingAttack {
            soldier_id: EntityId,
            target_id: EntityId,
            weapon_id: u32,
            fighting_ability: u16,
            blood_alcohol: u8,
            is_rank_soldier: bool,
            attacker_direction: i16,
            attacker_camp: crate::element::Camp,
            attacker_layer: u16,
            attacker_pos: (f32, f32),
            attacker_elevation: f32,
            boredom: Vec<u16>,
        }

        let mut attacks: Vec<PendingAttack> = Vec::new();
        let mut pending_weak_stunned: Vec<EntityId> = Vec::new();
        // Tired-soldier `SwordstrikeTired` elements collected here and
        // launched after the npc-iter loop ends — `launch_element` needs
        // `&mut self` and we still hold an immutable borrow on
        // `self.entities`.
        let mut pending_tired: Vec<EntityId> = Vec::new();

        for (npc_id, soldier) in self.entities.soldiers() {
            let npc_id = EntityId::from(npc_id);

            // Must be in swordfight substate and alive.
            //
            // This is a stuck-state detector: if a soldier has
            // opponents but isn't in `AttackingSwordfight`, something
            // upstream has set the substate and forgotten to transition
            // back.  Transient substates like `AttackingRunningToEnemy`
            // can legitimately appear for a few ticks while the NPC
            // closes distance — those clear on `EventReachPoint`, so a
            // short burst is normal.  `AttackingSwordfightParade` is
            // also a valid in-combat substate: the soldier has
            // launched ParrySword and waits for a timer before
            // StopParrySword returns him to `AttackingSwordfight`.
            // Only warn for substates outside those expected combat
            // transition states.
            if soldier.npc.ai_substate() != crate::ai::Substate::AttackingSwordfight {
                let substate = soldier.npc.ai_substate();
                if !soldier.human.opponents.is_empty()
                    && !matches!(
                        substate,
                        crate::ai::Substate::AttackingSwordfightParade
                            | crate::ai::Substate::AttackingRunningToEnemy
                            | crate::ai::Substate::AttackingWalkingToEnemy
                            | crate::ai::Substate::AttackingChargingEnemy
                            | crate::ai::Substate::AttackingSwordfightStepBack
                            | crate::ai::Substate::AttackingApproachingNewEnemy
                            | crate::ai::Substate::AttackingMovingAroundOldEnemy
                    )
                {
                    tracing::warn!(
                        npc = npc_id.index(),
                        substate = ?substate,
                        opponents = soldier.human.opponents.len(),
                        "tick_enemy_sword_attacks: NPC in combat but wrong substate"
                    );
                }
                continue;
            }
            if soldier.npc.life_points <= 0 || soldier.human.unconscious {
                continue;
            }

            // Don't propose a second strike while one is still in
            // flight.  We fold the special-strike substate into the
            // pending-flag instead of a distinct substate.
            if let crate::element::AiBrain::Enemy(ref ai) = soldier.npc.ai_brain
                && ai.pending_special_strike
            {
                continue;
            }

            // Check tiredness — if too tired, play weak animation
            // instead.  Launch a SwordstrikeTired element; the
            // dispatcher (tick.rs) wires the BeingWeakSword anim
            // through `active_ai_anim` + `do_next_order`.
            if soldier.human.tiredness >= TIREDNESS_WEAK_THRESHOLD {
                pending_weak_stunned.push(npc_id);
                let already_busy = self
                    .sequence_manager
                    .current_element_for_actor(npc_id)
                    .and_then(|(s, e)| self.sequence_manager.get_element(s, e))
                    .map(|el| {
                        !matches!(
                            el.command,
                            crate::element::Command::Wait | crate::element::Command::WaitTimer
                        )
                    })
                    .unwrap_or(false);
                if !already_busy {
                    pending_tired.push(npc_id);
                }
                continue;
            }

            // Check cooldown
            let ai = match &soldier.npc.ai_brain {
                crate::element::AiBrain::Enemy(ai) => ai,
                _ => continue,
            };
            if current_frame < ai.next_sword_strike_frame {
                continue;
            }

            let weapon_id = ai.hth_weapon_id;
            let target_id = EntityId::Pc(crate::entity_id::PcId(ai.base.primary_target));

            // Validate target
            let target_ok = self
                .get_entity(target_id)
                .map(|e| {
                    e.is_human()
                        && !e.is_dead()
                        && !e.human_data().map(|h| h.unconscious).unwrap_or(true)
                })
                .unwrap_or(false);
            if !target_ok {
                continue;
            }

            // Honour — don't hit an enemy in a recovery animation.
            // Target must also be in a sword action state.  These are
            // two separate checks: animation for visual recovery, then
            // action state for logical sword readiness.
            let (target_in_sword, target_in_recovery) = self
                .get_entity(target_id)
                .and_then(|e| e.actor_data().map(|a| (a.action_state, a.old_action)))
                .map(|(action, old_action)| {
                    let in_sword = action.is_sword();
                    let in_recovery = matches!(
                        old_action,
                        crate::order::OrderType::BeingHitSword
                            | crate::order::OrderType::ExtractingArrowSword
                            | crate::order::OrderType::DyingSword
                            | crate::order::OrderType::BeingDeadSword
                            | crate::order::OrderType::FallingBackSword
                            | crate::order::OrderType::BeingUnconsciousSword
                            | crate::order::OrderType::BeingDeadFallenBackSword
                            | crate::order::OrderType::StandingUpSword
                    );
                    (in_sword, in_recovery)
                })
                .unwrap_or((false, false));
            if target_in_recovery || !target_in_sword {
                tracing::debug!(
                    npc = npc_id.index(), target = target_id.index(),
                    %target_in_sword, %target_in_recovery,
                );
                continue;
            }

            let spi = soldier.soldier.soldier_profile_index;
            let sp = assets.profile_manager.get_soldier(spi);
            let fa = sp.map(|p| p.fighting).unwrap_or(50);
            let is_rank = sp
                .map(|p| p.rank == crate::profiles::ProfileRank::Soldier)
                .unwrap_or(true);
            let ba = soldier.npc.ai_brain.base().map_or(0, |a| a.blood_alcohol);

            attacks.push(PendingAttack {
                soldier_id: npc_id,
                target_id,
                weapon_id,
                fighting_ability: fa,
                blood_alcohol: ba,
                is_rank_soldier: is_rank,
                attacker_direction: soldier.element.direction(),
                attacker_camp: soldier.soldier.cached_camp,
                attacker_layer: soldier.element.layer(),
                attacker_pos: {
                    // Use ground position (includes elevation).
                    let map = &soldier.element.position_map();
                    let z = soldier
                        .element
                        .sprite
                        .position_iface
                        .get_plane()
                        .map(|plane| plane.compute_z(map.x, map.y))
                        .unwrap_or(0.0);
                    (map.x, map.y + z)
                },
                attacker_elevation: soldier.element.position().z,
                boredom: soldier.human.sword_strike_boredom.clone(),
            });
        }

        // Launch deferred SwordstrikeTired elements for soldiers who
        // crossed the tiredness threshold this tick.  The dispatcher
        // in `tick.rs` wires the BeingWeakSword anim through
        // `active_ai_anim`.
        for npc_id in pending_tired {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::SwordstrikeTired,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process attacks — launch SwordstrikeThrust* sequence
        // elements as Interaction(1, command, this,
        // principal_opponent).
        // Add weak/stunned star titbits (event-driven creation, deferred
        // to avoid borrow conflict with the npc_ids loop above).
        for id in pending_weak_stunned {
            self.add_weak_stunned_combat(id);
        }

        for mut attack in attacks {
            let distance = entity_distance(&self.entities, attack.soldier_id, attack.target_id);

            // Check melee range using weapon profile
            let in_range = assets
                .profile_manager
                .get_hth_weapon(attack.weapon_id)
                .map(|profile| {
                    let max = profile.distance[2] as f32; // MAXIMAL range
                    distance <= max
                })
                .unwrap_or(distance <= 50.0);

            if !in_range {
                continue;
            }

            // Skip if the soldier already has an active strike in progress
            let already_striking = self
                .get_entity(attack.soldier_id)
                .and_then(|e| e.actor_data())
                .map(|a| a.active_melee.is_active())
                .unwrap_or(false);
            if already_striking {
                continue;
            }

            // Select the best strike using ProposeGoodSwordStrike logic.
            let attacker_profile = assets.profile_manager.get_hth_weapon(attack.weapon_id);

            // ── Sprite timing ──────────────────────────────────────────
            // Compute opponent_time_limit from target's sprite.
            // If the target isn't in an active strike animation,
            // time_limit = 1000 (permissive).  Otherwise, take the
            // sprite's frames-from-now-till-action-done (or 1000 if
            // unavailable).
            let opponent_time_limit: Option<i16> =
                self.get_entity(attack.target_id).and_then(|e| {
                    let actor = e.actor_data()?;
                    let sprite = &e.element_data().sprite;
                    // Only active strike animations (A-I) yield a
                    // strike-from-animation lookup; WaitingSword /
                    // MovingSword yield None → time_limit = 1000
                    // (permissive).  Check the actual animation
                    // (`old_action`), not `action_state.is_sword()`.
                    use crate::order::OrderType as OT;
                    let in_active_strike = matches!(
                        actor.old_action,
                        OT::StrikingStraightSword
                            | OT::StrikingStraightStrongSword
                            | OT::StrikingRightSword
                            | OT::StrikingLeftSword
                            | OT::StrikingRoundRightSword
                            | OT::StrikingRoundLeftSword
                            | OT::StrikingSemiroundRightSword
                            | OT::StrikingSemiroundLeftSword
                            | OT::StrikingDownSword
                    );
                    if !in_active_strike {
                        return Some(1000i16);
                    }
                    let ftad = sprite.frames_from_now_till_action_done();
                    Some(if ftad == -1 { 1000 } else { ftad })
                });

            // Compute per-strike startup frames from attacker's
            // sprite (`frames_from_start_till_action_done(anim)`).
            let attacker_sprite_frames: Option<[i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES]> =
                self.get_entity(attack.soldier_id)
                    .map(|e| &e.element_data().sprite)
                    .map(|sprite| {
                        use crate::combat::NORMAL_STRIKES;
                        let mut frames = [0i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES];
                        for (i, &s) in NORMAL_STRIKES.iter().enumerate() {
                            let anim = strike_to_animation(s);
                            frames[i] = sprite.frames_from_start_till_action_done(anim) as i16;
                        }
                        frames
                    });

            // Parry startup frames from attacker's sprite.
            let parry_startup: Option<i16> = self
                .get_entity(attack.soldier_id)
                .map(|e| &e.element_data().sprite)
                .map(|sprite| {
                    sprite.frames_from_start_till_action_done(
                        crate::order::OrderType::TransitionWaitingSwordParryingSword,
                    ) as i16
                });

            // Collect nearby victims for multi-target strike
            // estimation.  Use `INVERSE_SWORDFIGHT_ASPECT_RATIO`
            // (= 1.0): the isometric correction is intentionally
            // disabled for sword-fight math.
            let inv_aspect = INVERSE_SWORDFIGHT_ASPECT_RATIO;
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.dynamic_sight_obstacles,
                static_active: &self.static_sight_obstacle_active,
            };
            let nearby: Vec<crate::combat::NearbyVictim> = self
                .entities_iter_with_id()
                .filter_map(|(eid, e)| {
                    if eid == attack.soldier_id {
                        return None;
                    }
                    if !is_possible_sword_strike_victim(
                        &self.entities,
                        attack.soldier_id,
                        e,
                        eid,
                        &assets.profile_manager,
                        &self.fast_grid,
                        obstacles,
                    ) {
                        return None;
                    }
                    let elem = e.element_data();
                    if elem.layer() != attack.attacker_layer {
                        return None;
                    }
                    let vdx = elem.position_map().x - attack.attacker_pos.0;
                    let vdy = (elem.position_map().y - attack.attacker_pos.1) * inv_aspect;
                    if vdx.abs().max(vdy.abs()) > 150.0 {
                        return None;
                    }
                    let dist = (vdx * vdx + vdy * vdy).sqrt();
                    let sector =
                        crate::position_interface::vector_to_sector_0_to_15(vdx, vdy) as u8;
                    let def_wid = get_hth_weapon_id_full(e, &assets.profile_manager);
                    let def_prof = def_wid.and_then(|id| assets.profile_manager.get_hth_weapon(id));
                    let lp = match e {
                        Entity::Pc(pc) => pc.pc.life_points,
                        Entity::Soldier(s) => s.npc.life_points,
                        _ => 0,
                    };
                    // Check if this victim is walking with a sword (for
                    // circle-strike approach tolerance).
                    let is_walking_with_sword = e
                        .actor_data()
                        .map(|a| a.action_state == ActionState::MovingSword)
                        .unwrap_or(false);
                    Some(crate::combat::NearbyVictim {
                        dx: vdx,
                        dy_stretched: vdy,
                        distance: dist,
                        direction_sector: sector,
                        camp: match e {
                            Entity::Pc(_) => crate::element::Camp::Royalists,
                            Entity::Soldier(s) => s.soldier.cached_camp,
                            Entity::Civilian(c) => c.civilian.cached_camp,
                            _ => crate::element::Camp::Error,
                        },
                        facing_direction: elem.direction(),
                        elevation: elem.position().z,
                        life_points: lp,
                        defender_profile: def_prof,
                        is_primary_target: eid == attack.target_id,
                        is_walking_with_sword,
                    })
                })
                .collect();

            let strike = if let Some(att_prof) = attacker_profile {
                let ctx = crate::combat::StrikeSelectionContext {
                    attacker_profile: att_prof,
                    fighting_ability: attack.fighting_ability,
                    blood_alcohol: attack.blood_alcohol,
                    is_rank_soldier: attack.is_rank_soldier,
                    attacker_direction: attack.attacker_direction,
                    attacker_elevation: attack.attacker_elevation,
                    attacker_camp: attack.attacker_camp,
                    is_swordfighting: true,
                    opponent_time_limit,
                    strike_startup_frames: attacker_sprite_frames,
                    parry_startup_frames: parry_startup,
                    is_npc: true,
                };
                match crate::combat::propose_good_sword_strike(
                    &ctx,
                    &nearby,
                    &mut attack.boredom,
                    false,
                ) {
                    Some(crate::combat::ProposedCombatAction::Strike(s)) => Some(s),
                    _ => None,
                }
            } else {
                None
            };

            let strike = match strike {
                Some(s) => s,
                None => continue, // No viable strike this tick
            };
            let command = strike.to_command();

            // When targeting a PC, start the hulk glow and insert a
            // difficulty-dependent preparation delay.
            let target_is_pc = self
                .get_entity(attack.target_id)
                .map(|e| e.kind().is_pc())
                .unwrap_or(false);

            let wait_time: u32 = if target_is_pc {
                // Start the striking-outline hulk with width 2.
                if let Some(Some(entity)) =
                    self.entities.get_mut(attack.soldier_id.index() as usize)
                {
                    if let Some(human) = entity.human_data_mut() {
                        human.start_hulk(true, 1.0);
                    }
                    let elem = entity.element_data_mut();
                    elem.current_outline = crate::element::OutlineColorName::Striking;
                    elem.outline_width = 2;
                }
                compute_special_strike_preparation_time(attack.fighting_ability)
            } else {
                0
            };

            // Flag the pending special strike and cancel movement so
            // the soldier stands still during the delay.  We fold
            // the special-strike substate into `AttackingSwordfight`
            // + `EnemyAi::pending_special_strike` (see the deletion
            // comment in `ai.rs`).  `begin_special_strike` sets the
            // flag and transitions to `AttackingSwordfight`; the
            // immediate stop-all side effect stays engine-side so it
            // runs before the new strike sequence is queued.
            self.stop_owner(
                attack.soldier_id,
                crate::sequence::SequencePriority::Preference,
            );
            if let Some(Some(Entity::Soldier(soldier))) =
                self.entities.get_mut(attack.soldier_id.index() as usize)
                && let crate::element::AiBrain::Enemy(ref mut ai) = soldier.npc.ai_brain
            {
                ai.begin_special_strike();
            }

            // War-cry remarks for thrusts C/F/G/H/I.  Placed after
            // the state-set + stop-all so the say-order is correct.
            if matches!(
                strike,
                SwordStrike::C | SwordStrike::F | SwordStrike::G | SwordStrike::H | SwordStrike::I
            ) && let Some(Some(Entity::Soldier(soldier))) =
                self.entities.get_mut(attack.soldier_id.index() as usize)
            {
                let is_vip = assets
                    .profile_manager
                    .get_soldier(soldier.soldier.soldier_profile_index)
                    .map(|p| p.vip)
                    .unwrap_or(false);
                if let Some(ai) = soldier.npc.ai_brain.base_mut() {
                    let remark = if is_vip {
                        crate::ai::Remark::VipWarcry
                    } else {
                        crate::ai::Remark::Warcry
                    };
                    ai.say(remark);
                }
            }

            // Build sequence: level-1 wait timer (preparation delay),
            // then level-2 interaction (the actual strike command).
            let mut seq = crate::sequence::Sequence::new();

            let mut wait_elem = crate::sequence::SequenceElement::new_generic(
                1,
                Command::WaitTimer,
                Some(attack.soldier_id),
            );
            wait_elem.priority = crate::sequence::SequencePriority::Normal;
            wait_elem.set_property(
                crate::sequence::Field::Timer,
                crate::sequence::FieldValue::Integer(wait_time),
            );
            seq.append_element(wait_elem);

            let mut strike_elem = crate::sequence::SequenceElement::new_interaction(
                2,
                command,
                Some(attack.soldier_id),
                Some(attack.target_id),
            );
            strike_elem.priority = crate::sequence::SequencePriority::Preference;
            seq.append_element(strike_elem);

            self.launch_sequence(seq);

            // Write back boredom state. The next-strike gate is set when
            // `reconcile_special_strike` observes that this sequence has
            // finished — equivalent to firing EventDone and a 20-frame timer.
            if let Some(Some(Entity::Soldier(soldier))) =
                self.entities.get_mut(attack.soldier_id.index() as usize)
            {
                soldier.human.sword_strike_boredom = attack.boredom;
            }

            tracing::debug!(
                soldier = ?attack.soldier_id,
                target = ?attack.target_id,
                ?command,
                ?strike,
                distance,
                "Enemy AI sword strike sequence launched"
            );
        }
    }

    /// Per-frame concussion healing for all humans.
    pub(super) fn tick_concussion_healing(&mut self, assets: &LevelAssets) {
        let mut pending_fit_again: Vec<EntityId> = Vec::new();
        // Standup / BeingStunnedSword chains discovered during the
        // entity-iter loop are launched after the loop ends to avoid
        // borrowing `self.entities` and `self` simultaneously.
        let mut pending_recover: Vec<crate::sequence::SequenceElement> = Vec::new();
        // Disjoint-borrow: pull the id counter out as a `&mut u32` so
        // the inner loop can stamp fresh ids via
        // `crate::order::alloc_order_id` while still holding
        // `self.entities.iter_mut()`.
        let next_order_id = &mut self.next_order_id;
        for (entity_id, entity) in self.entities.occupied_mut() {
            if !entity.is_human() || entity.is_dead() {
                continue;
            }

            // Scroll-attached beggars short-circuit
            // `add_concussion_of_the_brain`, and the per-frame heal
            // calls `add_concussion(-1)` — so the heal is suppressed
            // for them.  Skip the whole tick.
            if let Entity::Civilian(c) = entity
                && c.npc.scroll_attached
            {
                continue;
            }

            let life_points = get_life_points(entity);
            if life_points <= 0 {
                continue;
            }

            let ctx =
                concussion_ctx_full(entity, self.weather.is_forest_level, self.campaign.as_ref());

            // Determine healing speed: per-profile `wake_up` for PCs
            // and soldiers, civilian default otherwise.
            let healing_speed =
                concussion_healing_speed_for_entity(entity, &assets.profile_manager);

            let was_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);

            if let Some(human) = entity.human_data_mut() {
                combat::concussion_healing_tick(human, healing_speed, life_points, &ctx);
            }

            // Check if entity woke up
            let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
            if was_unconscious && !is_unconscious {
                // Wake up: restore posture and play standup
                // animation.  The standup path chains standup +
                // (optional) BeingStunnedSword as orders on the same
                // sequence element, so we launch a Recover element
                // with both orders pre-pushed and let `do_next_order`
                // play them in sequence.
                let standing_anim = {
                    let posture = entity.element_data().posture;
                    let action = entity
                        .actor_data()
                        .map(|a| a.action_state)
                        .unwrap_or_default();
                    select_combat_animations(posture, action).map(|a| a.standing_up)
                };
                if entity.element_data().posture == Posture::Lying {
                    entity.set_posture(Posture::Upright);
                }
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = ActionState::Waiting;
                }
                let concussion = entity
                    .human_data()
                    .map(|h| h.concussion_of_the_brain)
                    .unwrap_or(0);
                let still_stunned = concussion > STUNNING_THRESHOLD;
                // Reopen eyes for NPCs (standup side effect).
                if let Some(npc) = entity.npc_data_mut() {
                    crate::ai_vision::set_view_status(npc, EyeStatus::LookForward);
                }

                let npc_id = entity_id;
                if standing_anim.is_some() || still_stunned {
                    let mut elem = crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::Recover,
                        Some(npc_id),
                    );
                    if let Some(anim) = standing_anim {
                        elem.push_order(crate::order::Order::new(
                            anim,
                            0.0,
                            0.0,
                            crate::order::alloc_order_id(next_order_id),
                        ));
                    }
                    if still_stunned {
                        // Reference path only adds this if
                        // Swordfighting; we apply unconditionally here
                        // since `handle_post_concussion` only runs
                        // after damage that already implies a sword
                        // context for stunned soldiers.
                        elem.push_order(crate::order::Order::new(
                            crate::order::OrderType::BeingStunnedSword,
                            0.0,
                            0.0,
                            crate::order::alloc_order_id(next_order_id),
                        ));
                    }
                    pending_recover.push(elem);
                }

                // Dispatch EventFitAgain to the revived NPC's AI:
                // when concussion drops below threshold and the NPC
                // was in SleepingUnconscious, fire EventFitAgain so
                // it can leave Sleeping and return to duty.  Scripted
                // sleeps (SleepingForever, SleepingNapping, etc.)
                // don't trigger the wake-to-duty path even if
                // concussion happens to be cleared from outside.
                let in_sleeping_unconscious = entity
                    .ai_controller()
                    .map(|ai| ai.current_substate == crate::ai::Substate::SleepingUnconscious)
                    .unwrap_or(false);
                if in_sleeping_unconscious {
                    pending_fit_again.push(npc_id);
                }
            }
        }

        for elem in pending_recover {
            self.launch_element(elem);
        }

        for waker_id in &pending_fit_again {
            self.queue_wake_redetection_blinks(*waker_id);
        }

        for victim_id in pending_fit_again {
            self.dispatch_ai_stimulus(
                victim_id,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
            );
        }
    }
}
