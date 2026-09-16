//! Per-frame strike ticks (melee, sweep, push, rider, enemy AI) and concussion healing.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;

/// `PARITY_DEBUG_SWORD_DAMAGE=1` traces every sword-damage application and
/// every sweep-strike lifecycle step (seed, per-frame phase, per-frame arc
/// test) so a divergent hit frame can be attributed to a concrete attacker,
/// victim list and sweep angle.
pub(crate) fn sword_damage_debug_enabled() -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<0>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| ParityGate::from_env("PARITY_DEBUG_SWORD_DAMAGE", []))
        .enabled()
}

impl EngineInner {
    /// `[started, hit, completed]`; `sprite` is `(frame, frame_count,
    /// action_done_frame, action_done_counter)`.
    #[inline(never)]
    fn trace_strike_effect_pulse(
        &self,
        [attacker_id, target_id]: [EntityId; 2],
        [strike, animation, motion]: [&dyn std::fmt::Debug; 3],
        [started, hit, completed]: [bool; 3],
        sprite: [&dyn std::fmt::Display; 4],
    ) {
        let frame = self.control.frame_counter;
        let attacker_creation_order = self.world.original_creation_order(attacker_id);
        let victim_creation_order = self.world.original_creation_order(target_id);
        if !strike_effect_debug_matches(frame, attacker_creation_order, victim_creation_order) {
            return;
        }
        let [
            current_frame,
            frame_count,
            action_done_frame,
            action_done_counter,
        ] = sprite;
        eprintln!(
            "[STRIKE_EFFECT frame={frame} attacker={} attacker_co={attacker_creation_order} victim={} victim_co={victim_creation_order} phase=pulse strike={strike:?} animation={animation:?} motion={motion:?} started={started} hit={hit} completed={completed} sprite_frame={current_frame} frame_count={frame_count} action_done_frame={action_done_frame} action_done_counter={action_done_counter}]",
            attacker_id.index(),
            target_id.index(),
        );
    }

    #[inline(never)]
    fn trace_strike_effect_candidate(
        &self,
        [attacker_id, victim_id]: [EntityId; 2],
        strike: SwordStrike,
        distance: f32,
        in_range: bool,
        profile_idx: Option<u32>,
    ) {
        let frame = self.control.frame_counter;
        let attacker_creation_order = self.world.original_creation_order(attacker_id);
        let victim_creation_order = self.world.original_creation_order(victim_id);
        if !strike_effect_debug_matches(frame, attacker_creation_order, victim_creation_order) {
            return;
        }
        let attacker = self.expect_entity(attacker_id, "strike-effect diagnostic attacker");
        let victim = self.expect_entity(victim_id, "strike-effect diagnostic victim");
        let attacker_direction = attacker.element_data().direction();
        let victim_direction = direction_to(&self.world.entities, attacker_id, victim_id);
        let angle_delta = (attacker_direction - victim_direction).rem_euclid(16);
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

    /// `[before, after]` tiredness.
    #[inline(never)]
    fn trace_tiredness_strike_energy<T: std::fmt::Display>(
        frame: u32,
        creation_order: u32,
        [before, after]: [T; 2],
        strike: SwordStrike,
        energy: impl std::fmt::Display,
    ) {
        eprintln!(
            "RUST_TIREDNESS frame={frame} co={creation_order} site=strike_energy \
             before={before} after={after} strike={} energy={energy}",
            strike as u32
        );
    }

    #[inline(never)]
    fn trace_sweep_phase(&self, attacker_id: EntityId, phase: SweepTickPhase) {
        eprintln!(
            "[SWEEPPHASE f={} attacker={:?} (co {}) phase={:?} victims={:?}]",
            self.control.frame_counter,
            attacker_id,
            self.world.original_creation_order(attacker_id),
            phase,
            self.get_entity(attacker_id)
                .and_then(Entity::human_data)
                .map(|h| h.sword_sweep.victims.len()),
        );
    }

    /// `angles` is `[initial, current, final, signed rotation]`.
    #[inline(never)]
    fn trace_sweep_init(
        &self,
        attacker_id: EntityId,
        strike: SwordStrike,
        strike_kind: WeaponThrustKind,
        victims: &[EntityId],
        [initial, current, final_angle, rotation]: [f32; 4],
    ) {
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
            current,
            final_angle,
            rotation,
        );
    }

    /// `sectors` is `[initial, current]`.
    #[inline(never)]
    fn trace_sweep_tick(
        frame: u32,
        attacker_id: EntityId,
        strike_kind: WeaponThrustKind,
        sectors: [impl std::fmt::Display; 2],
        current_angle: f32,
        pending: usize,
    ) {
        let [initial_sector, current_sector] = sectors;
        eprintln!(
            "[SWEEPTICK f={} attacker={:?} kind={:?} init_sec={} cur_sec={} cur_angle={} pending={}]",
            frame, attacker_id, strike_kind, initial_sector, current_sector, current_angle, pending,
        );
    }

    /// One `SPECIAL_STRIKE` line; the owner's selected-element snapshot is appended.
    #[inline(never)]
    fn trace_special_strike(&self, frame: u32, owner: EntityId, detail: std::fmt::Arguments<'_>) {
        eprintln!(
            "SPECIAL_STRIKE frame={} owner={} {detail} selected={}",
            frame,
            owner.index(),
            special_strike_selected_snapshot(
                &self.world.entities,
                &self.orders.sequence_manager,
                owner
            ),
        );
    }

    /// `phase` is `after_begin`, `after_begin_drain` or `after_launch`.
    #[inline(never)]
    fn trace_special_strike_state(&self, frame: u32, owner: EntityId, phase: &str) {
        let ai = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::enemy_ai)
            .unwrap_or_else(|| panic!("special-strike owner lost Enemy AI at {phase}"));
        self.trace_special_strike(
            frame,
            owner,
            format_args!(
                "phase={phase} pending_special={} state={:?} substate={:?}",
                ai.pending_special_strike, ai.base.current_state, ai.base.current_substate,
            ),
        );
    }

    #[inline(never)]
    fn trace_opponent_sprite_timing(
        frame: u32,
        [owner, target]: [EntityId; 2],
        [owner_creation_order, target_creation_order]: [u32; 2],
        only_owner: Option<EntityId>,
        animation: crate::order::OrderType,
        target_entity: &Entity,
    ) {
        let sprite = &target_entity.element_data().sprite;
        let script_len = sprite
            .current_scripts_opt()
            .and_then(|scripts| scripts.get(sprite.current_row as usize))
            .map(|script| script.frame_ids.len());
        eprintln!(
            "OPPONENT_SPRITE_TIMING frame={} owner={} owner_co={} target={} target_co={} caller={:?} animation={animation:?} profile={:?} primary={:?} alternate={:?} use_alternate={} row={} sprite_frame={} frame_count={} action_done_frame={} action_done_counter={} script_len={script_len:?}",
            frame,
            owner.index(),
            owner_creation_order,
            target.index(),
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

    #[inline(never)]
    fn trace_reactive_sword_enemy_principal(
        &self,
        debug: crate::combat::SwordStrikeProposalDebug,
        target_id: EntityId,
        opponent_time_limit: Option<i16>,
    ) {
        let target_animation = self.live_actor_animation(target_id);
        let target_raw_frames = self.get_entity(target_id).map(|target| {
            target
                .element_data()
                .sprite
                .frames_from_now_till_action_done()
        });
        super::evaluate::trace_reactive_sword_for(
            debug,
            format_args!(
                "phase=enemy_principal target={} animation={:?} raw_frames_from_now={:?} time_limit={:?}",
                target_id.index(),
                target_animation,
                target_raw_frames,
                opponent_time_limit,
            ),
        );
    }
}

fn strike_effect_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<3> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<3>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_STRIKE_EFFECT",
            [
                "PARITY_DEBUG_STRIKE_EFFECT_FRAME",
                "PARITY_DEBUG_STRIKE_EFFECT_ATTACKER_CREATION_ORDER",
                "PARITY_DEBUG_STRIKE_EFFECT_VICTIM_CREATION_ORDER",
            ],
        )
    })
}

fn strike_effect_debug_matches(
    frame: u32,
    attacker_creation_order: u32,
    victim_creation_order: u32,
) -> bool {
    strike_effect_debug_gate().matches([
        Some(frame),
        Some(attacker_creation_order),
        Some(victim_creation_order),
    ])
}

fn special_strike_lifecycle_debug_matches(frame: u32, owner: u32) -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_SPECIAL_STRIKE_LIFECYCLE",
            [
                "PARITY_DEBUG_SPECIAL_STRIKE_FRAME",
                "PARITY_DEBUG_SPECIAL_STRIKE_OWNER_HANDLE",
            ],
        )
    })
    .matches([Some(frame), Some(owner)])
}

fn opponent_sprite_timing_debug_matches(frame: u32, owner: u32, target: u32) -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<3>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_OPPONENT_SPRITE_TIMING",
            [
                "PARITY_DEBUG_OPPONENT_SPRITE_TIMING_FRAME",
                "PARITY_DEBUG_OPPONENT_SPRITE_TIMING_OWNER",
                "PARITY_DEBUG_OPPONENT_SPRITE_TIMING_TARGET",
            ],
        )
    })
    .matches([Some(frame), Some(owner), Some(target)])
}

