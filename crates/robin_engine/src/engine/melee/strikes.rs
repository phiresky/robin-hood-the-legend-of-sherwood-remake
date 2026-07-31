//! Per-frame strike ticks (melee, sweep, push, rider, enemy AI) and concussion healing.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId, Posture};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SweepTickPhase {
    Dormant,
    Start,
    InProgress,
    Initialized,
}

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

fn advance_circle_angle(sweep: &mut crate::movement::SweepState) {
    let candidate = sweep.current_angle + sweep.rotation_per_frame;
    let past_final = match sweep.direction {
        crate::profiles::WeaponThrustDirection::LeftToRight => candidate >= sweep.final_angle,
        _ => candidate <= sweep.final_angle,
    };
    if !past_final || angle_to_sector(candidate) == angle_to_sector(sweep.final_angle) {
        sweep.current_angle = candidate;
    } else {
        sweep.current_angle = sweep.final_angle;
    }
}

fn advance_lateral_angle(sweep: &mut crate::movement::SweepState) {
    // ExecuteLateralSwordStrike applies the signed rotation directly. It has
    // no circle-style final-angle clamp or same-sector overshoot branch.
    sweep.current_angle += sweep.rotation_per_frame;
}

fn is_circle_sweep(kind: WeaponThrustKind) -> bool {
    matches!(
        kind,
        WeaponThrustKind::TrueHalfCircle
            | WeaponThrustKind::FalseHalfCircle
            | WeaponThrustKind::TrueCircle
            | WeaponThrustKind::FalseCircle
    )
}

fn is_falling_flight_order(order_type: crate::order::OrderType) -> bool {
    use crate::order::OrderType;

    matches!(
        order_type,
        OrderType::FallingHitUpright
            | OrderType::FallingHitWithBow
            | OrderType::FallingHitWithSword
            | OrderType::FallingHitCrouched
            | OrderType::FallingHitHarderUpright
            | OrderType::FallingHitHarderWithBow
            | OrderType::FallingHitHarderWithSword
            | OrderType::FallingHitHarderCrouched
            | OrderType::FallingPushedUpright
            | OrderType::FallingPushedWithBow
            | OrderType::FallingPushedWithSword
            | OrderType::FallingPushedCrouched
    )
}

fn set_flight_position(
    entity: &mut Entity,
    geometry: crate::element::FlightGeometry,
    map: crate::coordinates::MapPoint,
    world_z: f32,
) {
    match geometry {
        crate::element::FlightGeometry::GroundPlane => {
            entity.element_data_mut().set_position_map(map);
        }
        crate::element::FlightGeometry::World3d => {
            entity
                .position_iface_mut()
                .set_position(crate::coordinates::WorldPoint3D {
                    x: map.x,
                    y: map.y + world_z,
                    z: world_z,
                });
        }
    }
}

