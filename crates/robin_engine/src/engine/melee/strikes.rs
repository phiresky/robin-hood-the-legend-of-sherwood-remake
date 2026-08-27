//! Per-frame strike ticks (melee, sweep, push, rider, enemy AI) and concussion healing.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;

/// `PARITY_DEBUG_SWORD_DAMAGE=1` traces every sword-damage application and
/// every sweep-strike lifecycle step (seed, per-frame phase, per-frame arc
/// test) so a divergent hit frame can be attributed to a concrete attacker,
/// victim list and sweep angle.
pub(crate) fn sword_damage_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PARITY_DEBUG_SWORD_DAMAGE").is_some())
}

fn strike_effect_debug_matches(
    frame: u32,
    attacker_creation_order: u32,
    victim_creation_order: u32,
) -> bool {
    if std::env::var_os("PARITY_DEBUG_STRIKE_EFFECT").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for strike-effect diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_STRIKE_EFFECT_FRAME").is_none_or(|expected| expected == frame)
        && parse_filter("PARITY_DEBUG_STRIKE_EFFECT_ATTACKER_CREATION_ORDER")
            .is_none_or(|expected| expected == attacker_creation_order)
        && parse_filter("PARITY_DEBUG_STRIKE_EFFECT_VICTIM_CREATION_ORDER")
            .is_none_or(|expected| expected == victim_creation_order)
}

fn special_strike_lifecycle_debug_matches(frame: u32, owner: u32) -> bool {
    if std::env::var_os("PARITY_DEBUG_SPECIAL_STRIKE_LIFECYCLE").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for special-strike diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_SPECIAL_STRIKE_FRAME").is_none_or(|expected| expected == frame)
        && parse_filter("PARITY_DEBUG_SPECIAL_STRIKE_OWNER_HANDLE")
            .is_none_or(|expected| expected == owner)
}

fn opponent_sprite_timing_debug_matches(frame: u32, owner: u32, target: u32) -> bool {
    if std::env::var_os("PARITY_DEBUG_OPPONENT_SPRITE_TIMING").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for opponent-sprite-timing diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_OPPONENT_SPRITE_TIMING_FRAME")
        .is_none_or(|expected| expected == frame)
        && parse_filter("PARITY_DEBUG_OPPONENT_SPRITE_TIMING_OWNER")
            .is_none_or(|expected| expected == owner)
        && parse_filter("PARITY_DEBUG_OPPONENT_SPRITE_TIMING_TARGET")
            .is_none_or(|expected| expected == target)
}

fn special_strike_selected_snapshot(
    manager: &crate::sequence::SequenceManager,
    owner: EntityId,
) -> String {
    let selected = manager.current_element_for_actor(owner);
    let element = selected.and_then(|(sequence, index)| {
        manager.get_element(sequence, index).map(|element| {
            (
                sequence,
                index,
                element.command,
                element.state,
                element.priority,
                element.cross_postponed,
                element.orders.len(),
            )
        })
    });
    format!("{element:?}")
}
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId, Posture};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