fn special_strike_selected_snapshot(
    entities: &crate::entities::Entities,
    manager: &crate::sequence::SequenceManager,
    owner: EntityId,
) -> String {
    let selected = entities.current_element_for_actor(owner);
    let element = selected.and_then(|(sequence, index)| {
        manager.get_element(sequence, index).map(|element| {
            (
                sequence,
                index,
                element.command,
                element.state,
                element.priority,
                element
                    .postponed
                    .map(|reference| (reference.sequence_id, reference.element_index)),
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

/// Match rolling's rejected-slope behavior: publish only the current point as
/// the new goal. The surrounding line-crossing-check tail then calls
/// increment computation; keeping the increment uncached lets its
/// very-small-increment guard preserve the prior direction goal.
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

fn advance_circle_angle(
    sweep: &mut crate::element::HumanSwordSweepState,
    rotation_per_frame: f32,
    direction: crate::profiles::WeaponThrustDirection,
) {
    let candidate = sweep.current_angle + rotation_per_frame;
    let past_final = match direction {
        crate::profiles::WeaponThrustDirection::LeftToRight => candidate >= sweep.final_angle,
        _ => candidate <= sweep.final_angle,
    };
    if !past_final || angle_to_sector(candidate) == angle_to_sector(sweep.final_angle) {
        sweep.current_angle = candidate;
    } else {
        sweep.current_angle = sweep.final_angle;
    }
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

impl EngineInner {
    fn begin_selected_melee_motion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
    ) {
        let profile_idx = {
            let entity = self.expect_entity_mut(attacker_id, "melee MotionState::Start owner");
            let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager);
            entity.set_posture(Posture::Upright);
            let actor = entity.actor_data_mut().unwrap_or_else(|| {
                panic!("melee MotionState::Start owner {attacker_id:?} lost actor data")
            });
            actor.action_state = ActionState::WaitingSword;
            profile_idx
        };

        // Human-actor execution forecasts and warns only after
        // Action processing returns start. It passes
        // the animation-derived sword-strike kind to the strike warning, so a
        // sprite-selected replacement row, rather than the requested command,
        // identifies the strike defenders may recognize. This may
        // synchronously Think and draw RNG, so it belongs to the live owner
        // slot rather than instruction handling.
        let animation = self.live_actor_animation(attacker_id).unwrap_or_else(|| {
            panic!("melee MotionState::Start owner {attacker_id:?} has no live animation")
        });
        let strike = sword_strike_from_animation(animation).unwrap_or_else(|| {
            panic!(
                "melee MotionState::Start owner {attacker_id:?} has non-strike live animation {animation:?}"
            )
        });

        // The original game's initial warning forecast is not side-effect free:
        // Lateral and half-circle sword-strike victim collection
        // writes the human-owned
        // initial/current/final sweep angles even though they fill a temporary
        // warning-victim list. A replacement lateral/half-circle strike can
        // therefore keep an interrupted strike's victim FIFO while rebasing
        // its geometry to the replacement strike before the first IN_PROGRESS
        // execution. Circle sword-strike victim collection does not write those
        // angles, so full circles deliberately retain the old geometry here.
        // Do not create a sweep for an ordinary fresh strike here; its real
        // victim list is still initialized only at MotionState::Done.
        self.apply_strike_selection_sweep_rebase(
            assets,
            attacker_id,
            Some(crate::combat::StrikeSelectionSweepRebase { strike }),
        );

        let mut victims =
            self.collect_sword_strike_warning_victims(assets, attacker_id, strike, profile_idx);
        // Human-actor execution warns the list produced by
        // sword-strike victim collection. Every multi-victim collector fills
        // that list by walking actors in engine order, whose
        // The actor registry follows append-only element-insertion order. Rust entity
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
            .current_order_for_actor(&self.world.entities, attacker_id)
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
    ) -> Option<crate::sprite::MotionState> {
        let execution_frozen = self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .unwrap_or_else(|| panic!("selected melee owner {attacker_id:?} is missing actor data"))
            .execution_frozen;
        if execution_frozen {
            return Some(crate::sprite::MotionState::InProgress);
        }
        if !self.selected_melee_identity_is_live(attacker_id, selected) {
            return None;
        }
        self.tick_straight_melee_for(sim, assets, attacker_id, selected)
            .or_else(|| self.tick_nonstraight_melee_for(sim, assets, attacker_id, selected))
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

        self.tick_enemy_sword_attacks(sim, assets);
        self.tick_pc_combat_anim_speech(sim, assets);
        self.tick_refresh_purse_disable(assets);
    }

    /// Apply the parry hold countdown at the owning actor's legacy Execute
    /// slot, matching human-actor execution.
    ///
    /// This cannot be batched after entity traversal: a later-created actor
    /// may land a sword hit in the same frame and postpone the parry stop that
    /// an earlier-created defender just queued.
    pub(in crate::engine) fn tick_parry_counter_for_execute(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        order_type: crate::order::OrderType,
        motion: &mut crate::sprite::MotionState,
    ) {
        let low = match order_type {
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
                *motion = crate::sprite::MotionState::Terminated;
            } else {
                let elem =
                    crate::sequence::SequenceElement::new(1, Command::StopParrySword, Some(owner));
                // Ordinary instructions retain their sequence FIFO order
                // alongside later actors' damage instructions.
                self.launch_element(sim, assets, elem);
            }
        }
    }

    pub(super) fn receive_smalltalk_hint(
        &mut self,
        attacker_id: EntityId,
        target_id: EntityId,
        is_left: bool,
    ) {
        let target_human = self.world.entities.expect_human_data(
            target_id,
            format_args!("smalltalk attacker {attacker_id:?} hint target"),
        );
        let is_principal = target_human.opponents.first().copied() == Some(attacker_id);
        if !is_principal {
            return;
        }
        let human = self.world.entities.expect_human_data_mut(
            target_id,
            format_args!("smalltalk attacker {attacker_id:?} hint target while receiving hint"),
        );
        human.smalltalk_hint = if is_left {
            crate::element::SmalltalkHint::Left
        } else {
            crate::element::SmalltalkHint::Right
        };
        human.smalltalk_hint_opponent = Some(attacker_id);
    }

    pub(super) fn evaluate_smalltalk_hint<I: Into<EntityId>>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: I,
    ) -> bool {
        let entity_id = entity_id.into();
        let (hint, hint_opponent) = {
            let entity = self.expect_entity(entity_id, "smalltalk-hint evaluation owner");
            let human = entity.human_data().unwrap_or_else(|| {
                panic!("smalltalk-hint evaluation owner {entity_id:?} is not human")
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
            panic!(
                "smalltalk-hint evaluation owner {entity_id:?} has {hint:?} without a hint opponent"
            )
        });
        let opponent = self.world.entities.expect_entity(
            opponent_id,
            format_args!("smalltalk-hint evaluation owner {entity_id:?} hint opponent"),
        );
        assert!(
            opponent.human_data().is_some(),
            "smalltalk-hint evaluation owner {entity_id:?} hint opponent {opponent_id:?} is not human"
        );

        let human = self.world.entities.expect_human_data_mut(
            entity_id,
            format_args!("smalltalk-hint evaluation owner while clearing hint"),
        );
        human.smalltalk_hint = crate::element::SmalltalkHint::None;
        human.smalltalk_hint_opponent = None;

        let elem = crate::sequence::SequenceElement::new_interaction(
            1,
            parry_cmd,
            Some(entity_id),
            Some(opponent_id),
        );
        self.launch_element(sim, assets, elem);
        true
    }

    /// Advance one straight/assault sequence-driven strike at its actor's
    /// creation-order slot.
    ///
    /// The original-game actor update executes the current order inline,
    /// so a straight strike's synchronously-dispatched damage can interrupt a
    /// later-created actor before that actor gets its own update.
    /// Sweep/push work likewise runs from its owning attacker's slot; this
    /// helper is the narrow straight/assault owner-slot path.
    pub(in crate::engine) fn tick_straight_melee_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        selected: super::tick::MeleeOwnerSelection,
    ) -> Option<crate::sprite::MotionState> {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return None;
        }

        let Some((strike, target_id, animation)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, attacker_id)
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
            return None;
        };
        let gesture_quality = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .unwrap_or_else(|| {
                panic!(
                    "selected melee element {:?}/{} disappeared before its hit",
                    selected.seq_id, selected.elem_idx
                )
            })
            .gesture_quality;
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
            return None;
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
            return Some(crate::sprite::MotionState::InProgress);
        }

        let entity = self.expect_entity_mut(attacker_id, "selected melee attacker");
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

        if strike_effect_debug_gate().enabled() {
            self.trace_strike_effect_pulse(
                [attacker_id, target_id],
                [&strike, &animation, &motion],
                [started, hit, completed],
                [
                    &current_frame,
                    &frame_count,
                    &action_done_frame,
                    &action_done_counter,
                ],
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
                gesture_quality,
            );
        }
        if motion == crate::sprite::MotionState::Terminated {
            self.complete_melee_strike(sim, assets, attacker_id, strike, profile_idx);
        }
        Some(motion)
    }

    fn resolve_straight_melee_hit(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        victim_id: EntityId,
        strike: SwordStrike,
        profile_idx: Option<u32>,
        gesture_quality: crate::player_command::GestureQuality,
    ) {
        // Straight sword-strike execution uses the full
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
        if strike_effect_debug_gate().enabled() {
            self.trace_strike_effect_candidate(
                [attacker_id, victim_id],
                strike,
                distance,
                in_range,
                profile_idx,
            );
        }
        if in_range {
            if let Some(profile_idx) = profile_idx {
                self.queue_scaled_sword_damage(
                    sim,
                    assets,
                    victim_id,
                    attacker_id,
                    strike,
                    profile_idx,
                    gesture_quality,
                );
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
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
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
        // The sword-strike victim list belongs to push-strike execution
        // in the original game: its done-motion
        // arm clears and refills the list, and only its terminated-motion
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
        if completes_push_strike {
            while let Some(victim_id) = self
                .expect_entity(actor_id, "push completion attacker")
                .human_data()
                .expect("push attacker must be human")
                .sword_sweep
                .victims
                .first()
                .copied()
            {
                let attacker = self.expect_entity(actor_id, "push completion attacker");
                let victim = self.expect_entity(victim_id, "push completion victim");
                if should_enter_swordfight_after_strike(
                    attacker,
                    victim,
                    &assets.profile_manager,
                    &self.mission_domain.diplomacy,
                ) {
                    self.launch_enter_swordfight_after_strike(sim, assets, victim_id, actor_id);
                }
                self.expect_entity_mut(actor_id, "push completion attacker")
                    .human_data_mut()
                    .expect("push attacker must be human")
                    .sword_sweep
                    .victims
                    .remove(0);
            }
        } else if clears_shared_sweep {
            self.expect_entity_mut(actor_id, "sweep completion attacker")
                .human_data_mut()
                .expect("sweep attacker must be human")
                .sword_sweep
                .victims
                .clear();
        }

        match profile_idx.and_then(|idx| assets.profile_manager.get_hth_weapon(idx)) {
            Some(profile) => {
                let energy = combat::strike_energy_cost(profile, strike);
                let frame = self.control.frame_counter;
                // Resolve identity only for the enabled diagnostic; the write
                // below holds the entity borrow.
                let tiredness_debug_creation_order = combat::tiredness_debug_enabled()
                    .then(|| self.world.original_creation_order(actor_id));
                if let Some(entity) = self.get_entity_mut(actor_id)
                    && let Some(human) = entity.human_data_mut()
                {
                    let before = human.tiredness;
                    human.tiredness = combat::add_strike_tiredness(human.tiredness, energy);
                    if let Some(creation_order) = tiredness_debug_creation_order
                        && combat::tiredness_debug_matches(creation_order)
                    {
                        Self::trace_tiredness_strike_energy(
                            frame,
                            creation_order,
                            [before, human.tiredness],
                            strike,
                            energy,
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
    }

    /// Advance every sequence-driven melee strike.
    ///
    /// Kept as the complete low-level driver for focused tests.
    /// Production update orchestration runs every strike kind in its
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
                .current_order_for_actor(&self.world.entities, actor_id)
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
            self.tick_selected_melee_owner(sim, assets, actor_id, selected);
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
    ) -> Option<crate::sprite::MotionState> {
        if self
            .get_entity(attacker_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return None;
        }
        if self.actors_frozen() {
            return Some(crate::sprite::MotionState::InProgress);
        }

        let Some((strike, target_id, animation)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, attacker_id)
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
            return None;
        };
        let gesture_quality = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .unwrap_or_else(|| {
                panic!(
                    "selected melee element {:?}/{} disappeared before its hit",
                    selected.seq_id, selected.elem_idx
                )
            })
            .gesture_quality;
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
            return None;
        }

        let mut initialized_sweep = false;
        let mut started = false;
        let mut sweep_phase;
        let motion;

        // Phase 1: execute the selected order. Original derives strike and
        // target from this order and applies the hit on the sprite's one-shot
        // completed-motion result.
        {
            let entity_id = attacker_id;
            let Some(entity) = self.world.entities.get_mut(attacker_id) else {
                return None;
            };
            sweep_phase = SweepTickPhase::InProgress;
            let true_sweep_at_action_point = matches!(
                strike_kind,
                WeaponThrustKind::TrueCircle | WeaponThrustKind::TrueHalfCircle
            )
            .then(|| entity.human_data().map(|human| &human.sword_sweep))
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
                // Rotation holds the sprite at its action point and returns
                // in-progress motion until the sweep finishes.
                motion = crate::sprite::MotionState::InProgress;
                tracing::trace!(
                    "tick_melee_strikes: entity={} order_id={} strike={:?} holding true-circle sweep",
                    entity_id.index(),
                    selected.order_id,
                    strike
                );
            } else {
                // True-circle sword-strike execution tests action completion and
                // presents the current sweep angle before its terminal
                // action-processing step advances the frame counter. Preserve
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
                motion = entity.element_data_mut().sprite.perform_action(
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
            }
        }

        if started {
            self.begin_selected_melee_motion(sim, assets, attacker_id);
        }

        // Apply the hit before returning to the actor update.
        if motion == crate::sprite::MotionState::Done {
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
                let all_victims =
                    self.execute_multi_target_strike(assets, attacker_id, strike, profile_idx);
                self.initialize_sweep(
                    assets,
                    attacker_id,
                    strike,
                    profile_idx,
                    strike_kind,
                    all_victims,
                    gesture_quality,
                );
                initialized_sweep = profile_idx.is_some();
            } else if is_push {
                // Push strike: apply damage to all victims at the
                // hit frame (no AI warn tolerance), but defer the
                // EnterSwordfight command to the strike's completion
                // by stashing victim IDs on the actor.
                let all_victims =
                    self.execute_multi_target_strike(assets, attacker_id, strike, profile_idx);
                for victim_id in &all_victims {
                    if let Some(profile_idx) = profile_idx {
                        self.queue_scaled_sword_damage(
                            sim,
                            assets,
                            *victim_id,
                            attacker_id,
                            strike,
                            profile_idx,
                            gesture_quality,
                        );
                    }
                }
                if let Some(entity) = self.world.entities.get_mut(attacker_id)
                    && let Some(human) = entity.human_data_mut()
                {
                    human.sword_sweep.victims = all_victims;
                }
            } else {
                self.resolve_straight_melee_hit(
                    sim,
                    assets,
                    attacker_id,
                    target_id,
                    strike,
                    profile_idx,
                    gesture_quality,
                );
            }
        }

        if motion == crate::sprite::MotionState::Terminated {
            self.complete_melee_strike(sim, assets, attacker_id, strike, profile_idx);
        }
        if self.selected_melee_identity_is_live(attacker_id, selected) {
            let phase = if initialized_sweep {
                SweepTickPhase::Initialized
            } else {
                sweep_phase
            };
            self.tick_selected_sweep_phase(sim, assets, attacker_id, phase);
        }
        Some(motion)
    }

    pub(super) fn tick_selected_sweep_phase(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        phase: SweepTickPhase,
    ) {
        if sword_damage_debug_enabled() {
            self.trace_sweep_phase(attacker_id, phase);
        }
        match phase {
            SweepTickPhase::Dormant | SweepTickPhase::Start => {}
            SweepTickPhase::Initialized => {
                self.tick_sweep_for(sim, assets, attacker_id, true);
            }
            SweepTickPhase::InProgress => {
                // Circle sword-strike execution advances its retained angles only
                // after the sprite action becomes done. A new circle
                // strike can inherit the previous strike's human-owned victim
                // list/angles, but it must first play to its own action point;
                // otherwise the old geometry rotates the new animation on its
                // first IN_PROGRESS frame. Lateral strikes deliberately keep
                // their different legacy rule and advance any retained list
                // on IN_PROGRESS.
                let (active_strike, active_order_id) = self.orders.sequence_manager.current_order_for_actor(&self.world.entities, attacker_id)
                    .and_then(|(_, _, order)| {
                        sword_strike_from_animation(order.order_type)
                            .map(|strike| (strike, order.order_id))
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "selected non-straight melee attacker {attacker_id:?} lost its strike order"
                        )
                    });
                let entity =
                    self.expect_entity(attacker_id, "selected non-straight melee attacker");
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
                // enter lateral or circle sword-strike execution,
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
                let retained_circle_off_action_point =
                    is_circle_sweep(active_kind) && !at_action_point;
                if retained_circle_off_action_point {
                    // Circle sword-strike execution always runs the effect with
                    // the current Execute call's strike, even before that
                    // animation reaches its action point.  The action-point
                    // gate only protects the tail angle advance.  Preserve
                    // the retained victim/angle geometry, but rebind the
                    // payload and direction to the replacement strike.
                    self.tick_sweep_for_mode(sim, assets, attacker_id, false, true);
                    return;
                }
                self.tick_sweep_for(sim, assets, attacker_id, false);
            }
        }
    }

    /// Replay the angle side effect of sword-strike damage estimation.
    ///
    /// Strike selection estimates each candidate strike through
    /// Original-game strike estimation builds its victim list with a
    /// possible-victims collector. The lateral collector
    /// and the half-circle collector both
    /// overwrite the human-owned initial, final, and current strike angles
    /// before scanning, so an
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
        gesture_quality: crate::player_command::GestureQuality,
    ) {
        assert!(
            gesture_quality.is_strike_quality(),
            "invalid gesture quality reached sweep initialization"
        );
        // No weapon means no sweep; a referenced profile must exist.
        let Some(profile_idx) = profile_idx else {
            return;
        };
        let profile = assets
            .profile_manager
            .get_hth_weapon(profile_idx)
            .unwrap_or_else(|| panic!("sweep has missing weapon profile {profile_idx}"));
        let thrust = &profile.thrusts[strike as usize];
        let direction = thrust.direction;
        // Original-game sword strike-angle access evaluates authored-degree
        // expression in double precision and narrow only at the single-precision return.
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
            self.trace_sweep_init(
                attacker_id,
                strike,
                strike_kind,
                &victims,
                [initial, dir_angle, final_a, signed_rotation],
            );
        }
        let human = self
            .expect_entity_mut(attacker_id, "sweep initialization attacker")
            .human_data_mut()
            .expect("sweep attacker must be human");
        human.sword_sweep = crate::element::HumanSwordSweepState {
            victims,
            initial_angle: initial,
            current_angle: dir_angle,
            final_angle: final_a,
        };

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
        use crate::profiles::WeaponThrustDirection;
        let entity = self.expect_entity(attacker_id, "sweep attacker");
        if entity
            .actor_data()
            .expect("sweep attacker must be an actor")
            .execution_frozen
        {
            return;
        }
        let profile_idx = get_hth_weapon_id_full(entity, &assets.profile_manager)
            .expect("sweep attacker must have a melee weapon");
        let (sequence_id, element_index, order) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, attacker_id)
            .expect("sweep attacker must have a selected order");
        let strike = sword_strike_from_animation(order.order_type)
            .expect("sweep attacker must have a selected strike");
        let gesture_quality = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .expect("selected sweep sequence must exist")
            .gesture_quality;
        let thrust = &assets
            .profile_manager
            .get_hth_weapon(profile_idx)
            .expect("sweep weapon profile must exist")
            .thrusts[strike as usize];
        let kind = thrust.kind;
        let direction = thrust.direction;
        let rotation_per_frame = strike_profile_angle(thrust.rotation_angle)
            * if direction == WeaponThrustDirection::RightToLeft {
                -1.0
            } else {
                1.0
            };
        let circle = is_circle_sweep(kind);
        assert!(
            circle || kind == WeaponThrustKind::Lateral,
            "selected strike is not a sweep"
        );
        let sweep = &entity
            .human_data()
            .expect("sweep attacker must be human")
            .sword_sweep;
        if initialized_this_hourglass {
            if circle {
                let sweep = &mut self
                    .expect_entity_mut(attacker_id, "sweep attacker")
                    .human_data_mut()
                    .expect("sweep attacker must be human")
                    .sword_sweep;
                advance_circle_angle(sweep, rotation_per_frame, direction);
            }
            return;
        }
        if kind == WeaponThrustKind::Lateral {
            if sweep.victims.is_empty() {
                return;
            }
            self.expect_entity_mut(attacker_id, "sweep attacker")
                .human_data_mut()
                .expect("sweep attacker must be human")
                .sword_sweep
                .current_angle += rotation_per_frame;
        }
        if !effect_only_before_action_point
            && matches!(
                kind,
                WeaponThrustKind::TrueCircle | WeaponThrustKind::TrueHalfCircle
            )
        {
            let entity = self.expect_entity_mut(attacker_id, "sweep attacker");
            let new_dir = angle_to_sector(
                entity
                    .human_data()
                    .expect("sweep attacker must be human")
                    .sword_sweep
                    .current_angle,
            );
            let element = entity.element_data_mut();
            element.set_direction_instantly(new_dir as i16);
            element
                .sprite
                .force_action_direction(strike_to_animation(strike), new_dir.into());
        }
        let sweep = &self
            .expect_entity(attacker_id, "sweep attacker")
            .human_data()
            .expect("sweep attacker must be human")
            .sword_sweep;
        let initial_sector = angle_to_sector(sweep.initial_angle);
        let current_sector = angle_to_sector(sweep.current_angle);
        if sword_damage_debug_enabled() {
            Self::trace_sweep_tick(
                self.control.frame_counter,
                attacker_id,
                kind,
                [initial_sector, current_sector],
                sweep.current_angle,
                sweep.victims.len(),
            );
        }
        let mut index = 0;
        loop {
            let attacker = self.expect_entity(attacker_id, "sweep attacker");
            let Some(victim_id) = attacker
                .human_data()
                .expect("sweep attacker must be human")
                .sword_sweep
                .victims
                .get(index)
                .copied()
            else {
                break;
            };
            let position = attacker.element_data().position_map();
            let hit = self.get_entity(victim_id).map(|victim| {
                let victim_position = victim.element_data().position_map();
                let sector = crate::position_interface::vector_to_sector_0_to_15(
                    victim_position.x - position.x,
                    (victim_position.y - position.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO,
                ) as u8;
                match direction {
                    WeaponThrustDirection::LeftToRight => {
                        is_sector_between(sector, initial_sector, current_sector)
                    }
                    _ => is_sector_between(sector, current_sector, initial_sector),
                }
            });
            if hit == Some(false) {
                index += 1;
                continue;
            }
            if hit == Some(true) {
                self.queue_scaled_sword_damage(
                    sim,
                    assets,
                    victim_id,
                    attacker_id,
                    strike,
                    profile_idx,
                    gesture_quality,
                );
            }
            self.expect_entity_mut(attacker_id, "sweep attacker")
                .human_data_mut()
                .expect("sweep attacker must be human")
                .sword_sweep
                .victims
                .remove(index);
            if hit == Some(true)
                && should_enter_swordfight_after_strike(
                    self.expect_entity(attacker_id, "sweep attacker"),
                    self.expect_entity(victim_id, "sweep victim"),
                    &assets.profile_manager,
                    &self.mission_domain.diplomacy,
                )
            {
                self.launch_enter_swordfight_after_strike(sim, assets, victim_id, attacker_id);
            }
        }
        if circle && !effect_only_before_action_point {
            let sweep = &mut self
                .expect_entity_mut(attacker_id, "sweep attacker")
                .human_data_mut()
                .expect("sweep attacker must be human")
                .sword_sweep;
            advance_circle_angle(sweep, rotation_per_frame, direction);
        }
    }

    /// Launch swordfight entry after the victim's damage instruction.
    pub(in crate::engine) fn launch_enter_swordfight_after_strike(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
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
        self.launch_element(sim, assets, element);
    }

    // ─── Push flight tick ─────────────────────────────────────────

    /// Apply sprite-owned flight movement before the human's state callbacks.
    pub(in crate::engine) fn perform_combat_flight_position(
        &mut self,
        owner: EntityId,
        motion: crate::sprite::MotionState,
    ) -> crate::sprite::MotionState {
        use crate::sprite::MotionState;
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("flight owner disappeared");
        let before = crate::movement_diagnostics::parity_movement_capture_active()
            .then(|| entity.position_iface().v48_serialized_state());
        let sprite = entity.sprite();
        if perform_flight_stops_before_position_update(
            sprite.frame_count,
            sprite.current_frame,
            sprite.num_frames_for_row(sprite.current_row),
        ) {
            entity
                .position_iface_mut()
                .set_projectile_increment(crate::coordinates::WorldVec3D::ZERO);
        }
        entity.position_iface_mut().set_increment_3d_computed(true);
        let increment = entity.position_iface().get_increment();
        let position = entity.element_data().position();
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(
                position.x + increment.x,
                position.y + increment.y,
                position.z + increment.z,
            ));
        Self::publish_flight_position(entity);
        entity.element_data_mut().sprite.display_order_ref = None;
        entity.element_data_mut().sprite.behind_display_order_ref = true;
        let raw_post = entity.element_data().position();
        let raw_map = entity.element_data().position_map();
        match motion {
            MotionState::Start => entity.position_iface_mut().set_anti_collision_on(false),
            MotionState::Terminated => {
                let pi = entity.position_iface_mut();
                let goal = pi.world_goal();
                let layer = pi.layer_goal();
                let (sector, sector_index) = pi.get_goal_sector_topology();
                pi.set_anti_collision_on(true);
                entity.element_data_mut().set_layer(layer.get());
                entity
                    .element_data_mut()
                    .set_sector_topology(sector, sector_index);
                entity.element_data_mut().set_position(goal);
                entity
                    .position_iface_mut()
                    .set_projectile_increment(crate::coordinates::WorldVec3D::ZERO);
                Self::publish_flight_position(entity);
            }
            _ => {}
        }
        if let Some(before) = before {
            let installed = entity.actor_data().and_then(|actor| actor.installed_order);
            crate::movement_diagnostics::record_parity_flight_step(
                crate::movement_diagnostics::ParityFlightStep {
                    entity: owner,
                    phase: "execute".into(),
                    geometry: "World3d".into(),
                    order_id: installed.map(|order| order.order_id.get()),
                    order_type: installed.map(|order| format!("{:?}", order.order_type)),
                    entry_position: before.position.into(),
                    entry_position_map: before.map.into(),
                    old_position: before.old_position.into(),
                    old_position_map: before.old_map.into(),
                    goal: before.goal.into(),
                    cached_increment: before.increment.into(),
                    applied_increment: increment.into(),
                    raw_post_position: raw_post.into(),
                    raw_post_position_map: raw_map.into(),
                    motion_state: format!("{motion:?}"),
                    post_position: entity.element_data().position().into(),
                    post_position_map: entity.element_data().position_map().into(),
                    snapped_to_goal: motion == MotionState::Terminated,
                },
            );
        }
        motion
    }

    fn publish_flight_position(entity: &mut Entity) {
        let map = entity.element_data().position_map();
        let center = entity.sprite().center;
        entity.position_iface_mut().finish_flight_position_update(
            crate::coordinates::MapPoint::new(
                (map.x - center.x).floor(),
                (map.y - center.y).floor(),
            ),
        );
        entity.element_data_mut().update_grid_cell();
    }

    /// Human flight callbacks follow the sprite position and posture updates.
    pub(in crate::engine) fn finish_combat_flight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        motion: crate::sprite::MotionState,
    ) {
        let antagonist = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
            .expect("flight owner has no selected order")
            .2
            .antagonist;
        if motion == crate::sprite::MotionState::Terminated {
            self.update_script_sectors_after_flight(sim, assets, owner);
        }
        if let Some(antagonist) = antagonist {
            let increment = self
                .world
                .entities
                .get(owner)
                .expect("flight owner disappeared")
                .position_iface()
                .get_increment();
            self.apply_domino_effect(sim, assets, owner, antagonist, increment.x, increment.y);
        }
    }

    /// A ladder fall uses its selected destination and the actor's shared timer.
    pub(in crate::engine) fn execute_ladder_fall_position(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        motion: crate::sprite::MotionState,
    ) -> crate::sprite::MotionState {
        use crate::sprite::MotionState;
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("ladder fall owner disappeared");
        let actor = entity
            .actor_data_mut()
            .expect("ladder fall owner is not an actor");
        if actor.wait_time != 0 {
            actor.wait_time -= 1;
            if actor.wait_time == 0 {
                let destination = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(&self.world.entities, owner)
                    .expect("ladder fall has no selected order")
                    .2
                    .destination_3d;
                let target = crate::coordinates::WorldPoint3D::new(
                    destination[0],
                    destination[1],
                    destination[2],
                );
                let sector = self
                    .world
                    .entities
                    .get(owner)
                    .expect("ladder fall owner disappeared")
                    .element_data()
                    .sector()
                    .expect("ladder fall has no lift sector");
                let low = self
                    .find_lift_low_entry(assets, u16::from(sector))
                    .expect("ladder fall lift has no low exit");
                self.world
                    .entities
                    .get_mut(owner)
                    .expect("ladder fall owner disappeared")
                    .position_iface_mut()
                    .set_projectile_increment(crate::coordinates::WorldVec3D::ZERO);
                let entity = self
                    .world
                    .entities
                    .get_mut(owner)
                    .expect("ladder fall owner disappeared");
                entity.element_data_mut().set_layer(low.layer);
                entity
                    .element_data_mut()
                    .set_sector_topology(Some(low.sector), low.sector.arena_index());
                let obstacle =
                    self.get_projection_area_index(assets, low.sector, low.layer, target.to_map());
                self.set_obstacle_and_material(assets, owner, obstacle);
                let entity = self
                    .world
                    .entities
                    .get_mut(owner)
                    .expect("ladder fall owner disappeared");
                entity.element_data_mut().set_position(target);
                Self::publish_flight_position(entity);
                entity.position_iface_mut().new_move();
                let entity = self
                    .world
                    .entities
                    .get(owner)
                    .expect("ladder fall owner disappeared");
                let concussion = entity
                    .human_data()
                    .expect("ladder fall owner is not human")
                    .concussion_of_the_brain;
                let life_points = get_life_points(entity);
                let new_value =
                    crate::combat::compute_concussion_effect(concussion, 71, life_points);
                self.apply_concussion(sim, assets, owner, new_value, false);
                let entity = self
                    .world
                    .entities
                    .get_mut(owner)
                    .expect("ladder fall owner disappeared");
                entity.set_posture(if entity.is_dead() {
                    Posture::DeadBack
                } else {
                    Posture::Lying
                });
                entity
                    .actor_data_mut()
                    .expect("ladder fall owner is not an actor")
                    .action_state = ActionState::Waiting;
                return MotionState::Terminated;
            }
            let entity = self
                .world
                .entities
                .get_mut(owner)
                .expect("ladder fall owner disappeared");
            let increment = entity.position_iface().get_increment();
            let position = entity.element_data().position();
            entity
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(
                    position.x + increment.x,
                    position.y + increment.y,
                    position.z + increment.z,
                ));
            Self::publish_flight_position(entity);
        }
        if motion == MotionState::Terminated {
            self.update_script_sectors_after_flight(sim, assets, owner);
        }
        motion
    }

    /// Bud-Spencer-style domino punch propagation.
    ///
    /// Called by the selected falling order after its position and state callbacks.
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
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        flyer_id: EntityId,
        hitter_id: EntityId,
        inc_x: f32,
        inc_y: f32,
    ) {
        if inc_x == 0.0 && inc_y == 0.0 {
            return;
        }
        // Read flyer position + sector.
        let (flyer_pos_ground, flyer_sector) = {
            let elem = self
                .expect_entity(flyer_id, "domino effect flyer")
                .element_data();
            let position = elem.position();
            ((position.x, position.y), elem.sector())
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
            if elem.posture() != Posture::Upright {
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

            // Ground position is literal world X/Y. It is not the
            // projected map position (whose Y is world Y minus elevation).
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
            if !(inc_x * dx + inc_y * dy > 0.0) {
                continue;
            }

            let victim_id = candidate_id;
            let elem = crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveHitDamage,
                Some(victim_id),
                Some(hitter_id),
                0,             // damage stays 0
                DOMINO_DAMAGE, // concussion
            );
            self.launch_element(sim, assets, elem);
            tracing::trace!(
                ?flyer_id,
                ?hitter_id,
                ?victim_id,
                "domino effect: queued domino hit"
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
    /// current position. Otherwise the live order receives a new identity
    /// and publishes the new map goal.
    ///
    /// Early-outs if the entity is not currently in a Rolling combat
    /// animation.
    pub(crate) fn update_roll_after_crossing(&mut self, assets: &LevelAssets, entity_id: EntityId) {
        // Cheap early-out: only act while the actor is rolling.
        let Some((roll_seq_id, roll_elem_idx, roll_command)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id)
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

        // The roll command owns its authored destination. Original-game roll updates
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
                .expect("rolling order disappeared before roll-update rewrite");
            current.target_x = dest.x;
            current.target_y = dest.y;
            current.order_id = fresh_id;
            self.world.entities[entity_id]
                .as_mut()
                .expect("rolling actor disappeared before roll-update publication")
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

    /// Reconcile the lifetime of already launched special-strike sequences.
    pub(super) fn tick_enemy_sword_attacks(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let mut flagged: Vec<EntityId> = Vec::new();
        for npc_id in self.world.entities.ai_owner_ids() {
            if self
                .world
                .entities
                .get(npc_id)
                .and_then(Entity::enemy_ai)
                .is_some_and(|ai| ai.pending_special_strike)
            {
                flagged.push(npc_id);
            }
        }
        for npc_id in flagged {
            let has_active = self
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(npc_id, |cmd| {
                    cmd.is_swordstrike() || cmd == crate::element::Command::WaitTimer
                });
            self.reconcile_ai_special_strike(sim, assets, npc_id, has_active);
        }
    }

    /// Propose and launch one strike at the current swordfight decision statement.
    pub(in crate::engine) fn execute_ai_sword_strike_proposal(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let current_frame = self.control.frame_counter;
        if !self.tactical_allows_normal_strikes(owner) {
            return;
        }
        let player_selected = self
            .players
            .tactical
            .seats
            .iter()
            .any(|seat| seat.selection.contains(&owner));
        if player_selected
            && self
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(owner, |command| {
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
            return;
        }

        let attacker = self.expect_entity(owner, "sword-strike proposal owner");
        let ai = attacker
            .enemy_ai()
            .expect("sword-strike proposal requires Enemy AI");
        let weapon_id = ai.hth_weapon_id;
        let target_handle = ai
            .base
            .primary_target
            .expect("sword-strike proposal requires principal");
        let target_id =
            self.expect_entity_id_for_index(target_handle.get(), "sword-strike principal");
        let fighting_ability = fighting_ability_from_profile(
            attacker,
            &assets.profile_manager,
            sim.config().difficulty,
            &self.mission_domain.diplomacy,
        );
        let blood_alcohol = ai.base.blood_alcohol;
        let is_rank_soldier =
            ai.profile(&assets.profile_manager).rank == crate::profiles::ProfileRank::Soldier;
        let attacker_direction = attacker.element_data().direction();
        let attacker_camp = attacker.camp();
        let map = attacker.element_data().position_map();
        let attacker_pos = (map.x, map.y);
        let attacker_elevation = attacker.element_data().position().z;
        let human = attacker
            .human_data()
            .expect("sword-strike owner must be human");
        let is_swordfighting = !human.opponents.is_empty();
        let mut boredom = human.sword_strike_boredom.clone();

        let special_debug = special_strike_lifecycle_debug_matches(current_frame, owner.index());
        if special_debug {
            self.trace_special_strike(
                current_frame,
                owner,
                format_args!("phase=before_proposal target={}", target_id.index()),
            );
        }
        let distance = entity_distance(&self.world.entities, owner, target_id);

        // Select the best strike using the shared proposal logic.
        let attacker_profile = assets
            .profile_manager
            .get_hth_weapon(weapon_id)
            .unwrap_or_else(|| {
                panic!(
                    "authorized sword-strike proposal owner {:?} requires missing HtH weapon {}",
                    owner, weapon_id
                )
            });

        // ── Sprite timing ──────────────────────────────────────────
        // Compute opponent_time_limit from target's sprite.
        // If the target isn't in an active strike animation,
        // time_limit = 1000 (permissive).  Otherwise, take the
        // sprite's frames-from-now-till-action-done (or 1000 if
        // unavailable).
        let sprite_timing_debug =
            opponent_sprite_timing_debug_matches(current_frame, owner.index(), target_id.index());
        let sprite_timing_creation_orders = sprite_timing_debug.then(|| {
            (
                self.world.original_creation_order(owner),
                self.world.original_creation_order(target_id),
            )
        });
        let selected_opponent_time_limit =
            self.enemy_reconsider_sword_strike_time_limit_for_actor(owner, target_id);
        let opponent_time_limit: Option<i16> = self.get_entity(target_id).and_then(|e| {
            let animation = self.live_actor_animation(target_id)?;
            if let Some((owner_creation_order, target_creation_order)) =
                sprite_timing_creation_orders
            {
                Self::trace_opponent_sprite_timing(
                    current_frame,
                    [owner, target_id],
                    [owner_creation_order, target_creation_order],
                    Some(owner),
                    animation,
                    e,
                );
            }
            selected_opponent_time_limit
        });

        // Compute per-strike startup frames from attacker's
        // sprite (`frames_from_start_till_action_done(anim)`).
        let attacker_sprite_frames: Option<[i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES]> = self
            .get_entity(owner)
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
            .get_entity(owner)
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
        let nearby = self.collect_strike_estimation_victims(
            assets,
            owner,
            attacker_pos,
            Some(target_id),
            target_id,
        );

        let ctx = crate::combat::StrikeSelectionContext {
            attacker_profile,
            fighting_ability: fighting_ability,
            blood_alcohol: blood_alcohol,
            is_rank_soldier: is_rank_soldier,
            attacker_direction: attacker_direction,
            attacker_elevation: attacker_elevation,
            attacker_camp: attacker_camp,
            diplomacy: &self.mission_domain.diplomacy,
            is_swordfighting: is_swordfighting,
            opponent_time_limit,
            strike_startup_frames: attacker_sprite_frames,
            parry_startup_frames: parry_startup,
            is_npc: true,
        };
        let debug = super::evaluate::reactive_sword_debug_frame_matches(current_frame)
            .then(|| {
                let creation_order = self.world.original_creation_order(owner);
                super::evaluate::reactive_sword_debug_creation_order_matches(creation_order)
                    .then_some(crate::combat::SwordStrikeProposalDebug {
                        frame: current_frame,
                        victim: owner.index(),
                        victim_creation_order: creation_order,
                        attacker: target_id.index(),
                    })
            })
            .flatten();
        if let Some(debug) = debug {
            self.trace_reactive_sword_enemy_principal(debug, target_id, opponent_time_limit);
        }
        let rng_before = debug.and_then(|_| self.control.rng.original_replay_cursor());
        let mut sweep_rebase = None;
        let proposed = crate::combat::propose_good_sword_strike_with_debug(
            sim,
            &ctx,
            &nearby,
            &mut boredom,
            false,
            false,
            debug,
            &mut sweep_rebase,
        );
        self.apply_strike_selection_sweep_rebase(assets, owner, sweep_rebase);
        if special_debug {
            self.trace_special_strike(
                current_frame,
                owner,
                format_args!("phase=after_proposal result={:?}", proposed),
            );
        }
        if let Some(debug) = debug {
            self.trace_reactive_sword_proposal_boundary(
                debug,
                "enemy_reconsider",
                rng_before,
                &proposed,
            );
        }
        let strike = match proposed {
            Some(crate::combat::ProposedCombatAction::Strike(s)) => Some(s),
            _ => None,
        };

        // Strike selection mutates its boredom history even when no
        // viable strike is selected, so persist it before branching on
        // the proposal result.
        let owner_entity =
            self.expect_entity_mut(owner, "sword-strike proposal owner during selection");
        owner_entity
            .human_data_mut()
            .unwrap_or_else(|| panic!("sword-strike proposal owner {:?} is no longer human", owner))
            .sword_strike_boredom = boredom;

        let strike = match strike {
            Some(s) => s,
            None => return, // No viable strike for this proposal
        };
        let command = strike.to_command();

        // Telegraph attacks against a player-controlled PC with the hulk
        // glow and difficulty-dependent preparation delay. Autonomous PCs
        // are EnemyAi combatants rather than players awaiting a warning,
        // so PC-vs-PC battle missions use the normal immediate cadence.
        let target_is_player_controlled_pc = match self.get_entity(target_id) {
            Some(entity @ Entity::Pc(_)) => entity.accepts_hero_commands(),
            Some(_) => false,
            None => panic!(
                "sword-strike target {:?} disappeared before preparation",
                target_id
            ),
        };

        let wait_time: u32 = if target_is_player_controlled_pc {
            // Start the striking-outline hulk with width 2.
            if let Some(entity) = self.world.entities.get_mut(owner) {
                if let Some(human) = entity.human_data_mut() {
                    human.start_hulk(true, 1.0);
                }
                let elem = entity.element_data_mut();
                elem.current_outline = crate::element::OutlineColorName::Striking;
                elem.outline_width = 2;
            }
            compute_special_strike_preparation_time(sim.config().difficulty, fighting_ability)
        } else {
            0
        };

        if special_debug {
            self.trace_special_strike(
                current_frame,
                owner,
                format_args!(
                    "phase=before_begin strike={:?} command={:?} wait_time={}",
                    strike, command, wait_time,
                ),
            );
        }

        // Flag the pending special strike and cancel movement so the
        // EnemyAi owner stands still during the delay.
        // `begin_special_strike` sets the lifecycle latch and enters the
        // observable legacy special-strike substate; the
        // immediate stop-all side effect stays engine-side so it
        // runs before the new strike sequence is queued.
        self.begin_ai_special_strike(sim, assets, owner);
        self.stop_ai_owner(sim, assets, owner);
        if special_debug {
            self.trace_special_strike_state(current_frame, owner, "after_begin");
        }
        if special_debug {
            self.trace_special_strike_state(current_frame, owner, "after_begin_drain");
        }

        // War-cry remarks for thrusts C/F/G/H/I.  Placed after
        // the state-set + stop-all so the say-order is correct.
        if matches!(
            strike,
            SwordStrike::C | SwordStrike::F | SwordStrike::G | SwordStrike::H | SwordStrike::I
        ) {
            let owner_entity = self.expect_entity(owner, "warcry owner");
            let is_vip = is_vip_from_profile(owner_entity, &assets.profile_manager);
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: if is_vip {
                        crate::ai::Remark::VipWarcry
                    } else {
                        crate::ai::Remark::Warcry
                    },
                    flags: 0,
                },
            );
        }

        // Build sequence: level-1 wait timer (preparation delay),
        // then level-2 interaction (the actual strike command).
        let mut seq = crate::sequence::Sequence::new();

        let mut wait_elem =
            crate::sequence::SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        wait_elem.priority = crate::sequence::SequencePriority::Normal;
        wait_elem.set_property(
            crate::sequence::Field::Timer,
            crate::sequence::FieldValue::Integer(wait_time),
        );
        seq.append_element(wait_elem);

        let principal = *self
            .expect_entity(owner, "strike launch owner")
            .human_data()
            .expect("strike owner must remain human")
            .opponents
            .first()
            .expect("strike launch requires live principal opponent");
        let mut strike_elem = crate::sequence::SequenceElement::new_interaction(
            2,
            command,
            Some(owner),
            Some(principal),
        );
        strike_elem.priority = crate::sequence::SequencePriority::Preference;
        seq.append_element(strike_elem);

        self.launch_sequence(sim, assets, seq);

        if special_debug {
            self.trace_special_strike_state(current_frame, owner, "after_launch");
        }

        tracing::debug!(
            soldier = ?owner,
            target = ?target_id,
            ?command,
            ?strike,
            distance,
            "Enemy AI sword strike sequence launched"
        );
    }

    /// Run one human's concussion prelude and close a natural/script wake
    /// synchronously before the owner's base actor update begins.
    pub(crate) fn tick_concussion_healing_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
    ) {
        let mut recover = None;
        let is_sherwood = self.is_sherwood(&assets.profile_manager);
        let naturally_woke = {
            let entity = self
                .world
                .entities
                .expect_entity_mut(owner, format_args!("concussion owner from its legacy slot"));
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
                        entity.element_data().posture(),
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
            self.launch_element(sim, assets, element);
        }

        let owner_has_ai = self
            .world
            .entities
            .get(owner)
            .is_some_and(|entity| entity.ai_controller().is_some());
        if naturally_woke && owner_has_ai {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
            );
        }
        if naturally_woke {
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
    use crate::coordinates::{MapPoint, MapVec, WorldPoint3D, WorldVec3D};
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, Camp, ElementData, ElementKind, HumanData, NpcData,
        PcData, SoldierData,
    };
    use crate::position_interface::SectorHandle;
    use crate::sequence::SequenceElement;

    #[test]
    fn flight_start_disables_collision_and_advances_serialized_increment() {
        let mut engine = EngineInner::new();
        let victim = engine.add_test_entity(falling_pushed_soldier(false));
        install_falling_pushed_order(&mut engine, victim);
        let motion =
            engine.perform_combat_flight_position(victim, crate::sprite::MotionState::Start);
        assert_eq!(motion, crate::sprite::MotionState::Start);
        let entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            entity.element_data().position_map(),
            MapPoint::new(15.0, 20.0)
        );
        assert!(!entity.position_iface().is_anti_collision_on());
    }

    #[test]
    fn perform_flight_stops_on_first_tick_of_final_sprite_frame() {
        assert!(perform_flight_stops_before_position_update(0, 6, 7));
        assert!(!perform_flight_stops_before_position_update(1, 6, 7));
        assert!(!perform_flight_stops_before_position_update(0, 5, 7));
    }

    #[test]
    fn stopped_rolling_preserves_direction_through_shared_recompute() {
        let here = MapPoint::new(1_208.699_5, 1_156.473_4);
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
        let mut element = ElementData::from_initial_posture(Posture::Flying);
        element.kind = ElementKind::ActorSoldier;
        element.active = true;
        element.sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            frame_ids: vec![0, 1],
            ..Default::default()
        }]);
        element.set_position(WorldPoint3D::new(10.0, 20.0, 0.0));
        element.set_position_map(MapPoint::new(10.0, 20.0));
        element.set_layer(1);
        element.set_sector(SectorHandle::new(2));
        element
            .sprite
            .position_iface
            .set_layer_goal(crate::position_interface::Layer::new(3).unwrap());
        element.sprite.position_iface.set_flight_goal_and_increment(
            WorldPoint3D::new(15.0, 20.0, 0.0),
            WorldVec3D::new(5.0, 0.0, 0.0),
            SectorHandle::new(4),
            SectorHandle::new(4).and_then(SectorHandle::arena_index),
        );
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData {
                action_state: ActionState::WaitingSword,
                ..Default::default()
            },
            human: HumanData::default(),
            npc: NpcData {
                life_points: if dead { 0 } else { 50 },
                ..Default::default()
            },
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..Default::default()
            },
        })
    }

    fn install_falling_pushed_order(engine: &mut EngineInner, victim: EntityId) {
        let assets = LevelAssets::new();
        let damage =
            SequenceElement::new_damage(1, Command::ReceiveSwordDamage, Some(victim), None, 20, 0);
        let sequence = engine.orders.sequence_manager.insert_element(damage);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        let order_id =
            engine.push_new_order(sequence, 0, OrderType::FallingPushedUpright, 0.0, 0.0);
        engine.select_sequence_element(victim, Some((sequence, 0)));
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            sequence,
            0,
        );
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

    #[test]
    fn in_progress_falling_push_consumes_serialized_position_without_reconstruction() {
        let mut engine = EngineInner::new();
        let victim = engine.add_test_entity(falling_pushed_soldier(false));
        install_falling_pushed_order(&mut engine, victim);
        let position = engine.get_entity_mut(victim).unwrap().position_iface_mut();
        position.set_flight_goal_and_increment(
            WorldPoint3D::new(4.0, 14.0, 0.0),
            WorldVec3D::new(-2.0, -2.0, 0.0),
            SectorHandle::new(4),
            SectorHandle::new(4).and_then(SectorHandle::arena_index),
        );
        engine.perform_combat_flight_position(victim, crate::sprite::MotionState::InProgress);
        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().position(),
            WorldPoint3D::new(8.0, 18.0, 0.0)
        );
        engine.perform_combat_flight_position(victim, crate::sprite::MotionState::Terminated);
        let entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            entity.element_data().position(),
            WorldPoint3D::new(4.0, 14.0, 0.0)
        );
        assert_eq!(entity.element_data().layer(), 3);
        assert_eq!(entity.element_data().sector(), SectorHandle::new(4));
    }

    fn falling_ladder_pc(life_points: i16) -> Entity {
        let mut element = ElementData::from_initial_posture(Posture::Flying);
        element.kind = ElementKind::ActorPc;
        element.active = true;
        element.set_position(WorldPoint3D::new(10.0, 20.0, 0.0));
        element.set_position_map(MapPoint::new(10.0, 20.0));
        element.set_layer(1);
        element.set_sector(SectorHandle::new(2));
        element.sprite.position_iface.set_flight_goal_and_increment(
            WorldPoint3D::new(15.0, 20.0, 0.0),
            WorldVec3D::new(5.0, 0.0, 0.0),
            None,
            None,
        );
        let mut actor = ActorData {
            action_state: ActionState::Moving,
            wait_time: 1,
            ..Default::default()
        };
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
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "flight.scs",
        ));
        let door_index = engine.script_domains.interactables.doors.len();
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_out: MapPoint::new(15.0, 20.0),
                layer_out: 3,
                sector_out: crate::sector::SectorNumber::new(4),
                sector_out_index: crate::fast_find_grid::SectorIndex::new(1),
                ..Default::default()
            });
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        let sector_number = crate::sector::SectorNumber::new(2);
        level
            .sector_number_map
            .insert(sector_number, level.sectors.len());
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 1,
            sector_number,
            door_index: None,
            lift_type: Some(crate::sector::LiftType::Ladder),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: Some(door_index as u32),
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        let low_number = crate::sector::SectorNumber::new(4);
        level
            .sector_number_map
            .insert(low_number, level.sectors.len());
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION,
            layer: 3,
            sector_number: low_number,
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        let assets = LevelAssets::new();
        let damage =
            SequenceElement::new_damage(1, Command::ReceiveSwordDamage, Some(victim), None, 20, 0);
        let sequence = engine.orders.sequence_manager.insert_element(damage);
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        let order_id = engine.push_new_order(sequence, 0, OrderType::FallingLadderWall, 0.0, 0.0);
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 0)
            .unwrap()
            .orders
            .front_mut()
            .unwrap()
            .destination_3d = [15.0, 20.0, 0.0];
        engine.select_sequence_element(victim, Some((sequence, 0)));
        engine.element_in_progress(
            &crate::sim_rng::test_context(),
            &assets,
            &mut Vec::new(),
            sequence,
            0,
        );
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
        let victim = engine.add_test_entity(falling_ladder_pc(200));
        {
            let actor = engine
                .get_entity_mut(victim)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.wait_time = 1;
            actor.seek_target = Some(victim);
            actor.post_seek_sequence = Some(crate::sequence::Sequence::new().into_post_seek());
        }
        install_falling_ladder_order(&mut engine, victim);

        engine.execute_ladder_fall_position(
            &sim,
            &assets,
            victim,
            crate::sprite::MotionState::InProgress,
        );

        let actor = engine.get_entity(victim).unwrap().actor_data().unwrap();
        assert_eq!(actor.wait_time, 0);
        assert_eq!(actor.seek_target, Some(victim));
        assert!(actor.post_seek_sequence.is_some());
    }

    #[test]
    fn perform_flight_preserves_ready_for_takeoff_position_cache() {
        let mut engine = EngineInner::new();
        let mut victim = falling_pushed_soldier(false);
        let goal_sector_index = crate::fast_find_grid::SectorIndex::new(44).unwrap();
        victim.position_iface_mut().set_flight_goal_and_increment(
            WorldPoint3D::new(30.0, 45.0, 5.0),
            WorldVec3D::new(1.25, 0.75, 0.5),
            SectorHandle::new(4).map(|sector| sector.with_arena_index(goal_sector_index)),
            Some(goal_sector_index),
        );
        let victim_id = engine.add_test_entity(victim);
        install_falling_pushed_order(&mut engine, victim_id);
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::InProgress);
        let state = engine
            .get_entity(victim_id)
            .unwrap()
            .position_iface()
            .v48_serialized_state();
        assert_eq!(state.computed_position.bits(), 7);
        assert_eq!(
            state.computed_increment,
            crate::position_interface::IncrementComputed::INCREMENT
        );
        assert_eq!(state.goal, WorldPoint3D::new(30.0, 45.0, 5.0));
        assert_eq!(state.increment, WorldVec3D::new(1.25, 0.75, 0.5));
        assert_eq!(state.sector_goal, SectorHandle::new(4));
        assert_eq!(state.sector_goal_index, Some(goal_sector_index));
        assert_eq!(state.position, WorldPoint3D::new(11.25, 20.75, 0.5));
        assert_eq!(state.map, MapPoint::new(11.25, 20.25));
        assert_eq!(state.sprite, MapPoint::new(11.0, 20.0));
    }

    #[test]
    fn fatal_push_goal_preserves_flying_pose_until_animation_terminates() {
        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(falling_pushed_soldier(true));
        let goal_sector_index = crate::fast_find_grid::SectorIndex::new(44).unwrap();
        engine
            .get_entity_mut(victim_id)
            .unwrap()
            .position_iface_mut()
            .set_flight_goal_and_increment(
                WorldPoint3D::new(15.0, 20.0, 0.0),
                WorldVec3D::new(5.0, 0.0, 0.0),
                SectorHandle::new(4).map(|sector| sector.with_arena_index(goal_sector_index)),
                Some(goal_sector_index),
            );
        install_falling_pushed_order(&mut engine, victim_id);
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::InProgress);
        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().posture(), Posture::Flying);
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
            victim.position_iface().v48_serialized_state().increment,
            WorldVec3D::new(5.0, 0.0, 0.0)
        );
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::Terminated);
        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(
            victim.element_data().position_map(),
            MapPoint::new(15.0, 20.0)
        );
        assert_eq!(victim.element_data().layer(), 3);
        assert_eq!(
            victim
                .element_data()
                .sector()
                .and_then(SectorHandle::arena_index),
            Some(goal_sector_index)
        );
        assert_eq!(
            victim.position_iface().v48_serialized_state().increment,
            WorldVec3D::ZERO
        );
    }

    #[test]
    fn terminated_combat_flight_snaps_before_its_falling_order_retires() {
        let near_goal = MapPoint::new(1142.2267, 1230.4998);
        let exact_goal = MapPoint::new(1142.2262, 1230.5006);
        let mut entity = falling_pushed_soldier(false);
        entity
            .element_data_mut()
            .set_material(crate::element::GameMaterial::Grass);
        entity.element_data_mut().set_position_map(near_goal);
        entity.position_iface_mut().new_move();
        entity.position_iface_mut().set_flight_goal_and_increment(
            WorldPoint3D::new(exact_goal.x, exact_goal.y, 0.0),
            WorldVec3D::ZERO,
            SectorHandle::new(4),
            SectorHandle::new(4).and_then(SectorHandle::arena_index),
        );
        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(entity);
        install_falling_pushed_order(&mut engine, victim_id);
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::Terminated);
        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().position_map(), exact_goal);
        assert_eq!(victim.position_iface().old_map_position(), near_goal);
        assert!(victim.position_iface().is_moving_map());
        assert_eq!(
            victim.element_data().material(),
            crate::element::GameMaterial::Grass
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(&engine.world.entities, victim_id)
                .is_some()
        );
    }

    #[test]
    fn completed_combat_flight_preserves_authored_z_and_material_on_an_installed_slope() {
        let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(1);
        obstacle.top_plane_points = [
            [0.0, 0.0, 1_711.937_4],
            [1.0, 0.0, 1_712.386_2],
            [0.0, 1.0, 1_710.774],
        ];
        obstacle.material = 3;
        let plane =
            crate::position_interface::PlaneZCoeffs::from_plane_points(&obstacle.top_plane_points);
        // Adding world Z to map Y and subtracting it again rounds the map
        // coordinate. Reinstalling the already-current plane at landing would
        // consequently derive a different elevation from that rounded map Y.
        let goal_map = MapPoint::new(1_833.459, 1_617.051_6);
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
        entity.position_iface_mut().set_flight_goal_and_increment(
            WorldPoint3D::new(goal_map.x, goal_map.y + goal_z, goal_z),
            WorldVec3D::ZERO,
            SectorHandle::new(4),
            SectorHandle::new(4).and_then(SectorHandle::arena_index),
        );

        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(entity);
        install_falling_pushed_order(&mut engine, victim_id);
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::Terminated);

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
            "takeoff preparation installs the goal obstacle without changing material"
        );
    }

    #[test]
    fn completed_combat_flight_snaps_at_its_owner_boundary() {
        let near_x = 696.702_45_f32;
        let exact_x = f32::from_bits(near_x.to_bits() + 1);
        let near_goal = MapPoint::new(near_x, 2_077.693_8);
        let exact_goal = MapPoint::new(exact_x, 2_077.694_6);
        let mut entity = falling_pushed_soldier(false);
        entity.element_data_mut().set_position_map(near_goal);
        entity.position_iface_mut().new_move();
        entity.position_iface_mut().set_flight_goal_and_increment(
            WorldPoint3D::new(exact_goal.x, exact_goal.y, 0.0),
            WorldVec3D::ZERO,
            SectorHandle::new(4),
            SectorHandle::new(4).and_then(SectorHandle::arena_index),
        );
        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(entity);
        install_falling_pushed_order(&mut engine, victim_id);
        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::Terminated);
        let flights =
            crate::movement_diagnostics::take_parity_flight_capture().expect("capture started");
        let _ =
            crate::movement_diagnostics::take_parity_movement_capture().expect("capture started");
        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().position_map(), exact_goal);
        assert_eq!(victim.position_iface().old_map_position(), near_goal);
        assert!(victim.position_iface().is_moving_map());
        assert_eq!(flights.len(), 1);
        let flight = &flights[0];
        assert_eq!(flight.entity, victim_id);
        assert_eq!(flight.raw_post_position_map.x.bits, near_x.to_bits());
        assert_eq!(flight.post_position_map.x.bits, exact_x.to_bits());
        assert_eq!(flight.goal.x.bits, exact_x.to_bits());
        assert!(flight.snapped_to_goal);
    }

    #[test]
    fn owner_scoped_push_flight_advances_only_the_selected_creation_slot() {
        let mut engine = EngineInner::new();
        let earlier = engine.add_test_entity(falling_pushed_soldier(false));
        let later = engine.add_test_entity(falling_pushed_soldier(false));
        install_falling_pushed_order(&mut engine, earlier);
        install_falling_pushed_order(&mut engine, later);

        crate::movement_diagnostics::begin_parity_movement_capture();
        engine.perform_combat_flight_position(earlier, crate::sprite::MotionState::InProgress);

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
            "a later actor must retain its pre-update position"
        );

        engine.perform_combat_flight_position(later, crate::sprite::MotionState::InProgress);
        assert_eq!(
            engine
                .get_entity(later)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(15.0, 20.0)
        );
        let flights =
            crate::movement_diagnostics::take_parity_flight_capture().expect("capture started");
        let _ =
            crate::movement_diagnostics::take_parity_movement_capture().expect("capture started");
        assert_eq!(flights.len(), 2);
        assert_eq!(flights[0].entity, earlier);
        assert_eq!(flights[1].entity, later);
        assert!(!flights[0].snapped_to_goal);
        assert_eq!(
            flights[0].raw_post_position_map.x.bits,
            flights[0].post_position_map.x.bits
        );
        assert!(
            crate::movement_diagnostics::take_parity_flight_capture().is_none(),
            "taking one frame's flight diagnostics must isolate the next frame"
        );
    }

    #[test]
    fn ladder_arrival_returns_terminated_from_owner_execute_tail() {
        let sim = crate::sim_rng::test_context();
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles
            .soldiers
            .push(crate::profiles::SoldierProfile::default());
        profiles.hth_weapons.push(Default::default());
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..Default::default()
        };
        let mut entity = falling_pushed_soldier(false);
        let Entity::Soldier(soldier) = &mut entity else {
            unreachable!()
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi {
            hth_weapon_id: 1,
            ..Default::default()
        }));
        entity
            .position_iface_mut()
            .set_layer_goal(crate::position_interface::Layer::ZERO);
        let actor = entity.actor_data_mut().unwrap();
        actor.wait_time = 1;
        actor.continuation.motion_state = crate::sprite::MotionState::Start;
        let mut engine = EngineInner::new();
        let victim = engine.add_test_entity(entity);
        install_falling_ladder_order(&mut engine, victim);
        let installed = engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .installed_order;
        let motion = engine.execute_ladder_fall_position(
            &sim,
            &assets,
            victim,
            crate::sprite::MotionState::InProgress,
        );
        assert_eq!(motion, crate::sprite::MotionState::Terminated);
        let entity = engine.get_entity(victim).unwrap();
        let actor = entity.actor_data().unwrap();
        assert_eq!(
            actor.continuation.motion_state,
            crate::sprite::MotionState::Start
        );
        assert_eq!(
            actor.installed_order.map(|order| order.order_id),
            installed.map(|order| order.order_id)
        );
        assert_eq!(entity.element_data().layer(), 3);
        assert_eq!(entity.element_data().sector(), SectorHandle::new(4));
        assert_eq!(
            entity.npc_data().unwrap().eye_status,
            crate::element::EyeStatus::DieOrGetUnconscious
        );
        assert_eq!(
            entity.position_iface().layer_goal(),
            crate::position_interface::Layer::ZERO
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
        let victim = engine.add_test_entity(falling_ladder_pc(50));
        let mut opponent_entity = falling_pushed_soldier(false);
        let Entity::Soldier(soldier) = &mut opponent_entity else {
            unreachable!()
        };
        let enemy_ai = crate::ai_enemy::EnemyAi {
            hth_weapon_id: 1,
            ..Default::default()
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(enemy_ai));
        let opponent = engine.add_test_entity(opponent_entity);
        {
            let ai = engine
                .get_entity_mut(opponent)
                .unwrap()
                .ai_controller_mut()
                .unwrap();
            ai.set_ai_state(crate::ai::AiState::Attacking);
            ai.current_substate = crate::ai::Substate::AttackingSwordfight;
            ai.primary_target = Some(crate::ai::AiEntityHandle::new(victim.index()));
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

        engine.execute_ladder_fall_position(
            &sim,
            &assets,
            victim,
            crate::sprite::MotionState::InProgress,
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
        let victim = engine.add_test_entity(falling_ladder_pc(200));
        let opponent = engine.add_test_entity(falling_pushed_soldier(false));
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

        engine.execute_ladder_fall_position(
            &sim,
            &assets,
            victim,
            crate::sprite::MotionState::InProgress,
        );

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
    }

    #[test]
    fn combat_flight_in_progress_can_overshoot_goal_before_animation_terminates() {
        let mut entity = falling_pushed_soldier(false);
        entity.position_iface_mut().set_flight_goal_and_increment(
            WorldPoint3D::new(14.0, 20.0, 0.0),
            WorldVec3D::new(5.0, 0.0, 0.0),
            SectorHandle::new(4),
            SectorHandle::new(4).and_then(SectorHandle::arena_index),
        );
        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(entity);
        install_falling_pushed_order(&mut engine, victim_id);
        let motion = engine
            .perform_combat_flight_position(victim_id, crate::sprite::MotionState::InProgress);
        assert_eq!(motion, crate::sprite::MotionState::InProgress);
        assert_eq!(
            engine
                .get_entity(victim_id)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(15.0, 20.0)
        );
        engine.perform_combat_flight_position(victim_id, crate::sprite::MotionState::Terminated);
        assert_eq!(
            engine
                .get_entity(victim_id)
                .unwrap()
                .element_data()
                .position_map(),
            MapPoint::new(14.0, 20.0)
        );
    }

    #[test]
    fn knockout_push_goal_preserves_flying_pose_until_animation_terminates() {
        let mut entity = falling_pushed_soldier(false);
        entity.human_data_mut().unwrap().unconscious = true;
        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(entity);
        install_falling_pushed_order(&mut engine, victim_id);
        let motion = engine
            .perform_combat_flight_position(victim_id, crate::sprite::MotionState::InProgress);
        assert_eq!(motion, crate::sprite::MotionState::InProgress);
        let victim = engine.get_entity(victim_id).unwrap();
        assert_eq!(victim.element_data().posture(), Posture::Flying);
        assert_eq!(
            victim.actor_data().unwrap().action_state,
            ActionState::WaitingSword
        );
    }

    #[test]
    fn push_rechecks_current_relationship_before_queueing_enter_swordfight() {
        // Only push-strike execution walks the sword-strike victim list at
        // terminated motion, so the completing strike has to be a push.
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
        let attacker_id = engine.add_test_entity(attacker);
        let victim_id = engine.add_test_entity(victim);

        engine
            .get_entity_mut(attacker_id)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .sword_sweep
            .victims = vec![victim_id];
        engine.complete_melee_strike(
            &crate::sim_rng::test_context(),
            &assets,
            attacker_id,
            SwordStrike::A,
            Some(1),
        );
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);

        if let Entity::Soldier(soldier) = engine.get_entity_mut(victim_id).unwrap() {
            soldier.soldier.cached_camp = Camp::Royalists;
        }
        engine
            .get_entity_mut(attacker_id)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .sword_sweep
            .victims = vec![victim_id];
        engine.complete_melee_strike(
            &crate::sim_rng::test_context(),
            &assets,
            attacker_id,
            SwordStrike::A,
            Some(1),
        );

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