impl EngineInner {
    fn begin_selected_melee_motion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
    ) {
        let profile_idx = {
            let entity = self.get_entity_mut(attacker_id).unwrap_or_else(|| {
                panic!("melee MotionState::Start owner {attacker_id:?} disappeared")
            });
            let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);
            entity.set_posture(Posture::Upright);
            let actor = entity.actor_data_mut().unwrap_or_else(|| {
                panic!("melee MotionState::Start owner {attacker_id:?} lost actor data")
            });
            actor.action_state = ActionState::WaitingSword;
            profile_idx
        };

        // RHElementActorHuman::Execute forecasts and warns only after
        // PerformAction returns START. It passes
        // GetSwordStrikeFromAnimation(GetAnimation()) to WarnForStrike, so a
        // sprite-selected replacement row, rather than the requested command,
        // identifies the strike defenders may recognize. This may
        // synchronously Think and draw RNG, so it belongs to the live owner
        // slot rather than Instruct.
        let animation = self.live_actor_animation(attacker_id).unwrap_or_else(|| {
            panic!("melee MotionState::Start owner {attacker_id:?} has no live animation")
        });
        let strike = sword_strike_from_animation(animation).unwrap_or_else(|| {
            panic!(
                "melee MotionState::Start owner {attacker_id:?} has non-strike live animation {animation:?}"
            )
        });
        let victims =
            self.collect_sword_strike_warning_victims(assets, attacker_id, strike, profile_idx);
        self.warn_for_strike(sim, assets, attacker_id, &victims, strike);
    }

    fn selected_melee_identity_is_live(
        &self,
        attacker_id: EntityId,
        selected: super::tick::MeleeOwnerSelection,
    ) -> bool {
        let current_matches = self
            .orders
            .sequence_manager
            .current_order_for_actor(attacker_id)
            .is_some_and(|(seq_id, elem_idx, order)| {
                seq_id == selected.seq_id
                    && elem_idx == selected.elem_idx
                    && order.order_id == selected.order_id
            });
        let melee_matches = self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| {
                let melee = actor.active_melee;
                melee.is_active()
                    && melee.sequence_id == Some(selected.seq_id)
                    && melee.element_index == selected.elem_idx
                    && melee.order_id == Some(selected.order_id)
            });
        current_matches && melee_matches
    }

    /// Execute the active-melee Human Execute arm selected at base-Actor
    /// entry. Each sub-arm revalidates the same sequence/element/order tuple
    /// because synchronous damage and callbacks may replace it mid-dispatch.
    pub(in crate::engine) fn tick_selected_melee_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        selected: super::tick::MeleeOwnerSelection,
    ) {
        let execution_frozen = self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .unwrap_or_else(|| panic!("selected melee owner {attacker_id:?} is missing actor data"))
            .execution_frozen;
        if execution_frozen || !self.selected_melee_identity_is_live(attacker_id, selected) {
            return;
        }

        let sprite_frozen = self.actors_frozen();

        // The fixed-timer completion arm is already logically past its hit
        // frame. Close it before attempting to drive the strike sprite again.
        if !sprite_frozen {
            self.tick_melee_completion_for(sim, assets, attacker_id);
        }
        if !self.selected_melee_identity_is_live(attacker_id, selected) {
            return;
        }
        self.tick_straight_melee_for(sim, assets, attacker_id);
        let sweep_phase = if self.selected_melee_identity_is_live(attacker_id, selected) {
            self.tick_nonstraight_melee_for(sim, assets, attacker_id)
        } else {
            SweepTickPhase::Dormant
        };
        if self.selected_melee_identity_is_live(attacker_id, selected) {
            self.tick_selected_sweep_phase(sim, assets, attacker_id, sweep_phase);
        }
    }

    // ─── Per-frame melee tick ───────────────────────────────────────

    /// Per-frame melee maintenance outside the actor-owned Execute arms.
    ///
    /// Active sequence strikes run in [`Self::tick_selected_melee_owner`] at
    /// the attacker's legacy creation slot. This pass retains the periodic
    /// combat diagnostics and the remaining global melee bookkeeping.
    pub(crate) fn tick_melee_combat(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if self.actors_frozen() {
            return;
        }

        // Periodic combat state dump (every 64 frames)
        if self.control.frame_counter.is_multiple_of(64) {
            for (entity_id, entity) in self.world.entities.humans() {
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

        self.tick_push_flights(sim, assets);
        self.tick_enemy_sword_attacks(sim, assets);
        self.tick_refresh_hero_mouth();
        self.tick_pc_combat_anim_speech(sim, assets);
        self.tick_refresh_purse_disable(assets);
    }

    /// Apply the parry hold countdown at the owning actor's legacy Execute
    /// slot, matching `RHElementActorHuman::Execute`.
    ///
    /// This cannot be batched after entity traversal: a later-created actor
    /// may land a sword hit in the same frame and postpone the StopParry that
    /// an earlier-created defender just queued.
    pub(in crate::engine) fn tick_parry_counter_for_execute(
        &mut self,
        owner: EntityId,
        result: &mut crate::engine::animation::ActorExecuteResult,
    ) {
        let low = match result.order_type {
            crate::order::OrderType::ParryingSword => false,
            crate::order::OrderType::ParryingLowSword => true,
            _ => return,
        };

        let counter = self
            .world
            .entities
            .get_mut(owner)
            .unwrap_or_else(|| panic!("parry Execute references missing owner {owner:?}"))
            .human_data_mut()
            .unwrap_or_else(|| panic!("parry Execute owner {owner:?} is not human"));
        counter.parry_counter = counter.parry_counter.wrapping_sub(1);

        // The Original counter is unsigned but tests expiry through a signed
        // 16-bit cast, so zero wraps to -1 and still expires immediately.
        if (counter.parry_counter as i16) <= 0 {
            if low {
                result.motion = crate::sprite::MotionState::Terminated;
            } else {
                let elem =
                    crate::sequence::SequenceElement::new(1, Command::StopParrySword, Some(owner));
                // LaunchSequenceElement only registers actor-Execute
                // launches. SequenceManager::Hourglass instructs them after
                // all actor slots, in registration order alongside any
                // later-created attacker's damage element.
                self.register_owned_element_deferred(elem);
            }
        }
    }

    pub(super) fn receive_smalltalk_hint(
        &mut self,
        attacker_id: EntityId,
        target_id: EntityId,
        is_left: bool,
    ) {
        let target = self.get_entity(target_id).unwrap_or_else(|| {
            panic!("smalltalk attacker {attacker_id:?} tried to hint missing target {target_id:?}")
        });
        let target_human = target.human_data().unwrap_or_else(|| {
            panic!(
                "smalltalk attacker {attacker_id:?} tried to hint non-human target {target_id:?}"
            )
        });
        let is_principal = target_human.opponents.first().copied() == Some(attacker_id);
        if !is_principal {
            return;
        }
        let human = self
            .world
            .entities
            .get_mut(target_id)
            .and_then(Entity::human_data_mut)
            .unwrap_or_else(|| {
                panic!(
                    "smalltalk attacker {attacker_id:?} target {target_id:?} vanished while receiving hint"
                )
            });
        human.smalltalk_hint = if is_left {
            crate::element::SmalltalkHint::Left
        } else {
            crate::element::SmalltalkHint::Right
        };
        human.smalltalk_hint_opponent = Some(attacker_id);
    }

    pub(super) fn evaluate_smalltalk_hint<I: Into<EntityId>>(&mut self, entity_id: I) -> bool {
        let entity_id = entity_id.into();
        let (hint, hint_opponent) = {
            let entity = self
                .get_entity(entity_id)
                .unwrap_or_else(|| panic!("EvaluateSmalltalkHint owner {entity_id:?} is missing"));
            let human = entity.human_data().unwrap_or_else(|| {
                panic!("EvaluateSmalltalkHint owner {entity_id:?} is not human")
            });
            (human.smalltalk_hint, human.smalltalk_hint_opponent)
        };

        let parry_cmd = match hint {
            crate::element::SmalltalkHint::Left => crate::element::Command::ParrySmalltalkLeft,
            crate::element::SmalltalkHint::Right => crate::element::Command::ParrySmalltalkRight,
            crate::element::SmalltalkHint::Legs => crate::element::Command::ParrySwordLow,
            crate::element::SmalltalkHint::None => return false,
        };

        let opponent_id = hint_opponent.unwrap_or_else(|| {
            panic!("EvaluateSmalltalkHint owner {entity_id:?} has {hint:?} without a hint opponent")
        });
        let opponent = self.get_entity(opponent_id).unwrap_or_else(|| {
            panic!(
                "EvaluateSmalltalkHint owner {entity_id:?} references missing hint opponent {opponent_id:?}"
            )
        });
        assert!(
            opponent.human_data().is_some(),
            "EvaluateSmalltalkHint owner {entity_id:?} hint opponent {opponent_id:?} is not human"
        );

        let human = self
            .world
            .entities
            .get_mut(entity_id)
            .and_then(Entity::human_data_mut)
            .unwrap_or_else(|| {
                panic!("EvaluateSmalltalkHint owner {entity_id:?} vanished while clearing hint")
            });
        human.smalltalk_hint = crate::element::SmalltalkHint::None;
        human.smalltalk_hint_opponent = None;

        let elem = crate::sequence::SequenceElement::new_interaction(
            1,
            parry_cmd,
            Some(entity_id),
            Some(opponent_id),
        );
        self.register_owned_element_deferred(elem);
        true
    }

    /// Advance one straight/assault sequence-driven strike at its actor's
    /// creation-order slot.
    ///
    /// Original `RHElementActor::Hourglass` executes the current order inline,
    /// so a straight strike's synchronously-dispatched damage can interrupt a
    /// later-created actor before that actor gets its own Hourglass call.
    /// Sweep/push work likewise runs from its owning attacker's slot; this
    /// helper is the narrow straight/assault owner-slot path.
    pub(crate) fn tick_straight_melee_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
    ) {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return;
        }

        let Some((profile_idx, strike, target_id, strike_kind)) =
            self.get_entity(attacker_id).and_then(|entity| {
                let melee = entity.actor_data()?.active_melee;
                if !melee.is_active() {
                    return None;
                }
                let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);
                let strike_kind = profile_idx
                    .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                    .map(|profile| profile.thrusts[melee.strike as usize].kind)
                    .unwrap_or(WeaponThrustKind::Straight);
                Some((
                    profile_idx,
                    melee.strike,
                    melee
                        .target
                        .expect("an active melee strike must retain its target"),
                    strike_kind,
                ))
            })
        else {
            return;
        };
        if !matches!(
            strike_kind,
            WeaponThrustKind::Straight | WeaponThrustKind::Assault
        ) {
            return;
        }

        let direction = direction_to(&self.world.entities, attacker_id, target_id);
        if let Some(entity) = self.get_entity_mut(attacker_id) {
            let position = entity.position_iface_mut();
            position.set_direction(crate::position_interface::Direction::from_raw(i32::from(
                direction,
            )));
            position.turn();
        }
        if self.actors_frozen() {
            return;
        }

        let mut hit = false;
        let mut completion = None;
        let mut started = false;
        {
            let Some(entity) = self.get_entity_mut(attacker_id) else {
                return;
            };
            let direction = entity.element_data().direction() as u16;
            let actor = entity
                .actor_data()
                .expect("straight melee attacker must retain actor data");
            let order_id = actor.active_melee.order_id;
            if let Some(order_id) = order_id {
                let anim = strike_to_animation(strike);
                let motion = entity.element_data_mut().sprite.perform_action(
                    sim,
                    Some(order_id),
                    anim,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                );
                tracing::trace!(
                    "tick_straight_melee_for: entity={} order_id={} strike={:?} anim={:?} dir={} motion={:?}",
                    attacker_id.index(),
                    order_id,
                    strike,
                    anim,
                    direction,
                    motion
                );
                started = matches!(motion, crate::sprite::MotionState::Start);
                if !matches!(motion, crate::sprite::MotionState::Error) {
                    entity
                        .actor_data_mut()
                        .expect("straight melee attacker must retain actor data")
                        .active_melee
                        .sprite_driving_hit = true;
                }
                if matches!(motion, crate::sprite::MotionState::Done) {
                    let melee = &mut entity
                        .actor_data_mut()
                        .expect("straight melee attacker must retain actor data")
                        .active_melee;
                    if !melee.hit_applied {
                        melee.frames_remaining = crate::movement::MELEE_STRIKE_DURATION
                            - crate::movement::MELEE_HIT_FRAME;
                    }
                }
                if matches!(
                    motion,
                    crate::sprite::MotionState::Terminated | crate::sprite::MotionState::Aborted
                ) {
                    let melee = &mut entity
                        .actor_data_mut()
                        .expect("straight melee attacker must retain actor data")
                        .active_melee;
                    if !melee.hit_applied {
                        melee.frames_remaining = crate::movement::MELEE_STRIKE_DURATION
                            - crate::movement::MELEE_HIT_FRAME;
                    } else {
                        melee.frames_remaining = 0;
                    }
                }
            }

            let actor = entity
                .actor_data_mut()
                .expect("straight melee attacker must retain actor data");
            let melee = actor.active_melee;
            if !melee.sprite_driving_hit {
                actor.active_melee.frames_remaining = melee.frames_remaining.saturating_sub(1);
            }
            if melee.is_hit_frame() && !melee.hit_applied {
                actor.active_melee.hit_applied = true;
                hit = true;
            }
            if actor.active_melee.frames_remaining == 0 {
                actor.active_melee.clear();
                completion = Some((melee.sequence_id, melee.element_index));
            }
        }

        if started {
            self.begin_selected_melee_motion(sim, assets, attacker_id);
        }

        if hit {
            self.resolve_straight_melee_hit(
                sim,
                assets,
                attacker_id,
                target_id,
                strike,
                profile_idx,
            );
        }
        if let Some((sequence_id, element_index)) = completion {
            self.complete_melee_strike(
                sim,
                assets,
                attacker_id,
                sequence_id,
                element_index,
                strike,
                profile_idx,
            );
        }
    }

    fn resolve_straight_melee_hit(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        victim_id: EntityId,
        strike: SwordStrike,
        profile_idx: Option<u32>,
    ) {
        let distance = entity_distance(&self.world.entities, attacker_id, victim_id);
        let in_range = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|profile| combat::is_strike_in_range(profile, strike, distance))
            .unwrap_or(distance <= 50.0);
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };

        if in_range
            && is_possible_sword_strike_victim_id(
                &self.world.entities,
                attacker_id,
                victim_id,
                &assets.profile_manager,
                &self.world.fast_grid,
                obstacles,
            )
        {
            if let Some(profile_idx) = profile_idx {
                self.queue_sword_damage(sim, assets, victim_id, attacker_id, strike, profile_idx);
            }
        } else {
            tracing::debug!(
                attacker = ?attacker_id,
                victim = ?victim_id,
                distance,
                "Sword strike missed — out of range"
            );
        }
    }

    fn complete_melee_strike(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        sequence_id: Option<crate::sequence::SequenceId>,
        element_index: usize,
        strike: SwordStrike,
        profile_idx: Option<u32>,
    ) {
        let clears_shared_sweep = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .is_some_and(|profile| {
                !matches!(
                    profile.thrusts[strike as usize].kind,
                    WeaponThrustKind::Straight | WeaponThrustKind::Assault
                )
            });
        let pending_swordfights = if let Some(entity) = self.world.entities.get_mut(actor_id)
            && let Some(actor) = entity.actor_data_mut()
        {
            if clears_shared_sweep {
                actor.sweep_state = None;
            }
            std::mem::take(&mut actor.pending_push_swordfight)
        } else {
            Vec::new()
        };
        for victim_id in pending_swordfights {
            let attacker = self
                .world
                .entities
                .get(actor_id)
                .unwrap_or_else(|| panic!("push completion attacker {actor_id:?} disappeared"));
            let victim = self
                .world
                .entities
                .get(victim_id)
                .unwrap_or_else(|| panic!("push completion victim {victim_id:?} disappeared"));
            let should_enter =
                should_enter_swordfight_after_strike(attacker, victim, &assets.profile_manager);
            if should_enter {
                self.queue_enter_swordfight_after_strike(victim_id, actor_id);
            }
        }

        match profile_idx.and_then(|idx| assets.profile_manager.get_hth_weapon(idx)) {
            Some(profile) => {
                let energy = combat::strike_energy_cost(profile, strike);
                if let Some(entity) = self.get_entity_mut(actor_id)
                    && let Some(human) = entity.human_data_mut()
                {
                    human.tiredness = human.tiredness.saturating_add(energy);
                }
            }
            None => tracing::warn!(
                ?actor_id,
                ?strike,
                ?profile_idx,
                "completed sword strike has no attacker weapon profile; tiredness unchanged"
            ),
        }

        if let Some(sequence_id) = sequence_id {
            let stale = self
                .orders
                .sequence_manager
                .get_element(sequence_id, element_index)
                .is_some_and(|element| {
                    use crate::sequence::SequenceState;
                    matches!(
                        element.state,
                        SequenceState::Interrupted
                            | SequenceState::Impossible
                            | SequenceState::Terminated
                            | SequenceState::Done
                    )
                });
            if stale {
                tracing::debug!(
                    ?sequence_id,
                    elem_idx = element_index,
                    actor = ?actor_id,
                    "tick_melee_strikes: skipping stale completed strike callback"
                );
            } else {
                self.orders
                    .sequence_manager
                    .element_terminated(sequence_id, element_index);
            }
        }
    }

    /// Advance every sequence-driven melee strike.
    ///
    /// Kept as the complete low-level driver for focused tests.
    /// The real Hourglass orchestration runs every strike kind in its
    /// creation-ordered entity pass.
    #[cfg(test)]
    pub(crate) fn tick_melee_strikes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let actor_ids: Vec<EntityId> = self
            .world
            .entities
            .actors()
            .map(|(actor_id, _)| actor_id.into())
            .collect();
        for actor_id in actor_ids {
            self.tick_straight_melee_for(sim, assets, actor_id);
            let sweep_phase = self.tick_nonstraight_melee_for(sim, assets, actor_id);
            self.tick_selected_sweep_phase(sim, assets, actor_id, sweep_phase);
        }
    }

    /// Advance one non-straight sequence-driven melee strike at its actor's
    /// creation-order slot.
    pub(crate) fn tick_nonstraight_melee_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
    ) -> SweepTickPhase {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return SweepTickPhase::Dormant;
        }
        let sprite_frozen = self.actors_frozen();

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
        let mut initialized_sweep = false;
        let mut started = false;
        let mut sweep_phase;

        // Phase 1: advance this attacker's timer and collect its hit.
        {
            let entity_id = attacker_id;
            let Some(entity) = self.world.entities.get_mut(attacker_id) else {
                return SweepTickPhase::Dormant;
            };
            // Read weapon profile ID before taking mutable actor borrow
            let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);
            let Some(active_melee) = entity
                .actor_data()
                .map(|actor| actor.active_melee)
                .filter(|melee| melee.is_active())
            else {
                return SweepTickPhase::Dormant;
            };
            let strike_kind = profile_idx
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|profile| profile.thrusts[active_melee.strike as usize].kind)
                .unwrap_or(WeaponThrustKind::Straight);
            if matches!(
                strike_kind,
                WeaponThrustKind::Straight | WeaponThrustKind::Assault
            ) {
                return SweepTickPhase::Dormant;
            }

            if sprite_frozen {
                return SweepTickPhase::Dormant;
            }

            // Drive the strike animation through the sprite (like bow_shot).
            // This makes the character visually swing the sword.
            let direction = entity.element_data().direction() as u16;
            sweep_phase = SweepTickPhase::InProgress;
            {
                let actor = match entity.actor_data() {
                    Some(a) => a,
                    None => return SweepTickPhase::Dormant,
                };
                if !actor.active_melee.is_active() {
                    return SweepTickPhase::Dormant;
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
                            sim,
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
                        started = matches!(motion, crate::sprite::MotionState::Start);
                        sweep_phase = match motion {
                            crate::sprite::MotionState::Start => SweepTickPhase::Start,
                            crate::sprite::MotionState::InProgress
                            | crate::sprite::MotionState::Done => SweepTickPhase::InProgress,
                            crate::sprite::MotionState::Terminated
                            | crate::sprite::MotionState::Aborted
                            | crate::sprite::MotionState::Error => SweepTickPhase::Dormant,
                        };
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
                None => return SweepTickPhase::Dormant,
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
                let target = melee
                    .target
                    .expect("an active non-straight melee strike must retain its target");

                hits.push(StrikeHit {
                    attacker_id: attacker_id.into(),
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
                    // Extend by 1 frame — tick_sweep_for will advance
                    // the angle and eventually reach final_angle.
                    actor.active_melee.frames_remaining = 1;
                } else {
                    let seq_id = melee.sequence_id;
                    let elem_idx = melee.element_index;
                    actor.active_melee.clear();
                    completed.push(CompletedStrike {
                        actor_id: attacker_id.into(),
                        sequence_id: seq_id,
                        element_index: elem_idx,
                        strike: melee.strike,
                        profile_idx,
                    });
                }
            }
        }

        if started {
            self.begin_selected_melee_motion(sim, assets, attacker_id);
        }

        // Phase 2: apply this attacker's hit synchronously. Multi-target
        // victim vectors retain the original actor-list FIFO.
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
                );
                let mut all_victims = victims;
                if !all_victims.contains(&hit.victim_id) {
                    let obstacles = crate::sight_obstacle::ObstacleList {
                        static_obstacles: assets.static_sight_obstacles.as_slice(),
                        dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                        static_active: &self.world.static_sight_obstacle_active,
                    };
                    let distance =
                        entity_distance(&self.world.entities, hit.attacker_id, hit.victim_id);
                    let in_range = hit
                        .attacker_profile_idx
                        .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                        .map(|p| combat::is_strike_in_range(p, hit.strike, distance))
                        .unwrap_or(false);
                    if in_range
                        && is_possible_sword_strike_victim_id(
                            &self.world.entities,
                            hit.attacker_id,
                            hit.victim_id,
                            &assets.profile_manager,
                            &self.world.fast_grid,
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
                initialized_sweep = self
                    .get_entity(hit.attacker_id)
                    .and_then(|entity| entity.actor_data())
                    .is_some_and(|actor| actor.sweep_state.is_some());
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
                );
                let mut all_victims = victims;
                if !all_victims.contains(&hit.victim_id) {
                    let obstacles = crate::sight_obstacle::ObstacleList {
                        static_obstacles: assets.static_sight_obstacles.as_slice(),
                        dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                        static_active: &self.world.static_sight_obstacle_active,
                    };
                    let distance =
                        entity_distance(&self.world.entities, hit.attacker_id, hit.victim_id);
                    let in_range = hit
                        .attacker_profile_idx
                        .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                        .map(|p| combat::is_strike_in_range(p, hit.strike, distance))
                        .unwrap_or(false);
                    if in_range
                        && is_possible_sword_strike_victim_id(
                            &self.world.entities,
                            hit.attacker_id,
                            hit.victim_id,
                            &assets.profile_manager,
                            &self.world.fast_grid,
                            obstacles,
                        )
                    {
                        all_victims.push(hit.victim_id);
                    }
                }
                for victim_id in &all_victims {
                    if let Some(profile_idx) = hit.attacker_profile_idx {
                        self.queue_sword_damage(
                            sim,
                            assets,
                            *victim_id,
                            hit.attacker_id,
                            hit.strike,
                            profile_idx,
                        );
                    }
                }
                if let Some(entity) = self.world.entities.get_mut(hit.attacker_id)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.pending_push_swordfight = all_victims;
                }
            } else {
                self.resolve_straight_melee_hit(
                    sim,
                    assets,
                    hit.attacker_id,
                    hit.victim_id,
                    hit.strike,
                    hit.attacker_profile_idx,
                );
            }
        }

        // Phase 3: notify the sequence manager before the next creation slot.
        for completed_strike in completed {
            self.complete_melee_strike(
                sim,
                assets,
                completed_strike.actor_id,
                completed_strike.sequence_id,
                completed_strike.element_index,
                completed_strike.strike,
                completed_strike.profile_idx,
            );
        }
        if initialized_sweep {
            SweepTickPhase::Initialized
        } else {
            sweep_phase
        }
    }

    fn tick_selected_sweep_phase(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        phase: SweepTickPhase,
    ) {
        match phase {
            SweepTickPhase::Dormant | SweepTickPhase::Start => {}
            SweepTickPhase::Initialized => {
                self.tick_sweep_for(sim, assets, attacker_id, true);
            }
            SweepTickPhase::InProgress => {
                self.rebind_retained_sweep_to_active_strike(assets, attacker_id);
                self.tick_sweep_for(sim, assets, attacker_id, false);
            }
        }
    }

    /// Original stores sweep victims and angles on the human, but reads the
    /// strike direction, rotation, kind, and damage payload from the current
    /// Execute call. If a strike is interrupted after its action point, a new
    /// sweep strike therefore advances the retained geometry using its own
    /// semantics.
    fn rebind_retained_sweep_to_active_strike(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
    ) {
        let Some(entity) = self.get_entity(attacker_id) else {
            return;
        };
        let Some(active) = entity.actor_data().map(|actor| actor.active_melee) else {
            return;
        };
        if !active.is_active() {
            return;
        }
        let Some(profile_idx) = get_hth_weapon_id_full(entity, &assets.profile_manager) else {
            return;
        };
        let profile = assets
            .profile_manager
            .get_hth_weapon(profile_idx)
            .unwrap_or_else(|| {
                panic!(
                    "retained sweep attacker {attacker_id:?} references missing weapon profile {profile_idx}"
                )
            });
        let thrust = &profile.thrusts[active.strike as usize];
        if !matches!(
            thrust.kind,
            WeaponThrustKind::Lateral
                | WeaponThrustKind::TrueHalfCircle
                | WeaponThrustKind::FalseHalfCircle
                | WeaponThrustKind::TrueCircle
                | WeaponThrustKind::FalseCircle
        ) {
            return;
        }
        let signed_rotation = (thrust.rotation_angle as f32)
            * (std::f32::consts::PI / 180.0)
            * if thrust.direction == crate::profiles::WeaponThrustDirection::RightToLeft {
                -1.0
            } else {
                1.0
            };
        if let Some(sweep) = self
            .get_entity_mut(attacker_id)
            .and_then(Entity::actor_data_mut)
            .and_then(|actor| actor.sweep_state.as_mut())
        {
            sweep.rotation_per_frame = signed_rotation;
            sweep.direction = thrust.direction;
            sweep.strike = active.strike;
            sweep.attacker_profile_idx = Some(profile_idx);
            sweep.strike_kind = thrust.kind;
        }
    }

    /// Initialize a per-frame sweep for a lateral/circle sword strike.
    ///
    /// Collects potential victims and computes the sweep angles so that
    /// `tick_sweep_for` can advance the arc each frame and hit victims
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

        if let Some(entity) = self.world.entities.get_mut(attacker_id)
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

    /// Per-frame tick for one active sweep strike at its attacker's
    /// creation-order slot.
    ///
    /// Applies the kind-specific lateral/circle phase order and synchronously
    /// damages pending victims whose direction falls within the swept arc.
    ///
    pub(crate) fn tick_sweep_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        initialized_this_hourglass: bool,
    ) {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return;
        }

        use crate::profiles::WeaponThrustDirection;

        // Phase 1: collect active sweeps (clone to avoid borrow conflicts)
        struct ActiveSweep {
            attacker_id: EntityId,
            attacker_pos: (f32, f32),
            sweep: crate::movement::SweepState,
            rotation_complete_on_entry: bool,
        }
        let mut sweeps: Vec<ActiveSweep> = Vec::new();

        {
            let entity_id = attacker_id;
            let Some(entity) = self.world.entities.get(attacker_id) else {
                return;
            };
            let actor = match entity.actor_data() {
                Some(a) => a,
                None => return,
            };
            if let Some(sweep) = &actor.sweep_state {
                let pos = entity.element_data().position_map();
                sweeps.push(ActiveSweep {
                    attacker_id: entity_id.into(),
                    attacker_pos: (pos.x, pos.y),
                    rotation_complete_on_entry: sweep_rotation_complete(sweep),
                    sweep: sweep.clone(),
                });
            }
        }

        // Phase 2: preserve the two Original effect orders:
        // - lateral IN_PROGRESS advances, then tests victims;
        // - circle IN_PROGRESS tests the existing angle, then advances at
        //   ExecuteCircleSwordStrike's tail.
        // A circle DONE call still reaches that tail and advances once, but
        // neither family tests victims (or rotates a true-circle sprite) on
        // its initialization call.
        for active in &mut sweeps {
            let circle = is_circle_sweep(active.sweep.strike_kind);
            if initialized_this_hourglass {
                if circle {
                    advance_circle_angle(&mut active.sweep);
                }
                continue;
            }
            if matches!(active.sweep.strike_kind, WeaponThrustKind::Lateral) {
                advance_lateral_angle(&mut active.sweep);
            }

            // Rotate the attacker's sprite direction to follow the
            // circle using the angle that existed on entry. Only the TRUE
            // variants rotate; FALSE variants do not.
            if matches!(
                active.sweep.strike_kind,
                crate::profiles::WeaponThrustKind::TrueCircle
                    | crate::profiles::WeaponThrustKind::TrueHalfCircle
            ) {
                let new_dir = angle_to_sector(active.sweep.current_angle);
                if let Some(entity) = self.world.entities.get_mut(active.attacker_id) {
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
                    dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                    static_active: &self.world.static_sight_obstacle_active,
                };
                if !is_possible_sword_strike_victim_id(
                    &self.world.entities,
                    active.attacker_id,
                    victim_id,
                    &assets.profile_manager,
                    &self.world.fast_grid,
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
                    self.queue_sword_damage(
                        sim,
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
                    self.queue_enter_swordfight_after_strike(victim_id, active.attacker_id);
                }
            }

            // Remove hit victims (reverse to preserve indices)
            for &i in hit_indices.iter().rev() {
                active.sweep.pending_victims.remove(i);
            }

            if circle {
                advance_circle_angle(&mut active.sweep);
            }
        }

        // Phase 3: write back updated sweep states
        for active in sweeps {
            if let Some(entity) = self.world.entities.get_mut(active.attacker_id)
                && let Some(actor) = entity.actor_data_mut()
            {
                let true_sweep = matches!(
                    active.sweep.strike_kind,
                    WeaponThrustKind::TrueCircle | WeaponThrustKind::TrueHalfCircle
                );
                // An incomplete true sweep remains live while rotating. If
                // this tick's tail reaches the final angle, the same rule
                // retains it once more so the next Execute call can present
                // that terminal direction before clearing it.
                let keep_for_terminal_execute = true_sweep && !active.rotation_complete_on_entry;
                // Circle effects test victims before their tail advance. If
                // that advance reaches the final angle, retain pending
                // victims and true-circle rotation state for the next
                // Hourglass so the final sector is observable before the
                // state is cleared.
                if active.sweep.pending_victims.is_empty() && !keep_for_terminal_execute {
                    actor.sweep_state = None;
                } else {
                    actor.sweep_state = Some(active.sweep);
                }
            }
        }
    }

    /// Queue the `EnterSwordfight` instruction emitted after a successful
    /// sword hit.
    ///
    /// Original strike executors register this immediately after the
    /// victim's `ReceiveSwordDamage` element. Both are then instructed in
    /// sequence-manager FIFO order; entering synchronously here would let the
    /// relationship/state reset run before the injury has interrupted the
    /// victim's old action.
    pub(in crate::engine) fn queue_enter_swordfight_after_strike(
        &mut self,
        victim_id: EntityId,
        attacker_id: EntityId,
    ) {
        let mut element = crate::sequence::SequenceElement::new_generic(
            1,
            crate::element::Command::EnterSwordfight,
            Some(victim_id),
        );
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Element(attacker_id),
        );
        element.set_property(
            crate::sequence::Field::JumplineDestination,
            crate::sequence::FieldValue::Integer(0),
        );
        element.set_property(
            crate::sequence::Field::SwordfightPrepared,
            crate::sequence::FieldValue::Bool(false),
        );
        self.resolve_element_priority(&mut element);
        self.orders.sequence_manager.launch_element(element);
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
    pub(super) fn tick_push_flights(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
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
        let mut refresh_script_sectors = false;

        for (entity_id, entity) in self.world.entities.actors_mut() {
            // Read flight state without holding a mutable borrow.
            let flight_info = entity.actor_data().and_then(|a| a.active_flight);

            let flight = match flight_info {
                Some(f) => f,
                None => continue,
            };

            // `ReadyForTakeOff` is initialized by ExecuteFallingPushed /
            // ExecuteFallingHit, not by Translate*Damage. Rust stores the
            // computed flight eagerly, so hold it until the queued falling
            // order reports Start and has changed posture to Flying.
            let waiting_for_fall_start = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .is_some_and(|(_, _, order)| is_falling_flight_order(order.order_type))
                && entity.element_data().posture != Posture::Flying;
            if waiting_for_fall_start {
                continue;
            }

            // Capture the domino-sweep request *before* clearing the
            // flight on the final frame.  An exact zero increment
            // skips frames where the sprite isn't actually moving.
            let is_moving = flight.increment_x != 0.0 || flight.increment_y != 0.0;
            if is_moving && let Some(hitter) = flight.antagonist {
                domino_sweeps.push((
                    entity_id.into(),
                    hitter,
                    flight.increment_x,
                    flight.increment_y,
                ));
            }

            if flight.frames_remaining == 0 {
                // Combat falls retain their flight state at the geometric
                // goal until the sprite reports TERMINATED. The animation
                // handler changes posture first; only then does Original
                // apply the goal obstacle/layer/sector and refresh script
                // sectors.
                if flight.antagonist.is_some() && entity.element_data().posture != Posture::Flying {
                    set_flight_position(
                        entity,
                        flight.geometry,
                        crate::coordinates::MapPoint::new(flight.goal_x, flight.goal_y),
                        flight.goal_z,
                    );
                    entity.element_data_mut().set_layer(flight.goal_layer);
                    entity.element_data_mut().set_sector(flight.goal_sector);
                    landings.push((entity_id.into(), flight.obstacle.map(|h| h.get())));
                    refresh_script_sectors = true;
                    entity.actor_data_mut().unwrap().active_flight = None;
                }
            } else if flight.frames_remaining == 1 {
                // PerformFlight still adds its stored increment on the DONE
                // frame. It snaps to the exact PositionGoal only when the
                // sprite later reports TERMINATED.
                if flight.antagonist.is_some() {
                    let mut map = entity.element_data().position_map();
                    map.x += flight.increment_x;
                    map.y += flight.increment_y;
                    let z = entity.position_iface().get_elevation() + flight.increment_z;
                    set_flight_position(entity, flight.geometry, map, z);
                } else {
                    // Non-combat translations have no wrapper termination
                    // event, so their final flight tick owns the exact snap.
                    set_flight_position(
                        entity,
                        flight.geometry,
                        crate::coordinates::MapPoint::new(flight.goal_x, flight.goal_y),
                        flight.goal_z,
                    );
                }
                if flight.antagonist.is_some() {
                    let active = entity
                        .actor_data_mut()
                        .unwrap()
                        .active_flight
                        .as_mut()
                        .unwrap();
                    active.frames_remaining = 0;
                    active.increment_x = 0.0;
                    active.increment_y = 0.0;
                    active.increment_z = 0.0;
                } else {
                    entity.element_data_mut().set_layer(flight.goal_layer);
                    entity.element_data_mut().set_sector(flight.goal_sector);
                    landings.push((entity_id.into(), flight.obstacle.map(|h| h.get())));
                    entity.actor_data_mut().unwrap().active_flight = None;
                }
            } else {
                // Advance by increment.  The per-frame increment in
                // 3D is `(goal - position) / frames_of_flight`, so
                // the z advance is linear from start_z to goal_z.
                let mut m = entity.element_data().position_map();
                m.x += flight.increment_x;
                m.y += flight.increment_y;
                let z = entity.position_iface().get_elevation() + flight.increment_z;
                set_flight_position(entity, flight.geometry, m, z);
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
        for &(flyer_id, obstacle) in &landings {
            self.set_obstacle_and_material(assets, flyer_id, obstacle);
        }

        // The Original calls UpdateScriptSectorsAfterFlight on the terminal
        // motion event, before ApplyDominoEffect. This is an explicit
        // polygon reconciliation for the landed actor because flight motion
        // does not traverse ordinary LINE_SCRIPT boundaries.
        if refresh_script_sectors {
            for (flyer_id, _) in &landings {
                self.update_script_sectors_after_flight(sim, assets, *flyer_id);
            }
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
            .world
            .entities
            .npc_ids()
            .chain(self.world.pc_ids.iter().copied())
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
            if is_in_building_sector(elem.sector(), &self.world.fast_grid) {
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
            .orders
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

        if let Some(entity) = self.world.entities.get_mut(entity_id) {
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

    /// When a soldier's attack cooldown expires and its target is in sword
    /// range, apply a sword strike directly (bypassing the sequence system).
    ///
    /// Simplified version of the engine-level combat loop where the AI
    /// launches individual `SwordstrikeThrust*` sequence elements.
    pub(super) fn tick_enemy_sword_attacks(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_enemy_sword_attacks_for(sim, assets, None);
    }

    /// Consume an event-driven `ReconsiderSwordfight` authorization before
    /// the originating `Think` returns. Original calls
    /// `ProposeGoodSwordStrike` directly in that handler, so postponing this
    /// to the global melee pass can reorder its RNG draw behind later owners.
    pub(crate) fn consume_pending_enemy_sword_attack_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.tick_enemy_sword_attacks_for(sim, assets, Some(owner));
    }

    fn tick_enemy_sword_attacks_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        only_owner: Option<EntityId>,
    ) {
        let current_frame = self.control.frame_counter;

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
        if only_owner.is_none() {
            let mut flagged: Vec<EntityId> = Vec::new();
            for (npc_id, soldier) in self.world.entities.soldiers() {
                if let crate::element::AiBrain::Enemy(ref ai) = soldier.npc.ai_brain
                    && ai.pending_special_strike
                {
                    flagged.push(npc_id.into());
                }
            }
            for npc_id in flagged {
                let has_active = self
                    .orders
                    .sequence_manager
                    .has_live_element_for_actor_matching(npc_id, |cmd| {
                        cmd.is_swordstrike() || cmd == crate::element::Command::WaitTimer
                    });
                if let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(npc_id)
                    && let crate::element::AiBrain::Enemy(ref mut ai) = soldier.npc.ai_brain
                {
                    ai.reconcile_special_strike(has_active, current_frame);
                }
                // `reconcile_special_strike` stands in for cancellation paths
                // whose Original sequence teardown synchronously reaches
                // Think(EVENT_DONE).  Any resulting SetState therefore also
                // has to run its FilterAIEvent callback before this boundary
                // returns; otherwise owner-local work survives the later
                // global melee pass and is observed a frame late.
                self.drain_ai_owner_work_for(sim, assets, npc_id);
            }
        }

        // `RHArtificialMalignity::ReconsiderSwordfight` is event-driven and
        // calls ProposeGoodSwordStrike at most once per invocation.  Consume
        // that authorization up front so every downstream rejection
        // (substate, tiredness, honour, range, or selection failure) remains
        // one-shot instead of being retried by this per-frame engine pass.
        let pending_considerations: std::collections::HashSet<EntityId> = self
            .world
            .entities
            .soldiers_mut()
            .filter_map(|(npc_id, soldier)| {
                if only_owner.is_some_and(|owner| owner != EntityId::from(npc_id)) {
                    return None;
                }
                let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
                    return None;
                };
                std::mem::take(&mut ai.pending_sword_strike_consideration)
                    .then_some(EntityId::from(npc_id))
            })
            .collect();

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
            attacker_pos: (f32, f32),
            attacker_elevation: f32,
            boredom: Vec<u16>,
        }

        let mut attacks: Vec<PendingAttack> = Vec::new();
        for (npc_id, soldier) in self.world.entities.soldiers() {
            if !pending_considerations.contains(&EntityId::from(npc_id)) {
                continue;
            }

            // ReconsiderSwordfight emitted this one-shot authorization at
            // the exact point where Original calls ProposeGoodSwordStrike.
            // Do not reapply polling-era owner state, cooldown, tiredness, or
            // active-strike gates here: EventAfterCombatInjury can reach that
            // point from any real swordfight substate (including Parade),
            // and Original still performs the proposal before arbitration
            // decides what work survives.
            let ai = match &soldier.npc.ai_brain {
                crate::element::AiBrain::Enemy(ai) => ai,
                _ => continue,
            };

            let weapon_id = ai.hth_weapon_id;
            let target_id = self
                .world
                .entities
                .id_at_legacy_slot(ai.base.primary_target)
                .unwrap_or_else(|| {
                    panic!(
                        "authorized sword-strike proposal owner {:?} requires missing principal opponent slot {}",
                        EntityId::from(npc_id),
                        ai.base.primary_target
                    )
                });
            let target = self
                .get_entity(target_id)
                .unwrap_or_else(|| panic!("resolved principal opponent {target_id:?} disappeared"));
            assert!(
                target.is_human(),
                "authorized sword-strike principal opponent {target_id:?} is not human"
            );

            // Honour — don't hit an enemy in a recovery animation.
            // Target must also be in a sword action state.  These are
            // two separate checks: animation for visual recovery, then
            // action state for logical sword readiness.
            let target_in_sword = self
                .get_entity(target_id)
                .and_then(|e| e.actor_data())
                .unwrap_or_else(|| {
                    panic!("sword-strike principal opponent {target_id:?} requires actor data")
                })
                .action_state
                .is_sword();
            let target_in_recovery = self.actor_is_in_sword_recovery(target_id);
            if target_in_recovery || !target_in_sword {
                tracing::debug!(
                    npc = npc_id.index(), target = target_id.index(),
                    %target_in_sword, %target_in_recovery,
                );
                continue;
            }

            let spi = soldier.soldier.soldier_profile_index;
            let sp = assets.profile_manager.get_soldier(spi).unwrap_or_else(|| {
                panic!(
                    "authorized sword-strike owner {:?} requires missing soldier profile {}",
                    EntityId::from(npc_id),
                    spi
                )
            });
            // RHElementActorSoldier::GetFightingAbility applies the active
            // difficulty modifier for Lacklandists. Strike availability,
            // damage estimation, and the special-strike skill gate all call
            // that virtual getter in Original rather than reading the raw
            // soldier-profile capacity.
            let fa = fighting_ability_from_profile(
                self.get_entity(npc_id).unwrap_or_else(|| {
                    panic!(
                        "authorized sword-strike owner {:?} disappeared before ability lookup",
                        EntityId::from(npc_id)
                    )
                }),
                &assets.profile_manager,
                sim.config().difficulty,
            );
            let is_rank = sp.rank == crate::profiles::ProfileRank::Soldier;
            let ba = ai.base.blood_alcohol;

            attacks.push(PendingAttack {
                soldier_id: npc_id.into(),
                target_id,
                weapon_id,
                fighting_ability: fa,
                blood_alcohol: ba,
                is_rank_soldier: is_rank,
                attacker_direction: soldier.element.direction(),
                attacker_camp: soldier.soldier.cached_camp,
                attacker_pos: {
                    // `GetPossibleVictimsOfSwordStrike` subtracts
                    // `GetPositionMap()` for both actors. Elevation remains a
                    // separate input to the strike estimator; adding it to
                    // only the attacker's projected Y mixes world and map
                    // coordinates and can turn an adjacent opponent into a
                    // target hundreds of units away.
                    let map = &soldier.element.position_map();
                    (map.x, map.y)
                },
                attacker_elevation: soldier.element.position().z,
                boredom: soldier.human.sword_strike_boredom.clone(),
            });
        }

        // Process attacks — launch SwordstrikeThrust* sequence
        // elements as Interaction(1, command, this,
        // principal_opponent).
        for mut attack in attacks {
            let distance =
                entity_distance(&self.world.entities, attack.soldier_id, attack.target_id);

            // Select the best strike using ProposeGoodSwordStrike logic.
            let attacker_profile = assets
                .profile_manager
                .get_hth_weapon(attack.weapon_id)
                .unwrap_or_else(|| {
                    panic!(
                        "authorized sword-strike proposal owner {:?} requires missing HtH weapon {}",
                        attack.soldier_id, attack.weapon_id
                    )
                });

            // ── Sprite timing ──────────────────────────────────────────
            // Compute opponent_time_limit from target's sprite.
            // If the target isn't in an active strike animation,
            // time_limit = 1000 (permissive).  Otherwise, take the
            // sprite's frames-from-now-till-action-done (or 1000 if
            // unavailable).
            let opponent_time_limit: Option<i16> =
                self.get_entity(attack.target_id).and_then(|e| {
                    let sprite = &e.element_data().sprite;
                    // Only active strike animations (A-I) yield a
                    // strike-from-animation lookup; WaitingSword /
                    // MovingSword yield None → time_limit = 1000
                    // (permissive).  Check the actual animation
                    // (`GetAnimation()`), not `action_state.is_sword()`.
                    use crate::order::OrderType as OT;
                    let in_active_strike = matches!(
                        self.live_actor_animation(attack.target_id)?,
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
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            let nearby: Vec<crate::combat::NearbyVictim> = self
                .world
                .entities
                .humans()
                .filter_map(|(eid, e)| {
                    if eid == attack.soldier_id {
                        return None;
                    }
                    let elem = e.element_data();
                    if !elem.active {
                        return None;
                    }
                    let eligible_for_regular_strikes = is_possible_sword_strike_victim(
                        &self.world.entities,
                        attack.soldier_id,
                        e,
                        eid,
                        &assets.profile_manager,
                        &self.world.fast_grid,
                        obstacles,
                    );
                    let vdx = elem.position_map().x - attack.attacker_pos.0;
                    let vdy = (elem.position_map().y - attack.attacker_pos.1) * inv_aspect;
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
                        eligible_for_regular_strikes,
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

            let ctx = crate::combat::StrikeSelectionContext {
                attacker_profile,
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
            let strike = match crate::combat::propose_good_sword_strike(
                sim,
                &ctx,
                &nearby,
                &mut attack.boredom,
                false,
            ) {
                Some(crate::combat::ProposedCombatAction::Strike(s)) => Some(s),
                _ => None,
            };

            // ProposeGoodSwordStrike mutates its boredom history even when no
            // viable strike is selected, so persist it before branching on
            // the proposal result.
            let Entity::Soldier(soldier) = self
                .world
                .entities
                .get_mut(attack.soldier_id)
                .unwrap_or_else(|| {
                    panic!(
                        "sword-strike proposal owner {:?} disappeared during selection",
                        attack.soldier_id
                    )
                })
            else {
                panic!(
                    "sword-strike proposal owner {:?} changed entity kind",
                    attack.soldier_id
                );
            };
            soldier.human.sword_strike_boredom = attack.boredom;

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
                if let Some(entity) = self.world.entities.get_mut(attack.soldier_id) {
                    if let Some(human) = entity.human_data_mut() {
                        human.start_hulk(true, 1.0);
                    }
                    let elem = entity.element_data_mut();
                    elem.current_outline = crate::element::OutlineColorName::Striking;
                    elem.outline_width = 2;
                }
                compute_special_strike_preparation_time(
                    sim.config().difficulty,
                    attack.fighting_ability,
                )
            } else {
                0
            };

            // Flag the pending special strike and cancel movement so
            // the soldier stands still during the delay.
            // `begin_special_strike` sets the lifecycle latch and enters the
            // observable legacy special-strike substate; the
            // immediate stop-all side effect stays engine-side so it
            // runs before the new strike sequence is queued.
            if let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(attack.soldier_id)
                && let crate::element::AiBrain::Enemy(ref mut ai) = soldier.npc.ai_brain
            {
                ai.begin_special_strike();
                ai.base.stop_all();
            }
            self.drain_ai_owner_work_for(sim, assets, attack.soldier_id);
            self.apply_pending_ai_halt(attack.soldier_id);
            self.dispatch_condolations_for_owner_boundary(sim, attack.soldier_id, assets);

            // War-cry remarks for thrusts C/F/G/H/I.  Placed after
            // the state-set + stop-all so the say-order is correct.
            if matches!(
                strike,
                SwordStrike::C | SwordStrike::F | SwordStrike::G | SwordStrike::H | SwordStrike::I
            ) && let Some(Entity::Soldier(soldier)) =
                self.world.entities.get_mut(attack.soldier_id)
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
                self.drain_ai_owner_work_for(sim, assets, attack.soldier_id);
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

            tracing::debug!(
                soldier = ?attack.soldier_id,
                target = ?attack.target_id,
                ?command,
                ?strike,
                distance,
                "Enemy AI sword strike sequence launched"
            );
        }

        // Complete the statement immediately following Original's inline
        // ProposeGoodSwordStrike call. A successful proposal has entered the
        // (folded) special-strike state and therefore suppresses CombatInsult;
        // a rejection leaves the ordinary swordfight state and says it.
        // Keep this after every rejection/launch path, but before returning
        // to the dispatcher's owner-work drain.
        for owner in pending_considerations {
            let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(owner) else {
                continue;
            };
            let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
                continue;
            };
            if std::mem::take(&mut ai.pending_combat_insult_after_strike_consideration)
                && ai.base.current_substate == crate::ai::Substate::AttackingSwordfight
                && !ai.pending_special_strike
            {
                ai.base.say(crate::ai::Remark::CombatInsult);
            }
        }
    }

    /// Per-frame concussion healing for all humans.
    #[cfg(test)]
    pub(crate) fn tick_concussion_healing(&mut self, assets: &LevelAssets) {
        let mut pending_fit_again: Vec<EntityId> = Vec::new();
        // Standup / BeingStunnedSword chains discovered during the
        // entity-iter loop are launched after the loop ends to avoid
        // borrowing `self.world.entities` and `self` simultaneously.
        let mut pending_recover: Vec<crate::sequence::SequenceElement> = Vec::new();
        // Disjoint-borrow: pull the id counter out as a `&mut u32` so
        // the inner loop can stamp fresh ids via
        // `crate::order::alloc_order_id` while still holding
        // `self.world.entities.humans_mut()`.
        let next_order_id = &mut self.orders.next_order_id;
        for (entity_id, entity) in self.world.entities.humans_mut() {
            if entity.is_dead() {
                continue;
            }

            // Scroll-attached beggars short-circuit
            // `add_concussion_of_the_brain`, and the per-frame heal
            // calls `add_concussion(-1)` — so the heal is suppressed
            // for them.  Skip the whole tick.
            if let Entity::Civilian(c) = entity
                && c.npc.attached_scroll.is_some()
            {
                continue;
            }

            let life_points = get_life_points(entity);
            if life_points <= 0 {
                continue;
            }

            let ctx = concussion_ctx_full(
                entity,
                self.world.weather.is_forest_level,
                Some(&self.mission_domain.campaign),
                self.control.sim_config.difficulty,
            );

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
                let concussion = entity
                    .human_data()
                    .map(|h| h.concussion_of_the_brain)
                    .unwrap_or(0);
                let still_stunned = concussion > STUNNING_THRESHOLD;

                let npc_id = entity_id;
                if standing_anim.is_some() || still_stunned {
                    let mut elem = crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::Recover,
                        Some(npc_id.into()),
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
                    pending_fit_again.push(npc_id.into());
                }
            }
        }

        for elem in pending_recover {
            self.launch_element(elem);
        }

        for victim_id in pending_fit_again {
            self.dispatch_ai_stimulus(
                victim_id,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
            );
        }
    }

    /// Run one human's concussion prelude and close a natural/script wake
    /// synchronously before the owner's base Actor Hourglass begins.
    pub(crate) fn tick_concussion_healing_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
    ) {
        let mut recover = None;
        let naturally_woke = {
            let entity = self.world.entities.get_mut(owner).unwrap_or_else(|| {
                panic!(
                    "concussion owner {} disappeared from its legacy slot",
                    owner.index()
                )
            });
            assert!(
                entity.human_data().is_some(),
                "concussion owner {} is not human",
                owner.index()
            );
            if matches!(entity, Entity::Civilian(civilian) if civilian.npc.attached_scroll.is_some())
            {
                false
            } else if entity
                .human_data()
                .expect("validated concussion owner lost HumanData")
                .concussion_of_the_brain
                == 0
            {
                false
            } else {
                let life_points = get_life_points(entity);
                let ctx = concussion_ctx_full(
                    entity,
                    self.world.weather.is_forest_level,
                    Some(&self.mission_domain.campaign),
                    self.control.sim_config.difficulty,
                );
                let healing_speed =
                    concussion_healing_speed_for_entity(entity, &assets.profile_manager);
                let was_unconscious = entity
                    .human_data()
                    .expect("validated concussion owner lost HumanData")
                    .unconscious;
                combat::concussion_healing_tick(
                    entity
                        .human_data_mut()
                        .expect("validated concussion owner lost HumanData"),
                    healing_speed,
                    life_points,
                    &ctx,
                );
                let woke = was_unconscious
                    && !entity
                        .human_data()
                        .expect("validated concussion owner lost HumanData")
                        .unconscious;
                if woke {
                    let standing_anim = select_combat_animations(
                        entity.element_data().posture,
                        entity
                            .actor_data()
                            .expect("human concussion owner lost ActorData")
                            .action_state,
                    )
                    .map(|animations| animations.standing_up);
                    let still_stunned = entity
                        .human_data()
                        .expect("validated concussion owner lost HumanData")
                        .concussion_of_the_brain
                        > STUNNING_THRESHOLD;
                    if standing_anim.is_some() || still_stunned {
                        let mut element = crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::Recover,
                            Some(owner),
                        );
                        if let Some(animation) = standing_anim {
                            element.push_order(crate::order::Order::new(
                                animation,
                                0.0,
                                0.0,
                                crate::order::alloc_order_id(&mut self.orders.next_order_id),
                            ));
                        }
                        if still_stunned {
                            element.push_order(crate::order::Order::new(
                                crate::order::OrderType::BeingStunnedSword,
                                0.0,
                                0.0,
                                crate::order::alloc_order_id(&mut self.orders.next_order_id),
                            ));
                        }
                        recover = Some(element);
                    }
                }
                woke
            }
        };

        if let Some(element) = recover {
            self.launch_element(element);
            self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                .unwrap_or_else(|error| {
                    panic!(
                        "concussion owner {} failed to launch recovery synchronously: {error:?}",
                        owner.index()
                    )
                });
        }

        if naturally_woke && matches!(owner, EntityId::Soldier(_) | EntityId::Civilian(_)) {
            self.dispatch_ai_stimulus(
                owner,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
            );
        }
        let dispatched_wake = if matches!(owner, EntityId::Soldier(_) | EntityId::Civilian(_)) {
            self.dispatch_pending_fit_again_for_npc(sim, owner, assets)
        } else {
            naturally_woke
        };
        if dispatched_wake {
            // EVENT_FITAGAIN's resurrection fan-out and eye reset are inline
            // consequences of Think in Original, including under FrozenAll.
            self.tick_ai_pending_resurrection_and_eyes_for_npc(owner);
            self.apply_wake_redetection_blinks(owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::{
        ActorData, ActorSoldier, Camp, ElementData, ElementKind, HumanData, NpcData, SoldierData,
    };
    use crate::position_interface::SectorHandle;

    fn falling_pushed_soldier(dead: bool) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Flying,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D {
            x: 10.0,
            y: 20.0,
            z: 0.0,
        });
        element.set_position_map(MapPoint::new(10.0, 20.0));
        element.set_layer(1);
        element.set_sector(SectorHandle::new(2));

        let mut actor = ActorData::default();
        actor.action_state = ActionState::WaitingSword;
        actor.active_flight = Some(crate::element::ActiveFlight {
            increment_x: 5.0,
            goal_x: 15.0,
            goal_y: 20.0,
            frames_remaining: 1,
            antagonist: Some(EntityId::new(99, crate::entity_id::EntityIdKind::Pc)),
            goal_layer: 3,
            goal_sector: SectorHandle::new(4),
            ..Default::default()
        });

        Entity::Soldier(ActorSoldier {
            element,
            actor,
            human: HumanData::default(),
            npc: NpcData {
                life_points: if dead { 0 } else { 50 },
                ..NpcData::default()
            },
            soldier: SoldierData::default(),
        })
    }

    #[test]
    fn fatal_push_goal_preserves_flying_pose_until_animation_terminates() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(falling_pushed_soldier(true));

        engine.tick_push_flights(sim, &LevelAssets::default());

        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().posture, Posture::Flying);
        assert_eq!(
            victim.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );
        assert_eq!(
            victim.element_data().position_map(),
            MapPoint::new(15.0, 20.0)
        );
        assert_eq!(victim.element_data().layer(), 1);
        assert_eq!(victim.element_data().sector(), SectorHandle::new(2));
        assert_eq!(
            victim
                .actor_data()
                .unwrap()
                .active_flight
                .unwrap()
                .frames_remaining,
            0
        );

        // The animation completion pass owns this transition. Its posture
        // change lets the following PerformFlight tail apply landing
        // metadata and clear the retained flight.
        engine
            .get_entity_mut(victim_id)
            .unwrap()
            .set_posture(Posture::DeadBack);
        engine.tick_push_flights(sim, &LevelAssets::default());
        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().layer(), 3);
        assert_eq!(victim.element_data().sector(), SectorHandle::new(4));
        assert!(victim.actor_data().unwrap().active_flight.is_none());
    }

    #[test]
    fn combat_flight_done_applies_increment_before_terminal_snap() {
        let sim = crate::sim_rng::test_context();
        let mut entity = falling_pushed_soldier(false);
        entity
            .actor_data_mut()
            .unwrap()
            .active_flight
            .as_mut()
            .unwrap()
            .goal_x = 14.0;
        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(entity);

        engine.tick_push_flights(&sim, &LevelAssets::default());

        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(
            victim.element_data().position_map(),
            MapPoint::new(15.0, 20.0)
        );
        assert_eq!(
            victim
                .actor_data()
                .unwrap()
                .active_flight
                .unwrap()
                .frames_remaining,
            0
        );
    }

    #[test]
    fn knockout_push_goal_preserves_flying_pose_until_animation_terminates() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut entity = falling_pushed_soldier(false);
        entity.human_data_mut().unwrap().unconscious = true;
        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(entity);

        engine.tick_push_flights(sim, &LevelAssets::default());

        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().posture, Posture::Flying);
        assert_eq!(
            victim.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );
        assert_eq!(
            victim
                .actor_data()
                .unwrap()
                .active_flight
                .unwrap()
                .frames_remaining,
            0
        );
    }

    #[test]
    fn push_rechecks_current_relationship_before_queueing_enter_swordfight() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let mut engine = EngineInner::new();

        let mut attacker = falling_pushed_soldier(false);
        let mut victim = falling_pushed_soldier(false);
        if let Entity::Soldier(soldier) = &mut attacker {
            soldier.soldier.cached_camp = Camp::Lacklandists;
        }
        if let Entity::Soldier(soldier) = &mut victim {
            soldier.soldier.cached_camp = Camp::Lacklandists;
        }
        let attacker_id = engine.add_entity(attacker);
        let victim_id = engine.add_entity(victim);

        engine
            .get_entity_mut(attacker_id)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .pending_push_swordfight = vec![victim_id];
        engine.complete_melee_strike(&sim, &assets, attacker_id, None, 0, SwordStrike::A, None);
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);

        if let Entity::Soldier(soldier) = engine.get_entity_mut(victim_id).unwrap() {
            soldier.soldier.cached_camp = Camp::Royalists;
        }
        engine
            .get_entity_mut(attacker_id)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .pending_push_swordfight = vec![victim_id];
        engine.complete_melee_strike(&sim, &assets, attacker_id, None, 0, SwordStrike::A, None);

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("eligible push victim should queue EnterSwordfight");
        let enter = sequence.get(0).unwrap();
        assert_eq!(enter.command, Command::EnterSwordfight);
        assert_eq!(enter.owner, Some(victim_id));
        assert!(matches!(
            enter.get_property(crate::sequence::Field::Opponent),
            Some(crate::sequence::FieldValue::Element(opponent)) if *opponent == attacker_id
        ));
        assert!(matches!(
            enter.get_property(crate::sequence::Field::JumplineDestination),
            Some(crate::sequence::FieldValue::Integer(0))
        ));
        assert!(matches!(
            enter.get_property(crate::sequence::Field::SwordfightPrepared),
            Some(crate::sequence::FieldValue::Bool(false))
        ));
    }
}