/// Match `UpdateRoll`'s rejected-slope arm: publish only the current point as
/// the new goal. The surrounding `CheckForLineCrossing` tail then calls
/// `ComputeIncrementAll`; keeping the increment uncached lets its
/// `bVerySmallIncrement` guard preserve the prior direction goal.
fn stop_roll_at_current_position(
    position: &mut crate::position_interface::PositionInterface,
    here: crate::coordinates::MapPoint,
) {
    position.set_map_goal(here);
}

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

        // Original's START-only warning forecast is not side-effect free:
        // GetPossibleVictimsOfLateralSwordStrike and
        // GetPossibleVictimsOfHalfCircleSwordStrike write the human-owned
        // initial/current/final sweep angles even though they fill a temporary
        // warning-victim list. A replacement lateral/half-circle strike can
        // therefore keep an interrupted strike's victim FIFO while rebasing
        // its geometry to the replacement strike before the first IN_PROGRESS
        // Execute. GetPossibleVictimsOfCircleSwordStrike does not write those
        // angles, so full circles deliberately retain the old geometry here.
        // Do not create a sweep for an ordinary fresh strike here; its real
        // victim list is still initialized only at MotionState::Done.
        let retained_victims = self.get_entity(attacker_id).and_then(|entity| {
            entity
                .actor_data()
                .and_then(|actor| actor.sweep_state.as_ref())
                .map(|sweep| sweep.pending_victims.clone())
                .filter(|victims| !victims.is_empty())
                .or_else(|| {
                    entity
                        .human_data()
                        .map(|human| human.sword_sweep.victims.clone())
                        .filter(|victims| !victims.is_empty())
                })
                .or_else(|| {
                    entity
                        .actor_data()
                        .map(|actor| actor.pending_push_swordfight.clone())
                        .filter(|victims| !victims.is_empty())
                })
        });
        let strike_kind = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|profile| profile.thrusts[strike as usize].kind);
        if strike_kind.is_some_and(|kind| {
            matches!(
                kind,
                WeaponThrustKind::Lateral
                    | WeaponThrustKind::TrueHalfCircle
                    | WeaponThrustKind::FalseHalfCircle
            )
        }) && let Some(retained_victims) = retained_victims
        {
            self.initialize_sweep(
                assets,
                attacker_id,
                strike,
                profile_idx,
                strike_kind.expect("rebased warning strike kind disappeared"),
                retained_victims,
            );
            if let Some(actor) = self
                .get_entity_mut(attacker_id)
                .and_then(Entity::actor_data_mut)
            {
                // These are two Rust mirrors of Original's single shared
                // victim list. Once the replacement sweep takes ownership,
                // the push-completion mirror must not retain a duplicate.
                actor.pending_push_swordfight.clear();
            }
        }

        let mut victims =
            self.collect_sword_strike_warning_victims(assets, attacker_id, strike, profile_idx);
        // RHElementActorHuman::Execute warns the list produced by
        // GetPossibleVictimsOfSwordStrike. Every multi-victim collector fills
        // that list by walking RHEngine::GetActor(0..GetNumberOfActors), whose
        // marrayActors registry is append-only AddElement order. Rust entity
        // slots are grouped by kind and do not preserve that ordering after a
        // legacy save is adopted, so restore the authoritative actor order
        // before these synchronous callbacks consume RNG.
        victims.sort_by_key(|&victim_id| self.world.original_creation_order(victim_id));
        self.warn_for_strike(sim, assets, attacker_id, &victims, strike);
    }

    fn selected_melee_identity_is_live(
        &self,
        attacker_id: EntityId,
        selected: super::tick::MeleeOwnerSelection,
    ) -> bool {
        self.orders
            .sequence_manager
            .current_order_for_actor(attacker_id)
            .is_some_and(|(seq_id, elem_idx, order)| {
                seq_id == selected.seq_id
                    && elem_idx == selected.elem_idx
                    && order.order_id == selected.order_id
                    && sword_strike_from_animation(order.order_type).is_some()
            })
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

        self.tick_straight_melee_for(sim, assets, attacker_id, selected);
        let sweep_phase = if self.selected_melee_identity_is_live(attacker_id, selected) {
            self.tick_nonstraight_melee_for(sim, assets, attacker_id, selected)
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

        // In-flight displacement now runs at each actor's Execute slot. Keep
        // only terminal landing reconciliation here: existing synchronous
        // completion callbacks can refresh the position latch before this
        // legacy Rust cleanup boundary.
        // TODO(original-parity): move this terminal-only cleanup into the
        // owner slot once those callbacks preserve PerformFlight's final
        // NewMove delta themselves.
        self.tick_push_flight_terminal_landings(sim, assets);
        self.tick_enemy_sword_attacks(sim, assets);
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
    pub(in crate::engine) fn tick_straight_melee_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        selected: super::tick::MeleeOwnerSelection,
    ) {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return;
        }

        let Some((strike, target_id, animation)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(attacker_id)
            .filter(|(seq_id, elem_idx, order)| {
                *seq_id == selected.seq_id
                    && *elem_idx == selected.elem_idx
                    && order.order_id == selected.order_id
            })
            .and_then(|(_, _, order)| {
                let strike = sword_strike_from_animation(order.order_type)?;
                let target = order.antagonist.unwrap_or_else(|| {
                    panic!(
                        "selected melee order {:?}/{}/{} for {attacker_id:?} has no antagonist",
                        selected.seq_id, selected.elem_idx, selected.order_id
                    )
                });
                Some((strike, target, order.order_type))
            })
        else {
            return;
        };
        let profile_idx = self
            .get_entity(attacker_id)
            .map(|entity| get_hth_weapon_id_full(entity, &assets.profile_manager))
            .unwrap_or_else(|| panic!("selected melee attacker {attacker_id:?} disappeared"));
        let strike_kind = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|profile| profile.thrusts[strike as usize].kind)
            .unwrap_or(WeaponThrustKind::Straight);
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

        let entity = self
            .get_entity_mut(attacker_id)
            .unwrap_or_else(|| panic!("selected melee attacker {attacker_id:?} disappeared"));
        let direction = entity.element_data().direction() as u16;
        let motion = entity.element_data_mut().sprite.perform_action(
            sim,
            Some(selected.order_id),
            animation,
            direction,
            crate::sprite::FrameProgression::Default,
            false,
        );
        let (current_frame, frame_count, action_done_frame, action_done_counter) = {
            let sprite = &entity.element_data().sprite;
            (
                sprite.current_frame,
                sprite.frame_count,
                sprite.action_done_frame,
                sprite.action_done_counter,
            )
        };
        tracing::trace!(
            "tick_straight_melee_for: entity={} order_id={} strike={:?} anim={:?} dir={} motion={:?}",
            attacker_id.index(),
            selected.order_id,
            strike,
            animation,
            direction,
            motion
        );
        let started = matches!(motion, crate::sprite::MotionState::Start);
        let hit = matches!(motion, crate::sprite::MotionState::Done);
        let completed = matches!(
            motion,
            crate::sprite::MotionState::Terminated | crate::sprite::MotionState::Aborted
        );

        let frame = self.control.frame_counter;
        let attacker_creation_order = self.world.original_creation_order(attacker_id);
        let victim_creation_order = self.world.original_creation_order(target_id);
        if strike_effect_debug_matches(frame, attacker_creation_order, victim_creation_order) {
            eprintln!(
                "[STRIKE_EFFECT frame={frame} attacker={} attacker_co={attacker_creation_order} victim={} victim_co={victim_creation_order} phase=pulse strike={strike:?} animation={animation:?} motion={motion:?} started={started} hit={hit} completed={completed} sprite_frame={current_frame} frame_count={frame_count} action_done_frame={action_done_frame} action_done_counter={action_done_counter}]",
                attacker_id.index(),
                target_id.index(),
            );
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
        if completed {
            self.complete_melee_strike(
                sim,
                assets,
                attacker_id,
                Some(selected.seq_id),
                selected.elem_idx,
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
        // RHElementActorHuman::ExecuteStraightSwordStrike uses the full
        // stored 3-D positions here, unlike several swordfight planning
        // predicates that deliberately use map distance.  Keep the Rust-only
        // Assault fallback on its existing metric; Original reaches this
        // helper only for STRAIGHT weapon thrusts.
        let profile = profile_idx.map(|idx| {
            assets.profile_manager.get_hth_weapon(idx).unwrap_or_else(|| {
                panic!(
                    "straight-strike attacker {attacker_id:?} references missing HtH weapon profile {idx}"
                )
            })
        });
        let is_straight = profile.is_some_and(|profile| {
            profile.thrusts[strike as usize].kind == WeaponThrustKind::Straight
        });
        let distance = if is_straight {
            entity_world_distance(&self.world.entities, attacker_id, victim_id)
        } else {
            entity_distance(&self.world.entities, attacker_id, victim_id)
        };
        let in_range = profile
            .map(|profile| combat::is_strike_in_range(profile, strike, distance))
            .unwrap_or(distance <= 50.0);
        let frame = self.control.frame_counter;
        let attacker_creation_order = self.world.original_creation_order(attacker_id);
        let victim_creation_order = self.world.original_creation_order(victim_id);
        if strike_effect_debug_matches(frame, attacker_creation_order, victim_creation_order) {
            let attacker = self.expect_entity(attacker_id, "strike-effect diagnostic attacker");
            let victim = self.expect_entity(victim_id, "strike-effect diagnostic victim");
            let attacker_direction = attacker.element_data().direction();
            let victim_direction = direction_to(&self.world.entities, attacker_id, victim_id);
            let angle_delta = (i16::from(attacker_direction) - victim_direction).rem_euclid(16);
            let attacker_has_victim = attacker
                .human_data()
                .is_some_and(|human| human.opponents.contains(&victim_id));
            let victim_has_attacker = victim
                .human_data()
                .is_some_and(|human| human.opponents.contains(&attacker_id));
            let non_mutual = attacker_has_victim != victim_has_attacker;
            let already_hit = attacker
                .human_data()
                .is_some_and(|human| human.sword_sweep.victims.contains(&victim_id));
            let queued = in_range && profile_idx.is_some();
            eprintln!(
                "[STRIKE_EFFECT frame={frame} attacker={} attacker_co={attacker_creation_order} victim={} victim_co={victim_creation_order} phase=candidate strike={strike:?} attacker_sector={:?} victim_sector={:?} attacker_direction={attacker_direction} victim_direction={victim_direction} angle_delta={angle_delta} distance_bits={:#010x} in_range={in_range} attacker_has_victim={attacker_has_victim} victim_has_attacker={victim_has_attacker} non_mutual={non_mutual} already_hit={already_hit} profile_idx={profile_idx:?} queue={queued}]",
                attacker_id.index(),
                victim_id.index(),
                attacker.element_data().sector(),
                victim.element_data().sector(),
                distance.to_bits(),
            );
        }
        if in_range {
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

    pub(super) fn complete_melee_strike(
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
        // `mlistSwordStrikeVictims` belongs to `ExecutePushSwordStrike`
        // (original-code/RHelementactorhuman.cpp:9928): its `RHMOTION_DONE`
        // arm clears and refills the list, and only its `RHMOTION_TERMINATED`
        // arm walks the list to send each victim an `ENTER_SWORDFIGHT`. A
        // strike of any other kind runs a different executor and never
        // touches the list, so victims recorded by a push strike that was
        // interrupted before it terminated survive until the next push
        // strike's DONE refills them.
        let completes_push_strike = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .is_some_and(|profile| {
                matches!(
                    profile.thrusts[strike as usize].kind,
                    WeaponThrustKind::PushAside
                )
            });
        let pending_swordfights = if let Some(entity) = self.world.entities.get_mut(actor_id)
            && let Some(actor) = entity.actor_data_mut()
        {
            if clears_shared_sweep {
                actor.sweep_state = None;
            }
            let pending_swordfights = if completes_push_strike {
                std::mem::take(&mut actor.pending_push_swordfight)
            } else {
                Vec::new()
            };
            if clears_shared_sweep && let Some(human) = entity.human_data_mut() {
                // ExecuteLateralSwordStrike/ExecuteCircleSwordStrike delete
                // their human-owned victim list when the strike genuinely
                // terminates. Keep the serialized mirror in lockstep with
                // the executable Rust sweep so a later fresh strike cannot
                // mistake terminated geometry for a resumed saved sweep.
                human.sword_sweep = crate::element::HumanSwordSweepState::default();
            }
            pending_swordfights
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
                let frame = self.control.frame_counter;
                let creation_order = self.world.original_creation_order(actor_id);
                if let Some(entity) = self.get_entity_mut(actor_id)
                    && let Some(human) = entity.human_data_mut()
                {
                    let before = human.tiredness;
                    human.tiredness = combat::add_strike_tiredness(human.tiredness, energy);
                    if combat::tiredness_debug_matches(creation_order) {
                        eprintln!(
                            "RUST_TIREDNESS frame={frame} co={creation_order} site=strike_energy \
                             before={before} after={} strike={} energy={energy}",
                            human.tiredness, strike as u32
                        );
                    }
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
            let Some(selected) = self
                .orders
                .sequence_manager
                .current_order_for_actor(actor_id)
                .and_then(|(seq_id, elem_idx, order)| {
                    sword_strike_from_animation(order.order_type).map(|_| {
                        super::tick::MeleeOwnerSelection {
                            seq_id,
                            elem_idx,
                            order_id: order.order_id,
                        }
                    })
                })
            else {
                continue;
            };
            self.tick_straight_melee_for(sim, assets, actor_id, selected);
            let sweep_phase = self.tick_nonstraight_melee_for(sim, assets, actor_id, selected);
            self.tick_selected_sweep_phase(sim, assets, actor_id, sweep_phase);
        }
    }

    /// Advance one non-straight sequence-driven melee strike at its actor's
    /// creation-order slot.
    pub(in crate::engine) fn tick_nonstraight_melee_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        selected: super::tick::MeleeOwnerSelection,
    ) -> SweepTickPhase {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return SweepTickPhase::Dormant;
        }
        if self.actors_frozen() {
            return SweepTickPhase::Dormant;
        }

        let Some((strike, target_id, animation)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(attacker_id)
            .filter(|(seq_id, elem_idx, order)| {
                *seq_id == selected.seq_id
                    && *elem_idx == selected.elem_idx
                    && order.order_id == selected.order_id
            })
            .and_then(|(_, _, order)| {
                let strike = sword_strike_from_animation(order.order_type)?;
                let target = order.antagonist.unwrap_or_else(|| {
                    panic!(
                        "selected melee order {:?}/{}/{} for {attacker_id:?} has no antagonist",
                        selected.seq_id, selected.elem_idx, selected.order_id
                    )
                });
                Some((strike, target, order.order_type))
            })
        else {
            return SweepTickPhase::Dormant;
        };
        let profile_idx = self
            .get_entity(attacker_id)
            .map(|entity| get_hth_weapon_id_full(entity, &assets.profile_manager))
            .unwrap_or_else(|| panic!("selected melee attacker {attacker_id:?} disappeared"));
        let (strike_kind, strike_direction) = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|profile| {
                let thrust = &profile.thrusts[strike as usize];
                (thrust.kind, thrust.direction)
            })
            .unwrap_or((
                WeaponThrustKind::Straight,
                crate::profiles::WeaponThrustDirection::LeftToRight,
            ));
        if matches!(
            strike_kind,
            WeaponThrustKind::Straight | WeaponThrustKind::Assault
        ) {
            return SweepTickPhase::Dormant;
        }

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

        // Phase 1: execute the selected order. Original derives strike and
        // target from this order and applies the hit on the sprite's one-shot
        // RHMOTION_DONE result.
        {
            let entity_id = attacker_id;
            let Some(entity) = self.world.entities.get_mut(attacker_id) else {
                return SweepTickPhase::Dormant;
            };
            sweep_phase = SweepTickPhase::InProgress;
            let true_sweep_at_action_point = matches!(
                strike_kind,
                WeaponThrustKind::TrueCircle | WeaponThrustKind::TrueHalfCircle
            )
            .then(|| {
                entity
                    .actor_data()
                    .and_then(|actor| actor.sweep_state.as_ref())
            })
            .flatten()
            .filter(|_| {
                entity.element_data().sprite.last_processed_order_id == selected.order_id.get()
                    && entity.element_data().sprite.current_frame
                        == entity.element_data().sprite.action_done_frame
                    && entity.element_data().sprite.frame_count
                        == entity.element_data().sprite.action_done_counter
            })
            .map(|sweep| {
                let still_rotating = match strike_direction {
                    crate::profiles::WeaponThrustDirection::LeftToRight => {
                        sweep.current_angle < sweep.final_angle
                    }
                    _ => sweep.current_angle > sweep.final_angle,
                };
                (sweep.current_angle, still_rotating)
            });
            let hold_true_sweep =
                true_sweep_at_action_point.is_some_and(|(_, still_rotating)| still_rotating);
            if hold_true_sweep {
                // The rotation phase runs no Sprite method and reports
                // IN_PROGRESS outright. Latch that here: without it the arm
                // leaves the previous tick's one-shot DONE as the actor's
                // motion state for every frame of the sweep.
                entity.element_data_mut().sprite.last_motion_state =
                    Some(crate::sprite::MotionState::InProgress);
                tracing::trace!(
                    "tick_melee_strikes: entity={} order_id={} strike={:?} holding true-circle sweep",
                    entity_id.index(),
                    selected.order_id,
                    strike
                );
            } else {
                // ExecuteTrueCircleSwordStrikeAction tests IsActionDone and
                // presents the current sweep angle before its terminal
                // PerformAction call advances the frame counter.  Preserve
                // that exact equality boundary; the later effect phase sees
                // the already-advanced sprite and must not synthesize it.
                if let Some((current_angle, _)) = true_sweep_at_action_point {
                    let new_dir = angle_to_sector(current_angle);
                    let elem = entity.element_data_mut();
                    elem.set_direction_instantly(new_dir as i16);
                    elem.sprite
                        .force_action_direction(animation, new_dir.into());
                }
                let direction = entity.element_data().direction() as u16;
                let motion = entity.element_data_mut().sprite.perform_action(
                    sim,
                    Some(selected.order_id),
                    animation,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                );
                tracing::trace!(
                    "tick_melee_strikes: entity={} order_id={} strike={:?} anim={:?} dir={} motion={:?}",
                    entity_id.index(),
                    selected.order_id,
                    strike,
                    animation,
                    direction,
                    motion
                );
                started = matches!(motion, crate::sprite::MotionState::Start);
                sweep_phase = match motion {
                    crate::sprite::MotionState::Start => SweepTickPhase::Start,
                    crate::sprite::MotionState::InProgress | crate::sprite::MotionState::Done => {
                        SweepTickPhase::InProgress
                    }
                    crate::sprite::MotionState::Terminated
                    | crate::sprite::MotionState::Aborted
                    | crate::sprite::MotionState::Error => SweepTickPhase::Dormant,
                };
                if matches!(motion, crate::sprite::MotionState::Done) {
                    let attacker_id = entity_id;
                    hits.push(StrikeHit {
                        attacker_id,
                        victim_id: target_id,
                        strike,
                        attacker_profile_idx: profile_idx,
                    });
                }
                if matches!(
                    motion,
                    crate::sprite::MotionState::Terminated | crate::sprite::MotionState::Aborted
                ) {
                    completed.push(CompletedStrike {
                        actor_id: attacker_id,
                        sequence_id: Some(selected.seq_id),
                        element_index: selected.elem_idx,
                        strike,
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
                // no AI warn tolerance. Original seeds this list solely by
                // scanning actors through the strike-kind geometry; the
                // interaction antagonist is not recovered when that scan
                // rejects it (for example, a lateral target outside the
                // strike arc).
                let all_victims = self.execute_multi_target_strike(
                    assets,
                    hit.attacker_id,
                    hit.strike,
                    hit.attacker_profile_idx,
                );
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
                let all_victims = self.execute_multi_target_strike(
                    assets,
                    hit.attacker_id,
                    hit.strike,
                    hit.attacker_profile_idx,
                );
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

    pub(super) fn tick_selected_sweep_phase(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        phase: SweepTickPhase,
    ) {
        if sword_damage_debug_enabled() {
            eprintln!(
                "[SWEEPPHASE f={} attacker={:?} (co {}) phase={:?} sweep_state={:?} human_victims={:?}]",
                self.control.frame_counter,
                attacker_id,
                self.world.original_creation_order(attacker_id),
                phase,
                self.get_entity(attacker_id)
                    .and_then(Entity::actor_data)
                    .and_then(|a| a.sweep_state.as_ref())
                    .map(|s| s.pending_victims.len()),
                self.get_entity(attacker_id)
                    .and_then(Entity::human_data)
                    .map(|h| h.sword_sweep.victims.len()),
            );
        }
        match phase {
            SweepTickPhase::Dormant | SweepTickPhase::Start => {}
            SweepTickPhase::Initialized => {
                self.tick_sweep_for(sim, assets, attacker_id, true);
            }
            SweepTickPhase::InProgress => {
                // ExecuteCircleSwordStrike advances its retained angles only
                // after RHSprite::IsActionDone becomes true.  A new circle
                // strike can inherit the previous strike's human-owned victim
                // list/angles, but it must first play to its own action point;
                // otherwise the old geometry rotates the new animation on its
                // first IN_PROGRESS frame. Lateral strikes deliberately keep
                // their different legacy rule and advance any retained list
                // on IN_PROGRESS.
                let (active_strike, active_order_id) = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(attacker_id)
                    .and_then(|(_, _, order)| {
                        sword_strike_from_animation(order.order_type)
                            .map(|strike| (strike, order.order_id))
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "selected non-straight melee attacker {attacker_id:?} lost its strike order"
                        )
                    });
                let entity = self.get_entity(attacker_id).unwrap_or_else(|| {
                    panic!("selected non-straight melee attacker {attacker_id:?} disappeared")
                });
                let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager)
                    .unwrap_or_else(|| {
                        panic!(
                            "selected non-straight melee attacker {attacker_id:?} has no melee weapon profile"
                        )
                    });
                let profile = assets
                    .profile_manager
                    .get_hth_weapon(profile_idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "selected non-straight melee attacker {attacker_id:?} references missing weapon profile {profile_idx}"
                        )
                    });
                let active_kind = profile.thrusts[active_strike as usize].kind;
                // Execute's weapon-kind dispatch gives PushAside (and the
                // other non-sweep kinds) their own executor.  They do not
                // enter ExecuteLateralSwordStrike/ExecuteCircleSwordStrike,
                // so an interrupted strike's human-owned victim/angle state
                // must remain dormant while that replacement owns Execute.
                if !matches!(
                    active_kind,
                    WeaponThrustKind::Lateral
                        | WeaponThrustKind::TrueHalfCircle
                        | WeaponThrustKind::FalseHalfCircle
                        | WeaponThrustKind::TrueCircle
                        | WeaponThrustKind::FalseCircle
                ) {
                    return;
                }
                let at_action_point = entity.element_data().sprite.last_processed_order_id
                    == active_order_id.get()
                    && entity.element_data().sprite.current_frame
                        == entity.element_data().sprite.action_done_frame
                    && entity.element_data().sprite.frame_count
                        == entity.element_data().sprite.action_done_counter;
                let retained_circle_off_action_point = self
                    .get_entity(attacker_id)
                    .and_then(|entity| entity.actor_data())
                    .and_then(|actor| actor.sweep_state.as_ref())
                    .is_some_and(|_| is_circle_sweep(active_kind) && !at_action_point);
                if retained_circle_off_action_point {
                    // ExecuteCircleSwordStrike always runs the effect with
                    // the current Execute call's strike, even before that
                    // animation reaches its action point.  The action-point
                    // gate only protects the tail angle advance.  Preserve
                    // the retained victim/angle geometry, but rebind the
                    // payload and direction to the replacement strike.
                    self.rebind_retained_sweep_to_active_strike(assets, attacker_id);
                    self.tick_sweep_for_mode(sim, assets, attacker_id, false, true);
                    return;
                }
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
    pub(super) fn rebind_retained_sweep_to_active_strike(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
    ) {
        let Some((strike, active_order_id)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(attacker_id)
            .and_then(|(_, _, order)| {
                sword_strike_from_animation(order.order_type).map(|strike| (strike, order.order_id))
            })
        else {
            return;
        };
        let Some(entity) = self.get_entity(attacker_id) else {
            return;
        };
        // The legacy save owns these fields on RHElementActorHuman itself.
        // Rust's SweepState is only the executable mirror, so reconstruct it
        // lazily when a loaded strike resumes in the middle of its sweep.
        let serialized_sweep = entity.human_data().map(|human| human.sword_sweep.clone());
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
        let thrust = &profile.thrusts[strike as usize];
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
        // Original serializes the victim FIFO and the three sweep angles as
        // independent RHElementActorHuman fields.  The angles remain live
        // for a true-circle rotation even when its victim scan found nobody:
        // ExecuteTrueCircleSwordStrikeAction unconditionally presents
        // mfCurrentStrikeAngle once the sprite reaches its action point.
        //
        // A fresh strike's untouched serialized mirror is all zeroes.  Keep
        // that sentinel from fabricating a pre-action sweep, while accepting
        // non-default true-circle geometry without requiring a victim.
        let saved_sweep = serialized_sweep.filter(|saved| {
            let sprite = &entity.element_data().sprite;
            // This hook runs after the action executor, so the DONE call may
            // already have incremented the frame counter.  It must still be
            // on the action-done frame: accepting later animation frames
            // would resurrect the same serialized sweep on the next tick.
            let on_action_point_frame = sprite.last_processed_order_id == active_order_id.get()
                && sprite.current_frame == sprite.action_done_frame
                && sprite.frame_count >= sprite.action_done_counter;
            !saved.victims.is_empty()
                || (matches!(
                    thrust.kind,
                    WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::TrueCircle
                ) && on_action_point_frame
                    && (saved.initial_angle.to_bits() != 0
                        || saved.current_angle.to_bits() != 0
                        || saved.final_angle.to_bits() != 0))
        });
        let signed_rotation = strike_profile_angle(thrust.rotation_angle)
            * if thrust.direction == crate::profiles::WeaponThrustDirection::RightToLeft {
                -1.0
            } else {
                1.0
            };
        if let Some(actor) = self
            .get_entity_mut(attacker_id)
            .and_then(Entity::actor_data_mut)
        {
            if actor.sweep_state.is_none()
                && let Some(saved) = saved_sweep
            {
                actor.sweep_state = Some(crate::movement::SweepState {
                    pending_victims: saved.victims,
                    initial_angle: saved.initial_angle,
                    current_angle: saved.current_angle,
                    final_angle: saved.final_angle,
                    rotation_per_frame: signed_rotation,
                    direction: thrust.direction,
                    strike,
                    attacker_profile_idx: Some(profile_idx),
                    strike_kind: thrust.kind,
                });
            }
            let Some(sweep) = actor.sweep_state.as_mut() else {
                return;
            };
            sweep.rotation_per_frame = signed_rotation;
            sweep.direction = thrust.direction;
            sweep.strike = strike;
            sweep.attacker_profile_idx = Some(profile_idx);
            sweep.strike_kind = thrust.kind;
        }
    }

    /// Replay the angle side effect of `EstimateDamageOfSwordStrike`.
    ///
    /// `ProposeGoodSwordStrike` estimates each candidate strike through
    /// `EstimateDamageOfSwordStrike`
    /// (`original-code/RHelementactorhuman.cpp:12875`), which builds its
    /// victim list with `GetPossibleVictimsOfSwordStrike`
    /// (`original-code/RHelementactorhuman.cpp:11147`). The lateral collector
    /// (`:10958-10976`) and the half-circle collector (`:10873-10890`) both
    /// overwrite the human-owned `mfInitialStrikeAngle`,
    /// `mfFinalStrikeAngle` and `mfCurrentStrikeAngle` before scanning, so an
    /// interrupted sweep's retained victim list is later tested against the
    /// *last estimated* lateral/half-circle candidate's geometry rather than
    /// against the geometry its own strike installed. The circle, straight
    /// and push collectors write nothing.
    pub(super) fn apply_strike_selection_sweep_rebase(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        rebase: Option<crate::combat::StrikeSelectionSweepRebase>,
    ) {
        let Some(rebase) = rebase else {
            return;
        };
        let Some(entity) = self.get_entity(attacker_id) else {
            return;
        };
        let Some(profile_idx) = get_hth_weapon_id_full(entity, &assets.profile_manager) else {
            return;
        };
        let profile = assets
            .profile_manager
            .get_hth_weapon(profile_idx)
            .unwrap_or_else(|| {
                panic!(
                    "strike-selection sweep rebase for {attacker_id:?} references missing weapon profile {profile_idx}"
                )
            });
        let thrust = &profile.thrusts[rebase.strike as usize];
        let dir_angle = sector_to_angle(entity.element_data().direction());
        let initial_angle = strike_profile_angle(thrust.initial_angle);
        let final_angle = strike_profile_angle(thrust.final_angle);
        use crate::profiles::WeaponThrustDirection;
        let (initial, final_a) = match thrust.kind {
            WeaponThrustKind::Lateral => match thrust.direction {
                WeaponThrustDirection::RightToLeft => {
                    (dir_angle + initial_angle, dir_angle - final_angle)
                }
                _ => (dir_angle - initial_angle, dir_angle + final_angle),
            },
            WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => {
                match thrust.direction {
                    WeaponThrustDirection::RightToLeft => {
                        let init = dir_angle + initial_angle;
                        (init, init - std::f32::consts::PI)
                    }
                    _ => {
                        let init = dir_angle - initial_angle;
                        (init, init + std::f32::consts::PI)
                    }
                }
            }
            _ => return,
        };
        let Some(entity) = self.get_entity_mut(attacker_id) else {
            return;
        };
        if let Some(human) = entity.human_data_mut() {
            human.sword_sweep.initial_angle = initial;
            human.sword_sweep.current_angle = dir_angle;
            human.sword_sweep.final_angle = final_a;
        }
        // The executable mirror shares the same storage in the Original, so
        // a live sweep must follow the rebase too.
        if let Some(actor) = entity.actor_data_mut()
            && let Some(sweep) = actor.sweep_state.as_mut()
        {
            sweep.initial_angle = initial;
            sweep.current_angle = dir_angle;
            sweep.final_angle = final_a;
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
        // RHSword's GetStrike*Angle getters evaluate their authored-degree
        // expression in double precision and narrow only at the FLOAT return.
        let initial_angle = strike_profile_angle(thrust.initial_angle);
        let final_angle = strike_profile_angle(thrust.final_angle);
        let rotation_per_frame = strike_profile_angle(thrust.rotation_angle);

        let attacker_dir = self
            .get_entity(attacker_id)
            .map(|e| e.element_data().direction())
            .unwrap_or(0);
        let dir_angle = sector_to_angle(attacker_dir);

        use crate::profiles::WeaponThrustDirection;

        let (initial, final_a, signed_rotation) = match strike_kind {
            WeaponThrustKind::Lateral => match direction {
                WeaponThrustDirection::RightToLeft => {
                    let init = dir_angle + initial_angle;
                    let fin = dir_angle - final_angle;
                    (init, fin, -rotation_per_frame)
                }
                _ => {
                    let init = dir_angle - initial_angle;
                    let fin = dir_angle + final_angle;
                    (init, fin, rotation_per_frame)
                }
            },
            WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => match direction
            {
                WeaponThrustDirection::RightToLeft => {
                    let init = dir_angle + initial_angle;
                    let fin = init - std::f32::consts::PI;
                    (init, fin, -rotation_per_frame)
                }
                _ => {
                    let init = dir_angle - initial_angle;
                    let fin = init + std::f32::consts::PI;
                    (init, fin, rotation_per_frame)
                }
            },
            WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => match direction {
                WeaponThrustDirection::RightToLeft => {
                    let init = dir_angle + initial_angle;
                    let fin = dir_angle - 2.0 * std::f32::consts::PI;
                    (init, fin, -rotation_per_frame)
                }
                _ => {
                    let init = dir_angle - initial_angle;
                    let fin = dir_angle + 2.0 * std::f32::consts::PI;
                    (init, fin, rotation_per_frame)
                }
            },
            _ => return, // not a sweep type
        };

        let num_victims = victims.len();
        if sword_damage_debug_enabled() {
            eprintln!(
                "[SWEEPINIT f={} attacker={:?} (co {}) strike={:?} kind={:?} victims={:?} init={} cur={} fin={} rot={}]",
                self.control.frame_counter,
                attacker_id,
                self.world.original_creation_order(attacker_id),
                strike,
                strike_kind,
                victims
                    .iter()
                    .map(|&v| self.world.original_creation_order(v))
                    .collect::<Vec<_>>(),
                initial,
                dir_angle,
                final_a,
                signed_rotation,
            );
        }
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

        if let Some(entity) = self.world.entities.get_mut(attacker_id) {
            if let Some(human) = entity.human_data_mut() {
                human.sword_sweep = crate::element::HumanSwordSweepState {
                    victims: sweep.pending_victims.clone(),
                    initial_angle: sweep.initial_angle,
                    current_angle: sweep.current_angle,
                    final_angle: sweep.final_angle,
                };
            }
            if let Some(actor) = entity.actor_data_mut() {
                actor.sweep_state = Some(sweep);
            }
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
        self.tick_sweep_for_mode(sim, assets, attacker_id, initialized_this_hourglass, false);
    }

    fn tick_sweep_for_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        initialized_this_hourglass: bool,
        effect_only_before_action_point: bool,
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
            if !effect_only_before_action_point
                && matches!(active.sweep.strike_kind, WeaponThrustKind::Lateral)
            {
                advance_lateral_angle(&mut active.sweep);
            }

            // Rotate the attacker's sprite direction to follow the
            // circle using the angle that existed on entry. Only the TRUE
            // variants rotate; FALSE variants do not.
            if !effect_only_before_action_point
                && matches!(
                    active.sweep.strike_kind,
                    crate::profiles::WeaponThrustKind::TrueCircle
                        | crate::profiles::WeaponThrustKind::TrueHalfCircle
                )
            {
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
            if sword_damage_debug_enabled() {
                eprintln!(
                    "[SWEEPTICK f={} attacker={:?} kind={:?} init_sec={} cur_sec={} cur_angle={} pending={}]",
                    self.control.frame_counter,
                    active.attacker_id,
                    active.sweep.strike_kind,
                    initial_sector,
                    current_sector,
                    active.sweep.current_angle,
                    active.sweep.pending_victims.len(),
                );
            }

            let mut hit_indices = Vec::new();

            // Victim eligibility is settled once, when the sweep seeds its
            // list.  The per-frame pass only asks whether the arc has reached
            // the victim's sector; a victim who dies, falls unconscious or
            // otherwise stops qualifying mid-sweep still takes the blow that
            // was already on its way.
            for (i, &victim_id) in active.sweep.pending_victims.iter().enumerate() {
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

            if circle && !effect_only_before_action_point {
                advance_circle_angle(&mut active.sweep);
            }
        }

        // Phase 3: write back updated sweep states
        for active in sweeps {
            if let Some(entity) = self.world.entities.get_mut(active.attacker_id) {
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
                let retain_executable =
                    !active.sweep.pending_victims.is_empty() || keep_for_terminal_execute;
                if let Some(human) = entity.human_data_mut() {
                    human.sword_sweep = crate::element::HumanSwordSweepState {
                        victims: active.sweep.pending_victims.clone(),
                        initial_angle: active.sweep.initial_angle,
                        current_angle: active.sweep.current_angle,
                        final_angle: active.sweep.final_angle,
                    };
                }
                if let Some(actor) = entity.actor_data_mut() {
                    actor.sweep_state = retain_executable.then_some(active.sweep);
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
    #[cfg(test)]
    pub(super) fn tick_push_flights(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_push_flights_at_owner(sim, assets, None, false, false);
    }

    /// Advance flight work at one actor's live creation slot. Original
    /// `ExecuteFallingHit` / `ExecuteFallingPushed` calls `PerformFlight`
    /// inline, so a later NPC's detection pass observes the resulting
    /// position while an earlier NPC cannot. The optional owner also keeps
    /// the broad helper available to focused tests without restoring the
    /// production-wide post-detection batch.
    pub(in crate::engine) fn tick_push_flight_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> Option<crate::sprite::MotionState> {
        if self.actors_frozen()
            || self
                .get_entity(owner)
                .and_then(Entity::actor_data)
                .is_some_and(|actor| actor.execution_frozen)
        {
            return None;
        }
        // PerformFlight owns its terminal goal snap before Execute returns.
        // The generic animation arm has already published the landing posture
        // by this point, so process a retained zero-frame combat flight here
        // rather than postponing its exact PositionGoal until the later
        // gameplay-systems reconciliation pass.
        self.tick_push_flights_at_owner(sim, assets, Some(owner), false, false)
            .then_some(crate::sprite::MotionState::Terminated)
    }

    fn tick_push_flight_terminal_landings(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_push_flights_at_owner(sim, assets, None, true, false);
    }

    fn tick_push_flights_at_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        selected_owner: Option<EntityId>,
        terminal_only: bool,
        skip_terminal: bool,
    ) -> bool {
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
        // Ladder/wall falls that reached their tick countdown this
        // frame; the post-loop pass applies the landing concussion,
        // lying posture, and fall-order retirement.
        let mut ladder_arrivals: Vec<EntityId> = Vec::new();

        for (entity_id, entity) in self.world.entities.actors_mut() {
            if selected_owner.is_some_and(|owner| owner != EntityId::from(entity_id)) {
                continue;
            }
            // Read flight state without holding a mutable borrow.
            let flight_info = entity.actor_data().and_then(|a| a.active_flight);

            let mut flight = match flight_info {
                Some(f) => f,
                None => continue,
            };
            if (terminal_only && flight.frames_remaining != 0)
                || (skip_terminal && flight.frames_remaining == 0)
            {
                continue;
            }

            // Ladder/wall falls only progress while the FallingLadderWall
            // order is live — the original drives this flight from that
            // order's Execute arm.  Before the order starts, hold the
            // flight; if the order retired early (its sprite ran out
            // before the countdown), the increment is never applied
            // again, so drop the flight and leave the actor in place.
            if flight.ladder_fall {
                let fall_order_live = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(entity_id)
                    .is_some_and(|(_, _, order)| {
                        order.order_type == crate::order::OrderType::FallingLadderWall
                    });
                if !fall_order_live {
                    if entity.element_data().posture == Posture::Flying {
                        entity.actor_data_mut().unwrap().active_flight = None;
                    }
                    continue;
                }
            }

            // `ReadyForTakeOff` is initialized by ExecuteFallingPushed /
            // ExecuteFallingHit, not by Translate*Damage. Rust stores the
            // computed flight eagerly, so hold it until the queued falling
            // order is live and its Execute has changed posture to Flying.
            // `RHSprite::PerformFlight` calls `UpdatePosition` even when
            // `PerformAction` reports Start, so that first Execute owns the
            // first displacement too.
            let falling_order_live = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .is_some_and(|(_, _, order)| is_falling_flight_order(order.order_type));
            let waiting_for_fall_start = flight.frames_remaining != 0
                && flight.antagonist.is_some()
                && !flight.ladder_fall
                && (!falling_order_live || entity.element_data().posture != Posture::Flying);
            if waiting_for_fall_start {
                continue;
            }

            let flight_capture_before =
                crate::movement_diagnostics::parity_movement_capture_active().then(|| {
                    let position = entity.position_iface().v48_serialized_state();
                    (
                        position.position,
                        position.map,
                        position.old_position,
                        position.old_map,
                        crate::coordinates::WorldPoint3D::new(
                            flight.goal_x,
                            flight.goal_y + flight.goal_z,
                            flight.goal_z,
                        ),
                        crate::coordinates::WorldVec3D::new(
                            flight.increment_x,
                            flight.increment_y,
                            flight.increment_z,
                        ),
                        entity.actor_data().and_then(|actor| actor.installed_order),
                    )
                });
            let mut raw_post = None;
            let mut snapped_to_goal = false;

            // RHSprite::PerformFlight clears its increment after PerformAction
            // reaches the final sprite frame, before ApplyDominoEffect and
            // UpdatePosition.  The flight countdown is only our surrogate for
            // that sprite-owned lifetime, so retire it at the same boundary.
            let sprite = entity.sprite();
            if flight.antagonist.is_some()
                && !flight.ladder_fall
                && perform_flight_stops_before_position_update(
                    sprite.frame_count,
                    sprite.current_frame,
                    sprite.num_frames_for_row(sprite.current_row),
                )
            {
                flight.frames_remaining = 0;
                flight.increment_x = 0.0;
                flight.increment_y = 0.0;
                flight.increment_z = 0.0;
                entity.actor_data_mut().unwrap().active_flight = Some(flight);
            }

            // Capture the domino-sweep request *before* clearing the
            // flight on the final frame.  An exact zero increment
            // skips frames where the sprite isn't actually moving.
            // ApplyDominoEffect reads GetIncrement().x/y: the literal world
            // flight vector, not the projected map-space delta (whose Y also
            // contains elevation). Flat flights make the two spaces coincide,
            // while elevated flights do not.
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
                    snapped_to_goal = true;
                }
            } else if flight.frames_remaining == 1 {
                // PerformFlight still adds its stored increment on the DONE
                // frame. It snaps to the exact PositionGoal only when the
                // sprite later reports TERMINATED.
                if flight.antagonist.is_some() {
                    if matches!(flight.geometry, crate::element::FlightGeometry::World3d) {
                        let pos3 = entity.position_iface().get_position();
                        entity.position_iface_mut().set_position(
                            crate::coordinates::WorldPoint3D {
                                x: pos3.x + flight.increment_x,
                                y: pos3.y + flight.increment_y,
                                z: pos3.z + flight.increment_z,
                            },
                        );
                    } else {
                        let mut map = entity.element_data().position_map();
                        map.x += flight.increment_x;
                        map.y += flight.increment_y;
                        let z = entity.position_iface().get_elevation() + flight.increment_z;
                        set_flight_position(entity, flight.geometry, map, z);
                    }
                } else {
                    // Non-combat translations have no wrapper termination
                    // event, so their final flight tick owns the exact snap.
                    set_flight_position(
                        entity,
                        flight.geometry,
                        crate::coordinates::MapPoint::new(flight.goal_x, flight.goal_y),
                        flight.goal_z,
                    );
                    snapped_to_goal = true;
                }
                if flight_capture_before.is_some() {
                    raw_post = Some((
                        entity.position_iface().get_position(),
                        entity.element_data().position_map(),
                    ));
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
                    if flight.ladder_fall {
                        // Settle the landing like the original's
                        // position recompute + fresh-move snapshot on
                        // arrival: old position tracks the snapped
                        // position, so the landing frame reports no
                        // residual movement.
                        entity.position_iface_mut().new_move();
                    }
                    let actor = entity.actor_data_mut().unwrap();
                    actor.active_flight = None;
                    if flight.ladder_fall {
                        // The countdown just hit zero. Original stores the
                        // ladder countdown and seek-refresh countdown in the
                        // same `mulWaitTime` scalar, so landing overwrites the
                        // dormant seek mirror too. Keep the retained seek
                        // target/continuation themselves intact: only their
                        // split Rust countdown copy has become stale.
                        actor.wait_time = 0;
                        actor.seek_refresh_wait = 0;
                        ladder_arrivals.push(entity_id.into());
                    }
                }
            } else {
                // Advance by increment.  The per-frame increment in
                // 3D is `(goal - position) / frames_of_flight`, so
                // the z advance is linear from start_z to goal_z.
                if matches!(flight.geometry, crate::element::FlightGeometry::World3d) {
                    // ReadyForTakeOff stores and accumulates one complete 3D
                    // increment. Re-derive map position after the addition;
                    // separately accumulating map Y and Z rounds differently.
                    let pos3 = entity.position_iface().get_position();
                    entity
                        .position_iface_mut()
                        .set_position(crate::coordinates::WorldPoint3D {
                            x: pos3.x + flight.increment_x,
                            y: pos3.y + flight.increment_y,
                            z: pos3.z + flight.increment_z,
                        });
                } else {
                    let mut m = entity.element_data().position_map();
                    m.x += flight.increment_x;
                    m.y += flight.increment_y;
                    let z = entity.position_iface().get_elevation() + flight.increment_z;
                    set_flight_position(entity, flight.geometry, m, z);
                }
                let actor = entity.actor_data_mut().unwrap();
                let active = actor.active_flight.as_mut().unwrap();
                active.frames_remaining -= 1;
                let remaining = active.frames_remaining;
                if flight.ladder_fall {
                    // Mirror the original's per-tick fall countdown,
                    // which lives in the actor's wait-time counter.
                    actor.wait_time = u32::from(remaining);
                }
                if flight_capture_before.is_some() {
                    raw_post = Some((
                        entity.position_iface().get_position(),
                        entity.element_data().position_map(),
                    ));
                }
            }

            if let Some((
                entry_position,
                entry_position_map,
                old_position,
                old_position_map,
                goal,
                cached_increment,
                installed_order,
            )) = flight_capture_before
            {
                let (raw_post_position, raw_post_position_map) =
                    raw_post.unwrap_or((entry_position, entry_position_map));
                let post_position = entity.position_iface().get_position();
                let post_position_map = entity.element_data().position_map();
                let frames_remaining_after = entity
                    .actor_data()
                    .and_then(|actor| actor.active_flight)
                    .map(|active| active.frames_remaining);
                crate::movement_diagnostics::record_parity_flight_step(
                    crate::movement_diagnostics::ParityFlightStep {
                        entity: entity_id.into(),
                        phase: if selected_owner.is_some() {
                            "owner".to_owned()
                        } else {
                            "terminal_cleanup".to_owned()
                        },
                        geometry: format!("{:?}", flight.geometry),
                        order_id: installed_order.map(|order| order.order_id.get()),
                        order_type: installed_order.map(|order| format!("{:?}", order.order_type)),
                        frames_remaining_before: flight.frames_remaining,
                        frames_remaining_after,
                        entry_position: entry_position.into(),
                        entry_position_map: entry_position_map.into(),
                        old_position: old_position.into(),
                        old_position_map: old_position_map.into(),
                        goal: goal.into(),
                        cached_increment: cached_increment.into(),
                        applied_increment: if flight.frames_remaining == 0 {
                            crate::coordinates::WorldVec3D::ZERO.into()
                        } else {
                            cached_increment.into()
                        },
                        raw_post_position: raw_post_position.into(),
                        raw_post_position_map: raw_post_position_map.into(),
                        motion_state: if snapped_to_goal {
                            "Terminated".to_owned()
                        } else {
                            "InProgress".to_owned()
                        },
                        post_position: post_position.into(),
                        post_position_map: post_position_map.into(),
                        snapped_to_goal,
                    },
                );
            }
        }

        // Landing-resolution second pass: apply a goal obstacle that was not
        // already installed. Original ReadyForTakeOff calls SetObstacle (not
        // SetObstacleAndMaterial) before the flight and terminal PerformFlight
        // does not refresh the footstep material. Preserve both that material
        // and the authored 3D goal when the obstacle is already current.
        // Reinstalling the same plane would also reproject Z from rounded
        // map-space Y and can move a sloped landing by several ULPs (seed-2M
        // Savegame_023/replay-003).
        for &(flyer_id, obstacle) in &landings {
            let current_obstacle = self
                .get_entity(flyer_id)
                .and_then(|entity| entity.element_data().obstacle_index())
                .map(|handle| handle.get());
            if current_obstacle != obstacle {
                self.set_obstacle_and_material(assets, flyer_id, obstacle);
            }
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

        // Ladder/wall fall landings: knock the faller about the head,
        // put them on their back, and retire the fall order — the
        // original does all of this inside the fall Execute arm on the
        // tick its countdown reaches zero.
        let ladder_arrived = !ladder_arrivals.is_empty();
        for victim_id in ladder_arrivals {
            let (concussion, life_points) = {
                let entity = self
                    .get_entity(victim_id)
                    .expect("ladder-fall arrival entity vanished before landing effects");
                let concussion = entity
                    .human_data()
                    .map(|h| h.concussion_of_the_brain)
                    .unwrap_or(0);
                let life_points = match entity {
                    Entity::Pc(pc) => pc.pc.life_points,
                    Entity::Soldier(soldier) => soldier.npc.life_points,
                    Entity::Civilian(civilian) => civilian.npc.life_points,
                    _ => 0,
                };
                (concussion, life_points)
            };
            let new_value = crate::combat::compute_concussion_effect(concussion, 71, life_points);
            let concussion_outcome =
                self.apply_concussion(sim, assets, victim_id, new_value, false);
            if concussion_outcome == crate::combat::ConcussionOutcome::WentUnconscious {
                // Original FallingLadderWall::Execute calls
                // AddConcussionOfTheBrain here. Its threshold transition
                // synchronously closes QuitSwordFight (including reciprocal
                // DeleteOpponent / EVENT_QUIT_SWORDFIGHT), then NPC
                // SetConcussionOfTheBrain synchronously calls
                // Think(EVENT_LOSE_CONSCIOUSNESS). That StartThink arm calls
                // SetViewStatus(EYES_DIE_OR_GET_UNCONSCIOUS) before Execute
                // sets the landing posture and returns Terminated
                // (`original-code/RHelementactorhuman.cpp` and
                // `RHartificialintelligence.cpp`). The ordinary per-frame
                // concussion drain has already run by this actor slot, so
                // close every part of this newly-created knockout boundary
                // now. Drain the existing FIFO as well: earlier synchronous
                // calls represented by Rust's borrow-boundary queue precede
                // the just-appended lose-consciousness event.
                self.drain_pending_concussion_side_effects(sim, assets);
                if matches!(victim_id, EntityId::Soldier(_) | EntityId::Civilian(_)) {
                    self.tick_enemy_ai_drain_pending_stimuli_for_npc(
                        sim, victim_id, assets, None, None,
                    );
                    self.tick_ai_pending_resurrection_and_eyes_for_npc(victim_id);
                }
            }

            if let Some(entity) = self.get_entity_mut(victim_id) {
                let posture = if entity.is_dead() {
                    Posture::DeadBack
                } else {
                    Posture::Lying
                };
                entity.set_posture(posture);
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = ActionState::Waiting;
                }
            }

            if let Some((seq_id, elem_idx, order)) = self
                .orders
                .sequence_manager
                .current_order_for_actor(victim_id)
                && order.order_type == crate::order::OrderType::FallingLadderWall
            {
                // The fall's Execute arm reports Terminated on the
                // arrival tick; a synchronously exposed successor
                // order downgrades that to InProgress like the
                // original's next-order handling.
                if let Some(actor) = self
                    .get_entity_mut(victim_id)
                    .and_then(crate::element::Entity::actor_data_mut)
                {
                    actor.continuation.motion_state = crate::sprite::MotionState::Terminated;
                }
                self.do_next_order(seq_id, elem_idx);
                if self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(victim_id)
                    .is_some()
                    && let Some(actor) = self
                        .get_entity_mut(victim_id)
                        .and_then(crate::element::Entity::actor_data_mut)
                {
                    actor.continuation.motion_state = crate::sprite::MotionState::InProgress;
                }
            }
        }

        for (flyer_id, hitter_id, inc_x, inc_y) in domino_sweeps {
            self.apply_domino_effect(flyer_id, hitter_id, inc_x, inc_y);
        }

        ladder_arrived
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
        let (flyer_pos_ground, flyer_sector) = match self.get_entity(flyer_id) {
            Some(e) => {
                let elem = e.element_data();
                let position = elem.position();
                ((position.x, position.y), elem.sector())
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

            // GetPositionGround is literal world Position X/Y. It is not the
            // projected PositionMap (whose Y is world Y minus elevation).
            let candidate_position = elem.position();
            let dx = candidate_position.x - flyer_pos_ground.0;
            let dy = candidate_position.y - flyer_pos_ground.1;

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
    /// If the new obstacle isn't steep enough to roll, or the recomputed roll
    /// direction opposes the current increment, the map goal becomes the
    /// current position. Otherwise Original rewrites the live order, calls
    /// `NewID`, and publishes the new map goal.
    ///
    /// Early-outs if the entity is not currently in a Rolling combat
    /// animation.
    pub(crate) fn update_roll_after_crossing(&mut self, assets: &LevelAssets, entity_id: EntityId) {
        // Cheap early-out: only act while the actor is rolling.
        let Some((roll_seq_id, roll_elem_idx, roll_command)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .and_then(|(seq_id, elem_idx, order)| {
                (order.order_type == OrderType::Rolling).then(|| {
                    let command = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .expect("current rolling element disappeared")
                        .command;
                    (seq_id, elem_idx, command)
                })
            })
        else {
            return;
        };

        // RHCOMMAND_ROLL owns its authored destination. Original UpdateRoll
        // returns before inspecting the crossed obstacle for that command.
        if roll_command == Command::Roll {
            return;
        }

        // Recompute using the new obstacle's normal.
        let normal = self.get_roll_normal(assets, entity_id);
        let new_dest = normal.and_then(|n| self.find_roll_point(entity_id, n, true));

        if let Some(dest) = new_dest {
            let fresh_id = self.orders.allocate_order_id();
            let current = self
                .orders
                .sequence_manager
                .get_element_mut(roll_seq_id, roll_elem_idx)
                .and_then(|element| element.orders.front_mut())
                .expect("rolling order disappeared before UpdateRoll rewrite");
            current.target_x = dest.x;
            current.target_y = dest.y;
            current.order_id = fresh_id;
            self.world.entities[entity_id]
                .as_mut()
                .expect("rolling actor disappeared before UpdateRoll publication")
                .actor_data_mut()
                .expect("Rolling owner must have actor data")
                .installed_order = Some(crate::element::InstalledActorOrder {
                order_id: fresh_id,
                order_type: OrderType::Rolling,
            });

            let entity = self.world.entities[entity_id]
                .as_mut()
                .expect("rolling actor disappeared before goal publication");
            entity.position_iface_mut().set_map_goal(dest);
        } else if let Some(entity) = self.world.entities.get_mut(entity_id) {
            let here = entity.element_data().position_map();
            stop_roll_at_current_position(entity.position_iface_mut(), here);
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
        // Hold forbids ordinary damaging strikes. Also discard an AI proposal
        // for a selected ally while any player/smalltalk strike is already
        // live: the explicit action owns this combat turn and must not leave a
        // delayed AI strike queued behind it. The flag is still consumed below
        // so the rejected proposal cannot leak out after a stance change or
        // after the player action completes.
        let mut suppressed_considerations: std::collections::HashSet<EntityId> = self
            .players
            .allied
            .orders
            .keys()
            .copied()
            .filter(|&id| !self.allied_allows_normal_strikes(id))
            .collect();
        for seat in &self.players.allied.seats {
            for &id in &seat.selection {
                if self
                    .orders
                    .sequence_manager
                    .has_live_element_for_actor_matching(id, |command| {
                        command.is_swordstrike()
                            || matches!(
                                command,
                                Command::SwordstrikeSmalltalkLeft
                                    | Command::SwordstrikeSmalltalkRight
                                    | Command::ParrySmalltalkLeft
                                    | Command::ParrySmalltalkRight
                            )
                    })
                {
                    suppressed_considerations.insert(id);
                }
            }
        }

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
                let pending = std::mem::take(&mut ai.pending_sword_strike_consideration);
                (pending && !suppressed_considerations.contains(&EntityId::from(npc_id)))
                    .then_some(EntityId::from(npc_id))
            })
            .collect();
        for &owner in &pending_considerations {
            if special_strike_lifecycle_debug_matches(current_frame, owner.index()) {
                let ai = self
                    .world
                    .entities
                    .get(owner)
                    .and_then(|entity| entity.npc_data())
                    .and_then(|npc| npc.ai_brain.enemy())
                    .unwrap_or_else(|| panic!("special-strike owner {owner:?} lost Enemy AI"));
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=authorization_consumed pending_consideration={} pending_special={} state={:?} substate={:?} selected={}",
                    current_frame,
                    owner.index(),
                    ai.pending_sword_strike_consideration,
                    ai.pending_special_strike,
                    ai.base.current_state,
                    ai.base.current_substate,
                    special_strike_selected_snapshot(&self.orders.sequence_manager, owner),
                );
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
            attacker_pos: (f32, f32),
            attacker_elevation: f32,
            is_swordfighting: bool,
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
                is_swordfighting: !soldier.human.opponents.is_empty(),
                boredom: soldier.human.sword_strike_boredom.clone(),
            });
        }

        // Process attacks — launch SwordstrikeThrust* sequence
        // elements as Interaction(1, command, this,
        // principal_opponent).
        for mut attack in attacks {
            let special_debug =
                special_strike_lifecycle_debug_matches(current_frame, attack.soldier_id.index());
            if special_debug {
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=before_proposal target={} selected={}",
                    current_frame,
                    attack.soldier_id.index(),
                    attack.target_id.index(),
                    special_strike_selected_snapshot(
                        &self.orders.sequence_manager,
                        attack.soldier_id
                    ),
                );
            }
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
            let sprite_timing_debug = opponent_sprite_timing_debug_matches(
                current_frame,
                attack.soldier_id.index(),
                attack.target_id.index(),
            );
            let sprite_timing_creation_orders = sprite_timing_debug.then(|| {
                (
                    self.world.original_creation_order(attack.soldier_id),
                    self.world.original_creation_order(attack.target_id),
                )
            });
            let selected_opponent_time_limit = self
                .opponent_sword_strike_time_limit_for_actor(attack.soldier_id, attack.target_id);
            let opponent_time_limit: Option<i16> =
                self.get_entity(attack.target_id).and_then(|e| {
                    let animation = self.live_actor_animation(attack.target_id)?;
                    let sprite = &e.element_data().sprite;
                    if let Some((owner_creation_order, target_creation_order)) =
                        sprite_timing_creation_orders
                    {
                        let script_len = sprite
                            .current_scripts_opt()
                            .and_then(|scripts| scripts.get(sprite.current_row as usize))
                            .map(|script| script.frame_ids.len());
                        eprintln!(
                            "OPPONENT_SPRITE_TIMING frame={} owner={} owner_co={} target={} target_co={} caller={:?} animation={animation:?} profile={:?} primary={:?} alternate={:?} use_alternate={} row={} sprite_frame={} frame_count={} action_done_frame={} action_done_counter={} script_len={script_len:?}",
                            current_frame,
                            attack.soldier_id.index(),
                            owner_creation_order,
                            attack.target_id.index(),
                            target_creation_order,
                            only_owner.map(EntityId::index),
                            sprite.frame_profile_name,
                            sprite.profile_cache_key,
                            sprite.alternate_profile_cache_key,
                            sprite.use_alternate_profile,
                            sprite.current_row,
                            sprite.current_frame,
                            sprite.frame_count,
                            sprite.action_done_frame,
                            sprite.action_done_counter,
                        );
                    }
                    selected_opponent_time_limit
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
                    let lp = get_life_points(e);
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
                        camp: e.camp(),
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
                is_swordfighting: attack.is_swordfighting,
                opponent_time_limit,
                strike_startup_frames: attacker_sprite_frames,
                parry_startup_frames: parry_startup,
                is_npc: true,
            };
            let debug = super::evaluate::reactive_sword_debug_frame_matches(current_frame)
                .then(|| {
                    let creation_order = self.world.original_creation_order(attack.soldier_id);
                    super::evaluate::reactive_sword_debug_creation_order_matches(creation_order)
                        .then_some(crate::combat::SwordStrikeProposalDebug {
                            frame: current_frame,
                            victim: attack.soldier_id.index(),
                            victim_creation_order: creation_order,
                            attacker: attack.target_id.index(),
                        })
                })
                .flatten();
            if let Some(debug) = debug {
                let target_animation = self.live_actor_animation(attack.target_id);
                let target_raw_frames = self.get_entity(attack.target_id).map(|target| {
                    target
                        .element_data()
                        .sprite
                        .frames_from_now_till_action_done()
                });
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=enemy_principal target={} animation={:?} raw_frames_from_now={:?} time_limit={:?}]",
                    debug.frame,
                    debug.victim_creation_order,
                    debug.victim,
                    debug.attacker,
                    attack.target_id.index(),
                    target_animation,
                    target_raw_frames,
                    opponent_time_limit,
                );
            }
            let rng_before = debug.and_then(|_| self.control.rng.original_replay_cursor());
            let mut sweep_rebase = None;
            let proposed = crate::combat::propose_good_sword_strike_with_debug(
                sim,
                &ctx,
                &nearby,
                &mut attack.boredom,
                false,
                debug,
                &mut sweep_rebase,
            );
            self.apply_strike_selection_sweep_rebase(assets, attack.soldier_id, sweep_rebase);
            if special_debug {
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=after_proposal result={:?} selected={}",
                    current_frame,
                    attack.soldier_id.index(),
                    proposed,
                    special_strike_selected_snapshot(
                        &self.orders.sequence_manager,
                        attack.soldier_id
                    ),
                );
            }
            if let Some(debug) = debug {
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=proposal_boundary caller=enemy_reconsider rng_before={:?} rng_after={:?} result={:?}]",
                    debug.frame,
                    debug.victim_creation_order,
                    debug.victim,
                    debug.attacker,
                    rng_before,
                    self.control.rng.original_replay_cursor(),
                    proposed,
                );
            }
            let strike = match proposed {
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

            if special_debug {
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=before_begin strike={:?} command={:?} wait_time={} selected={}",
                    current_frame,
                    attack.soldier_id.index(),
                    strike,
                    command,
                    wait_time,
                    special_strike_selected_snapshot(
                        &self.orders.sequence_manager,
                        attack.soldier_id
                    ),
                );
            }

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
            if special_debug {
                let ai = self
                    .world
                    .entities
                    .get(attack.soldier_id)
                    .and_then(|entity| entity.npc_data())
                    .and_then(|npc| npc.ai_brain.enemy())
                    .expect("special-strike owner lost Enemy AI after begin");
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=after_begin pending_special={} state={:?} substate={:?} selected={}",
                    current_frame,
                    attack.soldier_id.index(),
                    ai.pending_special_strike,
                    ai.base.current_state,
                    ai.base.current_substate,
                    special_strike_selected_snapshot(
                        &self.orders.sequence_manager,
                        attack.soldier_id
                    ),
                );
            }
            self.drain_ai_owner_work_for(sim, assets, attack.soldier_id);
            self.apply_pending_ai_halt(attack.soldier_id);
            self.dispatch_condolations_for_owner_boundary(sim, attack.soldier_id, assets);
            if special_debug {
                let ai = self
                    .world
                    .entities
                    .get(attack.soldier_id)
                    .and_then(|entity| entity.npc_data())
                    .and_then(|npc| npc.ai_brain.enemy())
                    .expect("special-strike owner lost Enemy AI after drain");
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=after_begin_drain pending_special={} state={:?} substate={:?} selected={}",
                    current_frame,
                    attack.soldier_id.index(),
                    ai.pending_special_strike,
                    ai.base.current_state,
                    ai.base.current_substate,
                    special_strike_selected_snapshot(
                        &self.orders.sequence_manager,
                        attack.soldier_id
                    ),
                );
            }

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

            if special_debug {
                let ai = self
                    .world
                    .entities
                    .get(attack.soldier_id)
                    .and_then(|entity| entity.npc_data())
                    .and_then(|npc| npc.ai_brain.enemy())
                    .expect("special-strike owner lost Enemy AI after launch");
                eprintln!(
                    "SPECIAL_STRIKE frame={} owner={} phase=after_launch pending_special={} state={:?} substate={:?} selected={}",
                    current_frame,
                    attack.soldier_id.index(),
                    ai.pending_special_strike,
                    ai.base.current_state,
                    ai.base.current_substate,
                    special_strike_selected_snapshot(
                        &self.orders.sequence_manager,
                        attack.soldier_id
                    ),
                );
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
        let is_sherwood = self.is_sherwood(&assets.profile_manager);
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
                is_sherwood,
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
        let is_sherwood = self.is_sherwood(&assets.profile_manager);
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
                    is_sherwood,
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
        if dispatched_wake && matches!(owner, EntityId::Soldier(_) | EntityId::Civilian(_)) {
            // EVENT_FITAGAIN's resurrection fan-out and eye reset are inline
            // consequences of Think in Original, including under FrozenAll.
            self.tick_ai_pending_resurrection_and_eyes_for_npc(owner);
            self.apply_wake_redetection_blinks(owner);
        }
    }
}

#[inline]
fn perform_flight_stops_before_position_update(
    frame_count: u16,
    current_frame: u16,
    row_frames: u16,
) -> bool {
    frame_count == 0 && current_frame + 1 == row_frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::{MapPoint, MapVec, WorldPoint3D};
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, Camp, ElementData, ElementKind, HumanData, NpcData,
        PcData, SoldierData,
    };
    use crate::position_interface::SectorHandle;
    use crate::sequence::SequenceElement;

    #[test]
    fn perform_flight_stops_on_first_tick_of_final_sprite_frame() {
        assert!(perform_flight_stops_before_position_update(0, 6, 7));
        assert!(!perform_flight_stops_before_position_update(1, 6, 7));
        assert!(!perform_flight_stops_before_position_update(0, 5, 7));
    }

    #[test]
    fn stopped_rolling_preserves_direction_through_shared_recompute() {
        let here = MapPoint::new(1208.699_5, 1156.473_4);
        let mut position = crate::position_interface::PositionInterface::new();
        position.set_map_position(here);
        position.set_direction_instantly(crate::position_interface::Direction::from_raw(10));
        position.set_map_increment(MapVec::new(0.274_060_93, 0.961_712_3));

        stop_roll_at_current_position(&mut position, here);
        position.compute_increment_all(true);

        assert_eq!(position.map_goal(), here);
        assert_eq!(position.get_increment_map(), MapVec::ZERO);
        assert_eq!(position.get_direction_goal().as_u8(), 10);
    }

    fn falling_pushed_soldier(dead: bool) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Flying,
            ..ElementData::default()
        };
        // `PerformFlight` reads the live falling animation's frame count on
        // every combat-flight tick. Production actors are sprite-hydrated;
        // keep this synthetic actor subject to that same invariant.
        element.sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            frame_ids: vec![0, 1],
            ..Default::default()
        }]);
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

    fn install_falling_pushed_order(engine: &mut EngineInner, victim: EntityId) {
        let damage =
            SequenceElement::new_damage(1, Command::ReceiveSwordDamage, Some(victim), None, 20, 0);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let order_id =
            engine.push_new_order(sequence, 0, OrderType::FallingPushedUpright, 0.0, 0.0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(crate::element::InstalledActorOrder {
            order_id,
            order_type: OrderType::FallingPushedUpright,
        });
    }

    fn falling_ladder_pc(life_points: i16) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
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
        actor.action_state = ActionState::Moving;
        actor.active_flight = Some(crate::element::ActiveFlight {
            increment_x: 5.0,
            goal_x: 15.0,
            goal_y: 20.0,
            frames_remaining: 1,
            ladder_fall: true,
            goal_layer: 3,
            goal_sector: SectorHandle::new(4),
            ..Default::default()
        });
        actor.continuation.motion_state = crate::sprite::MotionState::Start;

        Entity::Pc(ActorPc {
            element,
            actor,
            human: HumanData::default(),
            pc: PcData {
                life_points,
                profile_index: crate::profiles::CharacterProfileIdx(0),
                ..PcData::default()
            },
        })
    }

    fn install_falling_ladder_order(engine: &mut EngineInner, victim: EntityId) {
        let damage =
            SequenceElement::new_damage(1, Command::ReceiveSwordDamage, Some(victim), None, 20, 0);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let order_id = engine.push_new_order(sequence, 0, OrderType::FallingLadderWall, 0.0, 0.0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(crate::element::InstalledActorOrder {
            order_id,
            order_type: OrderType::FallingLadderWall,
        });
    }

    #[test]
    fn ladder_arrival_publishes_zero_wait_without_dropping_dormant_seek() {
        let sim = crate::sim_rng::test_context();
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .characters
            .push(crate::profiles::CharacterProfile::default());
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::default()
        };
        let mut engine = EngineInner::new();
        let victim = engine.add_entity(falling_ladder_pc(200));
        {
            let actor = engine
                .get_entity_mut(victim)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.wait_time = 1;
            actor.seek_refresh_wait = 24;
            actor.seek_target = Some(victim);
            actor.post_seek_sequence = Some(crate::sequence::Sequence::new().into_post_seek());
        }
        install_falling_ladder_order(&mut engine, victim);

        engine.tick_push_flight_for_owner(&sim, &assets, victim);

        let actor = engine.get_entity(victim).unwrap().actor_data().unwrap();
        assert_eq!(actor.wait_time, 0);
        assert_eq!(actor.seek_refresh_wait, 0);
        assert_eq!(engine.actor_legacy_wait_time(victim), 0);
        assert_eq!(actor.seek_target, Some(victim));
        assert!(actor.post_seek_sequence.is_some());
    }

    #[test]
    fn fatal_push_goal_preserves_flying_pose_until_animation_terminates() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(falling_pushed_soldier(true));
        install_falling_pushed_order(&mut engine, victim_id);

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
    fn completed_combat_flight_snaps_after_its_falling_order_retires() {
        let sim = crate::sim_rng::test_context();
        let near_goal = MapPoint::new(1142.2267, 1230.4998);
        let exact_goal = MapPoint::new(1142.2262, 1230.5006);
        let mut entity = falling_pushed_soldier(false);
        entity.set_posture(Posture::Lying);
        entity
            .element_data_mut()
            .set_material(crate::element::GameMaterial::Grass);
        entity.element_data_mut().set_position_map(near_goal);
        entity.position_iface_mut().new_move();
        let flight = entity
            .actor_data_mut()
            .unwrap()
            .active_flight
            .as_mut()
            .unwrap();
        flight.frames_remaining = 0;
        flight.increment_x = 0.0;
        flight.increment_y = 0.0;
        flight.goal_x = exact_goal.x;
        flight.goal_y = exact_goal.y;
        flight.goal_z = 0.0;

        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(entity);

        // Actor::Hourglass has already retired the falling order and changed
        // posture. The later terminal PerformFlight reconciliation must not
        // mistake this completed flight for one that has yet to start.
        engine.tick_push_flight_terminal_landings(&sim, &LevelAssets::default());

        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().position_map(), exact_goal);
        assert_eq!(victim.position_iface().old_map_position(), near_goal);
        assert!(victim.position_iface().is_moving_map());
        assert_eq!(
            victim.element_data().material(),
            crate::element::GameMaterial::Grass,
            "terminal PerformFlight preserves the material held through flight"
        );
        assert!(victim.actor_data().unwrap().active_flight.is_none());
    }

    #[test]
    fn completed_combat_flight_preserves_authored_z_and_material_on_an_installed_slope() {
        let sim = crate::sim_rng::test_context();
        let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(1);
        obstacle.top_plane_points = [
            [0.0, 0.0, 1711.937_4],
            [1.0, 0.0, 1712.386_2],
            [0.0, 1.0, 1710.774_0],
        ];
        obstacle.material = 3;
        let plane =
            crate::position_interface::PlaneZCoeffs::from_plane_points(&obstacle.top_plane_points);
        let mut assets = LevelAssets::default();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

        // Adding world Z to map Y and subtracting it again rounds the map
        // coordinate. Reinstalling the already-current plane at landing would
        // consequently derive a different elevation from that rounded map Y.
        let goal_map = MapPoint::new(1833.458_984_375, 1617.051_635_742_187_5);
        let goal_z = f32::from_bits(1_133_974_934);
        let rounded_map_y = (goal_map.y + goal_z) - goal_z;
        assert_ne!(rounded_map_y.to_bits(), goal_map.y.to_bits());
        assert_ne!(
            plane.compute_z(goal_map.x, rounded_map_y).to_bits(),
            goal_z.to_bits()
        );

        let mut entity = falling_pushed_soldier(false);
        entity.set_posture(Posture::Lying);
        entity
            .element_data_mut()
            .set_material(crate::element::GameMaterial::Leaves);
        entity.element_data_mut().set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(0),
            Some(plane),
        );
        entity.position_iface_mut().set_position(WorldPoint3D::new(
            goal_map.x - 1.0,
            goal_map.y + goal_z,
            goal_z,
        ));
        let flight = entity
            .actor_data_mut()
            .unwrap()
            .active_flight
            .as_mut()
            .unwrap();
        flight.geometry = crate::element::FlightGeometry::World3d;
        flight.frames_remaining = 0;
        flight.increment_x = 0.0;
        flight.increment_y = 0.0;
        flight.increment_z = 0.0;
        flight.goal_x = goal_map.x;
        flight.goal_y = goal_map.y;
        flight.goal_z = goal_z;
        flight.obstacle = crate::position_interface::ObstacleHandle::new(0);

        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(entity);
        engine.tick_push_flight_terminal_landings(&sim, &assets);

        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(
            victim.position_iface().get_elevation().to_bits(),
            goal_z.to_bits()
        );
        assert_eq!(
            victim.element_data().obstacle_index(),
            crate::position_interface::ObstacleHandle::new(0)
        );
        assert_eq!(
            victim.element_data().material(),
            crate::element::GameMaterial::Leaves,
            "ReadyForTakeOff installs the goal obstacle without changing material"
        );
    }

    #[test]
    fn completed_combat_flight_snaps_at_its_owner_boundary() {
        let sim = crate::sim_rng::test_context();
        let near_x = 696.702_45_f32;
        let exact_x = f32::from_bits(near_x.to_bits() + 1);
        let near_goal = MapPoint::new(near_x, 2077.693_8);
        let exact_goal = MapPoint::new(exact_x, 2077.694_6);
        let mut entity = falling_pushed_soldier(false);
        entity.set_posture(Posture::Lying);
        entity.element_data_mut().set_position_map(near_goal);
        entity.position_iface_mut().new_move();
        let flight = entity
            .actor_data_mut()
            .unwrap()
            .active_flight
            .as_mut()
            .unwrap();
        flight.frames_remaining = 0;
        flight.increment_x = 0.0;
        flight.increment_y = 0.0;
        flight.goal_x = exact_goal.x;
        flight.goal_y = exact_goal.y;
        flight.goal_z = 0.0;

        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(entity);

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_push_flight_for_owner(&sim, &LevelAssets::default(), victim_id);
        let flights = crate::movement_diagnostics::take_parity_flight_capture();
        let _ = crate::movement_diagnostics::take_parity_movement_capture();

        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().position_map(), exact_goal);
        assert_eq!(victim.position_iface().old_map_position(), near_goal);
        assert!(victim.position_iface().is_moving_map());
        assert!(victim.actor_data().unwrap().active_flight.is_none());
        assert_eq!(flights.len(), 1);
        let flight = &flights[0];
        assert_eq!(flight.entity, victim_id);
        assert_eq!(flight.phase, "owner");
        assert_eq!(flight.frames_remaining_before, 0);
        assert_eq!(flight.frames_remaining_after, None);
        assert_eq!(flight.raw_post_position_map.x.bits, near_x.to_bits());
        assert_eq!(flight.post_position_map.x.bits, exact_x.to_bits());
        assert_eq!(flight.goal.x.bits, exact_x.to_bits());
        assert!(flight.snapped_to_goal);
    }

    #[test]
    fn owner_scoped_push_flight_advances_only_the_selected_creation_slot() {
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let mut engine = EngineInner::new();
        let earlier = engine.add_entity(falling_pushed_soldier(false));
        let later = engine.add_entity(falling_pushed_soldier(false));
        install_falling_pushed_order(&mut engine, earlier);
        install_falling_pushed_order(&mut engine, later);

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.tick_push_flight_for_owner(&sim, &assets, earlier);

        assert_eq!(
            engine
                .get_entity(earlier)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(15.0, 20.0)
        );
        assert_eq!(
            engine
                .get_entity(later)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(10.0, 20.0),
            "a later actor must retain its pre-Hourglass position"
        );

        engine.tick_push_flight_for_owner(&sim, &assets, later);
        assert_eq!(
            engine
                .get_entity(later)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(15.0, 20.0)
        );
        let flights = crate::movement_diagnostics::take_parity_flight_capture();
        let _ = crate::movement_diagnostics::take_parity_movement_capture();
        assert_eq!(flights.len(), 2);
        assert_eq!(flights[0].entity, earlier);
        assert_eq!(flights[1].entity, later);
        assert!(!flights[0].snapped_to_goal);
        assert_eq!(
            flights[0].raw_post_position_map.x.bits,
            flights[0].post_position_map.x.bits
        );
        assert!(
            crate::movement_diagnostics::take_parity_flight_capture().is_empty(),
            "taking one frame's flight diagnostics must isolate the next frame"
        );
    }

    #[test]
    fn ladder_arrival_returns_terminated_from_owner_execute_tail() {
        use crate::element::Command;
        use crate::order::OrderType;
        use crate::sequence::SequenceElement;

        let sim = crate::sim_rng::test_context();
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        profiles.hth_weapons.push(Default::default());
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::default()
        };
        let mut entity = falling_pushed_soldier(false);
        let Entity::Soldier(soldier) = &mut entity else {
            unreachable!()
        };
        let mut enemy_ai = crate::ai_enemy::EnemyAi::default();
        enemy_ai.hth_weapon_id = 1;
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(enemy_ai));
        entity
            .position_iface_mut()
            .set_layer_goal(crate::position_interface::Layer::ZERO);
        let actor = entity.actor_data_mut().unwrap();
        let flight = actor.active_flight.as_mut().unwrap();
        flight.antagonist = None;
        flight.ladder_fall = true;
        actor.continuation.motion_state = crate::sprite::MotionState::Start;

        let mut engine = EngineInner::new();
        let victim = engine.add_entity(entity);
        let damage =
            SequenceElement::new_damage(1, Command::ReceiveArrowDamage, Some(victim), None, 20, 0);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let order_id = engine.push_new_order(sequence, 0, OrderType::FallingLadderWall, 0.0, 0.0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(crate::element::InstalledActorOrder {
            order_id,
            order_type: OrderType::FallingLadderWall,
        });

        let motion = engine.tick_push_flight_for_owner(&sim, &assets, victim);

        assert_eq!(motion, Some(crate::sprite::MotionState::Terminated));
        let actor = engine.get_entity(victim).unwrap().actor_data().unwrap();
        assert_eq!(
            actor.continuation.motion_state,
            crate::sprite::MotionState::Terminated
        );
        assert_eq!(actor.installed_order, None);
        let entity = engine.get_entity(victim).unwrap();
        assert_eq!(entity.element_data().layer(), 3);
        assert_eq!(entity.element_data().sector(), SectorHandle::new(4));
        assert_eq!(
            entity.npc_data().unwrap().eye_status,
            crate::element::EyeStatus::DieOrGetUnconscious,
            "the ladder landing's synchronous lose-consciousness Think must close its eye write"
        );
        assert_eq!(
            entity
                .ai_controller()
                .unwrap()
                .outbox
                .recovery
                .set_eye_status,
            None,
            "the ladder landing must not leave SetViewStatus deferred past its actor slot"
        );
        assert_eq!(
            entity.position_iface().layer_goal(),
            crate::position_interface::Layer::ZERO,
            "ladder landing changes the actual layer without retroactively publishing a goal"
        );
    }

    #[test]
    fn ladder_arrival_knockout_closes_reciprocal_swordfight_inline() {
        let sim = crate::sim_rng::test_context();
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..Default::default()
        });
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        profiles.hth_weapons.push(Default::default());
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::default()
        };
        let mut engine = EngineInner::new();
        let victim = engine.add_entity(falling_ladder_pc(50));
        let mut opponent_entity = falling_pushed_soldier(false);
        let Entity::Soldier(soldier) = &mut opponent_entity else {
            unreachable!()
        };
        let mut enemy_ai = crate::ai_enemy::EnemyAi::default();
        enemy_ai.hth_weapon_id = 1;
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(enemy_ai));
        let opponent = engine.add_entity(opponent_entity);
        {
            let ai = engine
                .get_entity_mut(opponent)
                .unwrap()
                .ai_controller_mut()
                .unwrap();
            ai.set_ai_state(crate::ai::AiState::Attacking);
            ai.current_substate = crate::ai::Substate::AttackingSwordfight;
            ai.primary_target = victim.index();
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![opponent].into();
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![victim].into();
        install_falling_ladder_order(&mut engine, victim);

        engine.tick_push_flight_for_owner(&sim, &assets, victim);

        assert!(
            engine.orders.pending_concussion_side_effects.is_empty(),
            "the ladder Execute boundary must close its own knockout side effects"
        );
        for fighter in [victim, opponent] {
            assert!(
                engine
                    .get_entity(fighter)
                    .unwrap()
                    .human_data()
                    .unwrap()
                    .opponents
                    .is_empty(),
                "knockout must synchronously remove both reciprocal relationships"
            );
        }
        let opponent_ai = engine
            .get_entity(opponent)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert_eq!(
            opponent_ai.current_substate,
            crate::ai::Substate::AttackingQuittingSwordfight
        );
        assert!(opponent_ai.timer_is_running);
        assert_eq!(opponent_ai.when_does_timer_ring, 3);
    }

    #[test]
    fn ladder_arrival_below_knockout_threshold_preserves_swordfight() {
        let sim = crate::sim_rng::test_context();
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..Default::default()
        });
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::default()
        };
        let mut engine = EngineInner::new();
        let victim = engine.add_entity(falling_ladder_pc(200));
        let opponent = engine.add_entity(falling_pushed_soldier(false));
        engine
            .get_entity_mut(victim)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![opponent].into();
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![victim].into();
        install_falling_ladder_order(&mut engine, victim);

        engine.tick_push_flight_for_owner(&sim, &assets, victim);

        assert!(
            !engine
                .get_entity(victim)
                .unwrap()
                .human_data()
                .unwrap()
                .unconscious
        );
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![opponent]
        );
        assert_eq!(
            engine
                .get_entity(opponent)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![victim]
        );
        assert!(engine.orders.pending_concussion_side_effects.is_empty());
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
        install_falling_pushed_order(&mut engine, victim_id);

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
        install_falling_pushed_order(&mut engine, victim_id);

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
        // Only `ExecutePushSwordStrike` walks `mlistSwordStrikeVictims` at
        // RHMOTION_TERMINATED, so the completing strike has to be a push.
        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.thrusts[SwordStrike::A as usize].kind = WeaponThrustKind::PushAside;
        profile_manager.hth_weapons.push(weapon);
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        };
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
        engine.complete_melee_strike(&sim, &assets, attacker_id, None, 0, SwordStrike::A, Some(1));
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
        engine.complete_melee_strike(&sim, &assets, attacker_id, None, 0, SwordStrike::A, Some(1));

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
