//! Melee combat — sword fighting, damage application, knockouts, and death.
//!
//! Covers the Instruct/Execute paths for sword commands, hit detection
//! (distance/angle), protection and damage formulas, and the
//! damage-sequence-element dispatch.
//!
//! ## Combat flow
//!
//! ### Enemy AI strikes
//! The enemy AI (`engine/ai.rs`) transitions soldiers into `AttackingSwordfight`
//! substate when close to a PC. `tick_enemy_sword_attacks` proposes a strike
//! with the sprite-frame timing inputs, then launches the wait-timer
//! plus interaction sequence shape used by the malignity AI.
//!
//! ### Sequence-driven strikes (PCs and scripted)
//! When a `Command::SwordstrikeThrustA..I` sequence element is dispatched:
//! 1. `dispatch_sword_strike` appends the Original-style animation order with
//!    its antagonist to the selected sequence element.
//! 2. The actor's Execute slot derives the strike from that order and applies
//!    damage on the sprite's `MotionState::Done` pulse.
//! 3. `MotionState::Terminated` terminates the owning sequence element.
//!
//! ### Damage application
//! All damage flows through `combat::receive_sword_damage` (or the piercing/hit
//! variants).  After damage, this module checks for death and knockout state
//! transitions.
//!
//! ## Ported features
//!
//! - **Straight strikes**: Distance-based hit detection (ExecuteStraightSwordStrike)
//! - **Lateral strikes**: Angular sweep with per-frame victim checking
//! - **Push strikes**: Rectangle-based area hit detection (front distance + side width)
//! - **Circle/half-circle strikes**: Angular sweep similar to lateral
//! - **Opponent lists**: Per-entity opponent tracking, principal opponent selection
//! - **Push/stumble effects**: Repulsion movement from push/circle/charge strikes
//! - **Experience points**: Sword kill XP with skill-difference bonus
//! - **PC coma/amulet**: Death-save mechanic consuming amulets
//! - **Combat animations**: Posture/action-state-based animation selection
//!
//! ## Remaining simplifications
//!
//! - **Sprite-driven timing**: Hit detection is driven by sprite `MotionState::Done`
//!   (the action_done_frame in sprite data).  Falls back to fixed
//!   the sprite-authored action-done frame.

use super::*;
use crate::combat::{self, ConcussionContext, ConcussionOutcome};
use crate::element::{ActionState, Entity, EntityId, EyeStatus, Posture};
use crate::entities::Entities;
use crate::weapons::SwordStrike;
#[cfg(test)]
use crate::{element::Command, sequence::SequenceElementData};

fn sword_damage_lifecycle_debug_matches(frame: u32, creation_order: u32) -> bool {
    if std::env::var_os("PARITY_DEBUG_SWORD_DAMAGE_LIFECYCLE").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for sword-damage lifecycle diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_SWORD_DAMAGE_LIFECYCLE_FRAME")
        .is_none_or(|expected| expected == frame)
        && parse_filter("PARITY_DEBUG_SWORD_DAMAGE_LIFECYCLE_CREATION_ORDER")
            .is_none_or(|expected| expected == creation_order)
}

impl EngineInner {
    pub(in crate::engine) fn trace_reactive_sword_topology(
        &self,
        stage: &'static str,
        owner: EntityId,
        focus: Option<(crate::sequence::SequenceId, usize)>,
    ) {
        let frame = self.control.frame_counter;
        if !evaluate::reactive_sword_debug_frame_matches(frame) {
            return;
        }
        if self.get_entity(owner).is_none() {
            return;
        }
        let creation_order = self.world.original_creation_order(owner);
        if !evaluate::reactive_sword_debug_creation_order_matches(creation_order) {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let selected = manager.current_element_for_actor(owner);
        let current_order =
            manager
                .current_order_for_actor(owner)
                .map(|(sequence_id, element_index, order)| {
                    (
                        sequence_id,
                        element_index,
                        order.order_type,
                        order.order_id,
                        order.done,
                    )
                });
        let graph = manager
            .sequences_iter()
            .flat_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .filter(move |(_, element)| element.owner == Some(owner))
                    .map(move |(element_index, element)| {
                        (
                            sequence.id,
                            element_index,
                            element.command,
                            element.state,
                            element.priority,
                            element.postponed_element_index,
                            element.cross_postponed,
                            manager.is_registered_to_go(sequence.id, element_index),
                            element
                                .orders
                                .iter()
                                .map(|order| (order.order_type, order.order_id, order.done))
                                .collect::<Vec<_>>(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        let actor = self.get_entity(owner).and_then(|entity| {
            entity.actor_data().map(|actor| {
                (
                    actor.action_state,
                    actor
                        .installed_order
                        .as_ref()
                        .map(|order| (order.order_type, order.order_id)),
                )
            })
        });
        eprintln!(
            "[REACTIVE_SWORD frame={frame} co={creation_order} victim={} phase=topology stage={stage} focus={focus:?} selected={selected:?} current_order={current_order:?} actor={actor:?} graph={graph:?}]",
            owner.index(),
        );
    }

    fn trace_sword_damage_lifecycle(
        &self,
        stage: &'static str,
        victim: EntityId,
        attacker: Option<EntityId>,
        strike: Option<SwordStrike>,
        damage_element: Option<(crate::sequence::SequenceId, usize)>,
        result: Option<combat::SwordDamageResult>,
    ) {
        let frame = self.control.frame_counter;
        if self.get_entity(victim).is_none() {
            return;
        }
        let creation_order = self.world.original_creation_order(victim);
        if !sword_damage_lifecycle_debug_matches(frame, creation_order) {
            return;
        }

        let selected = self
            .orders
            .sequence_manager
            .current_element_for_actor(victim);
        let selected_graph = selected.and_then(|(sequence_id, _)| {
            self.orders
                .sequence_manager
                .get_sequence(sequence_id)
                .map(|sequence| {
                    sequence
                        .elements
                        .iter()
                        .enumerate()
                        .map(|(element_index, element)| {
                            (
                                element_index,
                                element.command,
                                element.state,
                                element.priority,
                                element
                                    .orders
                                    .iter()
                                    .map(|order| (order.order_type, order.order_id, order.done))
                                    .collect::<Vec<_>>(),
                                element.cross_postponed,
                            )
                        })
                        .collect::<Vec<_>>()
                })
        });
        let damage_orders = damage_element.and_then(|(sequence_id, element_index)| {
            self.orders
                .sequence_manager
                .get_element(sequence_id, element_index)
                .map(|element| {
                    element
                        .orders
                        .iter()
                        .map(|order| (order.order_type, order.order_id, order.done))
                        .collect::<Vec<_>>()
                })
        });
        let actor_state = self.get_entity(victim).and_then(|entity| {
            entity.actor_data().map(|actor| {
                (
                    actor.action_state,
                    actor
                        .installed_order
                        .as_ref()
                        .map(|order| (order.order_type, order.order_id)),
                )
            })
        });
        let owner_work = |owner: EntityId| {
            self.get_entity(owner)
                .and_then(Entity::ai_controller)
                .map(|ai| {
                    (
                        ai.outbox.reentrant.owner_work.len(),
                        ai.outbox.reentrant.self_stimuli.len(),
                        ai.outbox.reentrant.cross_npc_actions.len(),
                    )
                })
        };
        let victim_owner_work = owner_work(victim);
        let attacker_owner_work = attacker.and_then(owner_work);
        let rng_cursor = self.control.rng.original_replay_cursor();
        tracing::trace!(
            target: "parity_sword_damage_lifecycle",
            frame,
            stage,
            ?victim,
            creation_order,
            ?attacker,
            ?strike,
            ?damage_element,
            ?result,
            ?selected,
            ?selected_graph,
            ?damage_orders,
            ?actor_state,
            ?rng_cursor,
            ?victim_owner_work,
            ?attacker_owner_work,
            "sword-damage lifecycle"
        );
    }
}

#[cfg(test)]
thread_local! {
    static CAPTURED_STRIKE_WARNINGS: std::cell::RefCell<Option<Vec<(EntityId, EntityId)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(in crate::engine) fn capture_strike_warnings<R>(
    f: impl FnOnce() -> R,
) -> (R, Vec<(EntityId, EntityId)>) {
    CAPTURED_STRIKE_WARNINGS.with(|captured| {
        assert!(captured.borrow().is_none(), "nested strike-warning capture");
        *captured.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let warnings = CAPTURED_STRIKE_WARNINGS.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .expect("strike-warning capture disappeared")
    });
    (result, warnings)
}

#[cfg(test)]
fn record_strike_warning(attacker: EntityId, victim: EntityId) {
    CAPTURED_STRIKE_WARNINGS.with(|captured| {
        if let Some(warnings) = captured.borrow_mut().as_mut() {
            warnings.push((attacker, victim));
        }
    });
}

// ─── Constants ──────────────────────────────────────────────────────

/// Legacy constructor default retained for tests that explicitly exercise
/// SoldierData initialization. Runtime concussion healing requires the real
/// profile and never substitutes this value.
#[cfg(test)]
const SOLDIER_CONCUSSION_HEALING_SPEED: u16 = 300;

/// Per-entity concussion healing speed.
///
/// Reads the wake-up value from the PC's character profile (or the
/// soldier's profile for soldier entities). Civilians fall through to a
/// shared default (`CIVILIAN_CONCUSSION_HEALING_SPEED`).
pub fn concussion_healing_speed_for_entity(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> u16 {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.wake_up)
            .unwrap_or_else(|| {
                panic!(
                    "PC concussion healing requires missing character profile {:?}",
                    pc.pc.profile_index
                )
            }),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.wake_up)
            .unwrap_or_else(|| {
                panic!(
                    "soldier concussion healing requires missing soldier profile {:?}",
                    s.soldier.soldier_profile_index
                )
            }),
        _ => combat::CIVILIAN_CONCUSSION_HEALING_SPEED,
    }
}

/// Maximum elevation difference for swordfight engagement.
const MAX_ELEVATION_SWORDFIGHT: f32 = 40.0;

/// Inverse sword-fight aspect ratio.  This is `1.0` in the shipping
/// game — Eugen Systems disabled the isometric correction for sword
/// combat, so `StretchY(INVERSE_SWORDFIGHT_ASPECT_RATIO)` is a no-op.
/// An earlier revision of this port used `INVERSE_ASPECT_RATIO`
/// (1.7434), which was wrong — see
/// [`crate::position_interface::INVERSE_SWORDFIGHT_ASPECT_RATIO`].
const INVERSE_SWORDFIGHT_ASPECT_RATIO: f32 =
    crate::position_interface::INVERSE_SWORDFIGHT_ASPECT_RATIO;

/// Isometric aspect ratio.
const ASPECT_RATIO: f32 = crate::position_interface::ASPECT_RATIO;

/// Concussion level above which the "stunned" animation plays after push.
const STUNNING_THRESHOLD: u16 = 40;

/// Map-unit radius used by `ApplyDominoEffect` to scan for nearby
/// upright actors during a punch flight.
const DOMINO_DISTANCE: f32 = 15.0;

/// Concussion damage propagated to each domino-effect victim. Lands in
/// the concussion field of the receive-hit-damage sequence element;
/// the damage field stays `0`.
const DOMINO_DAMAGE: u16 = 3;

/// Bundled info about a push-type strike, passed to `apply_push_effect`.
struct PushStrikeInfo {
    repulsion: u16,
}

/// Tiredness threshold above which the SWORDSTRIKE_TIRED animation plays.
const TIREDNESS_WEAK_THRESHOLD: u16 = 100;

/// Belt-height Z offsets per posture.
const HUMAN_ELEVATION_BELT_UPRIGHT: f32 = 25.0;
const RIDER_ELEVATION_BELT_UPRIGHT: f32 = 30.0;

/// Compute the 3D belt point of a Human entity for SIGHTOBSTACLE_SOLID
/// sight checks.  Take the actor's 3D position and bump z by a
/// posture-dependent offset.  `position3d` already lives in world
/// ground-plane coords (the invariant `pos.y = map.y + pos.z` is
/// established in `position_interface::position_3d_from_map`), so x/y
/// are kept and only z is offset.
fn compute_belt_point(entity: &crate::element::Entity) -> crate::coordinates::WorldPoint3D {
    let pos = entity.element_data().position();
    let posture = entity.element_data().posture;
    let is_rider = entity.soldier_data().map(|s| s.rider).unwrap_or(false);

    let z_offset = match posture {
        Posture::Upright
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::OnLadder
        | Posture::OnWall
        | Posture::Flying => {
            if is_rider {
                RIDER_ELEVATION_BELT_UPRIGHT
            } else {
                HUMAN_ELEVATION_BELT_UPRIGHT
            }
        }
        Posture::OnShoulders => 65.0,
        Posture::Carried => 55.0,
        Posture::Sitting | Posture::Crouched | Posture::SimulatingBeggar | Posture::Tree => 10.0,
        Posture::Lying
        | Posture::Dead
        | Posture::DeadBack
        | Posture::StuckUnderNet
        | Posture::Tied => 5.0,
        // Undefined / Unused never appear at runtime (asserted in default arm);
        // treat as Upright since that's the load-time default.
        Posture::Undefined | Posture::Unused => HUMAN_ELEVATION_BELT_UPRIGHT,
    };

    crate::coordinates::WorldPoint3D {
        x: pos.x,
        y: pos.y,
        z: pos.z + z_offset,
    }
}

/// Number of frames per parry.
pub(crate) const TIME_TO_STAY_IN_PARRY_MODE: u16 = 30;

/// PC sword-walk pinch-abort corridor:
///
/// ```text
/// MAX_BLOCKING_ENEMIES_LATERAL_DISTANCE = 70
/// MIN_BLOCKING_ENEMY_FORWARD_DISTANCE   =  5
/// MAX_BLOCKING_ENEMY_FORWARD_DISTANCE   = 30
/// ```
///
/// Used by [`enemies_are_blocking_my_movement`].
const MAX_BLOCKING_ENEMIES_LATERAL_DISTANCE: f32 = 70.0;
const MIN_BLOCKING_ENEMY_FORWARD_DISTANCE: f32 = 5.0;
const MAX_BLOCKING_ENEMY_FORWARD_DISTANCE: f32 = 30.0;

/// Returns `true` when at least two of this PC's current swordfight
/// opponents pinch its forward movement corridor from opposite sides.
///
/// Called once per tick during `WALKING_WITH_SWORD` /
/// `RUNNING_WITH_SWORD` — when it returns true the PC's current
/// sequence element is marked `Impossible` so the PC stops and faces
/// the crowd instead of trying to slip between the two blockers.
///
/// The forward vector is `position_map - old_position_map`, with Y
/// stretched by `INVERSE_ASPECT_RATIO` to compensate for the isometric
/// projection.  Each opponent's delta is projected onto that forward
/// unit vector and onto its left-hand normal; the smallest positive
/// (left) and smallest negative-magnitude (right) lateral distances
/// among opponents whose forward projection falls inside
/// `[MIN_BLOCKING_ENEMY_FORWARD_DISTANCE, MAX_BLOCKING_ENEMY_FORWARD_DISTANCE]`
/// are summed.  If that sum is below
/// `MAX_BLOCKING_ENEMIES_LATERAL_DISTANCE`, two enemies are close
/// enough on either side to count as a pinch and the PC is blocked.
///
/// Returns `false` when the PC has fewer than two opponents.
pub(super) fn enemies_are_blocking_my_movement(
    entities: &crate::entities::Entities,
    entity_id: EntityId,
) -> bool {
    use crate::position_interface::INVERSE_ASPECT_RATIO;

    let Some(entity) = entities.get(entity_id) else {
        return false;
    };
    let Some(human) = entity.human_data() else {
        return false;
    };
    if human.opponents.len() < 2 {
        return false;
    }

    // Forward unit vector: (position - old_position), Y stretched by
    // `INVERSE_ASPECT_RATIO`, then normalised.
    let pt_me = entity.element_data().position_map();
    let old = entity.position_iface().old_map_position().to_geo();
    let mut dir_x = pt_me.x - old.x;
    let mut dir_y = (pt_me.y - old.y) * INVERSE_ASPECT_RATIO;
    let dir_len = (dir_x * dir_x + dir_y * dir_y).sqrt();
    if dir_len <= f32::EPSILON {
        // Normalising a zero vector leaves it at zero, which would
        // make every forward projection also zero and fail the
        // `>= MIN_FORWARD_DISTANCE` gate.
        return false;
    }
    dir_x /= dir_len;
    dir_y /= dir_len;
    // Left-hand normal of (x, y) is (-y, x).
    let left_x = -dir_y;
    let left_y = dir_x;

    let mut smallest_left = MAX_BLOCKING_ENEMIES_LATERAL_DISTANCE;
    let mut smallest_right = MAX_BLOCKING_ENEMIES_LATERAL_DISTANCE;
    for &opp_id in &human.opponents {
        let Some(opp) = entities.get(opp_id) else {
            continue;
        };
        let opp_pt = opp.element_data().position_map();
        let vx = opp_pt.x - pt_me.x;
        let vy = (opp_pt.y - pt_me.y) * INVERSE_ASPECT_RATIO;

        let forward = vx * dir_x + vy * dir_y;
        if !(MIN_BLOCKING_ENEMY_FORWARD_DISTANCE..=MAX_BLOCKING_ENEMY_FORWARD_DISTANCE)
            .contains(&forward)
        {
            continue;
        }

        let left = vx * left_x + vy * left_y;
        if left >= 0.0 {
            if left < smallest_left {
                smallest_left = left;
            }
        } else if -left < smallest_right {
            smallest_right = -left;
        }
    }

    smallest_left + smallest_right < MAX_BLOCKING_ENEMIES_LATERAL_DISTANCE
}

/// Compute the difficulty-dependent delay (in frames) before a special
/// strike against a PC.
fn compute_special_strike_preparation_time(
    difficulty: crate::player_profile::DifficultyLevel,
    fighting_ability: u16,
) -> u32 {
    use crate::player_profile::DifficultyLevel;
    match difficulty {
        DifficultyLevel::Easy => 13u32.saturating_sub(fighting_ability as u32 / 10),
        DifficultyLevel::Medium => 10u32.saturating_sub(fighting_ability as u32 / 10),
        DifficultyLevel::Hard => 0,
    }
}

// ─── Hero expression IDs ────────────────────────────────────────────

const HERO_PROVOKE_DUEL: u16 = 0;
const HERO_PROVOKE_OPPONENT: u16 = 1;
const HERO_SUCCESSFULL_BLOW: u16 = 2;
const HERO_SWEAR_AT: u16 = 3;
const HERO_WARCRY: u16 = 4;
const HERO_STUN_ENNEMY: u16 = 5;
pub(crate) const HERO_PROVOKE_VIP: u16 = 6;
const HERO_KILLED_OPPONENT: u16 = 7;
const HERO_HURT: u16 = 8;
const HERO_SOLDIERS_FIRING_AT: u16 = 9;
const HERO_DIE: u16 = 10;
pub const HERO_SELECT: u16 = 11;
pub(crate) const HERO_ACCEPT_COMMAND: u16 = 12;
pub(crate) const HERO_DONE_COMMAND: u16 = 13;
pub const HERO_UNABLE_TO_DO_SOMETHING: u16 = 14;
pub(crate) const HERO_PERCHED_AND_SEE_ENNEMY: u16 = 15;
pub(crate) const HERO_GIVE_MONEY: u16 = 16;
const HERO_GET_BONUS_A: u16 = 17;
const HERO_GET_BONUS_C: u16 = 18;
pub(crate) const HERO_USE_LEAF_CLOVER: u16 = 19;
pub(crate) const HERO_GET_MONEY: u16 = 20;
const HERO_FIND_MONEY: u16 = 21;
pub(crate) const HERO_HEALED: u16 = 22;
pub(crate) const HERO_RECOVER: u16 = 23;
pub(crate) const HERO_OUT_OF_AMMO: u16 = 24;
const HERO_CATCHED_BY_NET: u16 = 25;

/// Priority flags for hero speech.
const SPEECH_NORMAL: u16 = 0;
const SPEECH_EMERGENCY: u16 = 0x0002;
const SPEECH_SCRIPT: u16 = 0x0004;
const SPEECH_ALWAYS: u16 = 0x0008;

/// Default anti-chorus timer after playing a speech.
const DEFAULT_ANTI_CHORUS_TIMER: u16 = 25;

/// How long HERO_SELECT is forbidden after playback.
const TIME_FORBID_HERO_SELECT: u16 = 150;

/// Default forbid time for other hero expressions.
const HERO_EXPRESSION_DEFAULT_FORBID: u16 = 75;

/// Check if an expression is allowed given the user's
/// `SoundConfig.amount_of_speaking` setting.
///
/// Each level adds suppressed expressions cumulatively from the
/// previous levels.
fn expression_allowed_by_amount(expression: u16, amount: u16) -> bool {
    // Level 0: suppress everything
    if amount == 0 {
        return false;
    }
    // Level 1: suppress provoke_duel, hurt, die, unable_to_do_something
    if amount <= 1
        && matches!(
            expression,
            HERO_PROVOKE_DUEL | HERO_HURT | HERO_DIE | HERO_UNABLE_TO_DO_SOMETHING
        )
    {
        return false;
    }
    // Level 2: suppress give_money, out_of_ammo, catched_by_net, swear_at
    if amount <= 2
        && matches!(
            expression,
            HERO_GIVE_MONEY | HERO_OUT_OF_AMMO | HERO_CATCHED_BY_NET | HERO_SWEAR_AT
        )
    {
        return false;
    }
    // Level 3: suppress perched_and_see_ennemy, soldiers_firing_at,
    //          successfull_blow, stun_ennemy, killed_opponent
    if amount <= 3
        && matches!(
            expression,
            HERO_PERCHED_AND_SEE_ENNEMY
                | HERO_SOLDIERS_FIRING_AT
                | HERO_SUCCESSFULL_BLOW
                | HERO_STUN_ENNEMY
                | HERO_KILLED_OPPONENT
        )
    {
        return false;
    }
    // Level 4: suppress healed, recover
    if amount <= 4 && matches!(expression, HERO_HEALED | HERO_RECOVER) {
        return false;
    }
    // Level 5: suppress provoke_opponent, provoke_vip, warcry, bonus_a/c,
    //          use_leaf_clover, get_money, find_money
    if amount <= 5
        && matches!(
            expression,
            HERO_PROVOKE_OPPONENT
                | HERO_PROVOKE_VIP
                | HERO_WARCRY
                | HERO_GET_BONUS_A
                | HERO_GET_BONUS_C
                | HERO_USE_LEAF_CLOVER
                | HERO_GET_MONEY
                | HERO_FIND_MONEY
        )
    {
        return false;
    }
    // Level 6: suppress done_command
    if amount <= 6 && expression == HERO_DONE_COMMAND {
        return false;
    }
    // Level 7: suppress accept_command
    if amount <= 7 && expression == HERO_ACCEPT_COMMAND {
        return false;
    }
    // Level 8: suppress select
    if amount <= 8 && expression == HERO_SELECT {
        return false;
    }
    true
}

// ─── Helpers ────────────────────────────────────────────────────────

impl EngineInner {
    /// Build a [`ConcussionContext`] for a given entity id, reading the
    /// real invulnerable / tied / carried / script-locked / sherwood-pc
    /// flags off the entity instead of defaulting them. Used by console
    /// cheats that call `set_concussion` (WAKEUP, MORPHEUS, BUD SPENCER)
    /// so the guards — "invulnerable entity refuses concussion
    /// increase", "tied/carried keeps asleep below wakeup threshold" —
    /// still fire through the cheat path. A missing cheat target is explicitly
    /// optional and is reported to the caller instead of being represented by
    /// a fabricated all-false context.
    pub(crate) fn concussion_ctx_for<I: Into<EntityId>>(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        profiles: &crate::profiles::ProfileManager,
        id: I,
    ) -> Option<ConcussionContext> {
        let id = id.into();
        self.get_entity(id).map(|entity| {
            concussion_ctx_full(
                entity,
                self.is_sherwood(profiles),
                Some(&self.mission_domain.campaign),
                sim.config().difficulty,
            )
        })
    }

    /// Engine-level wrapper around `combat::set_concussion` that runs
    /// the cross-system side-effects the pure `combat::set_concussion`
    /// helper can't reach.
    ///
    /// Use this from cheat handlers and any non-damage-path caller
    /// (e.g. scripted force-wake) that needs the full set-concussion
    /// semantics.  The damage path keeps using `handle_knockout`
    /// directly because it also needs the falling-back animation
    /// queueing tied to a damage element.
    ///
    /// On a state transition this updates the healing timeout and queues the
    /// original cross-system finish work. Script-yield callers drain that work
    /// immediately before resuming the VM; ordinary combat callers drain it at
    /// the engine boundary.
    ///
    /// When `force_value` is true, bypass the script-lock
    /// stay-asleep clause so the call can wake a script-locked NPC.
    ///
    /// Returns the `ConcussionOutcome` from the underlying call.
    pub(crate) fn apply_concussion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: impl Into<EntityId>,
        value: u16,
        force_value: bool,
    ) -> crate::combat::ConcussionOutcome {
        use crate::combat::ConcussionOutcome;

        let entity_id = entity_id.into();
        let Some(mut ctx) = self.concussion_ctx_for(sim, &assets.profile_manager, entity_id) else {
            tracing::warn!(?entity_id, "optional concussion target does not exist");
            return ConcussionOutcome::NoChange;
        };
        ctx.force_value = force_value;

        let Some(human) = self
            .get_entity_mut(entity_id)
            .and_then(|entity| entity.human_data_mut())
        else {
            tracing::warn!(?entity_id, "optional concussion target is not human");
            return ConcussionOutcome::NoChange;
        };
        let outcome = combat::set_concussion(human, value, &ctx);

        self.finish_applied_concussion(assets, entity_id, outcome)
    }

    /// Strict scripted concussion path. Native validation guarantees the
    /// actor and its HumanData exist until this synchronous request applies;
    /// losing either is an engine invariant violation, never a false result.
    pub(crate) fn apply_scripted_concussion(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: impl Into<EntityId>,
        value: u16,
        force_value: bool,
    ) -> crate::combat::ConcussionOutcome {
        let entity_id = entity_id.into();
        let mut ctx = {
            let entity = self
                .get_entity(entity_id)
                .expect("validated scripted concussion target vanished before apply");
            entity
                .human_data()
                .expect("validated scripted concussion target lost HumanData");
            concussion_ctx_full(
                entity,
                self.is_sherwood(&assets.profile_manager),
                Some(&self.mission_domain.campaign),
                sim.config().difficulty,
            )
        };
        ctx.force_value = force_value;
        let human = self
            .get_entity_mut(entity_id)
            .expect("validated scripted concussion target vanished during apply")
            .human_data_mut()
            .expect("validated scripted concussion target lost HumanData during apply");
        let outcome = combat::set_concussion(human, value, &ctx);

        self.finish_applied_concussion(assets, entity_id, outcome)
    }

    fn finish_applied_concussion(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        outcome: crate::combat::ConcussionOutcome,
    ) -> crate::combat::ConcussionOutcome {
        self.finish_scripted_concussion(assets, entity_id, outcome);
        let pc_is_unconscious = self.get_entity(entity_id).is_some_and(|entity| {
            entity.is_pc()
                && entity
                    .human_data()
                    .expect("PC concussion target lost HumanData")
                    .unconscious
        });
        if pc_is_unconscious {
            self.unselect_single_pc(entity_id);
        }
        outcome
    }

    /// Complete cross-system concussion effects after a script native has
    /// already changed canonical HumanData synchronously.
    pub(crate) fn finish_scripted_concussion(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        outcome: ConcussionOutcome,
    ) {
        let healing_speed = concussion_healing_speed_for_entity(
            self.get_entity(entity_id)
                .expect("scripted concussion target vanished before finish"),
            &assets.profile_manager,
        );
        match outcome {
            ConcussionOutcome::WentUnconscious => {
                // Healing-timeout init.
                let h = self
                    .get_entity_mut(entity_id)
                    .and_then(|e| e.human_data_mut())
                    .expect("scripted concussion target lost HumanData during finish");
                if h.concussion_healing_timeout == 0 {
                    h.concussion_healing_timeout = healing_speed;
                }

                self.orders
                    .pending_concussion_side_effects
                    .push((entity_id, outcome));
            }
            ConcussionOutcome::WokeUp => {
                self.orders
                    .pending_concussion_side_effects
                    .push((entity_id, outcome));
            }
            ConcussionOutcome::NoChange => {}
        }
    }

    /// Apply the original script-level life setter as one synchronous engine
    /// operation, including PC speech and the death pipeline.
    pub(crate) fn apply_scripted_life_points(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        amount: i32,
    ) {
        let entity = self
            .get_entity(entity_id)
            .expect("scripted life-point target disappeared before synchronous apply");
        assert!(
            matches!(
                entity,
                Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
            ),
            "scripted life-point target is not a PC, soldier, or civilian"
        );
        entity
            .human_data()
            .expect("scripted life-point target has no HumanData");
        let is_pc = entity.kind().is_pc();
        let invulnerable = entity
            .human_data()
            .expect("validated scripted life-point target lost HumanData")
            .invulnerable;
        let max_life = get_max_life_points(entity);
        let sherwood_pc = self.world.weather.is_forest_level && is_pc;
        let before = get_life_points(entity);
        if before <= 0 {
            return;
        }

        // RHScript passes `int` to the SWORD setter, so preserve the original
        // two's-complement narrowing rather than saturating at Rust's bounds.
        let value = amount as i16;
        let died = match self.get_entity_mut(entity_id) {
            Some(Entity::Pc(entity)) if invulnerable => {
                entity.pc.life_points = 100;
                false
            }
            Some(Entity::Soldier(entity)) if invulnerable => {
                entity.npc.life_points = 100;
                false
            }
            Some(Entity::Civilian(entity)) if invulnerable => {
                entity.npc.life_points = 100;
                false
            }
            Some(Entity::Pc(entity)) => crate::combat::set_life_points(
                &mut entity.pc.life_points,
                value,
                false,
                max_life,
                sherwood_pc,
            ),
            Some(Entity::Soldier(entity)) => crate::combat::set_life_points(
                &mut entity.npc.life_points,
                value,
                false,
                max_life,
                sherwood_pc,
            ),
            Some(Entity::Civilian(entity)) => crate::combat::set_life_points(
                &mut entity.npc.life_points,
                value,
                false,
                max_life,
                sherwood_pc,
            ),
            _ => unreachable!("validated scripted life-point target changed entity kind"),
        };
        let after = get_life_points(
            self.get_entity(entity_id)
                .expect("scripted life-point target vanished after synchronous apply"),
        );
        let damage = (before - after).max(0) as u16;
        if is_pc && damage > 0 {
            self.say_ouch(sim, assets, entity_id, Some(damage));
        }
        if died {
            self.apply_scripted_virtual_kill(sim, assets, entity_id, None);
        }
    }

    /// Run the virtual PC/NPC/Soldier/Human `Kill` chain used by
    /// `RHElementActorHuman::SetLifePoints`, without synthesizing a damage
    /// element. Damage-only animation, roll, attacker attribution, and fight
    /// score belong to `ReceiveDamage`, not to a script setter.
    pub(in crate::engine) fn apply_scripted_virtual_kill(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        killer: Option<EntityId>,
    ) {
        let (is_pc, is_npc, allied_soldier, killer_is_pc) = {
            let entity = self
                .get_entity(entity_id)
                .expect("script-killed actor vanished before virtual Kill");
            (
                entity.is_pc(),
                entity.is_npc(),
                entity.is_soldier() && entity.camp() == crate::element::Camp::Royalists,
                killer
                    .and_then(|killer_id| self.get_entity(killer_id))
                    .is_some_and(|killer| killer.is_pc()),
            )
        };

        if is_pc {
            self.apply_pc_kill_cascade(sim, assets, entity_id);
        }
        if is_npc {
            self.delete_detectable_for_all_npc(entity_id, crate::element::DetectableType::Friend);
            self.delete_detectable_for_all_npc(
                entity_id,
                crate::element::DetectableType::MissedFriend,
            );
            let entity = self
                .get_entity_mut(entity_id)
                .expect("script-killed NPC vanished during virtual Kill");
            let forced_attentive = if entity.is_soldier() {
                entity
                    .enemy_ai()
                    .expect("script-killed soldier NPC has no EnemyAi")
                    .forced_attentive
            } else {
                false
            };
            let ai = entity
                .ai_controller_mut()
                .expect("script-killed NPC has no AI controller");
            ai.set_alert_status_with_flags(
                crate::ai::AlertLevel::Green,
                crate::ai::AlertFlags::INSTANT_MUSIC_CHANGE,
                forced_attentive,
            );
            ai.current_state = crate::ai::AiState::Sleeping;
            ai.current_substate = crate::ai::Substate::SleepingForever;
            ai.clear_emoticon();
            ai.clear_all_pending();
            let npc = entity
                .npc_data_mut()
                .expect("script-killed NPC has no NPCData");
            npc.alerted = false;
            if npc.eye_status != EyeStatus::Closed {
                crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
            }
            npc.inform_my_friends = killer_is_pc;
            if let Some(ai) = npc.ai_brain.base_mut() {
                ai.knocked_out_in_money_fight = false;
            }
        }
        if allied_soldier {
            self.mission_domain.mission_stat.add_killed_allied();
        }

        self.quit_swordfight(sim, assets, entity_id);
        let still_unconscious = self
            .get_entity(entity_id)
            .and_then(|entity| entity.human_data())
            .expect("script-killed human lost HumanData")
            .unconscious;
        self.feedback.titbit_manager.remove_unconscious_stars_if(
            crate::titbit::ElementHandle(entity_id.index()),
            still_unconscious,
        );
        let human = self
            .get_entity_mut(entity_id)
            .expect("script-killed human vanished during Human::Kill")
            .human_data_mut()
            .expect("script-killed human lost HumanData during Human::Kill");
        human.unconscious = false;
        human.concussion_of_the_brain = 0;
        human.concussion_healing_timeout = 0;
    }

    /// Drain `pending_concussion_side_effects` (queued by
    /// `apply_concussion`).  Runs inside `perform_hourglass` where
    /// `&LevelAssets` is available for `quit_swordfight`.
    ///
    /// On `WentUnconscious`: `quit_swordfight` + `add_unconscious_star`
    ///     + `EventLoseConsciousness` stimulus.
    ///
    /// On `WokeUp`: `EventFitAgain` stimulus + the PC/soldier
    /// `BlinkEnemy` redetect loop.
    pub(crate) fn drain_pending_concussion_side_effects(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        use crate::combat::ConcussionOutcome;

        if self.orders.pending_concussion_side_effects.is_empty() {
            return;
        }
        let entries = std::mem::take(&mut self.orders.pending_concussion_side_effects);
        for (entity_id, outcome) in entries {
            match outcome {
                ConcussionOutcome::WentUnconscious => {
                    self.quit_swordfight(sim, assets, entity_id);
                    self.add_unconscious_star(entity_id);
                    if let Some(npc) = self
                        .get_entity_mut(entity_id)
                        .and_then(|entity| entity.npc_data_mut())
                    {
                        npc.clear_all_suspects();
                    }
                    self.dispatch_ai_stimulus(
                        entity_id,
                        crate::ai::Stimulus::new(crate::ai::StimulusType::EventLoseConsciousness),
                    );
                    if let Some(npc) = self
                        .get_entity_mut(entity_id)
                        .and_then(|entity| entity.npc_data_mut())
                    {
                        // Script setters have no who-dunnit actor.
                        npc.inform_my_friends = false;
                    }
                }
                ConcussionOutcome::WokeUp => {
                    let is_pc = self
                        .get_entity(entity_id)
                        .is_some_and(crate::element::Entity::is_pc);
                    if is_pc {
                        // PCs have no NPC Hourglass slot or AI Think call.
                        self.apply_wake_redetection_blinks(entity_id);
                    } else {
                        // NPC soldiers/civilians dispatch FITAGAIN at their
                        // owner prelude. The inline BlinkEnemy fan-out follows
                        // that synchronous Think there.
                        self.dispatch_ai_stimulus(
                            entity_id,
                            crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
                        );
                    }
                }
                ConcussionOutcome::NoChange => {}
            }
        }
    }

    /// Apply Original `SetConcussionOfTheBrain`'s inline `BlinkEnemy(this)`
    /// fan-out to every opposing NPC. This runs at the waker's slot, after a
    /// synchronous NPC `EVENT_FITAGAIN`, and is not deferred to each observer.
    pub(crate) fn apply_wake_redetection_blinks<I: Into<EntityId>>(&mut self, waker_id: I) {
        let waker_id = waker_id.into();
        let waker = self.get_entity(waker_id).unwrap_or_else(|| {
            panic!(
                "wake BlinkEnemy fan-out requires missing waker {}",
                waker_id.index()
            )
        });
        let waker_is_pc = waker.is_pc();
        let waker_is_soldier = matches!(waker, Entity::Soldier(_));
        if !(waker_is_pc || (waker_is_soldier && self.ai.global.npcs_can_be_enemies())) {
            return;
        }
        let waker_camp = waker.camp();
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            if npc_id == waker_id {
                continue;
            }
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                continue;
            };
            if entity.camp() == waker_camp {
                continue;
            }
            let npc = entity.npc_data_mut().unwrap_or_else(|| {
                panic!(
                    "NPC {} lost its NPC data while queueing wake blink for {}",
                    npc_id.index(),
                    waker_id.index()
                )
            });
            npc.ai_brain.base_mut().unwrap_or_else(|| {
                panic!(
                    "NPC {} is missing its required AI controller while applying wake blink for {}",
                    npc_id.index(),
                    waker_id.index()
                )
            });
            let enemy_idx = crate::element::DetectableType::Enemy as usize;
            let detectables = npc.detectable_lists.get_mut(enemy_idx).unwrap_or_else(|| {
                panic!(
                    "NPC {} has no Enemy detectable bucket while applying wake blink for {}",
                    npc_id.index(),
                    waker_id.index()
                )
            });
            for detectable in detectables {
                if detectable.element == Some(waker_id) {
                    detectable.seen_now = false;
                    detectable.seen_last_frame = false;
                }
            }
        }
    }

    /// Drain the deferred `HADES` cheat queue.  Each queued id gets
    /// the full NPC-kill cascade via [`EngineInner::handle_death`]:
    /// alert-green, sleeping-forever state, eye close, friend /
    /// missed-friend detectable removal, emoticon clear, and the
    /// dying animation.
    pub(crate) fn drain_pending_hades_kills(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if self.orders.pending_hades_kills.is_empty() {
            return;
        }
        let victims: Vec<EntityId> = std::mem::take(&mut self.orders.pending_hades_kills);
        for victim_id in victims {
            self.handle_death(sim, assets, victim_id);
        }
    }
}

/// Build a `ConcussionContext` with PC in-coma lookup.  The PC
/// override of `SetConcussionOfTheBrain` forces the value to
/// `CONCUSSION_MAX` whenever the PC is in a coma; the coma flag lives
/// on the PC's serialized campaign description/status rather than the
/// entity, so callers that care about the coma override must pass
/// `campaign`.
pub(crate) fn concussion_ctx_full(
    entity: &Entity,
    is_sherwood: bool,
    campaign: Option<&crate::campaign::Campaign>,
    difficulty: crate::player_profile::DifficultyLevel,
) -> ConcussionContext {
    let human = entity.human_data();
    let posture = entity.element_data().posture;
    let is_in_coma = match entity {
        Entity::Pc(pc) => match campaign {
            // Pure/helper contexts can intentionally omit campaign state. A
            // live engine always supplies it.
            None => false,
            Some(campaign) => {
                // `mubListIndex` is independent actor/UI storage. Original
                // resolves this through the serialized `mpDescription`
                // pointer; `campaign_description_index` is that stable
                // identity. Profiles are not unique, so neither the UI index
                // nor a profile search is an admissible substitute here.
                let raw_index = pc.pc.campaign_description_index.expect(
                    "live PC concussion context is missing its campaign description identity",
                );
                let description = campaign.characters.get(raw_index as usize).unwrap_or_else(|| {
                    panic!(
                        "live PC concussion context campaign description index {raw_index} is outside character table of length {}",
                        campaign.characters.len()
                    )
                });
                assert_eq!(
                    description.character_profile_idx,
                    Some(pc.pc.profile_index),
                    "live PC concussion context campaign description {raw_index} does not match entity profile {}",
                    pc.pc.profile_index
                );
                description.status.in_coma
            }
        },
        _ => false,
    };
    ConcussionContext {
        difficulty,
        is_invulnerable: human.map(|h| h.invulnerable).unwrap_or(false),
        is_tied: posture == Posture::Tied,
        is_carried: posture == Posture::Carried || posture == Posture::OnShoulders,
        is_script_locked: match entity {
            Entity::Soldier(s) => s
                .npc
                .ai_brain
                .base()
                .map(|b| b.script_locked)
                .unwrap_or(false),
            Entity::Civilian(c) => c
                .npc
                .ai_brain
                .base()
                .map(|b| b.script_locked)
                .unwrap_or(false),
            _ => false,
        },
        // RHGame::IsSherwood is the current campaign mission's location,
        // not the proto level's broader `forest_level` rendering/AI flag.
        is_sherwood_pc: is_sherwood && entity.kind().is_pc(),
        is_in_coma,
        // `force_value` is a per-call parameter, not entity state.
        // Default to false; cheats / scripts that need force-wake set
        // it on the ctx returned by `concussion_ctx_for`.
        force_value: false,
        // Only civilians override the wounding/concussion primitives on
        // an attached scroll; a soldier carrying one takes damage
        // normally.
        scroll_attached: matches!(entity, Entity::Civilian(c) if c.npc.attached_scroll.is_some()),
    }
}

/// Get the entity's life points (works for both PCs and NPCs).
fn get_life_points(entity: &Entity) -> i16 {
    match entity {
        Entity::Pc(pc) => pc.pc.life_points,
        Entity::Soldier(s) => s.npc.life_points,
        Entity::Civilian(c) => c.npc.life_points,
        _ => 0,
    }
}

/// Get the entity's max life points.
fn get_max_life_points(entity: &Entity) -> i16 {
    match entity {
        Entity::Pc(_) => combat::LIFEPOINTS_PC,
        Entity::Soldier(s) => s.soldier.cached_max_life_points,
        Entity::Civilian(_) => 100, // civilians initialise at 100 HP
        _ => 100,
    }
}

/// Look up an entity's fighting ability from its character/soldier profile.
/// For Lacklandist soldiers, applies the supplied engine difficulty scaling.
fn fighting_ability_from_profile(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
    difficulty: crate::player_profile::DifficultyLevel,
) -> u16 {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.fighting)
            .unwrap_or(50),
        Entity::Soldier(s) => {
            let base = profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .map(|p| p.fighting)
                .unwrap_or(50);
            if s.soldier.cached_camp == crate::element::Camp::Lacklandists {
                difficulty.modify_capacity(
                    base,
                    crate::player_profile::difficulty_params::EASY_ENEMY_FIGHTING,
                    crate::player_profile::difficulty_params::HARD_ENEMY_FIGHTING,
                    100,
                )
            } else {
                base
            }
        }
        _ => 50,
    }
}

/// Look up an entity's endurance from its profile.
fn endurance_from_profile(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> u16 {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.endurance)
            .unwrap_or(50),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.endurance)
            .unwrap_or(50),
        _ => 50,
    }
}

/// Look up an entity's weapon material from its profile.
pub(super) fn weapon_material_from_profile(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> crate::profiles::WeaponMaterial {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.weapon_material)
            .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.weapon_material)
            .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood),
        _ => crate::profiles::WeaponMaterial::SteelAndWood,
    }
}

/// Look up an entity's armor material from its profile.
fn armor_material_from_profile(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> crate::profiles::ArmorMaterial {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.armor_material)
            .unwrap_or(crate::profiles::ArmorMaterial::Plate),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.armor_material)
            .unwrap_or(crate::profiles::ArmorMaterial::Plate),
        _ => crate::profiles::ArmorMaterial::Plate,
    }
}

/// Check if an entity is a VIP (from its profile).
pub(crate) fn is_vip_from_profile(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> bool {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.vip)
            .unwrap_or(false),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.vip)
            .unwrap_or(false),
        _ => false,
    }
}

/// Gate shared by every sword strike effect: a hit victim is only
/// dragged into a sword fight with the attacker when the victim is not
/// a civilian, is in the attacker's enemy camp, and neither side's
/// "non-Robin can't touch a VIP" protection triggers.
pub(in crate::engine) fn should_enter_swordfight_after_strike(
    attacker: &Entity,
    victim: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> bool {
    if victim.is_civilian() {
        return false;
    }
    if victim.camp() != attacker.camp().enemy() {
        return false;
    }
    let attacker_is_robin = matches!(attacker, Entity::Pc(pc) if pc.pc.robin);
    let victim_is_robin = matches!(victim, Entity::Pc(pc) if pc.pc.robin);
    let attacker_is_vip = is_vip_from_profile(attacker, profile_manager);
    let victim_is_vip = is_vip_from_profile(victim, profile_manager);
    if !attacker_is_robin && victim_is_vip {
        return false;
    }
    if !victim_is_robin && attacker_is_vip {
        return false;
    }
    true
}

/// Check if a soldier has rank SOLDIER (the lowest rank).
/// Used for cutting damage scaling.
fn is_rank_soldier(entity: &Entity, profile_manager: &crate::profiles::ProfileManager) -> bool {
    match entity {
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.rank == crate::profiles::ProfileRank::Soldier)
            .unwrap_or(true),
        _ => false,
    }
}

/// Full HtH weapon lookup using the profile_manager.  Returns the
/// weapon profile id from the character/soldier profile.  The id is
/// the raw 1-based value as stored in the profile; pass it to
/// [`ProfileManager::get_hth_weapon`], which handles the `-1`
/// conversion (matching `InitializeWeapons(hth_weapon_id - 1, ...)`
/// in the actor init paths).
pub(crate) fn get_hth_weapon_id_full(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> Option<u32> {
    match entity {
        Entity::Pc(pc) => profile_manager
            .get_character(pc.pc.profile_index)
            .map(|p| p.hth_weapon_id),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.hth_weapon_id),
        _ => None,
    }
}

/// Get the distance between two entities on the ground plane.
fn entity_distance<A: Into<EntityId>, B: Into<EntityId>>(
    entities: &crate::entities::Entities,
    a: A,
    b: B,
) -> f32 {
    let a = a.into();
    let b = b.into();
    let pos_a = match entities.get(a) {
        Some(e) => e.element_data().position_map(),
        None => return f32::MAX,
    };
    let pos_b = match entities.get(b) {
        Some(e) => e.element_data().position_map(),
        None => return f32::MAX,
    };
    let dx = pos_a.x - pos_b.x;
    let dy = pos_a.y - pos_b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Original `ExecuteStraightSwordStrike` measures the strike at its DONE
/// marker with `(GetPosition() - antagonist->GetPosition()).Norm()`.  Those
/// are stored world coordinates, including elevation in both `y` and `z`;
/// projected map distance can therefore admit a target on another height.
fn entity_world_distance<A: Into<EntityId>, B: Into<EntityId>>(
    entities: &crate::entities::Entities,
    a: A,
    b: B,
) -> f32 {
    let a = a.into();
    let b = b.into();
    let pos_a = entities
        .get(a)
        .unwrap_or_else(|| panic!("straight-strike distance references missing attacker {a:?}"))
        .element_data()
        .position();
    let pos_b = entities
        .get(b)
        .unwrap_or_else(|| panic!("straight-strike distance references missing victim {b:?}"))
        .element_data()
        .position();
    let dx = pos_a.x - pos_b.x;
    let dy = pos_a.y - pos_b.y;
    let dz = pos_a.z - pos_b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Get the 0-15 direction sector from entity A looking at entity B.
fn direction_to<F: Into<EntityId>, T: Into<EntityId>>(
    entities: &crate::entities::Entities,
    from: F,
    to: T,
) -> i16 {
    let from = from.into();
    let to = to.into();
    let pos_a = entities
        .get(from)
        .unwrap_or_else(|| panic!("melee direction source {from:?} must exist"))
        .ground_position();
    let pos_b = entities
        .get(to)
        .unwrap_or_else(|| panic!("melee direction target {to:?} must exist"))
        .ground_position();
    crate::position_interface::vector_to_sector_0_to_15_iso(pos_b.x - pos_a.x, pos_b.y - pos_a.y)
}

/// Sector to unit vector with isometric Y scaling.  Thin alias over
/// [`crate::position_interface::sector_to_vector_iso`].  Every caller
/// passes `aspect_ratio = ASPECT_RATIO`; the argument is retained for
/// signature stability.
fn sector_to_vector_iso(sector: u16, _aspect_ratio: f32) -> (f32, f32) {
    let [x, y] = crate::position_interface::sector_to_vector_iso(sector as i16);
    (x, y)
}

/// Get an entity's camp (faction). PCs are always Royalists.
fn entity_camp<I: Into<EntityId>>(
    entities: &crate::entities::Entities,
    id: I,
) -> crate::element::Camp {
    let id = id.into();
    match entities.get(id) {
        Some(Entity::Pc(_)) => crate::element::Camp::Royalists,
        Some(Entity::Soldier(s)) => s.soldier.cached_camp,
        Some(Entity::Civilian(c)) => c.civilian.cached_camp,
        _ => crate::element::Camp::Error,
    }
}

/// Check whether `sector` refers to a building sector in the grid.
fn is_in_building_sector(
    sector: Option<crate::position_interface::SectorHandle>,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
) -> bool {
    let Some(sector_num) = sector else {
        return false;
    };
    fast_grid
        .level
        .sector_number_map
        .get(&crate::sector::SectorNumber::new(
            u16::from(sector_num) as i16
        ))
        .and_then(|&idx| fast_grid.level.sectors.get(idx))
        .map(|gs| gs.sector_type.is_building())
        .unwrap_or(false)
}

/// Check whether `sector` is a lift sector with wall or ladder sub-type.
fn is_on_wall_or_ladder(
    sector: Option<crate::position_interface::SectorHandle>,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
) -> bool {
    let Some(sector_num) = sector else {
        return false;
    };
    fast_grid
        .level
        .sector_number_map
        .get(&crate::sector::SectorNumber::new(
            u16::from(sector_num) as i16
        ))
        .and_then(|&idx| fast_grid.level.sectors.get(idx))
        .map(|gs| {
            gs.sector_type.is_lift()
                && gs
                    .lift_type
                    .map(|lt| lt.is_wall_or_ladder())
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

// ─── Jump lines / table swordfight ─────────────────────────────────

/// Data-plane jump-line lookup keyed on sector numbers and a
/// world-space position (no entity handle).
///
/// Scans the home sector's jump lines and returns the index of the
/// nearest one whose paired line sits in `linked_sector_number`,
/// within `max_distance`.  Returns `None` when no jump line in the
/// caller's sector connects to the requested destination sector within
/// range.  Used by both `is_table_swordfight_needed` and the AI
/// snapshot pipeline.
pub(crate) fn nearest_jump_line_from_sector(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    home_sector_number: i16,
    from_position: crate::coordinates::MapPoint,
    max_distance: f32,
    linked_sector_number: i16,
) -> Option<u32> {
    let sector_idx = *fast_grid
        .level
        .sector_number_map
        .get(&crate::sector::SectorNumber::new(home_sector_number))?;
    let sector = fast_grid.level.sectors.get(sector_idx)?;
    if sector.jump_line_indices.is_empty() {
        return None;
    }

    let mut best: Option<(u32, f32)> = None;
    for &line_idx in &sector.jump_line_indices {
        let jump_line = match fast_grid.level.jump_lines.get(usize::from(line_idx)) {
            Some(l) => l,
            None => continue,
        };
        let assoc_idx = match jump_line.associated_line_index {
            Some(i) => i,
            None => continue,
        };
        let assoc = match fast_grid.level.jump_lines.get(assoc_idx as usize) {
            Some(l) => l,
            None => continue,
        };
        let assoc_sector_idx = match assoc.sector_index {
            Some(i) => i,
            None => continue,
        };
        let assoc_sector = match fast_grid.level.sectors.get(usize::from(assoc_sector_idx)) {
            Some(s) => s,
            None => continue,
        };
        if assoc_sector.sector_number != linked_sector_number {
            continue;
        }

        let d = jump_line.compute_distance(from_position);
        if d >= max_distance {
            continue;
        }
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((u32::from(line_idx), d));
        }
    }

    best.map(|(idx, _)| idx)
}

/// Data-plane variant of [`is_table_swordfight_needed`]: answers the
/// same "which jump line should the aggressor stand on?" question
/// given the attacker's and victim's sector + position, plus the
/// attacker's maximal weapon range.
///
/// Intended for AI callers that operate on `FighterSnapshot`s /
/// `AiContext`s rather than raw entities.  Returns the aggressor's
/// (PC/caller's side) jump-line index, or `None` when no pair reaches
/// across the gap.
pub(crate) fn table_swordfight_jump_line(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    attacker_sector_number: i16,
    victim_sector_number: i16,
    victim_position: crate::coordinates::MapPoint,
    max_range: f32,
) -> Option<u32> {
    // Same sector → no table swordfight needed.
    if attacker_sector_number == victim_sector_number {
        return None;
    }

    let victim_line_idx = nearest_jump_line_from_sector(
        fast_grid,
        victim_sector_number,
        victim_position,
        max_range,
        attacker_sector_number,
    )?;
    let victim_line = fast_grid.level.jump_lines.get(victim_line_idx as usize)?;
    let aggressor_line_idx = victim_line.associated_line_index?;
    let aggressor_line = fast_grid
        .level
        .jump_lines
        .get(aggressor_line_idx as usize)?;

    if (aggressor_line.z_a - victim_line.z_a).abs() > MAX_ELEVATION_SWORDFIGHT {
        return None;
    }

    let mid_aggressor = aggressor_line.get_middle_point();
    let mid_victim = victim_line.get_middle_point();
    let dx = mid_aggressor.x - mid_victim.x;
    let dy = mid_aggressor.y - mid_victim.y;
    let middle_distance = (dx * dx + dy * dy).sqrt();
    let victim_offset = victim_line.compute_distance(victim_position);
    if middle_distance + victim_offset > max_range {
        return None;
    }

    Some(aggressor_line_idx)
}

/// Returns the PC's (aggressor's) jump line index if the opponent is
/// in a different sector and both entities can reach each other via
/// a paired jump line pair within the PC's maximal weapon range.
///
/// Returns `None` when the opponents share a sector (normal fight)
/// or when no suitable jump-line pair reaches across the gap.
pub(crate) fn is_table_swordfight_needed(
    entities: &crate::entities::Entities,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    profile_manager: &crate::profiles::ProfileManager,
    pc_id: impl Into<EntityId>,
    victim_id: impl Into<EntityId>,
) -> Option<u32> {
    let pc_id = pc_id.into();
    let victim_id = victim_id.into();
    let pc = entities.get(pc_id)?;
    let victim = entities.get(victim_id)?;

    let pc_sector = pc.element_data().sector()?;
    let victim_sector = victim.element_data().sector()?;

    let weapon_profile = get_hth_weapon_id_full(pc, profile_manager)
        .and_then(|idx| profile_manager.get_hth_weapon(idx))?;
    let maximal_distance =
        weapon_profile.distance[crate::weapons::WeaponDistance::Maximal as usize] as f32;

    table_swordfight_jump_line(
        fast_grid,
        i16::from(pc_sector),
        i16::from(victim_sector),
        victim.element_data().position_map(),
        maximal_distance,
    )
}

/// Number of `opponent`'s current swordfight opponents that stand in
/// `from_sector` — i.e. how many fighters from the caller's side are
/// already engaged in this table fight.
pub(crate) fn number_of_table_swordfight_opponents(
    entities: &crate::entities::Entities,
    opponent_id: EntityId,
    from_sector: i16,
) -> u32 {
    let Some(opponent) = entities.get(opponent_id) else {
        return 0;
    };
    let Some(human) = opponent.human_data() else {
        return 0;
    };
    let mut count = 0;
    for fighter_id in &human.opponents {
        let Some(fighter) = entities.get(*fighter_id) else {
            continue;
        };
        if fighter.element_data().sector().map(i16::from) == Some(from_sector) {
            count += 1;
        }
    }
    count
}

/// Find a free spot on `jump_line` for the caller to stand on while
/// table-swordfighting `opponent`.  Slots avoid any of `opponent`'s
/// other current fighters that already sit on the caller's side of the
/// jump gap.
///
/// Returns `Some(position)` for a valid slot or `None` when the line
/// has no free slot within [0, 1] (caller should interrupt the sequence).
pub(crate) fn find_position_for_table_swordfight(
    entities: &crate::entities::Entities,
    self_position: crate::coordinates::MapPoint,
    self_sector: i16,
    self_id: EntityId,
    opponent_id: EntityId,
    jump_line: &crate::jump_line::JumpLine,
) -> Option<crate::coordinates::MapPoint> {
    // The opponent must already be swordfighting at least one fighter
    // (us) when this runs.
    let opponent = entities.get(opponent_id)?;
    let opp_human = opponent.human_data()?;

    let line_vec = jump_line.vector();
    let line_norm = jump_line.norm();
    if line_norm < f32::EPSILON {
        return None;
    }
    let displacement = 15.0 / line_norm;

    let position_current = jump_line.compute_nearest_point_param(self_position);

    // Collect the "friends" — enemies of my enemy. Every opponent of
    // `opponent` that is NOT me and shares my sector.
    let mut occupied: Vec<f32> = Vec::new();
    for fighter_id in &opp_human.opponents {
        if *fighter_id == self_id {
            continue;
        }
        let Some(friend) = entities.get(*fighter_id) else {
            continue;
        };
        if friend.element_data().sector().map(i16::from) != Some(self_sector) {
            continue;
        }
        let friend_pos = friend.element_data().position_map();
        occupied.push(jump_line.compute_nearest_point_param(friend_pos));
    }

    let (pos_left, pos_right) = match occupied.len() {
        0 => {
            // No one else here: clamp current projection onto the line.
            let pt = if position_current >= 1.0 {
                jump_line.point_b
            } else if position_current <= 0.0 {
                jump_line.point_a
            } else {
                crate::coordinates::MapPoint::new(
                    jump_line.point_a.x + position_current * line_vec.x,
                    jump_line.point_a.y + position_current * line_vec.y,
                )
            };
            return Some(pt);
        }
        1 => {
            let p = occupied[0];
            (p - displacement, p + displacement)
        }
        2 => {
            let (mut pl, mut pr) = (occupied[0], occupied[1]);
            if pl > pr {
                std::mem::swap(&mut pl, &mut pr);
            }
            (pl - displacement, pr + displacement)
        }
        _ => {
            // Unreachable — the caller guards with at most 2 table
            // opponents before invoking us.
            panic!(
                "find_position_for_table_swordfight: {} table opponents (must be <=2)",
                occupied.len()
            );
        }
    };

    // If already between the two slot bounds, stay put.
    if (0.0..=1.0).contains(&position_current)
        && (position_current <= pos_left || position_current >= pos_right)
    {
        return Some(self_position);
    }

    if pos_left >= 0.0 {
        if pos_right <= 1.0 {
            // Both sides valid — pick whichever is closer in world space.
            let right_pt = crate::coordinates::MapPoint::new(
                jump_line.point_a.x + pos_right * line_vec.x,
                jump_line.point_a.y + pos_right * line_vec.y,
            );
            let left_pt = crate::coordinates::MapPoint::new(
                jump_line.point_a.x + pos_left * line_vec.x,
                jump_line.point_a.y + pos_left * line_vec.y,
            );
            let dr = {
                let dx = self_position.x - right_pt.x;
                let dy = self_position.y - right_pt.y;
                dx * dx + dy * dy
            };
            let dl = {
                let dx = self_position.x - left_pt.x;
                let dy = self_position.y - left_pt.y;
                dx * dx + dy * dy
            };
            Some(if dl < dr { left_pt } else { right_pt })
        } else {
            // Only left side valid.
            Some(crate::coordinates::MapPoint::new(
                jump_line.point_a.x + pos_left * line_vec.x,
                jump_line.point_a.y + pos_left * line_vec.y,
            ))
        }
    } else if pos_right <= 1.0 {
        // Only right side valid.
        Some(crate::coordinates::MapPoint::new(
            jump_line.point_a.x + pos_right * line_vec.x,
            jump_line.point_a.y + pos_right * line_vec.y,
        ))
    } else {
        None
    }
}

/// Outcome of the table-swordfight positioning check performed on
/// entering a cross-sector swordfight.  See
/// `EngineInner::try_launch_table_swordfight_move`.
enum TableFightMove {
    /// No move required — either same-sector fight, or the caller is
    /// already at an acceptable slot on the jump line.
    Ok,
    /// A positioning movement element was enqueued.
    Launched,
    /// Jump line is oversubscribed (≥3 fighters on our side) or the
    /// free slot is physically unreachable.  Caller should interrupt
    /// the EnterSwordfight sequence element.
    Abort,
}

/// Check if two entities can enter a swordfight with each other.
fn can_enter_swordfight_with(
    entities: &crate::entities::Entities,
    a: EntityId,
    b: EntityId,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
) -> bool {
    let entity_a = match entities.get(a) {
        Some(e) => e,
        None => {
            tracing::info!(?a, ?b, "can_enter: entity_a missing");
            return false;
        }
    };
    let entity_b = match entities.get(b) {
        Some(e) => e,
        None => {
            tracing::info!(?a, ?b, "can_enter: entity_b missing");
            return false;
        }
    };

    if entity_a.is_dead() || entity_b.is_dead() {
        tracing::info!(?a, ?b, "can_enter: one is dead");
        return false;
    }

    let human_a = match entity_a.human_data() {
        Some(h) => h,
        None => {
            tracing::info!(?a, ?b, "can_enter: a not human");
            return false;
        }
    };
    let human_b = match entity_b.human_data() {
        Some(h) => h,
        None => {
            tracing::info!(?a, ?b, "can_enter: b not human");
            return false;
        }
    };

    if human_a.unconscious || human_b.unconscious {
        tracing::info!(?a, ?b, "can_enter: one is unconscious");
        return false;
    }
    if human_a.stuck_under_nets_counter > 0 || human_b.stuck_under_nets_counter > 0 {
        tracing::info!(?a, ?b, "can_enter: one stuck under net");
        return false;
    }

    // VIP soldiers only fight Robin.
    if entity_a.is_soldier()
        && is_vip_from_profile(entity_a, profile_manager)
        && !entity_b.pc_data().is_some_and(|pc| pc.robin)
    {
        tracing::info!(?a, ?b, "can_enter: VIP a can only fight Robin");
        return false;
    }
    if entity_b.is_soldier()
        && is_vip_from_profile(entity_b, profile_manager)
        && !entity_a.pc_data().is_some_and(|pc| pc.robin)
    {
        tracing::info!(?a, ?b, "can_enter: VIP b can only fight Robin");
        return false;
    }

    // CanEnterSwordfightWith asks the current sector directly whether it is
    // a building. Door transit is not part of this predicate: while crossing
    // an ordinary door, an actor may retain the outside sector and is still a
    // valid opponent for the thrust-A principal-opponent refresh.
    let sector_a = entity_a.element_data().sector();
    let sector_b = entity_b.element_data().sector();
    let inside_a = is_in_building_sector(sector_a, fast_grid);
    let inside_b = is_in_building_sector(sector_b, fast_grid);
    if inside_a || inside_b {
        tracing::info!(?a, ?b, ?sector_a, ?sector_b, "can_enter: building sector");
        return false;
    }

    // Wall/ladder lift check.
    if is_on_wall_or_ladder(sector_a, fast_grid) || is_on_wall_or_ladder(sector_b, fast_grid) {
        tracing::info!(?a, ?b, "can_enter: on wall or ladder");
        return false;
    }

    // NOTE: the cross-sector elevation gate lives inside
    // `enter_swordfight`'s `!already_opponent` branch, not here — two
    // fighters who are *already* opponents can re-enter even after
    // one drifts onto a different-sector elevation.

    true
}

// ─── Victim filtering ───────────────────────────────────────────────

/// Check if `target` is a valid sword strike victim for `attacker`.
pub(crate) fn is_possible_sword_strike_victim(
    entities: &crate::entities::Entities,
    attacker: impl Into<EntityId>,
    target_entity: &Entity,
    target_id: impl Into<EntityId>,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    let attacker = attacker.into();
    let target_id = target_id.into();
    if attacker == target_id {
        return false;
    }
    if !target_entity.is_human() || !target_entity.is_active() {
        return false;
    }
    if target_entity.is_dead() {
        return false;
    }
    if target_entity
        .human_data()
        .map(|h| h.unconscious)
        .unwrap_or(false)
    {
        return false;
    }
    let posture = target_entity.element_data().posture;
    if posture == Posture::Tree
        || target_entity
            .human_data()
            .map(|h| h.stuck_under_nets_counter > 0)
            .unwrap_or(false)
    {
        return false;
    }
    // Only Robin can hurt VIPs.  If the target is a VIP soldier and
    // the attacker is a PC who is NOT Robin, reject the victim.
    if target_entity.is_soldier()
        && is_vip_from_profile(target_entity, profile_manager)
        && let Some(attacker_entity) = entities.get(attacker)
    {
        let is_non_robin_pc = match attacker_entity {
            Entity::Pc(pc) => !pc.pc.robin,
            _ => false,
        };
        if is_non_robin_pc {
            return false;
        }
    }

    // Check sight obstacle between attacker and victim: 3D ray at
    // belt height with the SIGHTOBSTACLE_SOLID type filter, so low
    // walls / counters / fences a sword can be swung over no longer
    // falsely block the strike, and ground-only obstacles (rubble
    // below belt height) don't block either.
    if let Some(attacker_entity) = entities.get(attacker) {
        let att_belt = compute_belt_point(attacker_entity);
        let tgt_belt = compute_belt_point(target_entity);
        let att_layer = attacker_entity.element_data().layer();
        if !fast_grid.is_reachable_3d(
            att_belt,
            tgt_belt,
            att_layer,
            crate::sight_obstacle::SIGHTOBSTACLE_SOLID,
            obstacles,
        ) {
            return false;
        }
    }

    true
}

/// Collect possible victims for a lateral/circle sword strike within an angular arc.
///
/// Returns EntityIds of all valid targets within `[min_distance, max_distance]`
/// whose direction from the attacker falls between `begin_sector` and `end_sector`.
#[allow(clippy::too_many_arguments)]
fn collect_arc_victims(
    entities: &Entities,
    attacker_id: EntityId,
    attacker_pos: (f32, f32),
    min_distance: f32,
    max_distance: f32,
    begin_sector: u8,
    end_sector: u8,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<EntityId> {
    let mut victims = Vec::new();
    for (target_id, entity) in entities.humans() {
        if !is_possible_sword_strike_victim(
            entities,
            attacker_id,
            entity,
            target_id,
            profile_manager,
            fast_grid,
            obstacles,
        ) {
            continue;
        }
        // Original computes strike geometry from GetPositionMap(); elevation
        // is handled independently by IsPossibleSwordStrikeVictim.
        let pos = entity.element_data().position_map();
        let dx = pos.x - attacker_pos.0;
        let dy = (pos.y - attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        // Quick reject
        if dx.abs().max(dy.abs()) >= 150.0 {
            continue;
        }
        let distance = (dx * dx + dy * dy).sqrt();
        if distance < min_distance || distance > max_distance {
            continue;
        }
        // Check if direction is within the arc
        let sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
        if is_sector_between(sector, begin_sector, end_sector) {
            victims.push(target_id.into());
        }
    }
    victims
}

/// Collect the seed list for `ExecuteLateralSwordStrike`.
///
/// The Original deliberately mixes coordinate spaces here: admission uses
/// `GetPositionGround()` for the angular sector and the full 3D `GetPosition()`
/// norm for weapon range.  Once seeded, the per-frame sweep tests the moving
/// victim in map space.  Using map space for this initial test can admit an
/// actor on a different elevation whose ground-space direction lies outside
/// the strike arc.
#[allow(clippy::too_many_arguments)]
fn collect_lateral_strike_victims(
    entities: &Entities,
    attacker_id: EntityId,
    attacker_position: crate::coordinates::WorldPoint3D,
    min_distance: f32,
    max_distance: f32,
    begin_sector: u8,
    end_sector: u8,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<EntityId> {
    let mut victims = Vec::new();
    for (target_id, entity) in entities.humans() {
        if !is_possible_sword_strike_victim(
            entities,
            attacker_id,
            entity,
            target_id,
            profile_manager,
            fast_grid,
            obstacles,
        ) {
            continue;
        }

        let target_position = entity.element_data().position();
        let dx = target_position.x - attacker_position.x;
        let dy = (target_position.y - attacker_position.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        let dz = target_position.z - attacker_position.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        if distance < min_distance || distance > max_distance {
            continue;
        }

        let sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
        if is_sector_between(sector, begin_sector, end_sector) {
            victims.push(target_id.into());
        }
    }
    victims
}

/// Original lateral warning admission is intentionally looser than the hit
/// collector: active human, not self, geometry only.
fn collect_lateral_warning_victims(
    entities: &Entities,
    attacker_id: EntityId,
    attacker_pos: (f32, f32),
    min_distance: f32,
    max_distance: f32,
    begin_sector: u8,
    end_sector: u8,
) -> Vec<EntityId> {
    let mut victims = Vec::new();
    for (target_id, entity) in entities.humans() {
        let target_id: EntityId = target_id.into();
        if target_id == attacker_id || !entity.element_data().active {
            continue;
        }
        let pos = entity.element_data().position_map();
        let dx = pos.x - attacker_pos.0;
        let dy = (pos.y - attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        if dx.abs().max(dy.abs()) >= 150.0 {
            continue;
        }
        let distance = (dx * dx + dy * dy).sqrt();
        if distance < min_distance || distance > max_distance {
            continue;
        }
        let sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
        if is_sector_between(sector, begin_sector, end_sector) {
            victims.push(target_id);
        }
    }
    victims
}

/// Extra circle-strike warning range for an actor walking with a sword.
///
/// `RHElementActorHuman::GetPossibleVictimsOfCircleSwordStrike` divides by
/// `RHSword::GetStrikeRotationAngle`, whose profile degrees are converted to
/// radians before this formula is evaluated, then narrows the extension to
/// `UWORD` before adding it to the strike's maximal distance.
fn circle_warning_walking_tolerance(relative_sector: u16, rotation_angle_deg: u16) -> u16 {
    let rotation_angle =
        ((f64::from(rotation_angle_deg) / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
    (10.0 + (f32::from(relative_sector) * 5.0 * std::f32::consts::PI) / (8.0 * rotation_angle))
        as u16
}

/// Collect possible victims for a circle sword strike in the
/// WarnForStrike phase, with the per-victim distance extension for
/// walking-with-sword enemies.
#[allow(clippy::too_many_arguments)]
fn collect_circle_warn_victims(
    entities: &Entities,
    attacker_id: EntityId,
    attacker_pos: (f32, f32),
    attacker_direction: i16,
    base_max_distance: f32,
    rotation_angle_deg: u16,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<EntityId> {
    let mut victims = Vec::new();
    for (target_id, entity) in entities.humans() {
        if !is_possible_sword_strike_victim(
            entities,
            attacker_id,
            entity,
            target_id,
            profile_manager,
            fast_grid,
            obstacles,
        ) {
            continue;
        }
        let pos = entity.element_data().position_map();
        // GetPossibleVictimsOfCircleSwordStrike forms this vector as
        // attacker - victim. Distance is symmetric, but the same vector's
        // sector drives the walking-with-sword warning tolerance below.
        let dx = attacker_pos.0 - pos.x;
        let dy = (attacker_pos.1 - pos.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        if dx.abs().max(dy.abs()) >= 150.0 {
            continue;
        }
        let distance = (dx * dx + dy * dy).sqrt();

        // For walking-with-sword enemies, add a per-victim tolerance
        // so the warn covers actors about to enter the arc during the
        // strike's rotation.
        let mut max_dist = base_max_distance;
        let walking_with_sword = entity
            .actor_data()
            .map(|a| a.action_state == ActionState::MovingSword)
            .unwrap_or(false);
        if walking_with_sword {
            let enemy_sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
            let relative = ((enemy_sector + 16 - attacker_direction) % 16) as u16;
            max_dist += f32::from(circle_warning_walking_tolerance(
                relative,
                rotation_angle_deg,
            ));
        }
        if distance <= max_dist {
            victims.push(target_id.into());
        }
    }
    victims
}

/// Parameters for push-strike victim collection.
struct PushStrikeParams {
    attacker_id: EntityId,
    attacker_pos: (f32, f32),
    attacker_elevation: f32,
    position_space: PushStrikePositionSpace,
    dir_x: f32,
    dir_y: f32,
    min_distance: f32,
    max_distance: f32,
    half_width: f32,
}

#[derive(Clone, Copy)]
enum PushStrikePositionSpace {
    Map,
    Ground,
}

/// Collect possible victims for a push (rectangle) sword strike.
///
/// The hit area is a rectangle in front of the attacker: `[min_dist, max_dist]` deep
/// and `[-width/2, +width/2]` wide, measured along the attacker's facing direction.
fn collect_push_victims(
    entities: &Entities,
    params: &PushStrikeParams,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<EntityId> {
    let PushStrikeParams {
        attacker_id,
        attacker_pos,
        attacker_elevation,
        position_space,
        dir_x,
        dir_y,
        min_distance,
        max_distance,
        half_width,
    } = *params;
    // Direction vector (stretched Y for isometric)
    let dir_sy = dir_y * INVERSE_SWORDFIGHT_ASPECT_RATIO;
    let len = (dir_x * dir_x + dir_sy * dir_sy).sqrt();
    if len < 0.001 {
        return Vec::new();
    }
    let fx = dir_x / len;
    let fy = dir_sy / len;
    // Side vector (perpendicular)
    let sx = -fy;
    let sy = fx;

    let mut victims = Vec::new();
    for (target_id, entity) in entities.humans() {
        if !is_possible_sword_strike_victim(
            entities,
            attacker_id,
            entity,
            target_id,
            profile_manager,
            fast_grid,
            obstacles,
        ) {
            continue;
        }
        // Reject victims whose elevation differs from the attacker by
        // more than MAX_ELEVATION_SWORDFIGHT (prevents push strikes
        // across catwalks / stairs).
        let victim_elev = entity.position_iface().get_elevation();
        if (attacker_elevation - victim_elev).abs() > MAX_ELEVATION_SWORDFIGHT {
            continue;
        }
        let (pos_x, pos_y) = match position_space {
            // GetPossibleVictimsOfPushSwordStrike, used by WarnForStrike,
            // projects from GetPositionMap(). ExecutePushSwordStrike's DONE
            // effect instead projects from GetPositionGround().
            PushStrikePositionSpace::Map => {
                let map = entity.element_data().position_map();
                (map.x, map.y)
            }
            PushStrikePositionSpace::Ground => {
                let ground = entity.ground_position();
                (ground.x, ground.y)
            }
        };
        let dx = pos_x - attacker_pos.0;
        let dy = (pos_y - attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        if dx.abs().max(dy.abs()) >= 150.0 {
            continue;
        }
        let front_dist = dx * fx + dy * fy;
        let side_dist = (dx * sx + dy * sy).abs();
        if front_dist >= min_distance && front_dist <= max_distance && side_dist <= half_width {
            victims.push(target_id.into());
        }
    }
    victims
}

/// Check if `sector` is between `begin` and `end` (inclusive, wrapping 0-15).
fn is_sector_between(sector: u8, begin: u8, end: u8) -> bool {
    if begin <= end {
        sector >= begin && sector <= end
    } else {
        // Wraps around (e.g., begin=14, end=2 means 14,15,0,1,2)
        sector >= begin || sector <= end
    }
}

/// Convert a 0-15 direction sector to an angle in radians.
/// Sector 0 = north (negative Y), increasing clockwise.
/// The trailing `+ 0.1` rad nudges the result a fraction past the
/// sector's begin edge so `angle_to_sector` round-trips to the same sector.
fn sector_to_angle(sector: i16) -> f32 {
    // Original's `(sector / 16.0) * 2.0 * PI + 0.1` uses double
    // intermediates because its decimal literals are unsuffixed, then
    // narrows the result to FLOAT. That last bit matters to circle strikes:
    // repeated FLOAT rotation additions can otherwise stop an epsilon short
    // of the final angle and hold the animation for one extra Hourglass.
    ((sector as f64 / 16.0) * 2.0 * f64::from(std::f32::consts::PI) + 0.1) as f32
}

/// Map a SwordStrike to its animation OrderType.
fn strike_to_animation(strike: SwordStrike) -> crate::order::OrderType {
    use crate::order::OrderType;
    match strike {
        SwordStrike::A => OrderType::StrikingStraightSword,
        SwordStrike::B => OrderType::StrikingStraightStrongSword,
        SwordStrike::C => OrderType::ExecutingSword,
        SwordStrike::D => OrderType::StrikingLeftSword,
        SwordStrike::E => OrderType::StrikingRightSword,
        SwordStrike::F => OrderType::StrikingSemiroundLeftSword,
        SwordStrike::G => OrderType::StrikingSemiroundRightSword,
        SwordStrike::H => OrderType::StrikingRoundLeftSword,
        SwordStrike::I => OrderType::StrikingRoundRightSword,
        SwordStrike::Charge => OrderType::StrikingStraightStrongSword, // charge uses strong strike anim
        SwordStrike::SmalltalkLeft => OrderType::StrikingLeftSmalltalk,
        SwordStrike::SmalltalkRight => OrderType::StrikingRightSmalltalk,
    }
}

/// Original `RHElementActorHuman::GetSwordStrikeFromAnimation`.
///
/// Strike startup may select a replacement animation (for example, a requested
/// right strike can be rendered by the left-strike row). Reactive defenders
/// observe that live animation, not the sequence command that requested it.
pub(crate) fn sword_strike_from_animation(
    animation: crate::order::OrderType,
) -> Option<SwordStrike> {
    use crate::order::OrderType;
    match animation {
        OrderType::StrikingStraightSword => Some(SwordStrike::A),
        OrderType::StrikingStraightStrongSword => Some(SwordStrike::B),
        OrderType::ExecutingSword => Some(SwordStrike::C),
        OrderType::StrikingLeftSword => Some(SwordStrike::D),
        OrderType::StrikingRightSword => Some(SwordStrike::E),
        OrderType::StrikingSemiroundLeftSword => Some(SwordStrike::F),
        OrderType::StrikingSemiroundRightSword => Some(SwordStrike::G),
        OrderType::StrikingRoundLeftSword => Some(SwordStrike::H),
        OrderType::StrikingRoundRightSword => Some(SwordStrike::I),
        _ => None,
    }
}

/// Convert an angle in radians to a 0-15 sector.
///
/// Positive angles use Original's truncating ULONG conversion and modulo;
/// negative angles use its recursive mirror rule.
fn angle_to_sector(angle: f32) -> u8 {
    if angle >= 0.0 {
        ((f64::from(angle) / (2.0 * f64::from(std::f32::consts::PI)) * 16.0) as u32 % 16) as u8
    } else {
        // SBGeoVector2D::AngleToSector mirrors negative angles recursively;
        // this differs from normalization at exact negative boundaries.
        16 - angle_to_sector(-angle) - 1
    }
}

/// `RHSword::GetStrike*Angle` keeps its unsuffixed-literal expression in
/// double precision and narrows only at the FLOAT return boundary.
fn strike_profile_angle(degrees: u16) -> f32 {
    ((f64::from(degrees) / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32
}

/// Push collectors halve an authored UWORD width with integer division
/// before promoting the result for the GEOTYPE side-distance comparison.
fn push_strike_half_width(repulsion: u16) -> f32 {
    f32::from(repulsion / 2)
}

// ─── Animation selection ────────────────────────────────────────────

/// Animation category for combat state transitions.
///
/// Used by the damage-translation paths (sword / push / hit / arrow).
#[derive(Debug, Clone, Copy)]
struct CombatAnimations {
    falling_back: crate::order::OrderType,
    dying_forward: crate::order::OrderType,
    /// Used by stand-up-after-push sequences (TranslatePushDamage).
    standing_up: crate::order::OrderType,
    /// Used by non-KO hit reactions (TranslateSwordDamage simple hit path).
    simple_hit: crate::order::OrderType,
    /// Survivor animation for arrow / piercing hits.
    /// `ExtractingArrow{Upright,Crouched,Sword,Bow}` per the
    /// posture/action switch.
    arrow_extract: crate::order::OrderType,
}

/// Select combat animations based on current posture and action state.
fn select_combat_animations(
    posture: Posture,
    action_state: ActionState,
) -> Option<CombatAnimations> {
    use crate::order::OrderType;
    match posture {
        // `Undefined` is treated as `Upright` everywhere else (see
        // sprite-row selection in `element.rs`).  Without it in this
        // arm, NPCs that still carry the default load-time posture
        // (soldiers never explicitly set it) get no falling / push /
        // hit animation at KO time.
        Posture::Upright
        | Posture::Undefined
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::Sitting => {
            if action_state.is_sword() || action_state == ActionState::Menacing {
                Some(CombatAnimations {
                    falling_back: OrderType::FallingBackSword,
                    dying_forward: OrderType::DyingSword,
                    standing_up: OrderType::StandingUpSword,
                    simple_hit: OrderType::BeingHitSword,
                    arrow_extract: OrderType::ExtractingArrowSword,
                })
            } else if action_state.is_bow() {
                Some(CombatAnimations {
                    falling_back: OrderType::FallingBackBow,
                    dying_forward: OrderType::DyingBow,
                    standing_up: OrderType::StandingUpBow,
                    simple_hit: OrderType::FallingBackBow,
                    arrow_extract: OrderType::ExtractingArrowBow,
                })
            } else {
                Some(CombatAnimations {
                    falling_back: OrderType::FallingBackUpright,
                    dying_forward: OrderType::DyingUpright,
                    standing_up: OrderType::StandingUp,
                    simple_hit: OrderType::FallingBackUpright,
                    arrow_extract: OrderType::ExtractingArrowUpright,
                })
            }
        }
        Posture::Crouched | Posture::SimulatingBeggar | Posture::Tree => Some(CombatAnimations {
            falling_back: OrderType::FallingBackCrouched,
            dying_forward: OrderType::DyingCrouched,
            standing_up: OrderType::StandingUp,
            simple_hit: OrderType::FallingBackCrouched,
            arrow_extract: OrderType::ExtractingArrowCrouched,
        }),
        // Already lying / dead / carried — no animation needed
        _ => None,
    }
}

/// Push-damage animation set, selected based on posture and action state.
#[derive(Debug, Clone, Copy)]
struct PushDamageAnimations {
    /// The falling-pushed animation to play.
    falling: crate::order::OrderType,
    /// Standing-up animation (None for crouched / no standup).
    standing_up: Option<crate::order::OrderType>,
    /// Stunned animation if concussion > threshold (None if not applicable).
    stunned: Option<crate::order::OrderType>,
}

/// Select push-damage animations based on posture and action state.
///
/// Returns `None` for postures that don't get a falling animation
/// (already lying, dead, carried, on ladder/wall, etc.).
fn select_push_damage_animations(
    posture: Posture,
    action_state: ActionState,
) -> Option<PushDamageAnimations> {
    use crate::order::OrderType;
    match posture {
        // `Undefined` is treated as `Upright` everywhere else (see
        // sprite-row selection in `element.rs`).  Without it in this
        // arm, NPCs that still carry the default load-time posture
        // (soldiers never explicitly set it) get no falling / push /
        // hit animation at KO time.
        Posture::Upright
        | Posture::Undefined
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::Sitting => {
            if action_state.is_sword() || action_state == ActionState::Menacing {
                Some(PushDamageAnimations {
                    falling: OrderType::FallingPushedWithSword,
                    standing_up: Some(OrderType::StandingUpSword),
                    stunned: Some(OrderType::BeingStunnedSword),
                })
            } else if action_state.is_bow() {
                Some(PushDamageAnimations {
                    falling: OrderType::FallingPushedWithBow,
                    standing_up: Some(OrderType::StandingUpBow),
                    stunned: None,
                })
            } else {
                // Waiting, bored, moving, shield, sleeping, listening.
                Some(PushDamageAnimations {
                    falling: OrderType::FallingPushedUpright,
                    standing_up: Some(OrderType::StandingUp),
                    stunned: None,
                })
            }
        }
        Posture::Crouched | Posture::SimulatingBeggar | Posture::Tree => {
            Some(PushDamageAnimations {
                falling: OrderType::FallingPushedCrouched,
                standing_up: None,
                stunned: None,
            })
        }
        Posture::OnLadder | Posture::OnWall => {
            // `translate_ladder_wall_fall` handles this case.  The
            // caller's `apply_push_effect` detects OnLadder/OnWall
            // before calling this function and branches into that
            // helper instead; returning an animation here would double
            // up the work.
            None
        }
        // Already lying, dead, carried, tied, flying: no animation
        _ => None,
    }
}

/// Select the falling-hit animation for `TranslateHitDamage`.
///
/// Returns `None` for postures treated as already-falling / dead /
/// carried (LYING, FLYING, CARRIED, ON_SHOULDERS, TIED, DEAD,
/// DEAD_BACK, STUCK_UNDER_NET).
///
/// When `harder` is true, the `HARDER` variant is returned. The harder
/// variant plays in place and collapses to `Lying` at the end, while
/// the non-harder variant flights 30 units away from the attacker.
pub(in crate::engine) fn select_hit_fall_animation(
    posture: Posture,
    action_state: ActionState,
    harder: bool,
) -> Option<crate::order::OrderType> {
    use crate::order::OrderType;
    match posture {
        Posture::Upright
        | Posture::Undefined
        | Posture::Spy
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::AnonymousArcher
        | Posture::Sitting => {
            if action_state.is_bow() {
                Some(if harder {
                    OrderType::FallingHitHarderWithBow
                } else {
                    OrderType::FallingHitWithBow
                })
            } else if action_state.is_sword() || action_state == ActionState::Menacing {
                Some(if harder {
                    OrderType::FallingHitHarderWithSword
                } else {
                    OrderType::FallingHitWithSword
                })
            } else {
                Some(if harder {
                    OrderType::FallingHitHarderUpright
                } else {
                    OrderType::FallingHitUpright
                })
            }
        }
        Posture::Crouched | Posture::Tree | Posture::SimulatingBeggar => Some(if harder {
            OrderType::FallingHitHarderCrouched
        } else {
            OrderType::FallingHitCrouched
        }),
        // TranslateHitDamage just terminates for these postures —
        // no animation needed.
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════
//  EngineInner methods
// ═══════════════════════════════════════════════════════════════════

// Submodules (extracted from the original melee.rs mega-file).
mod damage;
#[cfg(test)]
pub(crate) use damage::{
    clear_test_sword_damage_observations, take_test_sword_damage_observations,
};
mod dispatch;
pub(super) use dispatch::ShieldCommandContext;
mod effects;
mod evaluate;
mod speech;
mod strikes;
mod swordfight;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActiveFlight, ActorCivilian, ActorData, ActorPc, ActorSoldier, CivilianData, ElementData,
        ElementKind, HumanData, NpcData, PcData, SoldierData,
    };
    use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};

    #[test]
    fn sector_to_angle_keeps_original_double_intermediate_rounding() {
        let direction_angle = sector_to_angle(9);
        assert_eq!(direction_angle.to_bits(), 0x4068_983d);

        // Profile angle getters likewise narrow 45 degrees to this FLOAT.
        // Three rotation ticks must reach the true-half-circle final angle
        // exactly, allowing ExecuteTrueCircleSwordStrikeAction to resume the
        // sprite animation on the terminal-direction Hourglass.
        let quarter_turn = ((45.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
        let initial_angle = direction_angle - quarter_turn;
        let final_angle = initial_angle + std::f32::consts::PI;
        let after_three_ticks = direction_angle + quarter_turn + quarter_turn + quarter_turn;

        assert_eq!(after_three_ticks.to_bits(), final_angle.to_bits());
        assert_eq!(final_angle.to_bits(), 0x40bf_b210);
    }

    #[test]
    fn strike_collector_angles_and_push_width_keep_original_conversions() {
        let expected_angle = ((7.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
        assert_eq!(strike_profile_angle(7).to_bits(), expected_angle.to_bits());
        assert_eq!(angle_to_sector(-std::f32::consts::PI / 8.0), 14);
        assert_eq!(push_strike_half_width(5), 2.0);
        assert_eq!(push_strike_half_width(6), 3.0);
    }

    #[test]
    fn sweep_state_uses_angles_returned_by_original_sword_getters() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(WorldPoint3D::new(10.0, 0.0, 0.0), None));
        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::D,
            crate::profiles::WeaponThrustKind::Lateral,
        );
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::D as usize];
        thrust.initial_angle = 5;
        thrust.final_angle = 5;
        thrust.rotation_angle = 5;

        engine.initialize_sweep(
            &assets,
            attacker,
            SwordStrike::D,
            Some(1),
            crate::profiles::WeaponThrustKind::Lateral,
            vec![victim],
        );
        let direction_angle = sector_to_angle(
            engine
                .get_entity(attacker)
                .unwrap()
                .element_data()
                .direction(),
        );
        let sweep = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .unwrap();
        let five_degrees = f32::from_bits(0x3db2_b8c3);
        assert_eq!(
            sweep.initial_angle.to_bits(),
            (direction_angle - five_degrees).to_bits()
        );
        assert_eq!(
            sweep.final_angle.to_bits(),
            (direction_angle + five_degrees).to_bits()
        );
        assert_eq!(sweep.rotation_per_frame.to_bits(), five_degrees.to_bits());

        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = None;
        engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .as_ref()
                .unwrap()
                .rotation_per_frame
                .to_bits(),
            five_degrees.to_bits(),
            "loaded sweep reconstruction uses the same RHSword getter conversion"
        );

        // Keep a common authored angle as a control alongside the one-bit 5° case.
        assert_eq!(strike_profile_angle(45).to_bits(), 0x3f49_0fdb);
    }

    #[test]
    fn circle_warning_tolerance_uses_radians_returned_by_sword_profile() {
        // The profile stores 180 degrees, but RHSword::GetStrikeRotationAngle
        // returns PI radians. At relative sector 8 Original therefore extends
        // the warning range by 15 units: 10 + (8 * 5 * PI) / (8 * PI).
        let tolerance = circle_warning_walking_tolerance(8, 180);
        assert_eq!(tolerance, 15);

        let base_max_distance = 60.0;
        let walking_target_distance = 74.0;
        assert!(walking_target_distance <= base_max_distance + f32::from(tolerance));

        // Dividing by the raw profile degrees, as the old port did, would
        // reject this moving defender and suppress its WarnForStrike callback.
        let raw_degrees_tolerance = 10.0 + (8.0 * 5.0 * std::f32::consts::PI) / (8.0 * 180.0);
        assert!(walking_target_distance > base_max_distance + raw_degrees_tolerance);
        assert!(walking_target_distance > base_max_distance);

        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let target = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: walking_target_distance,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::MovingSword;
        let assets = assets_with_sword_profile(0, base_max_distance as u16);
        let collect = |engine: &EngineInner| {
            collect_circle_warn_victims(
                &engine.world.entities,
                attacker,
                (0.0, 0.0),
                0,
                base_max_distance,
                180,
                &assets.profile_manager,
                &engine.world.fast_grid,
                crate::sight_obstacle::ObstacleList {
                    static_obstacles: assets.static_sight_obstacles.as_slice(),
                    dynamic_obstacles: &engine.world.dynamic_sight_obstacles,
                    static_active: &engine.world.static_sight_obstacle_active,
                },
            )
        };
        assert_eq!(collect(&engine), vec![target]);

        engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::WaitingSword;
        assert!(collect(&engine).is_empty());
    }

    #[test]
    fn straight_strike_range_uses_stored_world_position() {
        let mut engine = EngineInner::new();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let target = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(60.0, 40.0, 40.0));

        // The isometric projection subtracts elevation from world Y, so
        // these actors are only 60 map units apart while Original's
        // ExecuteStraightSwordStrike range check sees all three components.
        assert_eq!(
            entity_distance(&engine.world.entities, attacker, target),
            60.0
        );
        assert_eq!(
            entity_world_distance(&engine.world.entities, attacker, target),
            (60.0_f32 * 60.0 + 40.0 * 40.0 + 40.0 * 40.0).sqrt()
        );
    }

    #[test]
    #[should_panic(expected = "straight-strike distance references missing victim Soldier")]
    fn straight_strike_range_rejects_a_missing_victim() {
        let mut engine = EngineInner::new();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let missing = EntityId::Soldier(crate::entity_id::SoldierId(u32::MAX));

        let _ = entity_world_distance(&engine.world.entities, attacker, missing);
    }

    fn make_engine() -> EngineInner {
        let mut engine = EngineInner::new();
        // Every PC built by `make_pc` carries campaign-description index 0,
        // so the campaign character table needs a matching entry backing the
        // required live-PC identity.
        engine.mission_domain.campaign.characters = vec![crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            ..Default::default()
        }];
        engine
    }

    fn empty_mission_script() -> crate::engine::types::MissionScript {
        let startup = ClassEntry {
            source_file: "melee_test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: Vec::new(),
            quads: Vec::new(),
        };
        crate::engine::types::MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![startup],
        })
        .expect("minimal StartUp script must load")
    }

    fn make_soldier(
        pos: WorldPoint3D,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(pos);
        element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
        element.set_sector(sector);
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 50,
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..NpcData::default()
            },
            soldier: SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..SoldierData::default()
            },
        })
    }

    fn make_pc(
        pos: WorldPoint3D,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(pos);
        element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
        element.set_sector(sector);
        Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                life_points: 50,
                profile_index: crate::profiles::CharacterProfileIdx(0),
                campaign_description_index: Some(0),
                ..PcData::default()
            },
        })
    }

    #[test]
    fn damage_dispatcher_disables_direction_on_live_reaction_orders() {
        for (command, expected) in [
            (Command::ReceiveDamage, OrderType::FallingBackUpright),
            (Command::ReceiveMobileDamage, OrderType::FallingBackUpright),
            (
                Command::ReceiveArrowDamage,
                OrderType::ExtractingArrowUpright,
            ),
            (Command::ReceiveStoneDamage, OrderType::FallingBackUpright),
        ] {
            let sim = crate::sim_rng::test_context();
            let mut engine = make_engine();
            let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
            let victim = engine.add_entity(make_pc(
                WorldPoint3D {
                    x: 10.0,
                    ..WorldPoint3D::ZERO
                },
                None,
            ));
            let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
            let mut damage = crate::sequence::SequenceElement::new_damage(
                1,
                command,
                Some(victim),
                Some(attacker),
                1,
                0,
            );
            engine.resolve_element_priority(&mut damage);
            let sequence = engine.orders.sequence_manager.launch_element(damage);
            let mut display = crate::engine::HostDisplayState::default();
            engine.hourglass_phase_sequences(&sim, &mut display, &assets);

            let element = engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("translated damage command remains registered");
            assert_eq!(element.command, command);
            assert!(
                element
                    .orders
                    .iter()
                    .any(|order| order.order_type == expected),
                "{command:?} must author {expected:?}"
            );
            assert!(
                !element
                    .orders
                    .iter()
                    .find(|order| order.order_type == expected)
                    .unwrap()
                    .compute_direction
            );
        }
    }

    fn action_test_assets(actions: [crate::profiles::Action; 3]) -> LevelAssets {
        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(crate::profiles::CharacterProfile {
            actions,
            ..Default::default()
        });
        LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        }
    }

    fn make_civilian(pos: WorldPoint3D) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorCivilian,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(pos);
        element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
            pos.x, pos.y, pos.z,
        ));
        Entity::Civilian(ActorCivilian {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 100,
                ..NpcData::default()
            },
            civilian: CivilianData {
                cached_camp: crate::element::Camp::Royalists,
                ..CivilianData::default()
            },
        })
    }

    /// Set up a live falling-hit Execute flight on `flyer` so the per-frame
    /// `tick_push_flights` sweep fires `apply_domino_effect`.
    fn give_flight(
        engine: &mut EngineInner,
        flyer: EntityId,
        antagonist: EntityId,
        inc_x: f32,
        inc_y: f32,
        frames: u16,
    ) {
        let flyer_pos = engine
            .get_entity(flyer)
            .unwrap()
            .element_data()
            .position_map();

        // Original owns combat flight from the live falling order's Execute
        // arm. Mirror that lifecycle instead of manufacturing an orphaned
        // `active_flight`, which production correctly holds until the order is
        // current and its START edge has changed posture to Flying.
        let damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveHitDamage,
            Some(flyer),
            Some(antagonist),
            1,
            0,
        );
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let order_id = engine.push_new_order(
            sequence,
            0,
            crate::order::OrderType::FallingHitUpright,
            0.0,
            0.0,
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        if let Some(entity) = engine.world.entities.get_mut(flyer) {
            entity.set_posture(Posture::Flying);
            let actor = entity
                .actor_data_mut()
                .expect("combat flight owner must be an actor");
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id,
                order_type: crate::order::OrderType::FallingHitUpright,
            });
            actor.active_flight = Some(ActiveFlight {
                increment_x: inc_x,
                increment_y: inc_y,
                goal_x: flyer_pos.x + inc_x * frames as f32,
                goal_y: flyer_pos.y + inc_y * frames as f32,
                frames_remaining: frames,
                antagonist: Some(antagonist),
                ..Default::default()
            });
        }
    }

    fn count_domino_hits_for(engine: &EngineInner, victim: EntityId, hitter: EntityId) -> usize {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .filter(|e| {
                e.command == Command::ReceiveHitDamage
                    && e.owner == Some(victim)
                    && match &e.data {
                        SequenceElementData::Damage {
                            origin,
                            damage,
                            concussion,
                            is_harder_hit,
                            ..
                        } => {
                            *origin == Some(hitter)
                                && *damage == 0
                                && *concussion == DOMINO_DAMAGE
                                && !*is_harder_hit
                        }
                        _ => false,
                    }
            })
            .count()
    }

    #[test]
    fn hit_translation_defers_flight_facing_until_first_execute() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 30.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().set_layer(4);
            let position = victim_entity.position_iface_mut();
            position.set_direction_instantly(crate::position_interface::Direction::from_raw(5));
            position.set_move_box(crate::coordinates::MoveBox::from_coords(
                -5.0, -5.0, 5.0, 5.0,
            ));
        }

        let element =
            crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
        let seq_id = engine.launch_element(element);
        engine.dispatch_hit_fall_animation(
            &LevelAssets::default(),
            victim,
            Some(attacker),
            false,
            (seq_id, 0),
        );

        let queued = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .orders
            .back()
            .unwrap();
        assert_eq!(queued.order_type, OrderType::FallingHitUpright);
        assert_eq!(queued.antagonist, Some(attacker));
        assert!(!queued.compute_direction);
        let queued_type = queued.order_type;
        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(victim_entity.element_data().direction(), 5);
        assert_eq!(victim_entity.position_iface().layer_goal().get(), 0);
        assert!(victim_entity.actor_data().unwrap().active_flight.is_none());

        engine.initialize_hit_flight(&LevelAssets::default(), victim, Some(attacker), queued_type);

        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .layer_goal()
                .get(),
            4,
            "ReadyForTakeOff publishes its authored goal layer immediately"
        );
        assert_ne!(
            engine
                .get_entity(victim)
                .unwrap()
                .element_data()
                .direction(),
            5
        );
    }

    fn initialized_hit_flight_delta(
        engine: &EngineInner,
        victim: EntityId,
    ) -> crate::coordinates::MapPoint {
        let victim = engine.get_entity(victim).unwrap();
        let flight = victim
            .actor_data()
            .unwrap()
            .active_flight
            .as_ref()
            .expect("unobstructed falling hit must initialize a flight");
        let position = victim.element_data().position_map();
        crate::coordinates::MapPoint::new(flight.goal_x - position.x, flight.goal_y - position.y)
    }

    fn authorize_test_hit_flight(engine: &mut EngineInner, victim: EntityId) {
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -5.0, -5.0, 5.0, 5.0,
            ));
    }

    #[test]
    fn charging_rider_falling_hit_normalizes_non_cardinal_sector_vector() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 32.0,
                y: 1.0,
                z: 0.0,
            },
            None,
        ));
        authorize_test_hit_flight(&mut engine, victim);
        {
            let Entity::Soldier(attacker) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            attacker.soldier.rider = true;
            attacker.actor.active_rider_charge = Some(crate::element::ActiveRiderCharge {
                pending_victims: vec![victim],
            });
            attacker.element.set_direction_instantly(11);
        }

        engine.initialize_hit_flight(
            &LevelAssets::new(),
            victim,
            Some(attacker),
            OrderType::FallingHitUpright,
        );

        let delta = initialized_hit_flight_delta(&engine, victim);
        assert_eq!(delta.x.to_bits(), 0xc1e9_801b);
        assert_eq!(delta.y.to_bits(), 0x40dd_e72e);
        assert!(
            delta.x < 0.0 && delta.y > 0.0,
            "direction 11 flies southwest"
        );
    }

    #[test]
    fn antagonistless_falling_hit_normalizes_opposite_non_cardinal_sector_vector() {
        let mut engine = make_engine();
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 32.0,
                y: 1.0,
                z: 0.0,
            },
            None,
        ));
        authorize_test_hit_flight(&mut engine, victim);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(3));

        engine.initialize_hit_flight(
            &LevelAssets::new(),
            victim,
            None,
            OrderType::FallingHitUpright,
        );

        let delta = initialized_hit_flight_delta(&engine, victim);
        assert_eq!(delta.x.to_bits(), 0xc1e9_801b);
        assert_eq!(delta.y.to_bits(), 0x40dd_e72e);
        assert!(
            delta.x < 0.0 && delta.y > 0.0,
            "opposite direction 11 flies southwest"
        );
    }

    #[test]
    fn positioned_antagonist_falling_hit_keeps_radial_normalization() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: -2.0,
                y: -4.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            None,
        ));
        authorize_test_hit_flight(&mut engine, victim);

        engine.initialize_hit_flight(
            &LevelAssets::new(),
            victim,
            Some(attacker),
            OrderType::FallingHitUpright,
        );

        let delta = initialized_hit_flight_delta(&engine, victim);
        // Adding the exact source component 0x4176_f53d to x=1 and
        // subtracting the origin rounds the observable displacement once;
        // the old per-component normalization instead produced 0x4176_f53e.
        assert_eq!(delta.x.to_bits(), 0x4176_f53c);
        assert_eq!(delta.y.to_bits(), 0x41cd_cc5e);
    }

    #[test]
    fn ladder_fall_translation_retains_layer_goal_and_authors_landing_target() {
        let mut engine = make_engine();
        engine.scripts.mission = Some(empty_mission_script());

        let lift_sector = crate::sector::SectorNumber::new(42);
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        level.sector_number_map.insert(lift_sector, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number: lift_sector,
            door_index: None,
            lift_type: Some(crate::sector::LiftType::Ladder),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: Some(0),
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_out: crate::coordinates::MapPoint::new(30.0, 0.0),
                layer_out: 3,
                sector_out: crate::sector::SectorNumber::new(7),
                ..crate::gate::Door::default()
            });

        let victim = engine.add_entity(make_pc(
            WorldPoint3D::default(),
            crate::position_interface::SectorHandle::new(42),
        ));
        let damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
        let sequence = engine.launch_element(damage);

        engine.translate_ladder_wall_fall(&LevelAssets::default(), victim, (sequence, 0));

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_entity.position_iface().layer_goal().get(),
            0,
            "translation must not publish the destination layer before arrival"
        );
        let flight = victim_entity
            .actor_data()
            .unwrap()
            .active_flight
            .as_ref()
            .expect("a non-trivial ladder fall installs a flight");
        assert_eq!(flight.goal_layer, 3);
        assert_eq!(
            flight.goal_sector,
            crate::position_interface::SectorHandle::new(7)
        );
        assert!(flight.ladder_fall);
    }

    #[test]
    fn pc_hit_translation_inherits_silent_human_say_ouch() {
        let mut engine = make_engine();
        let victim = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
        let sequence_id = engine.launch_element(damage);

        engine.apply_hit_damage(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            victim,
            None,
            1,
            false,
            (sequence_id, 0),
        );

        assert!(
            engine.feedback.sound_sim.pending_exclamations.is_empty(),
            "PC inherits RHElementActorHuman::SayOuch's no-op on TranslateHitDamage"
        );
    }

    #[test]
    fn conscious_hit_applies_ai_eye_status_synchronously() {
        let mut engine = make_engine();
        let null_slot = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        engine
            .get_entity_mut(null_slot)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
        let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                y: 0.0,
                z: 0.0,
            },
            None,
        ));
        engine
            .get_entity_mut(victim)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
        let damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
        let seq_id = engine.launch_element(damage);
        let assets = assets_with_sword_profile(1, 50);

        engine.apply_hit_damage(
            &crate::sim_rng::test_context(),
            &assets,
            victim,
            Some(attacker),
            1,
            true,
            (seq_id, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_entity.npc_data().unwrap().eye_status,
            EyeStatus::DieOrGetUnconscious
        );
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .outbox
                .recovery
                .set_eye_status,
            None,
            "the synchronous EVENT_GOTHIT write must not wait for the next owner slot"
        );
    }

    #[test]
    fn conscious_lying_hit_applies_concussion_and_got_hit_before_terminating() {
        let mut engine = make_engine();
        let null_slot = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        engine
            .get_entity_mut(null_slot)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
        let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                y: 0.0,
                z: 0.0,
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.set_posture(Posture::Lying);
            victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        }
        let damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveHitDamage,
            Some(victim),
            Some(attacker),
            0,
            3,
        );
        let seq_id = engine.launch_element(damage);
        let assets = assets_with_sword_profile(1, 50);

        engine.dispatch_receive_damage(&crate::sim_rng::test_context(), &assets, victim, seq_id, 0);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_entity.human_data().unwrap().concussion_of_the_brain,
            6,
            "AddConcussionOfTheBrain scales the incoming 3 by 100 / 50 life before the lying early exit"
        );
        assert_eq!(
            victim_entity.npc_data().unwrap().eye_status,
            EyeStatus::DieOrGetUnconscious,
            "EVENT_GOTHIT runs before the lying early exit"
        );
        let damage = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
        assert!(
            damage.orders.is_empty(),
            "an already-lying victim must not receive another fall order"
        );
    }

    fn assets_with_sword_profile(energy: u16, max_distance: u16) -> LevelAssets {
        assets_with_sword_profile_effects(energy, max_distance, 4, 0)
    }

    fn assets_with_sword_profile_effects(
        energy: u16,
        max_distance: u16,
        cutting: u16,
        stunning: u16,
    ) -> LevelAssets {
        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = max_distance;
        weapon.thrusts[SwordStrike::A as usize].energy = energy;
        weapon.thrusts[SwordStrike::A as usize].minimal_distance = 0;
        weapon.thrusts[SwordStrike::A as usize].maximal_distance = max_distance;
        weapon.thrusts[SwordStrike::A as usize].cutting = cutting;
        weapon.thrusts[SwordStrike::A as usize].stunning = stunning;
        profile_manager.hth_weapons.push(weapon);
        profile_manager
            .characters
            .push(crate::profiles::CharacterProfile {
                hth_weapon_id: 1,
                ..crate::profiles::CharacterProfile::default()
            });
        profile_manager
            .soldiers
            .push(crate::profiles::SoldierProfile {
                hth_weapon_id: 1,
                fighting: 20,
                ..crate::profiles::SoldierProfile::default()
            });

        LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        }
    }

    #[test]
    fn postponed_non_entry_strike_translates_after_antagonist_dies() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        let target = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 20.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        match engine.get_entity_mut(target).unwrap() {
            Entity::Pc(pc) => pc.pc.life_points = 0,
            _ => unreachable!("test target must remain a PC"),
        }

        for (command, strike, expected_order) in [
            (
                Command::SwordstrikeThrustB,
                SwordStrike::B,
                OrderType::StrikingStraightStrongSword,
            ),
            (
                Command::SwordstrikeThrustC,
                SwordStrike::C,
                OrderType::ExecutingSword,
            ),
        ] {
            let element = crate::sequence::SequenceElement::new_interaction(
                1,
                command,
                Some(attacker),
                Some(target),
            );
            let sequence = engine.launch_element(element);
            engine.dispatch_sword_strike(
                &crate::sim_rng::test_context(),
                &LevelAssets::default(),
                attacker,
                target,
                strike,
                sequence,
                0,
            );

            let element = engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap();
            assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
            let order = element.current_order().unwrap();
            assert_eq!(order.order_type, expected_order);
            assert_eq!(order.antagonist, Some(target));
        }

        let thrust_a = crate::sequence::SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustA,
            Some(attacker),
            Some(target),
        );
        let sequence = engine.launch_element(thrust_a);
        engine.dispatch_sword_strike(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            attacker,
            target,
            SwordStrike::A,
            sequence,
            0,
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Impossible,
            "Thrust A must retain CanEnterSwordfightWith's dead-target admission check"
        );
    }

    #[test]
    fn thrust_a_accepts_an_existing_opponent_during_ordinary_door_transit() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D::default(),
            crate::position_interface::SectorHandle::new(42),
        ));
        let target = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                ..WorldPoint3D::default()
            },
            crate::position_interface::SectorHandle::new(43),
        ));
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(target);
        {
            let target_entity = engine.get_entity_mut(target).unwrap();
            target_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
            let target_actor = target_entity.actor_data_mut().unwrap();
            target_actor.active_door_pass = Some(crate::element::ActiveDoorPass {
                door_index: crate::gate::DoorIndex(7),
                direct: true,
                position_direct: true,
                steps: std::collections::VecDeque::new(),
                triggers_fired: 0,
                current_action: OrderType::WalkingWithSword,
                current_reverse: false,
                saved_action_state: None,
            });
            target_entity
                .position_iface_mut()
                .set_door_for_test(crate::position_interface::DoorHandle(7));
        }
        assert!(engine.get_entity(target).unwrap().is_in_door_transit());

        let assets = assets_with_sword_profile(1, 50);
        assert!(can_enter_swordfight_with(
            &engine.world.entities,
            attacker,
            target,
            &assets.profile_manager,
            &engine.world.fast_grid,
        ));

        let strike = crate::sequence::SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustA,
            Some(attacker),
            Some(target),
        );
        let sequence = engine.launch_element(strike);
        engine.dispatch_sword_strike(
            &crate::sim_rng::test_context(),
            &assets,
            attacker,
            target,
            SwordStrike::A,
            sequence,
            0,
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
        let order = element.current_order().unwrap();
        assert_eq!(order.order_type, OrderType::StrikingStraightSword);
        assert_eq!(order.antagonist, Some(target));
    }

    fn make_enemy_strike_pair(
        engine: &mut EngineInner,
        pending_consideration: bool,
    ) -> (EntityId, EntityId) {
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let target = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            soldier.actor.action_state = ActionState::WaitingSword;
            soldier.human.opponents.push(target);
            let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
                unreachable!()
            };
            ai.base.current_state = crate::ai::AiState::Attacking;
            ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
            ai.base.primary_target = target.index();
            ai.hth_weapon_id = 1;
            ai.pending_sword_strike_consideration = pending_consideration;
        }
        {
            let target_entity = engine.get_entity_mut(target).unwrap();
            target_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            target_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
        }
        (attacker, target)
    }

    #[test]
    fn entering_attacking_swordfight_without_reconsideration_does_not_propose() {
        let mut engine = make_engine();
        let (attacker, _) = make_enemy_strike_pair(&mut engine, false);
        let assets = assets_with_sword_profile(7, 30);
        engine.control.rng = SimulationRng::with_original_replay(Vec::new());

        engine.with_simulation_context(|engine, sim| {
            engine.tick_enemy_sword_attacks(sim, &assets);
        });

        assert_eq!(engine.control.rng.original_replay_cursor(), Some(0));
        assert!(
            !engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
            "entering AttackingSwordfight alone must not propose a strike"
        );
    }

    #[test]
    fn special_strike_cancellation_closes_its_set_state_callback_boundary() {
        let mut engine = make_engine();
        let (attacker, _) = make_enemy_strike_pair(&mut engine, false);
        let assets = assets_with_sword_profile(7, 30);
        {
            let ai = engine
                .get_entity_mut(attacker)
                .and_then(Entity::enemy_ai_mut)
                .unwrap();
            ai.begin_special_strike();
            ai.base.outbox.reentrant.owner_work.clear();
        }

        engine.with_simulation_context(|engine, sim| {
            engine.tick_enemy_sword_attacks(sim, &assets);
        });

        let ai = engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .unwrap();
        assert!(!ai.pending_special_strike);
        assert_eq!(
            ai.base.current_substate,
            crate::ai::Substate::AttackingSwordfight
        );
        assert!(
            ai.base.outbox.reentrant.owner_work.is_empty(),
            "the cancellation SetState callback must run synchronously"
        );
    }

    #[test]
    fn sword_strike_consideration_latch_is_one_shot_when_honour_rejects() {
        let mut engine = make_engine();
        let (attacker, target) = make_enemy_strike_pair(&mut engine, true);
        let assets = assets_with_sword_profile(7, 30);
        engine.control.rng = SimulationRng::with_original_replay(Vec::new());
        engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::Waiting;

        engine.with_simulation_context(|engine, sim| {
            engine.tick_enemy_sword_attacks(sim, &assets);
        });
        let cursor_after_first = engine.control.rng.original_replay_cursor().unwrap();
        assert_eq!(cursor_after_first, 0, "honour rejection precedes proposal");
        let pending_after_first = engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .pending_sword_strike_consideration;
        assert!(!pending_after_first, "the authorization must be one-shot");

        engine.with_simulation_context(|engine, sim| {
            engine.tick_enemy_sword_attacks(sim, &assets);
        });
        assert_eq!(
            engine.control.rng.original_replay_cursor(),
            Some(cursor_after_first),
            "the rejected, consumed latch must not retry next frame"
        );
    }

    #[test]
    fn sword_strike_honour_reads_live_animation_not_action_change_history() {
        let mut engine = make_engine();
        let (attacker, target) = make_enemy_strike_pair(&mut engine, true);
        let assets = assets_with_sword_profile(7, 30);
        engine.control.rng = SimulationRng::with_original_replay(Vec::new());
        {
            let target = engine.get_entity_mut(target).unwrap();
            let actor = target.actor_data_mut().unwrap();
            actor.old_action = OrderType::Invalid;
            // The live animation is the installed order (the Original's
            // mpOrder), not the action-change history in `old_action`.
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id: std::num::NonZeroU32::new(1).unwrap(),
                order_type: OrderType::BeingHitSword,
            });
            target.element_data_mut().sprite.last_action = OrderType::BeingHitSword;
        }

        engine.with_simulation_context(|engine, sim| {
            engine.tick_enemy_sword_attacks(sim, &assets);
        });

        assert_eq!(
            engine.control.rng.original_replay_cursor(),
            Some(0),
            "GetAnimation recovery rejection must precede strike selection"
        );
        assert!(
            !engine
                .get_entity(attacker)
                .and_then(Entity::enemy_ai)
                .unwrap()
                .pending_sword_strike_consideration,
            "the rejected reconsideration remains a one-shot event"
        );
    }

    #[test]
    fn owner_scoped_sword_consideration_precedes_later_owner_rng() {
        let mut engine = make_engine();
        let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
        let assets = assets_with_sword_profile(7, 30);
        {
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }
        engine.control.rng = SimulationRng::with_original_replay(vec![85, 36]);

        let later_roll = engine.with_simulation_context(|engine, sim| {
            engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
            crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ScriptRand, 0..100)
        });

        assert_eq!(
            later_roll, 36,
            "the reconsidering owner must consume its strike roll before a later owner's script"
        );
        assert_eq!(engine.control.rng.original_replay_cursor(), Some(2));
        assert!(
            !engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
            "the first roll rejects the strike and must not borrow the later owner's lower roll"
        );
    }

    #[test]
    fn event_authorized_parade_reconsideration_reaches_strike_proposal() {
        let mut engine = make_engine();
        let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
        let assets = assets_with_sword_profile(7, 30);
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            soldier.npc.ai_brain.base_mut().unwrap().current_substate =
                crate::ai::Substate::AttackingSwordfightParade;
            soldier.human.tiredness = TIREDNESS_WEAK_THRESHOLD;
            let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
                unreachable!()
            };
            ai.next_sword_strike_frame = u32::MAX;
            soldier.element.sprite.scripts =
                std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                    action_done: 0,
                    frame_ids: vec![0],
                    delays: vec![1],
                    distances: vec![0],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                    sound_ids: vec![0],
                    ..Default::default()
                }]);
            soldier.element.sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }
        engine.control.rng = SimulationRng::with_original_replay(vec![85]);

        engine.with_simulation_context(|engine, sim| {
            engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
        });

        assert_eq!(
            engine.control.rng.original_replay_cursor(),
            Some(1),
            "ReconsiderSwordfight already passed Original's state, cooldown, and tiredness gates"
        );
        assert!(
            !engine
                .get_entity(attacker)
                .and_then(Entity::enemy_ai)
                .unwrap()
                .pending_sword_strike_consideration
        );
    }

    #[test]
    fn deferred_combat_insult_depends_on_inline_strike_result() {
        fn install_minimal_sprite(engine: &mut EngineInner, attacker: EntityId) {
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }

        let assets = assets_with_sword_profile(7, 30);

        // A failed proposal leaves Original in ordinary Swordfight and the
        // caller's following statement says CombatInsult.
        let mut rejected = make_engine();
        let (rejected_attacker, _) = make_enemy_strike_pair(&mut rejected, true);
        install_minimal_sprite(&mut rejected, rejected_attacker);
        rejected
            .get_entity_mut(rejected_attacker)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .pending_combat_insult_after_strike_consideration = true;
        rejected.control.rng = SimulationRng::with_original_replay(vec![85]);
        rejected.with_simulation_context(|engine, sim| {
            engine.consume_pending_enemy_sword_attack_for(sim, &assets, rejected_attacker);
        });
        let rejected_ai = rejected
            .get_entity(rejected_attacker)
            .and_then(Entity::enemy_ai)
            .unwrap();
        assert!(
            rejected_ai
                .base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(
                    work,
                    crate::ai::AiOwnerWork::Speech(attempt)
                        if attempt.remark == crate::ai::Remark::CombatInsult
                ))
        );

        // A successful proposal changes Original to SpecialStrike before the
        // same following statement tests the substate, suppressing the bark.
        let mut accepted = make_engine();
        let (accepted_attacker, _) = make_enemy_strike_pair(&mut accepted, true);
        install_minimal_sprite(&mut accepted, accepted_attacker);
        accepted
            .get_entity_mut(accepted_attacker)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .pending_combat_insult_after_strike_consideration = true;
        accepted.control.rng = SimulationRng::with_original_replay(vec![0]);
        accepted.with_simulation_context(|engine, sim| {
            engine.consume_pending_enemy_sword_attack_for(sim, &assets, accepted_attacker);
        });
        let accepted_ai = accepted
            .get_entity(accepted_attacker)
            .and_then(Entity::enemy_ai)
            .unwrap();
        assert!(accepted_ai.pending_special_strike);
        assert!(
            !accepted_ai
                .base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(
                    work,
                    crate::ai::AiOwnerWork::Speech(attempt)
                        if attempt.remark == crate::ai::Remark::CombatInsult
                ))
        );
    }

    #[test]
    fn reactive_counterstrike_uses_difficulty_modified_soldier_fighting_ability() {
        let mut engine = make_engine();
        engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Hard;
        let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
        for actor in [victim, attacker] {
            let sprite = &mut engine
                .get_entity_mut(actor)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }
        let mut assets = assets_with_sword_profile(7, 30);
        std::sync::Arc::get_mut(&mut assets.profile_manager)
            .unwrap()
            .soldiers[0]
            .fighting = 40;
        {
            let ai = engine
                .get_entity_mut(victim)
                .and_then(Entity::enemy_ai_mut)
                .unwrap();
            ai.known_enemy_strike_1 = Some(SwordStrike::A);
        }

        // The replay victim is still performing a selected smalltalk parry
        // when the reactive counterstrike replaces it. StopAll must mark the
        // interrupted element's condolence as coming from Halt; otherwise its
        // later EventDone immediately leaves the new SpecialStrike substate.
        let old_parry =
            engine
                .orders
                .sequence_manager
                .launch_element(crate::sequence::SequenceElement::new(
                    1,
                    Command::ParrySmalltalkLeft,
                    Some(victim),
                ));
        engine
            .orders
            .sequence_manager
            .element_in_progress(old_parry, 0);

        // 65 rejects raw fighting 40 and produces a parade, but Hard's
        // Lacklandist modifier raises it to 80, allowing the counterstrike.
        engine.control.rng = SimulationRng::with_original_replay(vec![65]);
        engine.with_simulation_context(|engine, sim| {
            engine.consider_to_begin_parade(
                sim,
                &assets,
                victim,
                attacker,
                Some(SwordStrike::A),
                SwordStrike::A,
            );
            engine.dispatch_condolations(sim, &assets);
        });

        let ai = engine
            .get_entity(victim)
            .and_then(Entity::enemy_ai)
            .unwrap();
        assert!(ai.pending_special_strike);
        assert_eq!(
            ai.base.current_substate,
            crate::ai::Substate::AttackingSwordfightSpecialStrike
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(victim, Command::is_swordstrike)
        );
    }

    #[test]
    fn reactive_strike_recognition_uses_command_not_replacement_animation() {
        fn run(remembered: SwordStrike, busy: bool) -> (usize, crate::ai::Substate, usize) {
            let mut engine = make_engine();
            let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
            engine.world.fast_grid_mut().size_map(4, 4);
            engine.world.fast_grid_mut().allocate_layers(1);
            engine.world.fast_grid_mut().add_sector(
                crate::fast_find_grid::GridSector {
                    points: vec![
                        crate::coordinates::MapPoint::new(0.0, 0.0),
                        crate::coordinates::MapPoint::new(256.0, 0.0),
                        crate::coordinates::MapPoint::new(256.0, 256.0),
                        crate::coordinates::MapPoint::new(0.0, 256.0),
                    ],
                    bounding_box: crate::coordinates::MapBBox::from_coords(0.0, 0.0, 256.0, 256.0),
                    sector_type: crate::sector::SectorType::MOTION
                        | crate::sector::SectorType::AREA,
                    layer: 0,
                    sector_number: crate::sector::SectorNumber::new(0),
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
                },
                0,
            );
            {
                let victim_element = engine.get_entity_mut(victim).unwrap().element_data_mut();
                victim_element.set_position(WorldPoint3D::new(100.0, 100.0, 0.0));
                victim_element.set_sector(crate::position_interface::SectorHandle::new(0));
                victim_element.sprite.position_iface.set_move_box(
                    crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
                );
            }
            engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .set_position(WorldPoint3D::new(180.0, 100.0, 0.0));
            for actor in [victim, attacker] {
                let sprite = &mut engine
                    .get_entity_mut(actor)
                    .unwrap()
                    .element_data_mut()
                    .sprite;
                let mut scripts = vec![
                    crate::sprite_script::SpriteScript {
                        action_done: 10,
                        frame_ids: (0..16).collect(),
                        delays: vec![1; 16],
                        distances: vec![0; 16],
                        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 16],
                        sound_ids: vec![0; 16],
                        ..Default::default()
                    };
                    16
                ];
                // The incoming H animation has ten frames left, while the
                // victim's authored parry transition starts in three. This
                // satisfies Original's strict startup deadline and lets the
                // test observe the later PushAside geometry branch.
                scripts[1].action_done = 3;
                sprite.scripts = std::sync::Arc::new(scripts);
                let mut conversion = vec![0; crate::sprite_script::NONANIMATION_END];
                conversion[OrderType::TransitionWaitingSwordParryingSword as usize] = 1;
                sprite.conversion = std::sync::Arc::new(conversion);
            }

            let old_movement = crate::sequence::SequenceElement::new_movement(
                1,
                Command::MoveOk,
                Some(victim),
                OrderType::WalkingWithSword,
            );
            let old_sequence = engine.launch_element(old_movement);
            let old_order =
                engine.push_new_order(old_sequence, 0, OrderType::WalkingWithSword, 90.0, 100.0);
            engine
                .orders
                .sequence_manager
                .element_in_progress(old_sequence, 0);
            {
                let actor = engine
                    .get_entity_mut(victim)
                    .unwrap()
                    .actor_data_mut()
                    .unwrap();
                actor.action_state = ActionState::MovingSword;
                actor.installed_order = Some(crate::element::InstalledActorOrder {
                    order_id: old_order,
                    order_type: OrderType::WalkingWithSword,
                });
                actor.active_movement = crate::movement::ActiveMovement::new(old_sequence, 0);
            }

            // The selected request remains F while its installed replacement
            // row is H, exactly separating Original GetCommand from
            // GetAnimation at WarnForStrike.
            let strike_element = crate::sequence::SequenceElement::new_interaction(
                1,
                Command::SwordstrikeThrustF,
                Some(attacker),
                Some(victim),
            );
            let strike_sequence = engine.launch_element(strike_element);
            let strike_order = engine.push_new_order(
                strike_sequence,
                0,
                OrderType::StrikingRoundLeftSword,
                100.0,
                100.0,
            );
            engine
                .orders
                .sequence_manager
                .element_in_progress(strike_sequence, 0);
            {
                let attacker_entity = engine.get_entity_mut(attacker).unwrap();
                let actor = attacker_entity.actor_data_mut().unwrap();
                actor.action_state = ActionState::WaitingSword;
                actor.installed_order = Some(crate::element::InstalledActorOrder {
                    order_id: strike_order,
                    order_type: OrderType::StrikingRoundLeftSword,
                });
                let sprite = &mut attacker_entity.element_data_mut().sprite;
                sprite.current_row = 0;
                sprite.current_frame = 0;
                sprite.frame_count = 0;
                sprite.action_done_frame = 10;
                sprite.action_done_counter = 1;
                sprite.last_action = OrderType::StrikingRoundLeftSword;
            }

            let mut assets = assets_with_sword_profile(7, 30);
            let profiles = std::sync::Arc::get_mut(&mut assets.profile_manager).unwrap();
            profiles.soldiers[0].fighting = 50;
            profiles.hth_weapons[0].thrusts[SwordStrike::H as usize].kind =
                crate::profiles::WeaponThrustKind::PushAside;
            profiles.hth_weapons[0].thrusts[SwordStrike::H as usize].maximal_distance = 30;
            let ai = engine
                .get_entity_mut(victim)
                .and_then(Entity::enemy_ai_mut)
                .unwrap();
            ai.known_enemy_strike_1 = Some(remembered);
            if busy {
                ai.base.locks_flag_field = crate::ai::AiLockFlags::BUSY;
            }

            // 65 selects parade at ability 50. Only the H animation's
            // PushAside geometry can turn that parade into a step-back.
            engine.control.rng = SimulationRng::with_original_replay(vec![85]);
            engine.with_simulation_context(|engine, sim| {
                engine.warn_for_strike(sim, &assets, attacker, &[victim], SwordStrike::H);
            });
            let ai = engine
                .get_entity(victim)
                .and_then(Entity::enemy_ai)
                .unwrap();
            if busy {
                assert_eq!(
                    ai.base.stimulus_queue[0].stimulus_type,
                    crate::ai::StimulusType::EventSwordStrike,
                );
                assert_eq!(
                    ai.base.stimulus_queue[0].info,
                    crate::ai::StimulusInfo::Human(attacker.index()),
                    "queued EVENT_SWORDSTRIKE must retain the attacking human"
                );
            }
            (
                engine.control.rng.original_replay_cursor().unwrap(),
                ai.base.current_substate,
                ai.base.stimulus_queue.len(),
            )
        }

        assert_eq!(
            run(SwordStrike::F, false),
            (1, crate::ai::Substate::AttackingSwordfightStepBack, 0,),
            "command F must admit the proposal while animation H supplies PushAside geometry"
        );
        assert_eq!(
            run(SwordStrike::H, false),
            (0, crate::ai::Substate::AttackingSwordfight, 0),
            "remembering only replacement H must not admit selected command F"
        );
        let locked = run(SwordStrike::F, true);
        assert_eq!(
            locked.0, 0,
            "BUSY StartThink must not reach the proposal RNG"
        );
        assert_eq!(
            locked.1,
            crate::ai::Substate::AttackingSwordfight,
            "BUSY warning must not launch a parade or counter-strike"
        );
        assert_eq!(locked.2, 1);
    }

    #[test]
    fn reactive_step_back_launches_replacement_move_before_returning() {
        for rider in [false, true] {
            let mut engine = make_engine();
            let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
            let Entity::Soldier(victim_soldier) = engine.get_entity_mut(victim).unwrap() else {
                unreachable!()
            };
            victim_soldier.soldier.rider = rider;
            engine.world.fast_grid_mut().size_map(4, 4);
            engine.world.fast_grid_mut().allocate_layers(1);
            let sector_points = vec![
                crate::coordinates::MapPoint::new(0.0, 0.0),
                crate::coordinates::MapPoint::new(256.0, 0.0),
                crate::coordinates::MapPoint::new(256.0, 256.0),
                crate::coordinates::MapPoint::new(0.0, 256.0),
            ];
            engine.world.fast_grid_mut().add_sector(
                crate::fast_find_grid::GridSector {
                    points: sector_points,
                    bounding_box: crate::coordinates::MapBBox::from_coords(0.0, 0.0, 256.0, 256.0),
                    sector_type: crate::sector::SectorType::MOTION
                        | crate::sector::SectorType::AREA,
                    layer: 0,
                    sector_number: crate::sector::SectorNumber::new(0),
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
                },
                0,
            );
            {
                let victim_element = engine.get_entity_mut(victim).unwrap().element_data_mut();
                victim_element.set_position(WorldPoint3D::new(100.0, 100.0, 0.0));
                victim_element.set_sector(crate::position_interface::SectorHandle::new(0));
                victim_element.sprite.position_iface.set_move_box(
                    crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
                );
            }
            engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .set_position(WorldPoint3D::new(180.0, 100.0, 0.0));
            for actor in [victim, attacker] {
                let sprite = &mut engine
                    .get_entity_mut(actor)
                    .unwrap()
                    .element_data_mut()
                    .sprite;
                sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                    action_done: 0,
                    frame_ids: vec![0],
                    delays: vec![1],
                    distances: vec![0],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                    sound_ids: vec![0],
                    ..Default::default()
                }]);
                sprite.conversion =
                    std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
            }

            // The replay victim is already moving with a sword. StopAll
            // interrupts that selected movement, but Original's immediately
            // following GoTo still reads its installed sword animation. The
            // attacker is already beyond the desired separation, making the
            // proposed step-back goal the victim's current point and exposing
            // the animation-gated zero-distance path.
            let old_movement = crate::sequence::SequenceElement::new_movement(
                1,
                Command::MoveOk,
                Some(victim),
                OrderType::WalkingWithSword,
            );
            let old_sequence = engine.launch_element(old_movement);
            let old_order =
                engine.push_new_order(old_sequence, 0, OrderType::WalkingWithSword, 90.0, 100.0);
            engine
                .orders
                .sequence_manager
                .element_in_progress(old_sequence, 0);
            {
                let actor = engine
                    .get_entity_mut(victim)
                    .unwrap()
                    .actor_data_mut()
                    .unwrap();
                actor.action_state = ActionState::MovingSword;
                actor.installed_order = Some(crate::element::InstalledActorOrder {
                    order_id: old_order,
                    order_type: OrderType::WalkingWithSword,
                });
                actor.active_movement = crate::movement::ActiveMovement::new(old_sequence, 0);
            }
            let mut assets = assets_with_sword_profile(7, 30);
            let profiles = std::sync::Arc::get_mut(&mut assets.profile_manager).unwrap();
            profiles.soldiers[0].fighting = 50;
            let incoming_thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
            incoming_thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
            incoming_thrust.maximal_distance = 30;
            {
                let ai = engine
                    .get_entity_mut(victim)
                    .and_then(Entity::enemy_ai_mut)
                    .unwrap();
                ai.known_enemy_strike_1 = Some(SwordStrike::A);
            }

            // 65 rejects an offensive response at fighting ability 50, selecting
            // the parade path. A push-aside strike turns that parade into a
            // step-back GoTo, which Original launches synchronously before this
            // callback returns.
            engine.control.rng = SimulationRng::with_original_replay(vec![65]);
            engine.with_simulation_context(|engine, sim| {
                engine.consider_to_begin_parade(
                    sim,
                    &assets,
                    victim,
                    attacker,
                    Some(SwordStrike::A),
                    SwordStrike::A,
                );
            });

            let ai = engine
                .get_entity(victim)
                .and_then(Entity::enemy_ai)
                .unwrap();
            assert_eq!(
                ai.base.current_substate,
                crate::ai::Substate::AttackingSwordfightStepBack
            );
            assert!(
                ai.base
                    .last_goto_flags
                    .contains(crate::ai::GotoFlags::SWORD)
            );
            assert_eq!(
                ai.base.last_goto_flags.contains(crate::ai::GotoFlags::RUN),
                !rider,
                "Original's rider step-back omits GOTO_RUN"
            );
            let owned_elements: Vec<_> = engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .filter(|element| element.owner == Some(victim))
                .map(|element| {
                    (
                        element.command,
                        element.state,
                        element.current_order().map(|order| order.order_type),
                    )
                })
                .collect();
            assert!(
                !owned_elements
                    .iter()
                    .any(|(command, _, _)| *command == Command::EnterSwordfight),
                "a soldier already in WaitingSword must not receive a spurious raise-sword prefix: {owned_elements:?}"
            );
            let (move_sequence, move_element) = engine
            .orders
            .sequence_manager
            .live_element_for_actor_matching(victim, |element| {
                matches!(element.command, Command::Move | Command::MoveOk)
            })
            .unwrap_or_else(|| {
                panic!(
                    "step-back GoTo must not remain queued until the next owner slot; owned elements: {owned_elements:?}; pending moves: {:?}",
                    engine.orders.pending_move_requests
                )
            });
            let movement = engine
                .orders
                .sequence_manager
                .get_element(move_sequence, move_element)
                .expect("live step-back movement remains inspectable");
            assert_eq!(
                movement.movement_action_for_test(),
                Some(if rider {
                    OrderType::WalkingUpright
                } else {
                    OrderType::RunningUpright
                }),
                "GOTO_RUN authors only the non-rider replacement's pre-instruction movement action"
            );
            assert!(
                movement.movement_flags_for_test().is_some_and(
                    |flags| flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT)
                ),
                "the live soldier context must preserve GOTO_SWORD on the replacement movement"
            );
        }
    }

    fn assets_with_nonstraight_profile(
        strike: SwordStrike,
        kind: crate::profiles::WeaponThrustKind,
    ) -> LevelAssets {
        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        let thrust = &mut weapon.thrusts[strike as usize];
        thrust.kind = kind;
        thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
        thrust.minimal_distance = 0;
        thrust.maximal_distance = 100;
        thrust.initial_angle = 0;
        thrust.final_angle = 180;
        thrust.rotation_angle = 90;
        thrust.repulsion = 100;
        thrust.cutting = 100;
        profile_manager.hth_weapons.push(weapon);
        profile_manager
            .characters
            .push(crate::profiles::CharacterProfile {
                hth_weapon_id: 1,
                ..crate::profiles::CharacterProfile::default()
            });
        profile_manager
            .soldiers
            .push(crate::profiles::SoldierProfile {
                hth_weapon_id: 1,
                ..crate::profiles::SoldierProfile::default()
            });

        LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        }
    }

    #[test]
    fn civilian_health_counts_toward_round_strike_and_warcry() {
        let mut engine = make_engine();
        let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
        {
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }
        engine.add_entity(make_civilian(WorldPoint3D {
            x: 15.0,
            y: 100.0,
            z: 0.0,
        }));

        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::H,
            crate::profiles::WeaponThrustKind::TrueCircle,
        );
        std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].fighting = 100;
        engine.control.rng = SimulationRng::with_original_replay(vec![0]);

        engine.with_simulation_context(|engine, sim| {
            engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
        });

        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(attacker, |command| {
                    command == Command::SwordstrikeThrustH
                }),
            "the PC and civilian are two live round-strike victims"
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .and_then(Entity::enemy_ai)
                .expect("fixture attacker keeps Enemy AI")
                .base
                .current_remark,
            crate::ai::Remark::Warcry,
            "Original says REMARK_WARCRY when selecting thrust H"
        );
    }

    fn soldier_life(engine: &EngineInner, soldier_id: EntityId) -> i16 {
        match engine
            .get_entity(soldier_id)
            .expect("test soldier must remain present")
        {
            Entity::Soldier(soldier) => soldier.npc.life_points,
            _ => panic!("test victim must be a soldier"),
        }
    }

    fn install_test_melee_order(
        engine: &mut EngineInner,
        attacker: EntityId,
        target: EntityId,
        strike: SwordStrike,
        past_action_done: bool,
    ) -> crate::engine::tick::MeleeOwnerSelection {
        let order_type = strike_to_animation(strike);
        let sequence = engine.orders.sequence_manager.launch_element(
            crate::sequence::SequenceElement::new_interaction(
                1,
                strike.to_command(),
                Some(attacker),
                Some(target),
            ),
        );
        let order_id = engine.orders.allocate_order_id();
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, order_id);
        order.antagonist = Some(target);
        engine
            .orders
            .sequence_manager
            .push_order_on(sequence, 0, order);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        let script = crate::sprite_script::SpriteScript {
            action_id: order_type as u16,
            action_done: 1,
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
            ..Default::default()
        };
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[order_type as usize] = 0;
        let entity = engine.get_entity_mut(attacker).unwrap();
        let position_iface = entity.element_data().sprite.position_iface.clone();
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );
        sprite.position_iface = position_iface;
        entity.element_data_mut().sprite = sprite;
        let direction = entity.element_data().direction() as u16;
        let sim = crate::sim_rng::test_context();
        let sprite = &mut entity.element_data_mut().sprite;
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::Start
        );
        while sprite.frames_from_now_till_action_done() > 0 {
            assert_eq!(
                sprite.perform_action(
                    &sim,
                    Some(order_id),
                    order_type,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                ),
                crate::sprite::MotionState::InProgress
            );
        }
        if past_action_done {
            assert_eq!(
                sprite.perform_action(
                    &sim,
                    Some(order_id),
                    order_type,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                ),
                crate::sprite::MotionState::Done
            );
        }
        crate::engine::tick::MeleeOwnerSelection {
            seq_id: sequence,
            elem_idx: 0,
            order_id,
        }
    }

    #[test]
    fn completed_missed_sword_strike_adds_tiredness_once() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let target = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 500.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_sword_profile(7, 30);

        install_test_melee_order(&mut engine, attacker, target, SwordStrike::A, true);

        engine.tick_melee_strikes(sim, &assets);

        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .tiredness,
            7,
            "out-of-range strikes still cost tiredness when the active strike terminates"
        );
    }

    #[test]
    fn empty_true_circle_sweep_advances_until_rotation_complete() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        if let Some(actor) = engine.get_entity_mut(attacker).unwrap().actor_data_mut() {
            actor.sweep_state = Some(crate::movement::SweepState {
                pending_victims: Vec::new(),
                current_angle: 0.0,
                final_angle: std::f32::consts::PI * 2.0,
                rotation_per_frame: std::f32::consts::PI,
                direction: crate::profiles::WeaponThrustDirection::LeftToRight,
                strike: SwordStrike::H,
                strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
                ..Default::default()
            });
        }

        engine.tick_sweep_for(sim, &LevelAssets::default(), attacker, false);
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_some(),
            "true-circle sweep with no victims must still rotate instead of clearing immediately"
        );

        engine.tick_sweep_for(sim, &LevelAssets::default(), attacker, false);
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_some(),
            "the tick that reaches the final angle must retain it for the terminal Execute call"
        );

        engine.tick_sweep_for(sim, &LevelAssets::default(), attacker, false);
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_none(),
            "empty true-circle sweep should clear after presenting its terminal angle"
        );
    }

    #[test]
    fn circle_done_initialization_advances_without_rotating_or_hitting() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::F,
            crate::profiles::WeaponThrustKind::TrueHalfCircle,
        );

        engine.initialize_sweep(
            &assets,
            attacker,
            SwordStrike::F,
            Some(1),
            crate::profiles::WeaponThrustKind::TrueHalfCircle,
            vec![victim],
        );
        let initial_angle = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .unwrap()
            .current_angle;

        engine.tick_sweep_for(sim, &assets, attacker, true);

        let attacker_entity = engine.get_entity(attacker).unwrap();
        let sweep = attacker_entity
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("true half-circle must retain its initialized sweep");
        assert!(
            (sweep.current_angle - (initial_angle + std::f32::consts::FRAC_PI_2)).abs()
                < f32::EPSILON,
            "ExecuteCircleSwordStrike advances its internal angle at the DONE-call tail"
        );
        assert_eq!(
            attacker_entity.element_data().direction(),
            0,
            "the DONE call must not rotate the true-circle sprite"
        );
        assert_eq!(
            soldier_life(&engine, victim),
            50,
            "the DONE effect branch only initializes victims and cannot hit"
        );
    }

    #[test]
    fn lateral_done_initialization_does_not_advance_or_hit() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::D,
            crate::profiles::WeaponThrustKind::Lateral,
        );
        let selected =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);

        let phase = engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected);
        assert!(
            phase == strikes::SweepTickPhase::Initialized,
            "the lateral DONE branch must initialize a sweep"
        );
        let initial_current = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .unwrap()
            .current_angle;
        engine.tick_sweep_for(sim, &assets, attacker, true);

        let current = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("lateral victim must remain pending after DONE")
            .current_angle;
        assert_eq!(
            current, initial_current,
            "ExecuteLateralSwordStrike uses an else-if, so DONE cannot also run its IN_PROGRESS advance"
        );
        assert_eq!(
            soldier_life(&engine, victim),
            50,
            "lateral initialization cannot hit until a later Hourglass"
        );
    }

    #[test]
    fn lateral_done_keeps_actor_scan_order_and_does_not_recover_out_of_arc_antagonist() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        // Facing south (sector 8), thrust D covers sectors 4..=9. Keep the
        // valid victims on either side of the out-of-arc antagonist in actor
        // creation order so the assertion also guards the collector FIFO.
        let first_in_arc = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let antagonist = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: -20.0,
                y: 120.0,
                z: 0.0,
            },
            None,
        ));
        let second_in_arc = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 120.0,
                z: 0.0,
            },
            None,
        ));
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(8);

        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::D,
            crate::profiles::WeaponThrustKind::Lateral,
        );
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::D as usize];
        thrust.initial_angle = 90;
        thrust.final_angle = 22;
        thrust.rotation_angle = 45;

        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15(-20.0, 20.0),
            10,
            "the interaction antagonist must be outside thrust D's actor-scan arc"
        );
        let selected =
            install_test_melee_order(&mut engine, attacker, antagonist, SwordStrike::D, false);

        assert_eq!(
            engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
            strikes::SweepTickPhase::Initialized
        );
        let pending = &engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("the in-arc actors initialize the lateral sweep")
            .pending_victims;
        assert_eq!(pending, &[first_in_arc, second_in_arc]);
        assert!(!pending.contains(&antagonist));
    }

    #[test]
    fn lateral_seed_uses_ground_direction_instead_of_map_direction() {
        let mut engine = make_engine();
        // Both actors have the same ground Y, so the victim is due west
        // (sector 12).  Its lower elevation projects six units south in map
        // space, which moves the same vector into sector 11.
        let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 100.0,
            });
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D {
                x: -15.0,
                y: 100.0,
                z: 94.0,
            });
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(9);

        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::E,
            crate::profiles::WeaponThrustKind::Lateral,
        );
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::E as usize];
        thrust.direction = crate::profiles::WeaponThrustDirection::RightToLeft;
        thrust.initial_angle = 45;
        thrust.final_angle = 90;

        let attacker_map = engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .position_map();
        let victim_map = engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .position_map();
        let attacker_ground = engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .position();
        let victim_ground = engine.get_entity(victim).unwrap().element_data().position();
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15(
                victim_ground.x - attacker_ground.x,
                victim_ground.y - attacker_ground.y,
            ),
            12,
        );
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15(
                victim_map.x - attacker_map.x,
                victim_map.y - attacker_map.y,
            ),
            11,
            "the old map-space seed would admit this victim at the arc boundary"
        );

        let victims =
            engine.execute_multi_target_strike(&assets, attacker, SwordStrike::E, Some(1));
        assert!(
            victims.is_empty(),
            "Original seeds lateral victims from ground-space direction, where this actor is sector 12 and outside sectors 5..=11"
        );
    }

    #[test]
    fn interrupted_lateral_sweep_is_retained_and_rebound_by_next_strike() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let unreached_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: -10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        let retained = &mut weapon.thrusts[SwordStrike::D as usize];
        retained.kind = crate::profiles::WeaponThrustKind::Lateral;
        retained.direction = crate::profiles::WeaponThrustDirection::RightToLeft;
        retained.minimal_distance = 0;
        retained.maximal_distance = 100;
        retained.rotation_angle = 5;
        retained.cutting = 1;
        let replacement = &mut weapon.thrusts[SwordStrike::E as usize];
        replacement.kind = crate::profiles::WeaponThrustKind::Lateral;
        replacement.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
        replacement.minimal_distance = 0;
        replacement.maximal_distance = 100;
        replacement.rotation_angle = 90;
        replacement.cutting = 100;
        profile_manager.hth_weapons.push(weapon);
        profile_manager
            .characters
            .push(crate::profiles::CharacterProfile {
                hth_weapon_id: 1,
                ..crate::profiles::CharacterProfile::default()
            });
        profile_manager
            .soldiers
            .push(crate::profiles::SoldierProfile {
                hth_weapon_id: 1,
                ..crate::profiles::SoldierProfile::default()
            });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        };

        let retained_selection =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![victim, unreached_victim],
            initial_angle: 0.0,
            current_angle: 0.0,
            final_angle: -std::f32::consts::PI,
            rotation_per_frame: -5.0_f32.to_radians(),
            direction: crate::profiles::WeaponThrustDirection::RightToLeft,
            strike: SwordStrike::D,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::Lateral,
        });

        engine.stop_owner_active_mechanics(attacker);
        let retained_after_interrupt = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("interrupting the D sequence must retain its human-owned sweep");
        assert_eq!(retained_after_interrupt.strike, SwordStrike::D);
        assert_eq!(
            retained_after_interrupt.pending_victims,
            vec![victim, unreached_victim]
        );

        let replacement_order_id = engine.orders.allocate_order_id();
        let replacement_element = engine
            .orders
            .sequence_manager
            .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
            .expect("retained strike element exists");
        replacement_element.command = SwordStrike::E.to_command();
        let replacement_order = replacement_element
            .orders
            .front_mut()
            .expect("retained strike order exists");
        replacement_order.order_type = strike_to_animation(SwordStrike::E);
        replacement_order.antagonist = Some(victim);
        replacement_order.reseed_id(replacement_order_id);
        // A live replacement strike is published as the actor's installed
        // order at Instruct; Execute's Start arm resolves the strike from
        // that installed animation, not from the sequence element.
        engine.publish_selected_order_as_installed(attacker);
        {
            let entity = engine.get_entity_mut(attacker).unwrap();
            let sprite = &mut entity.element_data_mut().sprite;
            sprite.scripts = std::sync::Arc::new(vec![
                crate::sprite_script::SpriteScript {
                    action_done: 3,
                    frame_ids: vec![0, 1, 2, 3],
                    delays: vec![1, 1, 1, 1],
                    distances: vec![0, 0, 0, 0],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
                    sound_ids: vec![0; 4],
                    ..Default::default()
                };
                16
            ]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }

        engine.tick_melee_strikes(sim, &assets);
        let retained_on_start = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("the replacement strike START must not consume the retained sweep");
        let replacement_direction_angle = sector_to_angle(
            engine
                .get_entity(attacker)
                .unwrap()
                .element_data()
                .direction(),
        );
        assert_eq!(retained_on_start.strike, SwordStrike::E);
        assert_eq!(
            retained_on_start.pending_victims,
            vec![victim, unreached_victim],
            "the START warning forecast rebases geometry but keeps the interrupted victim FIFO"
        );
        assert_eq!(retained_on_start.initial_angle, replacement_direction_angle);
        assert_eq!(retained_on_start.current_angle, replacement_direction_angle);
        assert_eq!(retained_on_start.final_angle, replacement_direction_angle);
        assert_eq!(soldier_life(&engine, victim), 50);

        engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
        engine.tick_sweep_for(sim, &assets, attacker, false);

        let retained_after_hit = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("a lateral sweep remains allocated until its animation terminates");
        assert_eq!(retained_after_hit.strike, SwordStrike::E);
        assert_eq!(
            retained_after_hit.direction,
            crate::profiles::WeaponThrustDirection::LeftToRight
        );
        assert!(
            (retained_after_hit.rotation_per_frame - std::f32::consts::FRAC_PI_2).abs()
                < f32::EPSILON,
            "the retained geometry must advance using E's rotation, not D's"
        );
    }

    #[test]
    fn interrupted_circle_sweep_preserves_geometry_before_replacement_action_point() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        engine
            .get_entity_mut(victim)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .hth_weapon_id = 1;

        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        for strike in [SwordStrike::I, SwordStrike::F] {
            let thrust = &mut weapon.thrusts[strike as usize];
            thrust.kind = crate::profiles::WeaponThrustKind::TrueCircle;
            thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
            thrust.minimal_distance = 0;
            thrust.maximal_distance = 100;
            thrust.initial_angle = 0;
            thrust.final_angle = 360;
            thrust.rotation_angle = 45;
        }
        profile_manager.hth_weapons.push(weapon);
        profile_manager
            .characters
            .push(crate::profiles::CharacterProfile {
                hth_weapon_id: 1,
                ..crate::profiles::CharacterProfile::default()
            });
        profile_manager
            .soldiers
            .push(crate::profiles::SoldierProfile {
                hth_weapon_id: 1,
                ..crate::profiles::SoldierProfile::default()
            });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        };

        let retained_selection =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::I, true);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(7);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![victim],
            initial_angle: 0.0,
            current_angle: 0.0,
            final_angle: std::f32::consts::TAU,
            rotation_per_frame: std::f32::consts::FRAC_PI_4,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::I,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
        });

        let replacement_order_id = engine.orders.allocate_order_id();
        let replacement_element = engine
            .orders
            .sequence_manager
            .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
            .expect("retained strike element exists");
        replacement_element.command = SwordStrike::F.to_command();
        let replacement_order = replacement_element
            .orders
            .front_mut()
            .expect("retained strike order exists");
        replacement_order.order_type = strike_to_animation(SwordStrike::F);
        replacement_order.antagonist = Some(victim);
        replacement_order.reseed_id(replacement_order_id);
        engine.publish_selected_order_as_installed(attacker);
        {
            let entity = engine.get_entity_mut(attacker).unwrap();
            let sprite = &mut entity.element_data_mut().sprite;
            sprite.use_alternate_profile = false;
            sprite.scripts = std::sync::Arc::new(vec![
                crate::sprite_script::SpriteScript {
                    action_done: 5,
                    frame_ids: vec![0, 1, 2, 3, 4, 5, 6],
                    delays: vec![1; 7],
                    distances: vec![0; 7],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 7],
                    sound_ids: vec![0; 7],
                    ..Default::default()
                };
                16
            ]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }

        engine.tick_melee_strikes(sim, &assets);
        {
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            assert_eq!(sprite.action_done_frame, 5);
            assert_eq!(sprite.action_done_counter, 0);
            sprite.current_frame = 3;
            sprite.frame_count = 0;
        }
        engine.tick_melee_strikes(sim, &assets);

        let attacker_entity = engine.get_entity(attacker).unwrap();
        let retained_before_action = attacker_entity
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("replacement pre-action frames retain the interrupted circle sweep");
        assert_eq!(
            retained_before_action.strike,
            SwordStrike::F,
            "Original reads replacement F's effect parameters before its action point"
        );
        assert_eq!(retained_before_action.current_angle, 0.0);
        assert_eq!(
            attacker_entity.element_data().direction(),
            7,
            "the interrupted circle geometry must not rotate replacement strike F before its action point"
        );

        engine.tick_melee_strikes(sim, &assets);
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .as_ref()
                .expect("replacement circle initializes its own sweep at action done")
                .strike,
            SwordStrike::F,
        );
    }

    #[test]
    fn interrupted_h_circle_runs_replacement_i_effect_without_advancing_geometry() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: -10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let unreached_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 90.0,
                z: 0.0,
            },
            None,
        ));
        for target in [victim, unreached_victim] {
            engine
                .get_entity_mut(target)
                .and_then(Entity::enemy_ai_mut)
                .unwrap()
                .hth_weapon_id = 1;
        }

        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        for strike in [SwordStrike::H, SwordStrike::I] {
            let thrust = &mut weapon.thrusts[strike as usize];
            thrust.kind = crate::profiles::WeaponThrustKind::TrueCircle;
            thrust.minimal_distance = 0;
            thrust.maximal_distance = 100;
            thrust.initial_angle = 0;
            thrust.final_angle = 360;
            thrust.rotation_angle = 22;
        }
        weapon.thrusts[SwordStrike::H as usize].direction =
            crate::profiles::WeaponThrustDirection::LeftToRight;
        weapon.thrusts[SwordStrike::I as usize].direction =
            crate::profiles::WeaponThrustDirection::RightToLeft;
        profile_manager.hth_weapons.push(weapon);
        profile_manager
            .characters
            .push(crate::profiles::CharacterProfile {
                hth_weapon_id: 1,
                ..crate::profiles::CharacterProfile::default()
            });
        profile_manager
            .soldiers
            .push(crate::profiles::SoldierProfile {
                hth_weapon_id: 1,
                ..crate::profiles::SoldierProfile::default()
            });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        };

        let retained_selection =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::H, true);
        let retained_initial_angle = 0.1;
        let retained_current_angle = 1.251_917_2;
        let retained_final_angle = std::f32::consts::TAU + retained_initial_angle;
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(7);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![victim, unreached_victim],
            initial_angle: retained_initial_angle,
            current_angle: retained_current_angle,
            final_angle: retained_final_angle,
            rotation_per_frame: 0.383_972_44,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::H,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
        });

        let replacement_order_id = engine.orders.allocate_order_id();
        let replacement_element = engine
            .orders
            .sequence_manager
            .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
            .expect("retained strike element exists");
        replacement_element.command = SwordStrike::I.to_command();
        let replacement_order = replacement_element
            .orders
            .front_mut()
            .expect("retained strike order exists");
        replacement_order.order_type = strike_to_animation(SwordStrike::I);
        replacement_order.antagonist = Some(victim);
        replacement_order.reseed_id(replacement_order_id);
        engine.publish_selected_order_as_installed(attacker);
        {
            let entity = engine.get_entity_mut(attacker).unwrap();
            let sprite = &mut entity.element_data_mut().sprite;
            sprite.use_alternate_profile = false;
            sprite.scripts = std::sync::Arc::new(vec![
                crate::sprite_script::SpriteScript {
                    action_done: 5,
                    frame_ids: vec![0, 1, 2, 3, 4, 5, 6],
                    delays: vec![1; 7],
                    distances: vec![0; 7],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 7],
                    sound_ids: vec![0; 7],
                    ..Default::default()
                };
                16
            ]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }

        engine.tick_melee_strikes(sim, &assets);
        {
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            assert_eq!(sprite.action_done_frame, 5);
            sprite.current_frame = 2;
            sprite.frame_count = 0;
        }
        engine.tick_melee_strikes(sim, &assets);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .filter(|element| {
                    element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
                })
                .count(),
            1,
            "the first replacement pre-action effect consumes the reached retained victim"
        );
        engine.tick_melee_strikes(sim, &assets);
        {
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.current_frame = 4;
            sprite.frame_count = 1;
            assert_eq!(
                sprite.frames_from_now_till_action_done(),
                0,
                "the forecast can reach zero one tick before the exact action point"
            );
            assert_ne!(sprite.current_frame, sprite.action_done_frame);
        }
        engine.tick_selected_sweep_phase(
            sim,
            &assets,
            attacker,
            strikes::SweepTickPhase::InProgress,
        );

        let queued_damage: Vec<&crate::sequence::SequenceElement> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            })
            .collect();
        assert_eq!(
            queued_damage.len(),
            1,
            "replacement I's right-to-left effect must reach the retained H victim before I's action point"
        );
        assert!(matches!(
            &queued_damage[0].data,
            crate::sequence::SequenceElementData::Damage {
                sword_strike: Some(SwordStrike::I),
                ..
            }
        ));

        let attacker_entity = engine.get_entity(attacker).unwrap();
        let retained = &attacker_entity.human_data().unwrap().sword_sweep;
        assert_eq!(retained.victims, vec![unreached_victim]);
        assert_eq!(retained.initial_angle, retained_initial_angle);
        assert_eq!(retained.current_angle, retained_current_angle);
        assert_eq!(retained.final_angle, retained_final_angle);
        let executable = attacker_entity
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("the unreached victim keeps retained geometry executable");
        assert_eq!(executable.strike, SwordStrike::I);
        assert_eq!(executable.current_angle, retained_current_angle);
        assert_eq!(
            attacker_entity
                .element_data()
                .sprite
                .frames_from_now_till_action_done(),
            0,
            "the zero-forecast pre-action effect must not advance the replacement sprite"
        );
        assert_eq!(
            attacker_entity.element_data().direction(),
            7,
            "the pre-action effect must not rotate the replacement sprite"
        );
    }

    #[test]
    fn replacement_half_circle_start_rebases_retained_angles_before_effect() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let retained_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: -10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let replacement_target = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        for target in [retained_victim, replacement_target] {
            engine
                .get_entity_mut(target)
                .and_then(Entity::enemy_ai_mut)
                .unwrap()
                .hth_weapon_id = 1;
        }
        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::G,
            crate::profiles::WeaponThrustKind::TrueHalfCircle,
        );
        std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
            [SwordStrike::G as usize]
            .direction = crate::profiles::WeaponThrustDirection::RightToLeft;
        let selected = install_test_melee_order(
            &mut engine,
            attacker,
            replacement_target,
            SwordStrike::G,
            false,
        );
        // Actor::Instruct publishes the selected G order to mpOrder before
        // Execute reaches its START warning boundary. The low-level fixture
        // installs the sequence order directly, so mirror that publication.
        engine.publish_selected_order_as_installed(attacker);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(0);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![retained_victim],
            // Stale left-to-right H geometry already spans the victim's
            // sector. Without the START warning-query rebase, G's first
            // IN_PROGRESS effect would queue a second sword hit.
            initial_angle: 0.0,
            current_angle: std::f32::consts::PI,
            final_angle: std::f32::consts::TAU,
            rotation_per_frame: std::f32::consts::FRAC_PI_2,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::H,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
        });
        {
            // The shared order fixture advances to the action point for most
            // sweep tests. Rewind only its sprite identity so the established
            // selected-owner dispatcher observes G's real START boundary.
            let sprite = &mut engine
                .get_entity_mut(attacker)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.last_processed_order_id = u32::MAX;
            sprite.last_action = crate::order::OrderType::WaitingSword;
            sprite.current_frame = 0;
            sprite.frame_count = 0;
        }

        // Human::Execute calls WarnForStrike on MotionState::Start. Its
        // half-circle victim query mutates the shared angles even though the
        // retained victim FIFO belongs to the interrupted H strike.
        engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .element_data()
                .sprite
                .last_motion_state,
            Some(crate::sprite::MotionState::Start),
            "the first selected-owner tick must exercise G's START warning boundary"
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .element_data()
                .direction(),
            0,
            "replacement G must turn right while the stale H victim remains on the left"
        );

        let sweep = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("replacement G keeps the interrupted victim FIFO");
        assert_eq!(sweep.pending_victims, vec![retained_victim]);
        assert_eq!(sweep.strike, SwordStrike::G);
        assert_eq!(
            sweep.direction,
            crate::profiles::WeaponThrustDirection::RightToLeft
        );
        let replacement_direction_angle = sector_to_angle(0);
        assert!((sweep.initial_angle - replacement_direction_angle).abs() < f32::EPSILON);
        assert!((sweep.current_angle - replacement_direction_angle).abs() < f32::EPSILON);
        assert!(
            (sweep.final_angle - (replacement_direction_angle - std::f32::consts::PI)).abs()
                < f32::EPSILON
        );

        engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .element_data()
                .sprite
                .last_motion_state,
            Some(crate::sprite::MotionState::InProgress),
            "the second selected-owner tick must exercise retained G geometry before DONE"
        );
        assert!(
            !engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| {
                    element.command == Command::ReceiveSwordDamage
                        && element.owner == Some(retained_victim)
                }),
            "G's first effect must use the START-rebased half-circle angles"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(selected.seq_id, selected.elem_idx)
                .is_some(),
            "the focused effect check keeps the replacement strike selected"
        );
    }

    #[test]
    fn terminal_true_circle_direction_is_presented_before_done_progresses() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::G,
            crate::profiles::WeaponThrustKind::TrueHalfCircle,
        );
        let selected =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::G, true);

        let terminal_angle = sector_to_angle(13);
        {
            let entity = engine.get_entity_mut(attacker).unwrap();
            entity.element_data_mut().set_direction_instantly(15);
            let sprite = &mut entity.element_data_mut().sprite;
            assert_eq!(sprite.current_frame, sprite.action_done_frame);
            assert_eq!(sprite.frame_count, sprite.action_done_counter);
            entity.actor_data_mut().unwrap().sweep_state = Some(crate::movement::SweepState {
                pending_victims: Vec::new(),
                initial_angle: terminal_angle + std::f32::consts::PI,
                current_angle: terminal_angle,
                final_angle: terminal_angle,
                rotation_per_frame: -std::f32::consts::FRAC_PI_2,
                // Deliberately stale retained metadata: Original dispatches
                // terminal action semantics from the current G call.
                direction: crate::profiles::WeaponThrustDirection::RightToLeft,
                strike: SwordStrike::F,
                attacker_profile_idx: Some(1),
                strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
            });
        }

        engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

        let attacker_entity = engine.get_entity(attacker).unwrap();
        assert_eq!(
            attacker_entity.element_data().direction(),
            13,
            "Original presents the terminal true-circle angle before the exact action-done call advances the sprite"
        );
        let sprite = &attacker_entity.element_data().sprite;
        let current_g_row = sprite
            .row_for_action(strike_to_animation(SwordStrike::G))
            .expect("current G animation remains mapped");
        assert_eq!(
            sprite.current_row,
            current_g_row + 13,
            "terminal presentation must force the current G animation row, not retained F"
        );
        assert_eq!(
            sprite.current_frame,
            sprite.action_done_frame + 1,
            "the zero-delay fixture advances one frame after presenting the terminal angle"
        );
        assert_eq!(
            sprite.frame_count, 0,
            "the zero-delay next frame begins at counter zero"
        );
        assert!(
            attacker_entity.actor_data().unwrap().sweep_state.is_none(),
            "the terminal presentation call clears an exhausted sweep mirror"
        );
    }

    #[test]
    fn replacement_true_circle_uses_current_direction_at_action_done() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::G,
            crate::profiles::WeaponThrustKind::TrueHalfCircle,
        );
        let selected =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::G, true);

        let current_angle = sector_to_angle(13);
        {
            let entity = engine.get_entity_mut(attacker).unwrap();
            entity.element_data_mut().set_direction_instantly(15);
            entity.actor_data_mut().unwrap().sweep_state = Some(crate::movement::SweepState {
                pending_victims: Vec::new(),
                initial_angle: current_angle,
                current_angle,
                final_angle: current_angle + std::f32::consts::FRAC_PI_2,
                rotation_per_frame: std::f32::consts::FRAC_PI_2,
                // Stale retained right-to-left/false-F metadata says the
                // sweep is complete. Current G is a left-to-right true
                // circle and must keep rotating without progressing sprite.
                direction: crate::profiles::WeaponThrustDirection::RightToLeft,
                strike: SwordStrike::F,
                attacker_profile_idx: Some(1),
                strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
            });
        }

        engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

        let attacker_entity = engine.get_entity(attacker).unwrap();
        let sprite = &attacker_entity.element_data().sprite;
        assert_eq!(attacker_entity.element_data().direction(), 13);
        assert_eq!(sprite.current_frame, sprite.action_done_frame);
        assert_eq!(sprite.frame_count, sprite.action_done_counter);
        assert!(
            attacker_entity.actor_data().unwrap().sweep_state.is_some(),
            "current G's left-to-right direction keeps the retained geometry rotating"
        );
    }

    #[test]
    fn saved_human_sweep_is_rehydrated_for_the_live_strike_order() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::E,
            crate::profiles::WeaponThrustKind::Lateral,
        );
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::E, true);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .sword_sweep = crate::element::HumanSwordSweepState {
            victims: vec![victim],
            initial_angle: 0.0,
            current_angle: 0.0,
            final_angle: std::f32::consts::PI,
        };
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_none()
        );

        engine.rebind_retained_sweep_to_active_strike(&assets, attacker);

        let sweep = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("serialized human sweep must regain its executable mirror");
        assert_eq!(sweep.pending_victims, vec![victim]);
        assert_eq!(sweep.initial_angle, 0.0);
        assert_eq!(sweep.current_angle, 0.0);
        assert_eq!(sweep.final_angle, std::f32::consts::PI);
        assert_eq!(sweep.strike, SwordStrike::E);
        assert_eq!(
            sweep.strike_kind,
            crate::profiles::WeaponThrustKind::Lateral
        );

        engine.tick_sweep_for(&crate::sim_rng::test_context(), &assets, attacker, false);
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .sword_sweep
                .victims
                .is_empty(),
            "consuming the executable victim must consume the serialized human mirror too"
        );
        engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_none(),
            "the consumed save victim must not be rehydrated and hit again next frame"
        );
    }

    #[test]
    fn terminated_lateral_sweep_cannot_rehydrate_into_a_fresh_strike() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::D,
            crate::profiles::WeaponThrustKind::Lateral,
        );

        engine.initialize_sweep(
            &assets,
            attacker,
            SwordStrike::D,
            Some(1),
            crate::profiles::WeaponThrustKind::Lateral,
            vec![victim],
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .sword_sweep
                .victims,
            vec![victim]
        );

        engine.complete_melee_strike(&sim, &assets, attacker, None, 0, SwordStrike::D, Some(1));

        let attacker_entity = engine.get_entity(attacker).unwrap();
        assert!(
            attacker_entity.actor_data().unwrap().sweep_state.is_none(),
            "termination clears the executable sweep"
        );
        assert!(
            attacker_entity
                .human_data()
                .unwrap()
                .sword_sweep
                .victims
                .is_empty(),
            "Original deletes the human-owned victim list on RHMOTION_TERMINATED"
        );

        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
        engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_none(),
            "a fresh lateral strike must wait for its own action-done initialization"
        );
    }

    #[test]
    fn later_circle_frame_tests_existing_angle_before_tail_advance() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::F,
            crate::profiles::WeaponThrustKind::FalseHalfCircle,
        );
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![victim],
            initial_angle: 0.0,
            current_angle: 0.0,
            final_angle: std::f32::consts::PI,
            rotation_per_frame: std::f32::consts::FRAC_PI_2,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
        });

        let queued_damage_count = |engine: &EngineInner| {
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .filter(|element| {
                    element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
                })
                .count()
        };

        engine.tick_sweep_for(sim, &assets, attacker, false);
        assert_eq!(
            queued_damage_count(&engine),
            0,
            "the victim in the newly reached sector cannot be tested before the circle tail advance"
        );
        let sweep = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("pending final-sector victim must keep the sweep alive");
        assert!((sweep.current_angle - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);

        engine.tick_sweep_for(sim, &assets, attacker, false);
        assert_eq!(
            queued_damage_count(&engine),
            1,
            "the next IN_PROGRESS effect must test the angle reached by the prior tail advance"
        );
    }

    #[test]
    fn circle_tail_retains_candidate_past_final_in_the_same_sector() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let pending_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::F,
            crate::profiles::WeaponThrustKind::FalseHalfCircle,
        );
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![pending_victim],
            initial_angle: 0.0,
            current_angle: 0.0,
            final_angle: 0.70,
            rotation_per_frame: 0.75,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
        });

        engine.tick_sweep_for(sim, &assets, attacker, false);

        let current = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("unreached victim keeps the circle sweep observable")
            .current_angle;
        assert!(
            (current - 0.75).abs() < f32::EPSILON,
            "a candidate past 0.70 in the same final sector must be retained instead of clamped"
        );
    }

    #[test]
    fn lateral_advance_is_raw_and_does_not_use_circle_final_clamping() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let pending_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::D,
            crate::profiles::WeaponThrustKind::Lateral,
        );
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![pending_victim],
            initial_angle: 0.0,
            current_angle: 0.0,
            final_angle: 0.70,
            rotation_per_frame: 1.20,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::D,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::Lateral,
        });

        engine.tick_sweep_for(sim, &assets, attacker, false);

        let current = engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("unreached victim keeps the lateral sweep observable")
            .current_angle;
        assert!(
            (current - 1.20).abs() < f32::EPSILON,
            "lateral Execute applies its signed rotation directly even past final_angle"
        );
    }

    #[test]
    fn push_victims_queue_damage_in_creation_fifo() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let first_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 80.0,
                z: 0.0,
            },
            None,
        ));
        let second_victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 60.0,
                z: 0.0,
            },
            None,
        ));
        for victim in [first_victim, second_victim] {
            engine
                .get_entity_mut(victim)
                .unwrap()
                .element_data_mut()
                .sprite
                .position_iface
                .set_move_box(crate::coordinates::MoveBox::from_corners(
                    crate::coordinates::MapVec::new(-5.0, -5.0),
                    crate::coordinates::MapVec::new(5.0, 5.0),
                ));
        }
        let assets = assets_with_nonstraight_profile(
            SwordStrike::D,
            crate::profiles::WeaponThrustKind::PushAside,
        );
        let selected =
            install_test_melee_order(&mut engine, attacker, first_victim, SwordStrike::D, false);

        assert_eq!(
            engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
            strikes::SweepTickPhase::InProgress
        );

        let first_life = soldier_life(&engine, first_victim);
        let second_life = soldier_life(&engine, second_victim);
        let damage_fifo: Vec<EntityId> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| element.command == Command::ReceiveSwordDamage)
            .filter_map(|element| element.owner)
            .collect();
        assert_eq!(
            damage_fifo,
            vec![first_victim, second_victim],
            "push damage launches must retain the original actor-list victim FIFO; lives were {first_life}/{second_life}"
        );
    }

    #[test]
    fn push_replacement_executes_without_advancing_retained_circle_sweep() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 140.0,
                z: 0.0,
            },
            None,
        ));
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::A,
            crate::profiles::WeaponThrustKind::PushAside,
        );
        let selected =
            install_test_melee_order(&mut engine, attacker, victim, SwordStrike::A, false);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(8);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .sweep_state = Some(crate::movement::SweepState {
            pending_victims: vec![victim],
            initial_angle: 2.0,
            current_angle: 5.5,
            final_angle: 5.5,
            rotation_per_frame: std::f32::consts::FRAC_PI_4,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::TrueHalfCircle,
        });

        engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .element_data()
                .direction(),
            8,
            "PushAside must not present the retained F sweep's terminal direction"
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .as_ref()
                .expect("PushAside leaves interrupted sweep storage dormant")
                .strike,
            SwordStrike::F,
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| {
                    element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
                }),
            "the replacement PushAside must still execute and queue its damage"
        );
    }

    #[test]
    fn push_strike_does_not_recover_antagonist_outside_rectangle() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let target = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 11.0,
                y: 80.0,
                z: 0.0,
            },
            None,
        ));
        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::A,
            crate::profiles::WeaponThrustKind::PushAside,
        );
        std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
            [SwordStrike::A as usize]
            .repulsion = 20;
        let selected =
            install_test_melee_order(&mut engine, attacker, target, SwordStrike::A, false);

        assert_eq!(
            engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
            strikes::SweepTickPhase::InProgress
        );

        assert!(
            !engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| {
                    element.command == Command::ReceiveSwordDamage && element.owner == Some(target)
                }),
            "Original's PushAside scan rejects side projection 11 outside half-width 10 even when the actor is the interaction antagonist"
        );
        assert_eq!(soldier_life(&engine, target), 50);
    }

    #[test]
    fn launching_sword_damage_does_not_add_attacker_tiredness() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_sword_profile(7, 30);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .tiredness = 11;

        crate::sim_rng::with_seed(1, |sim| {
            engine.queue_sword_damage(sim, &assets, victim, attacker, SwordStrike::A, 1);
        });

        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .tiredness,
            11,
            "damage application is victim-count dependent and must not charge strike energy"
        );
    }

    #[test]
    fn helping_climb_shoulder_damage_keeps_posture_until_fall_executes() {
        let sim = crate::sim_rng::SimulationContext::with_seed(0x183);
        let mut engine = make_engine();
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        engine
            .get_entity_mut(victim)
            .expect("test victim must exist")
            .set_posture(Posture::HelpingToClimb);

        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new(
            1,
            Command::ReceiveSwordDamage,
            Some(victim),
        ));
        let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        engine.translate_shoulder_damage(&sim, &assets, victim, (sequence_id, 0));

        assert_eq!(
            engine
                .get_entity(victim)
                .expect("test victim must remain live")
                .element_data()
                .posture,
            Posture::HelpingToClimb,
            "TranslateShoulderDamage only queues FallingBackUpright; its Execute START changes posture on the actor's next slot"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence_id, 0)
                .expect("damage element must remain registered")
                .orders
                .back()
                .expect("shoulder damage must queue a fall order")
                .order_type,
            OrderType::FallingBackUpright
        );
    }

    #[test]
    fn shoulder_damage_dispatches_partner_fall_without_direction_recompute() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let carrier = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let carried = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        engine
            .get_entity_mut(carrier)
            .unwrap()
            .set_posture(Posture::HelpingToClimb);
        engine
            .get_entity_mut(carrier)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(carried);
        engine
            .get_entity_mut(carried)
            .unwrap()
            .set_posture(Posture::OnShoulders);
        engine
            .get_entity_mut(carried)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .carrier = Some(carrier);

        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveDamage,
            Some(carrier),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        engine.orders.sequence_manager.launch_element(damage);
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let partner_fall = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| element.command == Command::Fall && element.owner == Some(carried))
            .expect("shoulder damage must dispatch Fall to the carried partner");
        let order = partner_fall
            .orders
            .iter()
            .find(|order| order.order_type == OrderType::FallingShoulders)
            .expect("partner Fall command must translate to FallingShoulders");
        assert!(!order.compute_direction);
    }

    #[test]
    fn slope_translate_roll_order_keeps_its_source_authored_direction_recompute() {
        let mut engine = make_engine();
        let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
        obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
        let mut assets = LevelAssets::new();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
        {
            let victim = engine.get_entity_mut(victim).unwrap();
            victim.element_data_mut().set_obstacle_index(
                crate::position_interface::ObstacleHandle::new(0),
                Some(crate::position_interface::PlaneZCoeffs {
                    az: 1.0,
                    bz: 0.0,
                    dz: 0.0,
                }),
            );
            victim
                .position_iface_mut()
                .set_move_box(crate::coordinates::MoveBox::from_corners(
                    crate::coordinates::MapVec::new(-5.0, -5.0),
                    crate::coordinates::MapVec::new(5.0, 5.0),
                ));
        }
        let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveDamage, Some(victim));
        let sequence = engine.orders.sequence_manager.launch_element(damage);

        engine.try_queue_roll(&assets, victim, (sequence, 0));

        let rolling = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .iter()
            .find(|order| order.order_type == OrderType::Rolling)
            .expect("TranslateRoll must append its Rolling order");
        assert!(rolling.compute_direction);
    }

    #[test]
    fn parried_damage_still_learns_attackers_live_strike() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_sword_profile(7, 30);

        let mut attacker_sequence = crate::sequence::Sequence::new();
        attacker_sequence.append_element(crate::sequence::SequenceElement::new(
            1,
            Command::SwordstrikeThrustE,
            Some(attacker),
        ));
        let attacker_sequence_id = engine
            .orders
            .sequence_manager
            .launch_sequence(attacker_sequence);
        engine
            .orders
            .sequence_manager
            .element_in_progress(attacker_sequence_id, 0);

        let mut damage_sequence = crate::sequence::Sequence::new();
        let mut damage_element =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage_element.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::E, 1);
        damage_sequence.append_element(damage_element);
        let damage_sequence_id = engine
            .orders
            .sequence_manager
            .launch_sequence(damage_sequence);
        engine
            .orders
            .sequence_manager
            .element_in_progress(damage_sequence_id, 0);

        let Entity::Soldier(soldier) = engine.get_entity_mut(victim).unwrap() else {
            unreachable!()
        };
        soldier.actor.action_state = ActionState::ParryingSword;
        let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
            unreachable!()
        };
        ai.known_enemy_strike_1 = Some(SwordStrike::D);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::E),
            Some(1),
            (damage_sequence_id, 0),
        );

        let Entity::Soldier(soldier) = engine.get_entity(victim).unwrap() else {
            unreachable!()
        };
        let crate::element::AiBrain::Enemy(ai) = &soldier.npc.ai_brain else {
            unreachable!()
        };
        assert_eq!(ai.known_enemy_strike_1, Some(SwordStrike::E));
        assert_eq!(
            ai.known_enemy_strike_2, None,
            "a low-skill guard forgets its previous strike when the parried live strike is learned"
        );
    }

    #[test]
    fn push_damage_virtual_say_ouch_is_silent_for_pc() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::H,
            crate::profiles::WeaponThrustKind::TrueCircle,
        );
        let damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);

        assert!(engine.apply_push_effect(
            &sim,
            &assets,
            victim,
            attacker,
            &PushStrikeInfo { repulsion: 100 },
            combat::SwordDamageResult::NO_DAMAGE_PARRIED,
            (sequence_id, 0),
            false,
        ));
        assert!(
            engine.feedback.sound_sim.pending_exclamations.is_empty(),
            "PC inherits RHElementActorHuman::SayOuch's no-op on TranslatePushDamage"
        );
    }

    #[test]
    fn push_damage_command_disables_direction_on_fall_and_successors() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim = engine
                .get_entity_mut(victim)
                .expect("push victim remains live");
            victim.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            victim.human_data_mut().unwrap().concussion_of_the_brain = STUNNING_THRESHOLD + 1;
            victim.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
        let assets = assets_with_nonstraight_profile(
            SwordStrike::H,
            crate::profiles::WeaponThrustKind::TrueCircle,
        );
        let damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveSwordDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        assert!(engine.apply_push_effect(
            &sim,
            &assets,
            victim,
            attacker,
            &PushStrikeInfo { repulsion: 100 },
            combat::SwordDamageResult::STUNNING_DAMAGE,
            (sequence, 0),
            false,
        ));

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("translated push damage remains registered");
        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![
                OrderType::FallingPushedWithSword,
                OrderType::StandingUpSword,
                OrderType::BeingStunnedSword,
            ]
        );
        assert!(
            element
                .orders
                .iter()
                .filter(|order| order.order_type != OrderType::Rolling)
                .all(|order| !order.compute_direction),
            "TranslatePushDamage sets bComputeDirection=false on the fall, stand-up, and stunned orders"
        );
    }

    #[test]
    fn pc_hurt_speech_uses_applied_life_loss_not_attempted_damage() {
        let mut engine = make_engine();
        let victim = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);

        // The protected task-188 control attempts more than twenty points of
        // damage, but only 18 LP are ultimately stored (82 -> 64).
        let attempted_damage = 25;
        assert!(attempted_damage > 20);
        engine.pc_life_points_speech(&assets, victim, 82, 64);
        assert!(
            engine.feedback.sound_sim.pending_exclamations.is_empty(),
            "RHElementActorPC::SetLifePoints compares the applied LP delta"
        );

        engine.pc_life_points_speech(&assets, victim, 82, 61);
        assert_eq!(
            engine
                .feedback
                .sound_sim
                .pending_exclamations
                .iter()
                .map(|pending| pending.exclamation_id)
                .collect::<Vec<_>>(),
            vec![HERO_HURT]
        );
    }

    #[test]
    fn push_strike_does_not_inform_soldier_of_good_strike() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = attacker.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
            ai.hth_weapon_id = 1;
        }
        let assets = assets_with_nonstraight_profile(
            SwordStrike::H,
            crate::profiles::WeaponThrustKind::TrueCircle,
        );
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::H),
            Some(1),
            (sequence_id, 0),
        );

        let ai = engine
            .get_entity(attacker)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert!(
            !ai.ai_log.iter().any(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventGoodStrike as u16
            }),
            "Original skips TranslateSwordDamage, and therefore EVENT_GOOD_STRIKE, for push strikes"
        );
        assert_eq!(
            ai.current_substate,
            Substate::AttackingSwordfightSpecialStrike
        );
    }

    #[test]
    fn ordinary_cutting_strike_still_informs_soldier_of_good_strike() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            soldier.human.opponents.push(victim);
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = attacker.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
            ai.hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let assets = assets_with_sword_profile(1, 50);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence_id, 0),
        );

        let ai = engine
            .get_entity(attacker)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert!(ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        }));
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![victim],
            "a conscious surviving victim must retain the swordfight"
        );
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![attacker]
        );
        let damage = engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("cutting damage command remains registered");
        assert!(
            damage
                .orders
                .iter()
                .filter(|order| order.order_type != OrderType::Rolling)
                .all(|order| !order.compute_direction),
            "TranslateSwordDamage sets bComputeDirection=false on its cutting-hit order"
        );
    }

    #[test]
    fn pc_shoulder_sword_damage_skips_good_strike_but_keeps_fall_translation() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};
        use crate::sequence::SequencePriority;

        for posture in [
            Posture::HelpingToClimb,
            Posture::CarryingOnShoulders,
            Posture::OnShoulders,
        ] {
            let sim = crate::sim_rng::test_context();
            let mut engine = make_engine();
            let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
            let victim = engine.add_entity(make_pc(
                WorldPoint3D {
                    x: 10.0,
                    ..WorldPoint3D::default()
                },
                None,
            ));
            let partner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
            {
                let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                    unreachable!()
                };
                soldier.human.opponents.push(victim);
                let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
                ai.base.me = attacker.index();
                ai.base.current_state = AiState::Attacking;
                ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
                ai.hth_weapon_id = 1;
            }
            engine
                .get_entity_mut(victim)
                .unwrap()
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
            engine.get_entity_mut(victim).unwrap().set_posture(posture);
            if posture == Posture::OnShoulders {
                engine
                    .get_entity_mut(victim)
                    .unwrap()
                    .human_data_mut()
                    .unwrap()
                    .carrier = Some(partner);
                let partner_entity = engine.get_entity_mut(partner).unwrap();
                partner_entity.set_posture(Posture::CarryingOnShoulders);
                partner_entity.pc_data_mut().unwrap().carried = Some(victim);
            } else {
                engine
                    .get_entity_mut(victim)
                    .unwrap()
                    .pc_data_mut()
                    .unwrap()
                    .carried = Some(partner);
                let partner_entity = engine.get_entity_mut(partner).unwrap();
                partner_entity.set_posture(Posture::OnShoulders);
                partner_entity.human_data_mut().unwrap().carrier = Some(victim);
            }

            let assets = assets_with_sword_profile(1, 50);
            let mut damage =
                crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
            damage.data =
                crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
            let sequence_id = engine.orders.sequence_manager.launch_element(damage);
            engine
                .orders
                .sequence_manager
                .element_in_progress(sequence_id, 0);

            engine.apply_sword_damage(
                &sim,
                &assets,
                victim,
                Some(attacker),
                Some(SwordStrike::A),
                Some(1),
                (sequence_id, 0),
            );

            let attacker_ai = engine
                .get_entity(attacker)
                .unwrap()
                .ai_controller()
                .unwrap();
            assert!(
                !attacker_ai.ai_log.iter().any(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventGoodStrike as u16
                }),
                "PC posture {posture:?} must use the PC shoulder override without EventGoodStrike"
            );
            assert_eq!(
                attacker_ai.current_substate,
                Substate::AttackingSwordfightSpecialStrike,
                "suppressed EventGoodStrike must not advance the attacker AI"
            );

            let damage = engine
                .orders
                .sequence_manager
                .get_element(sequence_id, 0)
                .expect("shoulder damage command remains registered");
            let expected_fall = if posture == Posture::OnShoulders {
                assert_eq!(damage.priority, SequencePriority::NonInterruptable);
                OrderType::FallingShoulders
            } else {
                OrderType::FallingBackUpright
            };
            assert!(
                damage
                    .orders
                    .iter()
                    .any(|order| order.order_type == expected_fall),
                "PC posture {posture:?} must retain its shoulder fall translation"
            );
            assert!(
                engine
                    .orders
                    .sequence_manager
                    .sequences_iter()
                    .flat_map(|sequence| sequence.elements.iter())
                    .any(|element| element.command == Command::Fall
                        && element.owner == Some(partner)),
                "PC posture {posture:?} must still dispatch Fall to its shoulder partner"
            );
        }
    }

    #[test]
    fn non_pc_helping_to_climb_still_informs_soldier_of_good_strike() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = attacker.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
            ai.hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .set_posture(Posture::HelpingToClimb);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
        let assets = assets_with_sword_profile(1, 50);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence_id, 0),
        );

        let attacker_ai = engine
            .get_entity(attacker)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert!(attacker_ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        }));
    }

    #[test]
    fn lateral_done_processes_victims_in_original_actor_order_before_good_strike() {
        use crate::ai::{AiState, LogLineType, Remark, StimulusType, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        // Allocate the survivor first so typed entity iteration disagrees with
        // Original's actor registry below. This is the Save016 shape: the
        // later-ID victim must knock out and unlink the attacker first.
        let survivor = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 20.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        let knockout = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        engine.world.install_original_creation_orders(
            [(attacker, 0), (knockout, 1), (survivor, 2)]
                .into_iter()
                .collect(),
            3,
        );

        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            // Only the first Original-order victim is an opponent. Its KO
            // therefore synchronously sends EventQuitSwordfight to the
            // attacker before damage reaches the later survivor.
            soldier.human.opponents.push(knockout);
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = attacker.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
            ai.hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(knockout)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        // Keep the later victim conscious while retaining real cutting damage,
        // so it would emit GoodStrike if processed before the KO callback.
        engine
            .get_entity_mut(survivor)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .invulnerable = true;

        let mut assets = assets_with_sword_profile_effects(1, 100, 4, 100);
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::A as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::Lateral;
        thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
        thrust.initial_angle = 0;
        thrust.final_angle = 180;
        thrust.rotation_angle = 90;

        let victims =
            engine.execute_multi_target_strike(&assets, attacker, SwordStrike::A, Some(1));
        assert_eq!(
            victims,
            [knockout, survivor],
            "DONE membership is unchanged, but follows Original GetActor FIFO rather than typed IDs"
        );

        for victim in victims {
            let mut damage =
                crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
            damage.data =
                crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
            let sequence_id = engine.orders.sequence_manager.launch_element(damage);
            engine
                .orders
                .sequence_manager
                .element_in_progress(sequence_id, 0);
            engine.apply_sword_damage(
                &sim,
                &assets,
                victim,
                Some(attacker),
                Some(SwordStrike::A),
                Some(1),
                (sequence_id, 0),
            );
        }

        assert!(
            engine
                .get_entity(knockout)
                .unwrap()
                .human_data()
                .unwrap()
                .unconscious,
            "first victim must exercise the synchronous knockout/quit arm"
        );
        assert!(
            !engine
                .get_entity(survivor)
                .unwrap()
                .human_data()
                .unwrap()
                .unconscious,
            "later cutting victim must remain a genuine surviving control"
        );
        let ai = engine
            .get_entity(attacker)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
        assert!(
            ai.ai_log.iter().any(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventGoodStrike as u16
            }),
            "later survivor must deliver a real GoodStrike after the first victim quits"
        );
        assert!(
            !ai.ai_log.iter().any(|entry| {
                entry.line_type == LogLineType::Speak
                    && entry.info == Remark::GoodStrikeCombat as u16
            }),
            "later GoodStrike is delivered after quit and must not start speech"
        );
        assert!(
            engine.feedback.sound_sim.pending_exclamations.is_empty(),
            "ignored later GoodStrike must not leave a pending combat exclamation"
        );
    }

    #[test]
    fn surviving_sword_knockout_quits_before_good_strike_and_fall_translation() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            soldier.human.opponents.push(victim);
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = attacker.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
            ai.hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);

        let mut assets = assets_with_sword_profile_effects(1, 50, 4, 100);
        let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
        obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
        assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(0),
            Some(crate::position_interface::PlaneZCoeffs {
                az: 1.0,
                bz: 0.0,
                dz: 0.0,
            }),
        );
        victim_entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence_id, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(victim_entity.human_data().unwrap().unconscious);
        assert!(
            victim_entity.pc_data().unwrap().life_points > 0,
            "fixture must exercise the surviving-knockout arm"
        );
        assert!(victim_entity.human_data().unwrap().opponents.is_empty());
        let attacker_entity = engine.get_entity(attacker).unwrap();
        assert!(attacker_entity.human_data().unwrap().opponents.is_empty());
        let ai = attacker_entity.ai_controller().unwrap();
        assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
        let good_strike_index = ai
            .ai_log
            .iter()
            .position(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventGoodStrike as u16
            })
            .expect("soldier origin must receive EVENT_GOOD_STRIKE");
        let quit_index = ai
            .ai_log
            .iter()
            .position(|entry| {
                entry.line_type == LogLineType::ChangeState
                    && entry.info == Substate::AttackingQuittingSwordfight as u16
            })
            .expect("reciprocal unlink must synchronously enter the quitting substate");
        assert!(
            quit_index < good_strike_index,
            "SetConcussionOfTheBrain quits before TranslateSwordDamage informs the hitter"
        );
        let translated_orders = &engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("knockout damage element remains registered")
            .orders;
        assert_eq!(
            translated_orders.front().map(|order| order.order_type),
            Some(OrderType::FallingBackUpright),
            "TranslateSwordDamage's second quit remains before its knockout fall"
        );
        assert!(
            translated_orders
                .iter()
                .any(|order| order.order_type == OrderType::Rolling),
            "the real surviving-KO translation must still append Roll"
        );
    }

    #[test]
    fn preexisting_unconscious_smalltalk_hit_preserves_closed_eyes_and_plain_quit() {
        use crate::ai::{LogLineType, StimulusType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().posture = Posture::Upright;
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
            victim_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(victim);

        let assets = assets_with_sword_profile(1, 50);

        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data = crate::sequence::SequenceElementData::new_sword_damage(
            attacker,
            SwordStrike::SmalltalkRight,
            1,
        );
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(victim_entity.human_data().unwrap().unconscious);
        assert_eq!(
            victim_entity.npc_data().unwrap().eye_status,
            EyeStatus::Closed
        );
        assert!(victim_entity.human_data().unwrap().opponents.is_empty());
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents
                .is_empty(),
            "TranslateSwordDamage's plain quit removes the reciprocal opponent"
        );
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .filter(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventQuitSwordfight as u16
                })
                .count(),
            1,
            "the pre-existing-unconscious translation owns exactly one plain quit"
        );
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .filter(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventLoseConsciousness as u16
                })
                .count(),
            0,
            "the hit must not replay SetConcussion's KO callback"
        );
        assert_eq!(
            engine
                .feedback
                .titbit_manager
                .titbits()
                .iter()
                .filter(|titbit| {
                    titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                        && titbit.element_supplier.0 == victim.index()
                })
                .count(),
            0,
            "the hit must not recreate the existing unconscious star"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .orders
                .iter()
                .any(|order| order.order_type == OrderType::FallingBackSword),
            "upright WaitingSword translation still queues FallingBackSword"
        );
    }

    #[test]
    fn protected_preexisting_unconscious_smalltalk_hit_has_no_translation() {
        use crate::ai::{LogLineType, StimulusType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().posture = Posture::Upright;
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
            victim_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(victim);

        let mut assets = assets_with_sword_profile(1, 50);
        std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .protection_by_localization = [99; 5];

        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data = crate::sequence::SequenceElementData::new_sword_damage(
            attacker,
            SwordStrike::SmalltalkRight,
            1,
        );
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_entity.human_data().unwrap().opponents,
            vec![attacker],
            "NO_DAMAGE must not enter TranslateSwordDamage's plain-quit path"
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![victim]
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .orders
                .is_empty(),
            "NO_DAMAGE must not translate a FallingBack/Roll order"
        );
        assert!(
            !victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .any(|entry| {
                    entry.line_type == LogLineType::Event
                        && matches!(
                            entry.info,
                            value if value == StimulusType::EventQuitSwordfight as u16
                                || value == StimulusType::EventLoseConsciousness as u16
                        )
                }),
            "NO_DAMAGE must neither quit nor replay the knockout callback"
        );
    }

    #[test]
    fn grounded_preexisting_unconscious_smalltalk_hit_terminates_without_quit() {
        use crate::ai::{LogLineType, StimulusType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().posture = Posture::Lying;
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
            victim_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(victim);

        let assets = assets_with_sword_profile(1, 50);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data = crate::sequence::SequenceElementData::new_sword_damage(
            attacker,
            SwordStrike::SmalltalkRight,
            1,
        );
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_entity.npc_data().unwrap().eye_status,
            EyeStatus::Closed
        );
        assert_eq!(
            victim_entity.human_data().unwrap().opponents,
            vec![attacker]
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![victim]
        );
        let damage = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
        assert!(damage.orders.is_empty());
        assert!(
            !victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .any(|entry| {
                    entry.line_type == LogLineType::Event
                        && matches!(
                            entry.info,
                            value if value == StimulusType::EventQuitSwordfight as u16
                                || value == StimulusType::EventLoseConsciousness as u16
                        )
                })
        );
    }

    #[test]
    fn lethal_sword_hit_kills_unconscious_npc_before_say_ouch_translation() {
        use crate::ai::{AiState, LogLineType, Remark, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.npc_data_mut().unwrap().life_points = 15;
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            let ai = victim_entity.enemy_ai_mut().unwrap();
            ai.hth_weapon_id = 1;
            ai.base.current_state = AiState::Sleeping;
            ai.base.current_substate = Substate::SleepingUnconscious;
        }

        let assets = assets_with_sword_profile_effects(1, 50, 100, 0);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(victim_entity.is_dead());
        assert!(!victim_entity.human_data().unwrap().unconscious);
        let ai = victim_entity.ai_controller().unwrap();
        assert_eq!(ai.current_substate, Substate::SleepingForever);
        assert!(ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Speak && entry.info == Remark::Dies as u16
        }));
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .orders
                .iter()
                .any(|order| order.order_type == OrderType::DyingSword),
            "TranslateSwordDamage must retain ownership of the dying visual after synchronous Kill"
        );
    }

    #[test]
    fn nonlethal_sword_hit_keeps_unconscious_npc_silent() {
        use crate::ai::{AiState, LogLineType, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            let ai = victim_entity.enemy_ai_mut().unwrap();
            ai.hth_weapon_id = 1;
            ai.base.current_state = AiState::Sleeping;
            ai.base.current_substate = Substate::SleepingUnconscious;
        }

        let assets = assets_with_sword_profile_effects(1, 50, 1, 0);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(victim_entity.npc_data().unwrap().life_points, 49);
        assert!(victim_entity.human_data().unwrap().unconscious);
        assert!(
            !victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .any(|entry| entry.line_type == LogLineType::Speak),
            "the ordinary unconscious SayOuch early return must remain intact for survivors"
        );
    }

    #[test]
    fn lethal_push_runs_npc_kill_cascade_before_owning_the_fall() {
        use crate::ai::{AiState, AlertLevel, Substate};
        use crate::element::{Detectable, DetectableType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        let observer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.npc_data_mut().unwrap().life_points = 1;
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            victim_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
            let ai = victim_entity.enemy_ai_mut().unwrap();
            ai.hth_weapon_id = 1;
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfight;
            ai.base.current_music_alert_status = AlertLevel::Red;
            ai.base.view_alert_status = AlertLevel::Red;
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(victim);
        {
            let observer_npc = engine
                .get_entity_mut(observer)
                .unwrap()
                .npc_data_mut()
                .unwrap();
            observer_npc.ai_brain.enemy_mut().unwrap().hth_weapon_id = 1;
            for detectable_type in [DetectableType::Friend, DetectableType::MissedFriend] {
                observer_npc.detectable_lists[detectable_type as usize].push(Detectable {
                    element: Some(victim),
                    detectable_type,
                    ..Detectable::default()
                });
            }
        }

        let mut assets = assets_with_sword_profile_effects(1, 50, 100, 0);
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::A as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
        thrust.repulsion = 100;
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        let score_before = engine
            .mission_domain
            .campaign
            .get_value(crate::campaign::CampaignValue::Score);
        let killed_allied_before = engine.mission_domain.mission_stat.killed_allied_count;

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        let victim_ai = victim_entity.ai_controller().unwrap();
        assert!(victim_entity.is_dead());
        assert_eq!(victim_ai.current_state, AiState::Sleeping);
        assert_eq!(victim_ai.current_substate, Substate::SleepingForever);
        assert_eq!(victim_ai.current_music_alert_status, AlertLevel::Green);
        assert_eq!(victim_ai.view_alert_status, AlertLevel::Green);
        assert!(victim_entity.human_data().unwrap().opponents.is_empty());
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents
                .is_empty()
        );
        let observer_npc = engine.get_entity(observer).unwrap().npc_data().unwrap();
        assert!(
            observer_npc.detectable_lists[DetectableType::Friend as usize].is_empty()
                && observer_npc.detectable_lists[DetectableType::MissedFriend as usize].is_empty()
        );
        let damage = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("push damage remains the visual owner");
        assert_eq!(
            damage
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![OrderType::FallingPushedWithSword]
        );
        assert_eq!(
            engine
                .mission_domain
                .campaign
                .get_value(crate::campaign::CampaignValue::Score),
            score_before + 50,
            "the Lacklandist lethal push applies the Kill score exactly once"
        );
        assert_eq!(
            engine.mission_domain.mission_stat.killed_allied_count, killed_allied_before,
            "an enemy death must not enter the allied-death statistic arm"
        );
    }

    #[test]
    fn surviving_push_does_not_run_npc_kill_cascade() {
        use crate::ai::{AiState, AlertLevel, Substate};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
            victim_entity
                .human_data_mut()
                .unwrap()
                .opponents
                .push(attacker);
            let ai = victim_entity.enemy_ai_mut().unwrap();
            ai.hth_weapon_id = 1;
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfight;
            ai.base.current_music_alert_status = AlertLevel::Red;
            ai.base.view_alert_status = AlertLevel::Red;
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(victim);

        let mut assets = assets_with_sword_profile_effects(1, 50, 4, 0);
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::A as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
        thrust.repulsion = 100;
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        let victim_ai = victim_entity.ai_controller().unwrap();
        assert!(get_life_points(victim_entity) > 0);
        assert_eq!(victim_ai.current_state, AiState::Attacking);
        assert_eq!(victim_ai.current_substate, Substate::AttackingSwordfight);
        assert_eq!(victim_ai.current_music_alert_status, AlertLevel::Red);
        assert_eq!(
            victim_entity.human_data().unwrap().opponents,
            vec![attacker]
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![victim]
        );
    }

    #[test]
    fn surviving_push_sword_knockout_applies_one_ko_callback_and_star() {
        use crate::ai::{LogLineType, StimulusType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        engine
            .get_entity_mut(victim)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
        let mut assets = assets_with_sword_profile_effects(1, 50, 4, 100);
        let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
            .thrusts[SwordStrike::A as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
        thrust.repulsion = 100;

        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence_id, 0),
        );

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(victim_entity.human_data().unwrap().unconscious);
        assert!(
            get_life_points(victim_entity) > 0,
            "fixture must exercise a surviving push knockout"
        );
        let lose_consciousness_callbacks = victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventLoseConsciousness as u16
            })
            .count();
        assert_eq!(
            lose_consciousness_callbacks, 1,
            "TranslatePushDamage must not repeat SetConcussionOfTheBrain's synchronous callback"
        );
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .filter(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventQuitSwordfight as u16
                })
                .count(),
            2,
            "fresh animated push owns SetConcussion's first quit and TranslatePushDamage's second quit"
        );
        assert_eq!(
            engine
                .feedback
                .titbit_manager
                .titbits()
                .iter()
                .filter(|titbit| {
                    titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                        && titbit.element_supplier.0 == victim.index()
                })
                .count(),
            1,
            "a fresh push knockout creates one unconscious-star visual"
        );
    }

    #[test]
    fn no_animation_fresh_push_knockout_does_not_repeat_ko_side_effects() {
        use crate::ai::{LogLineType, StimulusType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().posture = Posture::Carried;
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        }
        let assets = assets_with_sword_profile(1, 50);
        // Model SetConcussionOfTheBrain's already-completed fresh-KO prefix,
        // then enter the no-animation TranslatePushDamage arm.
        engine.apply_knockout_side_effects(&sim, &assets, victim, true, false);
        let damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .set_translating_element(Some((
                victim,
                crate::sequence::SequenceElementRef::new(sequence, 0),
            )));

        assert!(engine.apply_push_effect(
            &sim,
            &assets,
            victim,
            attacker,
            &PushStrikeInfo { repulsion: 100 },
            combat::SwordDamageResult::STUNNING_DAMAGE,
            (sequence, 0),
            true,
        ));
        engine.orders.sequence_manager.set_translating_element(None);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(victim_entity.human_data().unwrap().unconscious);
        assert_eq!(victim_entity.element_data().posture, Posture::Lying);
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .filter(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventLoseConsciousness as u16
                })
                .count(),
            1,
            "the no-animation TranslatePushDamage arm must not repeat a fresh KO callback"
        );
    }

    #[test]
    fn preexisting_unconscious_push_preserves_closed_eyes_without_replaying_ko() {
        use crate::ai::{LogLineType, StimulusType};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().posture = Posture::Upright;
            victim_entity.human_data_mut().unwrap().unconscious = true;
            victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
            victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(victim);
        let assets = assets_with_sword_profile(1, 50);
        let damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        let sequence = engine.orders.sequence_manager.launch_element(damage);

        assert!(engine.apply_push_effect(
            &sim,
            &assets,
            victim,
            attacker,
            &PushStrikeInfo { repulsion: 100 },
            combat::SwordDamageResult::STUNNING_DAMAGE,
            (sequence, 0),
            false,
        ));

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(victim_entity.human_data().unwrap().unconscious);
        assert_eq!(
            victim_entity.npc_data().unwrap().eye_status,
            EyeStatus::Closed
        );
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .filter(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventLoseConsciousness as u16
                })
                .count(),
            0,
            "TranslatePushDamage must not replay SetConcussion's conscious-to-unconscious callback"
        );
        assert_eq!(
            engine
                .feedback
                .titbit_manager
                .titbits()
                .iter()
                .filter(|titbit| {
                    titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                        && titbit.element_supplier.0 == victim.index()
                })
                .count(),
            0,
            "TranslatePushDamage must not recreate the existing unconscious star"
        );
        assert!(
            victim_entity.human_data().unwrap().opponents.is_empty(),
            "the animated translation removes the victim's opponent"
        );
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents
                .is_empty(),
            "the animated translation removes the reciprocal opponent"
        );
        assert_eq!(
            victim_entity
                .ai_controller()
                .unwrap()
                .ai_log
                .iter()
                .filter(|entry| {
                    entry.line_type == LogLineType::Event
                        && entry.info == StimulusType::EventQuitSwordfight as u16
                })
                .count(),
            1,
            "pre-existing unconscious animated translation owns exactly one plain quit"
        );
        let orders = &engine
            .orders
            .sequence_manager
            .get_sequence(sequence)
            .unwrap()
            .elements[0]
            .orders;
        assert!(
            orders
                .iter()
                .any(|order| order.order_type == OrderType::FallingPushedWithSword),
            "the already-unconscious victim still receives the authored push animation"
        );
    }

    #[test]
    fn parried_true_circle_still_queues_push_fall() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let assets = assets_with_nonstraight_profile(
            SwordStrike::H,
            crate::profiles::WeaponThrustKind::TrueCircle,
        );

        let mut damage_sequence = crate::sequence::Sequence::new();
        let mut damage_element =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage_element.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
        damage_sequence.append_element(damage_element);
        let damage_sequence_id = engine
            .orders
            .sequence_manager
            .launch_sequence(damage_sequence);
        engine
            .orders
            .sequence_manager
            .element_in_progress(damage_sequence_id, 0);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = ActionState::ParryingSword;
        let victim_position_before = engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .position_map();
        let victim_moving_before = engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .is_moving_map();

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::H),
            Some(1),
            (damage_sequence_id, 0),
        );

        let damage = engine
            .orders
            .sequence_manager
            .get_element(damage_sequence_id, 0)
            .expect("parried push damage must retain its sequence element");
        assert_eq!(
            damage
                .orders
                .back()
                .expect("Original TranslatePushDamage queues a fall even when the hit is parried")
                .order_type,
            OrderType::FallingPushedWithSword
        );
        assert!(
            damage
                .orders
                .iter()
                .filter(|order| order.order_type != OrderType::Rolling)
                .all(|order| !order.compute_direction),
            "TranslatePushDamage sets bComputeDirection=false on the falling-pushed order"
        );
        let victim_after_translation = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_after_translation.element_data().position_map(),
            victim_position_before,
            "TranslatePushDamage only queues the falling order; ExecuteFallingPushed owns movement"
        );
        assert_eq!(
            victim_after_translation.position_iface().is_moving_map(),
            victim_moving_before,
            "translation must not introduce movement before the falling order executes"
        );

        // Model the replay boundary: the damage element has authored a push
        // fall, but the victim's still-selected order is its postponed parry.
        // ReadyForTakeOff must not initialize until FallingPushedWithSword
        // becomes current and reports Start.
        engine
            .orders
            .sequence_manager
            .postpone_element(damage_sequence_id, 0);
        let mut parry_sequence = crate::sequence::Sequence::new();
        let mut parry_element =
            crate::sequence::SequenceElement::new(1, Command::ParrySword, Some(victim));
        parry_element.orders.push_back(crate::order::Order::new(
            OrderType::ParryingSword,
            0.0,
            0.0,
            engine.orders.allocate_order_id(),
        ));
        parry_sequence.append_element(parry_element);
        let parry_sequence_id = engine
            .orders
            .sequence_manager
            .launch_sequence(parry_sequence);
        engine
            .orders
            .sequence_manager
            .element_in_progress(parry_sequence_id, 0);
        assert!(
            engine
                .get_entity(victim)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_flight
                .is_none(),
            "TranslatePushDamage must not run ReadyForTakeOff eagerly"
        );

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(victim)
                .map(|(_, _, order)| order.order_type),
            Some(OrderType::ParryingSword)
        );
        engine.tick_push_flights(&sim, &assets);
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .element_data()
                .position_map(),
            victim_position_before,
            "prepared push flight must wait behind the still-selected parry order"
        );

        engine
            .orders
            .sequence_manager
            .element_terminated(parry_sequence_id, 0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(damage_sequence_id, 0);
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.set_posture(Posture::Flying);
            victim_entity
                .actor_data_mut()
                .unwrap()
                .execute_order_initialising = true;
        }
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(victim)
                .map(|(_, _, order)| order.order_type),
            Some(OrderType::FallingPushedWithSword)
        );
        let material_before_takeoff = engine.get_entity(victim).unwrap().element_data().material();
        engine.initialize_push_flight(
            &assets,
            victim,
            (damage_sequence_id, 0),
            OrderType::FallingPushedWithSword,
        );
        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().material(),
            material_before_takeoff,
            "ReadyForTakeOff installs only the goal obstacle/plane, not its material"
        );
        assert!(
            engine
                .get_entity(victim)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_flight
                .is_none(),
            "a fully rejected ReadyForTakeOff retains zero displacement"
        );
        let accepted_increment = 1.0;
        engine
            .get_entity_mut(victim)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_flight = Some(ActiveFlight {
            increment_x: accepted_increment,
            goal_x: victim_position_before.x + 8.0,
            goal_y: victim_position_before.y,
            frames_remaining: 8,
            antagonist: Some(attacker),
            ..Default::default()
        });
        engine.tick_push_flights(&sim, &assets);
        let victim_after_fall_start = engine.get_entity(victim).unwrap();
        assert_eq!(
            victim_after_fall_start.element_data().posture,
            Posture::Flying
        );
        assert_eq!(
            victim_after_fall_start.element_data().position_map(),
            crate::coordinates::MapPoint::new(
                victim_position_before.x + accepted_increment,
                victim_position_before.y
            ),
            "PerformFlight applies its first increment on the Start Execute"
        );
        assert_eq!(
            victim_after_fall_start
                .actor_data()
                .unwrap()
                .active_flight
                .unwrap()
                .frames_remaining,
            7
        );

        engine
            .get_entity_mut(victim)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = false;
        engine.tick_push_flights(&sim, &assets);
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .element_data()
                .position_map(),
            crate::coordinates::MapPoint::new(
                victim_position_before.x + 2.0 * accepted_increment,
                victim_position_before.y
            ),
            "the following Execute applies the second push-flight increment"
        );
        assert!(
            !engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(attacker, |command| {
                    command == Command::Provoke
                }),
            "parried push strikes still skip the later provoke branch"
        );
    }

    #[test]
    fn pushed_flight_starts_from_cached_takeoff_elevation_after_installing_goal_plane() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            crate::position_interface::SectorHandle::new(32),
        ));

        // The landing projection is ten units above the takeoff point. An
        // empty test grid rejects the horizontal push, which isolates the
        // vertical ReadyForTakeOff behavior: installing this goal plane must
        // not eagerly lift the actor before PerformFlight's first increment.
        let mut obstacle = crate::sight_obstacle::SightObstacle::new(
            0,
            crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
        );
        obstacle.layer = 0;
        obstacle.sector = 32;
        obstacle.obstacle_points = vec![
            crate::sight_obstacle::ObstaclePoint {
                x: -1000.0,
                y: -1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 1000.0,
                y: -1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 1000.0,
                y: 1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: -1000.0,
                y: 1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
        ];
        obstacle.top_plane_points = [
            [-1000.0, -1000.0, 10.0],
            [1000.0, -1000.0, 10.0],
            [-1000.0, 1000.0, 10.0],
        ];
        obstacle.rebuild_geometry();

        let mut assets = assets_with_nonstraight_profile(
            SwordStrike::H,
            crate::profiles::WeaponThrustKind::TrueCircle,
        );
        assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
        damage.orders.push_back(crate::order::Order::new(
            OrderType::FallingPushedWithSword,
            0.0,
            0.0,
            engine.orders.allocate_order_id(),
        ));
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .set_posture(Posture::Flying);

        engine.initialize_push_flight(
            &assets,
            victim,
            (sequence, 0),
            OrderType::FallingPushedWithSword,
        );
        let flight = engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_flight
            .expect("elevated landing plane must author a flight");
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .get_elevation()
                .to_bits(),
            0.0_f32.to_bits(),
            "SetObstacle must preserve ReadyForTakeOff's cached starting 3D point"
        );

        engine.tick_push_flights(&sim, &assets);
        let position = engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .get_position();
        assert_eq!(
            position.z.to_bits(),
            flight.increment_z.to_bits(),
            "the first PerformFlight tick advances from takeoff Z, not the landing plane"
        );
        assert_eq!(
            position.y.to_bits(),
            (100.0_f32 + flight.increment_y).to_bits(),
            "PerformFlight accumulates the authored world-space Y increment before re-projecting map Y"
        );
    }

    #[test]
    fn hit_flight_starts_from_cached_takeoff_elevation_after_installing_goal_plane() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            crate::position_interface::SectorHandle::new(32),
        ));

        let mut obstacle = crate::sight_obstacle::SightObstacle::new(
            0,
            crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
        );
        obstacle.layer = 0;
        obstacle.sector = 32;
        obstacle.obstacle_points = vec![
            crate::sight_obstacle::ObstaclePoint {
                x: -1000.0,
                y: -1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 1000.0,
                y: -1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 1000.0,
                y: 1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: -1000.0,
                y: 1000.0,
                z_top: 10.0,
                z_bottom: 0.0,
            },
        ];
        obstacle.top_plane_points = [
            [-1000.0, -1000.0, 10.0],
            [1000.0, -1000.0, 10.0],
            [-1000.0, 1000.0, 10.0],
        ];
        obstacle.rebuild_geometry();
        let mut assets = LevelAssets::new();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveHitDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        let mut fall = crate::order::Order::new(
            OrderType::FallingHitUpright,
            0.0,
            0.0,
            engine.orders.allocate_order_id(),
        );
        fall.antagonist = Some(attacker);
        damage.orders.push_back(fall);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(victim)
            .unwrap()
            .set_posture(Posture::Flying);

        engine.initialize_hit_flight(
            &assets,
            victim,
            Some(attacker),
            OrderType::FallingHitUpright,
        );
        let flight = engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_flight
            .expect("elevated landing plane must author a hit flight");
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .position_iface()
                .get_elevation()
                .to_bits(),
            0.0_f32.to_bits(),
            "FallingHit must retain ReadyForTakeOff's cached starting 3D point"
        );

        engine.tick_push_flights(&sim, &assets);
        let position = engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .get_position();
        assert_eq!(position.z.to_bits(), flight.increment_z.to_bits());
        assert_eq!(
            position.y.to_bits(),
            (100.0_f32 + flight.increment_y).to_bits(),
            "FallingHit accumulates the authored world-space Y increment"
        );
    }

    #[test]
    fn damage_to_already_dead_pc_does_not_repeat_virtual_kill() {
        let sim = crate::sim_rng::SimulationContext::with_seed(0x181);
        let mut engine = make_engine();
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let Entity::Pc(pc) = engine.get_entity_mut(victim).unwrap() else {
            unreachable!()
        };
        pc.pc.life_points = 0;
        pc.pc.trumpet_enabled = false;

        let seed_before = sim.seed();
        engine.handle_post_damage(
            &sim,
            &LevelAssets::new(),
            victim,
            0,
            None,
            false,
            (crate::sequence::SequenceId(999), 0),
            None,
        );

        assert_eq!(
            sim.seed(),
            seed_before,
            "SetLifePoints returns before the repeated Kill cascade can select a replacement peasant"
        );
        let Entity::Pc(pc) = engine.get_entity(victim).unwrap() else {
            unreachable!()
        };
        assert!(!pc.pc.trumpet_enabled);
    }

    #[test]
    fn lethal_sword_hit_preserves_queued_second_damage_fifo() {
        let sim = crate::sim_rng::SimulationContext::with_seed(0x38);
        let mut engine = make_engine();
        let attacker_a = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let attacker_b = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        for attacker in [attacker_a, attacker_b] {
            let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain
            else {
                unreachable!()
            };
            attacker_ai.hth_weapon_id = 1;
        }
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.pc_data_mut().unwrap().life_points = 1;
            victim_entity.actor_data_mut().unwrap().action_state =
                crate::element::ActionState::WaitingSword;
        }

        let queue_damage = |engine: &mut EngineInner, attacker| {
            let mut damage =
                crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
            damage.data =
                crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
            engine.resolve_element_priority(&mut damage);
            engine.orders.sequence_manager.launch_element(damage)
        };
        let first_damage = queue_damage(&mut engine, attacker_a);
        let second_damage = queue_damage(&mut engine, attacker_b);

        let mut unrelated =
            crate::sequence::SequenceElement::new(1, Command::WaitTimer, Some(victim));
        engine.resolve_element_priority(&mut unrelated);
        let unrelated = engine.orders.sequence_manager.launch_element(unrelated);

        let assets = assets_with_sword_profile(200, 30);
        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            engine.hourglass_phase_sequences(
                &sim,
                &mut crate::engine::HostDisplayState::default(),
                &assets,
            );
        });

        assert_eq!(
            draws,
            vec![
                crate::sim_rng::RngSite::SwordDamageProtection,
                crate::sim_rng::RngSite::SwordDamageProtection,
                crate::sim_rng::RngSite::MeleeProvoke,
                crate::sim_rng::RngSite::SwordDamageProtection,
                crate::sim_rng::RngSite::SwordDamageProtection,
                crate::sim_rng::RngSite::MeleeProvoke,
            ],
            "both simultaneous sword hits must execute their exact damage RNG sites"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(second_damage, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::InProgress,
            "the already-dead second hit must translate into its own live dying order"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(unrelated, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Interrupted,
            "death cleanup must still discard unrelated queued owner work"
        );
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .pc_data()
                .unwrap()
                .life_points,
            0
        );
        assert_ne!(
            engine
                .orders
                .sequence_manager
                .get_element(first_damage, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Todo
        );
        assert_eq!(engine.actor_command(victim), Command::ReceiveSwordDamage);
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .actor_data()
                .unwrap()
                .installed_order
                .map(|order| order.order_type),
            Some(crate::order::OrderType::DyingSword),
            "the second damage card replaces the first while retaining Original's dying-sword lifecycle"
        );
    }

    #[test]
    fn sword_damage_on_dying_pc_preserves_the_fresh_sprite_start() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
            unreachable!()
        };
        attacker_ai.hth_weapon_id = 1;
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.pc_data_mut().unwrap().life_points = 0;
            victim_entity.set_posture(Posture::Dead);
            let actor = victim_entity.actor_data_mut().unwrap();
            actor.action_state = crate::element::ActionState::WaitingSword;
            actor.continuation.motion_state = crate::sprite::MotionState::Start;
        }

        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        engine.resolve_element_priority(&mut damage);
        let damage_sequence = engine.orders.sequence_manager.launch_element(damage);

        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets_with_sword_profile(200, 30));

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(damage_sequence, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Terminated
        );
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::Start,
            "TranslateSwordDamage changes the selected pointer before Actor::Instruct can stamp InProgress"
        );
    }

    #[test]
    fn lethal_sword_damage_to_grounded_non_rider_publishes_dead_before_terminating() {
        for initial_posture in [
            Posture::Lying,
            Posture::StuckUnderNet,
            Posture::Flying,
            Posture::Carried,
            Posture::OnShoulders,
            Posture::Tied,
        ] {
            let sim = crate::sim_rng::test_context();
            let mut engine = make_engine();
            let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
            let victim = engine.add_entity(make_soldier(
                WorldPoint3D {
                    x: 10.0,
                    ..WorldPoint3D::ZERO
                },
                None,
            ));
            {
                let victim_entity = engine.get_entity_mut(victim).unwrap();
                victim_entity.element_data_mut().posture = initial_posture;
                victim_entity.npc_data_mut().unwrap().life_points = 1;
                victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
            }
            let assets = assets_with_sword_profile_effects(200, 50, 100, 0);
            let mut damage =
                crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
            damage.data =
                crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
            let sequence = engine.orders.sequence_manager.launch_element(damage);
            engine
                .orders
                .sequence_manager
                .element_in_progress(sequence, 0);

            engine.apply_sword_damage(
                &sim,
                &assets,
                victim,
                Some(attacker),
                Some(SwordStrike::A),
                Some(1),
                (sequence, 0),
            );

            let element = engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("grounded sword damage remains registered");
            assert_eq!(
                engine.get_entity(victim).unwrap().element_data().posture,
                Posture::Dead,
                "TranslateSwordDamage must publish Dead for lethal {initial_posture:?} non-riders"
            );
            assert_eq!(element.state, crate::sequence::SequenceState::Terminated);
            assert!(
                element.orders.is_empty(),
                "grounded lethal {initial_posture:?} must not author a replacement animation"
            );
        }
    }

    #[test]
    fn grounded_sword_damage_preserves_living_and_dead_rider_posture_controls() {
        for (life_points, rider, expected_state) in [
            (50, false, crate::sequence::SequenceState::Terminated),
            (1, true, crate::sequence::SequenceState::InProgress),
        ] {
            let sim = crate::sim_rng::test_context();
            let mut engine = make_engine();
            let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
            let victim = engine.add_entity(make_soldier(
                WorldPoint3D {
                    x: 10.0,
                    ..WorldPoint3D::ZERO
                },
                None,
            ));
            {
                let Entity::Soldier(victim_entity) = engine.get_entity_mut(victim).unwrap() else {
                    unreachable!()
                };
                victim_entity.element.posture = Posture::Lying;
                victim_entity.npc.life_points = life_points;
                victim_entity.soldier.rider = rider;
                victim_entity
                    .npc
                    .ai_brain
                    .enemy_mut()
                    .unwrap()
                    .hth_weapon_id = 1;
            }
            let assets = if rider {
                assets_with_sword_profile_effects(200, 50, 100, 0)
            } else {
                assets_with_sword_profile_effects(1, 50, 1, 0)
            };
            let mut damage =
                crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
            damage.data =
                crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
            let sequence = engine.orders.sequence_manager.launch_element(damage);
            engine
                .orders
                .sequence_manager
                .element_in_progress(sequence, 0);

            engine.apply_sword_damage(
                &sim,
                &assets,
                victim,
                Some(attacker),
                Some(SwordStrike::A),
                Some(1),
                (sequence, 0),
            );

            assert_eq!(
                engine.get_entity(victim).unwrap().element_data().posture,
                Posture::Lying,
                "living grounded actors and lethal riders bypass the Dead rewrite"
            );
            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, 0)
                    .unwrap()
                    .state,
                expected_state,
                "dead riders fall through while living grounded non-riders terminate"
            );
        }
    }

    #[test]
    fn sword_damage_amulet_coma_terminates_during_translation() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
            unreachable!()
        };
        attacker_ai.hth_weapon_id = 1;
        let sprite_script = crate::sprite_script::SpriteScript {
            action_id: crate::order::OrderType::WaitingUpright as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![sprite_script]),
            std::sync::Arc::new(vec![0]),
        );
        let mut assets = assets_with_sword_profile(200, 30);
        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].vip = true;
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets] = 1;

        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.pc_data_mut().unwrap().life_points = 1;
            victim_entity
                .position_iface_mut()
                .set_map_goal(crate::coordinates::MapPoint::new(25.0, 100.0));
            victim_entity
                .actor_data_mut()
                .unwrap()
                .continuation
                .motion_state = crate::sprite::MotionState::Start;
        }

        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        engine.resolve_element_priority(&mut damage);
        engine.orders.sequence_manager.launch_element(damage);

        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
        assert_eq!(victim_entity.element_data().posture, Posture::Lying);
        assert_eq!(
            victim_entity.position_iface().map_goal(),
            crate::coordinates::MapPoint::ZERO,
            "translation-time termination must clear the interrupted movement goal"
        );
        assert_eq!(
            victim_entity
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::Start,
            "Actor::Instruct must preserve the motion produced before damage translation"
        );
    }

    #[test]
    fn consecutive_lethal_arrow_damage_preserves_new_amulet_coma() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let sprite_script = crate::sprite_script::SpriteScript {
            action_id: crate::order::OrderType::WaitingUpright as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![sprite_script]),
            std::sync::Arc::new(vec![0]),
        );
        let mut assets = assets_with_sword_profile(200, 30);
        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].vip = true;
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets] = 1;

        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.pc_data_mut().unwrap().life_points = 10;
            victim_entity
                .position_iface_mut()
                .set_map_goal(crate::coordinates::MapPoint::new(25.0, 100.0));
            victim_entity
                .actor_data_mut()
                .unwrap()
                .continuation
                .motion_state = crate::sprite::MotionState::Start;
        }

        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            10,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        engine.orders.sequence_manager.launch_element(damage);

        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        {
            let victim_entity = engine.get_entity(victim).unwrap();
            assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
            assert_eq!(victim_entity.pc_data().unwrap().life_points, 5);
            assert_eq!(victim_entity.element_data().posture, Posture::Lying);
            assert_eq!(
                victim_entity.position_iface().map_goal(),
                crate::coordinates::MapPoint::ZERO,
                "post-damage Lying translation must terminate and clear the movement goal"
            );
            assert_eq!(
                victim_entity
                    .actor_data()
                    .unwrap()
                    .continuation
                    .motion_state,
                crate::sprite::MotionState::Start,
                "terminal arrow translation must preserve the pre-damage motion state"
            );
        }
        assert_eq!(
            engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
            0,
            "the first lethal arrow must establish coma and consume one amulet"
        );

        let mut second_damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            10,
            0,
        );
        engine.resolve_element_priority(&mut second_damage);
        engine.orders.sequence_manager.launch_element(second_damage);
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let victim_entity = engine.get_entity(victim).unwrap();
        assert_eq!(victim_entity.pc_data().unwrap().life_points, 5);
        assert!(!victim_entity.is_dead());
        assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
        assert_eq!(
            engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
            0,
            "the second lethal arrow must not consume another amulet"
        );
    }

    #[test]
    fn same_frame_arrow_after_death_replaces_dying_order_and_then_rolls() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        engine
            .get_entity_mut(victim)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .life_points = 1;

        let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
        obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
        let mut assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
        {
            let victim = engine.get_entity_mut(victim).unwrap();
            victim.element_data_mut().set_obstacle_index(
                crate::position_interface::ObstacleHandle::new(0),
                Some(crate::position_interface::PlaneZCoeffs {
                    az: 1.0,
                    bz: 0.0,
                    dz: 0.0,
                }),
            );
            victim
                .position_iface_mut()
                .set_move_box(crate::coordinates::MoveBox::from_corners(
                    crate::coordinates::MapVec::new(-5.0, -5.0),
                    crate::coordinates::MapVec::new(5.0, 5.0),
                ));
        }

        let mut launched = Vec::new();
        for _ in 0..2 {
            let mut damage = crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveArrowDamage,
                Some(victim),
                Some(attacker),
                1,
                0,
            );
            engine.resolve_element_priority(&mut damage);
            launched.push(engine.orders.sequence_manager.launch_element(damage));
        }

        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(victim),
            Some((launched[1], 0)),
            "the second injury must replace the first dying element"
        );
        let second = engine
            .orders
            .sequence_manager
            .get_element(launched[1], 0)
            .expect("second arrow damage remains registered");
        assert_eq!(second.state, crate::sequence::SequenceState::InProgress);
        assert_eq!(
            second
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![OrderType::DyingUpright, OrderType::Rolling],
            "TranslateArrowDamage must author DyingUpright before TranslateRoll"
        );
        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .actor_data()
                .unwrap()
                .installed_order
                .as_ref()
                .map(|order| order.order_type),
            Some(OrderType::DyingUpright)
        );
    }

    #[test]
    fn arrow_damage_to_dead_grounded_actor_sets_dead_and_terminates_without_orders() {
        for (initial_posture, use_pc) in [
            (Posture::Lying, true),
            (Posture::StuckUnderNet, true),
            (Posture::Flying, true),
            (Posture::Carried, true),
            // PC virtual dispatch intercepts OnShoulders.  A Soldier reaches
            // RHElementActorHuman's literal OnShoulders fallthrough.
            (Posture::OnShoulders, false),
            (Posture::Tied, true),
        ] {
            let sim = crate::sim_rng::test_context();
            let mut engine = make_engine();
            let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
            let victim = engine.add_entity(if use_pc {
                make_pc(WorldPoint3D::ZERO, None)
            } else {
                make_soldier(WorldPoint3D::ZERO, None)
            });
            {
                let victim = engine.get_entity_mut(victim).unwrap();
                let (_, life_points) = victim
                    .human_and_life_points_mut()
                    .expect("grounded test victim must be human");
                *life_points = 0;
                victim.element_data_mut().posture = initial_posture;
            }
            let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
            let mut damage = crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveArrowDamage,
                Some(victim),
                Some(attacker),
                1,
                0,
            );
            engine.resolve_element_priority(&mut damage);
            let sequence = engine.orders.sequence_manager.launch_element(damage);

            let mut display = crate::engine::HostDisplayState::default();
            engine.hourglass_phase_sequences(&sim, &mut display, &assets);

            let element = engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("dead-body arrow damage remains registered");
            assert_eq!(
                element.state,
                crate::sequence::SequenceState::Terminated,
                "{initial_posture:?} must enter the terminating fallthrough"
            );
            assert!(element.orders.is_empty());
            assert_eq!(
                engine.get_entity(victim).unwrap().element_data().posture,
                Posture::Dead,
                "TranslateArrowDamage changes dead {initial_posture:?} non-riders to Dead"
            );
        }
    }

    #[test]
    fn arrow_damage_to_pc_on_shoulders_uses_virtual_shoulder_translation() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let carrier = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        engine
            .get_entity_mut(carrier)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .carried = Some(victim);
        {
            let victim = engine.get_entity_mut(victim).unwrap();
            victim.element_data_mut().posture = Posture::OnShoulders;
            victim.human_data_mut().unwrap().carrier = Some(carrier);
        }

        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("shoulder arrow damage remains registered");
        assert_ne!(element.state, crate::sequence::SequenceState::Terminated);
        assert_eq!(
            element.orders.front().map(|order| order.order_type),
            Some(OrderType::FallingShoulders),
            "PC virtual TranslateArrowDamage must dispatch TranslateShoulderDamage"
        );
        assert_ne!(
            engine.get_entity(victim).unwrap().element_data().posture,
            Posture::Dead,
            "PC OnShoulders must not enter Human's dead-grounded fallthrough"
        );
    }

    /// `SwordstrikeThrustA` promotes both principal opponents before
    /// the strike, so clicking a secondary opponent during a
    /// swordfight switches the primary target.
    #[test]
    fn thrust_a_promotes_clicked_secondary_opponent() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let pc = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let current = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let clicked = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 20.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        if let Some(human) = engine.get_entity_mut(pc).unwrap().human_data_mut() {
            human.opponents = vec![current, clicked];
            human.opponent_jump_lines = vec![None, None];
        }
        if let Some(human) = engine.get_entity_mut(clicked).unwrap().human_data_mut() {
            human.opponents = vec![current, pc];
            human.opponent_jump_lines = vec![None, None];
        }
        engine
            .get_entity_mut(pc)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(8);
        let direction_before_dispatch = engine.get_entity(pc).unwrap().element_data().direction();
        let direction_goal_before_dispatch = engine
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal();

        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustA,
            Some(pc),
            Some(clicked),
        ));
        let seq_id = engine.launch_sequence(sequence);
        let action_state_before_dispatch = engine
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state;

        engine.dispatch_sword_strike(
            sim,
            &LevelAssets::default(),
            pc,
            clicked,
            SwordStrike::A,
            seq_id,
            0,
        );
        assert_eq!(
            engine
                .get_entity(pc)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            action_state_before_dispatch,
            "Instruct must not apply the Execute MotionState::Start WaitingSword transition"
        );
        assert_eq!(
            engine.get_entity(pc).unwrap().element_data().direction(),
            direction_before_dispatch,
            "strike translation must leave facing to the following Execute call"
        );
        assert_eq!(
            engine
                .get_entity(pc)
                .unwrap()
                .position_iface()
                .get_direction_goal(),
            direction_goal_before_dispatch,
            "strike translation must not install the Execute-time facing goal"
        );

        assert_eq!(
            engine
                .get_entity(pc)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![clicked, current],
            "thrust-A against an existing secondary opponent must make it principal"
        );
        assert_eq!(
            engine
                .get_entity(clicked)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![pc, current],
            "the attacker is also promoted as the target's principal opponent"
        );
    }

    #[test]
    fn melee_direction_uses_original_aspect_ratio_classifier() {
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 663.552_37,
                y: 1755.932_5,
                z: 0.0,
            },
            None,
        ));
        let target = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 726.867_3,
                y: 1763.275_3,
                z: 0.0,
            },
            None,
        ));

        assert_eq!(direction_to(&engine.world.entities, attacker, target), 5);
    }

    #[test]
    fn enter_swordfight_instruct_queues_transition_without_execute_side_effects() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let owner = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .set_direction_goal(7);

        let mut element =
            crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Element(opponent),
        );
        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(element);
        let seq_id = engine.launch_sequence(sequence);

        engine.dispatch_enter_swordfight(
            sim,
            &LevelAssets::default(),
            owner,
            Some(opponent),
            seq_id,
            0,
        );

        let owner_entity = engine.get_entity(owner).unwrap();
        assert_eq!(
            owner_entity.actor_data().unwrap().action_state,
            ActionState::Waiting,
            "Instruct must not apply the raising-sword Execute state"
        );
        assert_eq!(
            i16::from(owner_entity.position_iface().get_direction_goal()),
            7,
            "Instruct must not apply the raising-sword Execute facing"
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
        let order = element.current_order().unwrap();
        assert_eq!(
            order.order_type,
            crate::order::OrderType::TransitionRaisingSword
        );
        assert_eq!(order.antagonist, Some(opponent));
        assert!(
            owner_entity
                .human_data()
                .unwrap()
                .opponents
                .contains(&opponent),
            "relationship changes still belong to Instruct"
        );
    }

    #[test]
    fn enter_swordfight_instruct_preserves_live_sprite_destination() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let owner = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let retained_goal = crate::coordinates::MapPoint::new(768.0, 1796.0);
        engine
            .get_entity_mut(owner)
            .unwrap()
            .position_iface_mut()
            .set_map_goal(retained_goal);

        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new_generic(
            1,
            Command::EnterSwordfight,
            Some(owner),
        ));
        let seq_id = engine.launch_sequence(sequence);
        engine.dispatch_enter_swordfight(&sim, &LevelAssets::default(), owner, None, seq_id, 0);

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .position_iface()
                .map_goal(),
            retained_goal,
            "translation must not apply TransitionRaisingSword's zero destination before Execute"
        );
    }

    #[test]
    fn satisfied_enter_swordfight_skips_outer_instruct_epilogue() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let owner = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        if let Some(actor) = engine.get_entity_mut(owner).unwrap().actor_data_mut() {
            actor.action_state = ActionState::WaitingSword;
        }
        if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
            human.opponents = vec![opponent];
            human.opponent_jump_lines = vec![None];
        }
        if let Some(human) = engine.get_entity_mut(opponent).unwrap().human_data_mut() {
            human.opponents = vec![owner];
            human.opponent_jump_lines = vec![None];
        }

        let mut element =
            crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
        element.set_property(
            crate::sequence::Field::Opponent,
            crate::sequence::FieldValue::Element(opponent),
        );
        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(element);
        let seq_id = engine.launch_sequence(sequence);

        let barrier = engine.dispatch_enter_swordfight(
            &sim,
            &LevelAssets::default(),
            owner,
            Some(opponent),
            seq_id,
            0,
        );

        assert_eq!(
            barrier,
            crate::engine::sequence_runtime::OwnerActionBarrier::Skip,
            "terminal Translate changes the selected element before Actor::Instruct's epilogue"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Terminated
        );
    }

    #[test]
    fn reconsider_rebalance_updates_opponents_without_recursive_enter_command() {
        use crate::ai::EnterSwordfightRequest;

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let owner = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let old_primary = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let replacement = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 20.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
            human.opponents = vec![old_primary, replacement];
            human.opponent_jump_lines = vec![None, None];
        }
        if let Some(human) = engine.get_entity_mut(replacement).unwrap().human_data_mut() {
            human.opponents = vec![owner];
            human.opponent_jump_lines = vec![None];
        }
        let replacement_handle = (0..3)
            .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(replacement))
            .expect("replacement PC must occupy a legacy entity slot");
        let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .unwrap()
            .base
            .outbox
            .actor
            .enter_swordfight = Some(EnterSwordfightRequest::Rebalance(replacement_handle));

        engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents
                .first(),
            Some(&replacement),
            "direct EnterSwordFight must promote the replacement opponent"
        );
        let Entity::Soldier(soldier) = engine.get_entity(owner).unwrap() else {
            unreachable!()
        };
        assert_eq!(
            soldier.npc.ai_brain.enemy().unwrap().base.primary_target,
            replacement_handle,
            "successful rebalance must promote the AI primary target"
        );
        assert!(
            !engine
                .orders
                .sequence_manager
                .element_is_about_to_be_launched(owner, Command::EnterSwordfight),
            "ReconsiderSwordfight's direct call must not author another Enter command"
        );
    }

    #[test]
    fn reconsider_rebalance_rejection_preserves_opponent_and_ai_primary_target() {
        use crate::ai::EnterSwordfightRequest;

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let owner = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let old_primary = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let replacement = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 20.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
            human.opponents = vec![old_primary];
            human.opponent_jump_lines = vec![None];
        }
        if let Some(human) = engine.get_entity_mut(replacement).unwrap().human_data_mut() {
            human.unconscious = true;
        }
        let old_primary_handle = (0..3)
            .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(old_primary))
            .expect("old primary PC must occupy a legacy entity slot");
        let replacement_handle = (0..3)
            .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(replacement))
            .expect("replacement PC must occupy a legacy entity slot");
        let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.primary_target = old_primary_handle;
        ai.base.outbox.actor.enter_swordfight =
            Some(EnterSwordfightRequest::Rebalance(replacement_handle));

        engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());

        let Entity::Soldier(soldier) = engine.get_entity(owner).unwrap() else {
            unreachable!()
        };
        assert_eq!(soldier.human.opponents, vec![old_primary]);
        assert_eq!(
            soldier.npc.ai_brain.enemy().unwrap().base.primary_target,
            old_primary_handle,
            "failed EnterSwordFight must preserve the old AI primary target"
        );
    }

    #[test]
    fn got_hit_direct_entry_authors_reciprocal_enter_on_attacker() {
        use crate::ai::EnterSwordfightRequest;

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let existing_opponent = engine.add_entity(make_pc(
            WorldPoint3D {
                x: -10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let attacker = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let Entity::Soldier(attacker_soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        attacker_soldier.soldier.cached_camp = crate::element::Camp::Royalists;

        if let Some(human) = engine.get_entity_mut(victim).unwrap().human_data_mut() {
            human.opponents = vec![existing_opponent];
            human.opponent_jump_lines = vec![None];
        }
        if let Some(human) = engine
            .get_entity_mut(existing_opponent)
            .unwrap()
            .human_data_mut()
        {
            human.opponents = vec![victim];
            human.opponent_jump_lines = vec![None];
        }

        let mut strike_element =
            crate::sequence::SequenceElement::new(1, Command::SwordstrikeThrustA, Some(attacker));
        strike_element.priority = crate::sequence::SequencePriority::Preference;
        let mut strike = crate::sequence::Sequence::new();
        strike.append_element(strike_element);
        let strike_id = engine.launch_sequence(strike);
        let strike_order_id = engine.orders.allocate_order_id();
        let mut strike_order = crate::order::Order::new(
            crate::order::OrderType::StrikingStraightSword,
            0.0,
            0.0,
            strike_order_id,
        );
        strike_order.antagonist = Some(victim);
        engine
            .orders
            .sequence_manager
            .push_order_on(strike_id, 0, strike_order);
        engine
            .orders
            .sequence_manager
            .element_in_progress(strike_id, 0);

        let attacker_handle = (0..3)
            .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(attacker))
            .expect("attacker must occupy a legacy entity slot");
        let Entity::Soldier(soldier) = engine.get_entity_mut(victim).unwrap() else {
            unreachable!()
        };
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .unwrap()
            .base
            .outbox
            .actor
            .enter_swordfight = Some(EnterSwordfightRequest::Direct(attacker_handle));

        engine.drain_pending_for_npc(&sim, victim, &LevelAssets::default());

        assert_eq!(
            engine
                .get_entity(victim)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![attacker, existing_opponent],
            "Original AddOpponent installs the new attacker as principal"
        );
        assert_eq!(
            engine
                .get_entity(attacker)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![victim],
            "direct entry synchronously installs the reciprocal relationship"
        );
        let (enter_sequence, enter_index) = engine
            .orders
            .sequence_manager
            .pending_elements_for_owner(attacker)
            .into_iter()
            .find(|(sequence, index)| {
                engine
                    .orders
                    .sequence_manager
                    .get_element(*sequence, *index)
                    .is_some_and(|element| element.command == Command::EnterSwordfight)
            })
            .expect("the reciprocal ENTER_SWORDFIGHT must be attacker-owned");
        let enter = engine
            .orders
            .sequence_manager
            .get_element(enter_sequence, enter_index)
            .unwrap();
        assert_eq!(enter.owner, Some(attacker));
        assert!(matches!(
            enter.get_property(crate::sequence::Field::Opponent),
            Some(crate::sequence::FieldValue::Element(opponent)) if *opponent == victim
        ));
        assert!(
            !engine
                .orders
                .sequence_manager
                .element_is_about_to_be_launched(victim, Command::EnterSwordfight),
            "EVENT_GOTHIT must not defer a self-owned Engage command"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(strike_id, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::InProgress,
            "the direct call bypasses PrepareToEnterSwordFight; interruption belongs to the reciprocal command scheduler"
        );

        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &LevelAssets::default());
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(enter_sequence, enter_index)
                .unwrap()
                .state,
            crate::sequence::SequenceState::InProgress,
            "the reciprocal high-priority ENTER becomes attacker-current"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(strike_id, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::Postponed,
            "the reciprocal ENTER displaces the attacker's Preference strike"
        );
    }

    #[test]
    fn direct_enter_swordfight_accepts_typed_slot_zero_opponent() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let opponent = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let initiator = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        assert_eq!(opponent.index(), 0, "control requires typed slot zero");

        assert!(
            engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,)
        );
        assert_eq!(
            engine
                .get_entity(initiator)
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
            vec![initiator]
        );
    }

    #[test]
    fn reconsider_direct_entry_does_not_prepare_or_stop_opponent() {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let initiator = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        let mut selected = crate::sequence::Sequence::new();
        selected.append_element(crate::sequence::SequenceElement::new(
            1,
            Command::Point,
            Some(opponent),
        ));
        let selected_id = engine.launch_sequence(selected);
        engine
            .orders
            .sequence_manager
            .element_in_progress(selected_id, 0);
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::InProgress;

        assert!(
            engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,)
        );

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(selected_id, 0)
                .unwrap()
                .state,
            crate::sequence::SequenceState::InProgress,
            "direct EnterSwordFight must not run PrepareToEnterSwordFight's Stop"
        );
        assert_eq!(
            engine
                .get_entity(opponent)
                .unwrap()
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::InProgress
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .element_is_about_to_be_launched(opponent, Command::EnterSwordfight),
            "direct entry still queues the reciprocal Enter command"
        );
    }

    #[test]
    fn selected_pc_entering_swordfight_does_not_restore_armed_action_on_quit() {
        use crate::profiles::Action;

        let sim = crate::sim_rng::test_context();
        let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
        let mut engine = make_engine();
        let pc = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        engine.players.seats[0].selection.push(pc);
        {
            let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
            pc_data.current_action = Action::Purse;
            pc_data.disabled_actions = vec![false; 3];
            pc_data.disabled_actions_temp = vec![false; 3];
        }

        assert!(engine.enter_swordfight(&sim, &assets, pc, opponent, false,));
        {
            let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
            assert_eq!(pc_data.current_action, Action::NoAction);
            assert_eq!(pc_data.saved_action, Action::NoAction);
            assert_eq!(pc_data.disabled_actions_temp, vec![true; 3]);
        }

        engine.quit_swordfight(&sim, &assets, pc);
        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    }

    #[test]
    fn unselected_pc_entering_swordfight_saves_targeted_no_action() {
        use crate::profiles::Action;

        let sim = crate::sim_rng::test_context();
        let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
        let mut engine = make_engine();
        let pc = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        {
            let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
            pc_data.current_action = Action::Bow;
            pc_data.disabled_actions = vec![false; 3];
            pc_data.disabled_actions_temp = vec![false; 3];
        }

        assert!(engine.enter_swordfight(&sim, &assets, pc, opponent, false,));
        {
            let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
            assert_eq!(pc_data.current_action, Action::NoAction);
            assert_eq!(pc_data.saved_action, Action::NoAction);
            assert_eq!(pc_data.disabled_actions_temp, vec![true; 3]);
        }

        engine.quit_swordfight(&sim, &assets, pc);
        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
        assert!(
            !engine
                .orders
                .sequence_manager
                .element_is_about_to_be_launched(pc, Command::EquipBow),
            "quitting must not restore the action that was armed before entry"
        );
    }

    #[test]
    fn quit_swordfight_resets_moving_survivor_smalltalk_initiative() {
        use crate::profiles::Action;

        let sim = crate::sim_rng::test_context();
        let assets = action_test_assets([Action::NoAction; 3]);
        let mut engine = make_engine();
        let quitter = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let survivor = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let principal = engine.add_entity(make_pc(WorldPoint3D::default(), None));

        {
            let human = engine
                .get_entity_mut(quitter)
                .unwrap()
                .human_data_mut()
                .unwrap();
            human.opponents = vec![survivor];
            human.opponent_jump_lines = vec![None];
        }
        {
            let survivor_entity = engine.get_entity_mut(survivor).unwrap();
            survivor_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
            let human = survivor_entity.human_data_mut().unwrap();
            human.opponents = vec![quitter, principal];
            human.opponent_jump_lines = vec![None, None];
            human.smalltalk_initiative = false;
            human.received_smalltalk_initiative = false;
        }
        {
            let human = engine
                .get_entity_mut(principal)
                .unwrap()
                .human_data_mut()
                .unwrap();
            human.opponents = vec![survivor];
            human.opponent_jump_lines = vec![None];
            human.smalltalk_initiative = true;
        }

        engine.quit_swordfight(&sim, &assets, quitter);

        let survivor_human = engine.get_entity(survivor).unwrap().human_data().unwrap();
        assert_eq!(survivor_human.opponents, vec![principal]);
        assert!(survivor_human.smalltalk_initiative);
        assert!(survivor_human.received_smalltalk_initiative);
        assert!(
            !engine
                .get_entity(principal)
                .unwrap()
                .human_data()
                .unwrap()
                .smalltalk_initiative,
            "mutual principal must lose initiative even while the survivor is Moving"
        );
    }

    #[test]
    fn quit_swordfight_does_not_reset_initiative_without_surviving_opponents() {
        use crate::profiles::Action;

        let sim = crate::sim_rng::test_context();
        let assets = action_test_assets([Action::NoAction; 3]);
        let mut engine = make_engine();
        let quitter = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let survivor = engine.add_entity(make_pc(WorldPoint3D::default(), None));

        {
            let human = engine
                .get_entity_mut(quitter)
                .unwrap()
                .human_data_mut()
                .unwrap();
            human.opponents = vec![survivor];
            human.opponent_jump_lines = vec![None];
        }
        {
            let survivor_entity = engine.get_entity_mut(survivor).unwrap();
            survivor_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
            let human = survivor_entity.human_data_mut().unwrap();
            human.opponents = vec![quitter];
            human.opponent_jump_lines = vec![None];
            human.smalltalk_initiative = false;
            human.received_smalltalk_initiative = false;
        }

        engine.quit_swordfight(&sim, &assets, quitter);

        let survivor_human = engine.get_entity(survivor).unwrap().human_data().unwrap();
        assert!(survivor_human.opponents.is_empty());
        assert!(!survivor_human.smalltalk_initiative);
        assert!(!survivor_human.received_smalltalk_initiative);
    }

    #[test]
    fn enabling_temp_actions_restores_matching_slot_after_targeted_selection_collapse() {
        use crate::profiles::Action;

        let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
        let mut engine = make_engine();
        let pc = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        let companion = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        engine.players.seats[0].selection = vec![pc, companion];
        {
            let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
            pc_data.current_action = Action::NoAction;
            pc_data.saved_action = Action::Purse;
            pc_data.disabled_actions = vec![false; 3];
            pc_data.disabled_actions_temp = vec![true; 3];
        }
        engine
            .get_entity_mut(companion)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .current_action = Action::Bow;

        engine.enable_pc_actions_temp(&assets, 0, pc);

        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::Purse);
        assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
        assert_eq!(
            engine
                .get_entity(companion)
                .unwrap()
                .pc_data()
                .unwrap()
                .current_action,
            Action::Bow,
            "RHMessenger removes the companion before fanning out the targeted restored action"
        );
        assert_eq!(engine.players.seats[0].selection, vec![pc]);
        assert_eq!(engine.players.seats[0].selected_action, Action::Purse);
        assert!(
            engine
                .feedback
                .pending_side_effects
                .invalidate_trajectory_preview
        );
    }

    #[test]
    fn enabling_temp_actions_does_not_restore_action_absent_from_profile_slots() {
        use crate::profiles::Action;

        let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
        let mut engine = make_engine();
        let pc = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        engine.players.seats[0].selection.push(pc);
        {
            let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
            pc_data.current_action = Action::NoAction;
            pc_data.saved_action = Action::Stone;
            pc_data.disabled_actions = vec![false; 3];
            pc_data.disabled_actions_temp = vec![true; 3];
        }

        engine.enable_pc_actions_temp(&assets, 0, pc);

        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
        assert_eq!(engine.players.seats[0].selected_action, Action::NoAction);
        assert!(
            !engine
                .feedback
                .pending_side_effects
                .invalidate_trajectory_preview
        );
    }

    #[test]
    fn preparing_swordfight_orders_done_enter_then_queues_reciprocal() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};
        use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let initiator = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(initiator).unwrap() else {
                unreachable!()
            };
            soldier.soldier.cached_camp = crate::element::Camp::Royalists;
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = initiator.index();
            ai.hth_weapon_id = 1;
        }
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(opponent).unwrap() else {
                unreachable!()
            };
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = opponent.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingOfficerGivingOrders;
            ai.hth_weapon_id = 1;
        }

        // Give the opponent a selected command for PrepareToEnterSwordFight's
        // Stop(PREFERENCE) to interrupt. Its condolence sends EventDone.
        let mut selected = crate::sequence::Sequence::new();
        selected.append_element(crate::sequence::SequenceElement::new(
            1,
            Command::Point,
            Some(opponent),
        ));
        let selected_id = engine.launch_sequence(selected);
        engine
            .orders
            .sequence_manager
            .element_in_progress(selected_id, 0);

        let mut profiles = ProfileManager::new();
        profiles.hth_weapons.push(HtHWeaponProfile {
            distance: [30, 50, 60, 70],
            ..HtHWeaponProfile::default()
        });
        profiles.characters.push(CharacterProfile {
            hth_weapon_id: 1,
            ..CharacterProfile::default()
        });
        profiles.soldiers.push(SoldierProfile {
            hth_weapon_id: 1,
            hostile: true,
            ..SoldierProfile::default()
        });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        };

        assert!(engine.enter_swordfight(sim, &assets, initiator, opponent, false));

        assert!(
            engine
                .orders
                .sequence_manager
                .element_is_about_to_be_launched_or_postponed_by_current(
                    opponent,
                    Command::EnterSwordfight,
                ),
            "non-Wait reciprocal enter remains on the manager FIFO after EnterSwordFight returns"
        );
        let ai = engine
            .get_entity(opponent)
            .unwrap()
            .ai_controller()
            .unwrap();
        let events: Vec<_> = ai
            .ai_log
            .iter()
            .filter(|entry| entry.line_type == LogLineType::Event)
            .map(|entry| entry.info)
            .collect();
        assert_eq!(
            events,
            vec![
                StimulusType::EventDone as u16,
                StimulusType::EventEnterSwordfight as u16
            ],
            "the interrupted command must complete in the old substate before swordfight entry"
        );
        assert_eq!(ai.current_substate, Substate::AttackingSwordfight);
    }

    #[test]
    fn deleting_final_opponent_synchronously_quits_soldier_ai() {
        use crate::ai::{AiState, LogLineType, StimulusType, Substate};
        use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let soldier = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let opponent = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        if let Some(human) = engine.get_entity_mut(soldier).unwrap().human_data_mut() {
            human.opponents = vec![opponent];
            human.opponent_jump_lines = vec![None];
        }
        let Entity::Soldier(soldier_entity) = engine.get_entity_mut(soldier).unwrap() else {
            unreachable!()
        };
        let ai = soldier_entity.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = soldier.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
        let Entity::Soldier(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
            unreachable!()
        };
        let opponent_ai = opponent_entity.npc.ai_brain.enemy_mut().unwrap();
        opponent_ai.base.me = opponent.index();
        opponent_ai.hth_weapon_id = 1;

        let mut profiles = ProfileManager::new();
        profiles.hth_weapons.push(HtHWeaponProfile::default());
        profiles.characters.push(CharacterProfile {
            hth_weapon_id: 1,
            ..CharacterProfile::default()
        });
        profiles.soldiers.push(SoldierProfile {
            hth_weapon_id: 1,
            hostile: true,
            ..SoldierProfile::default()
        });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        };

        assert!(engine.delete_opponent(&sim, &assets, soldier, opponent));

        let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
        assert_eq!(
            ai.current_substate,
            Substate::AttackingQuittingSwordfight,
            "DeleteOpponent must synchronously deliver the final-opponent quit event"
        );
        assert!(ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventQuitSwordfight as u16
        }));
    }

    /// Bud-Spencer-style line of three: PC punches the first soldier,
    /// who is launched along +X into a second soldier directly in
    /// front, and a third soldier behind the second. The flight tick
    /// should fire a domino RECEIVE_HIT_DAMAGE on both downstream
    /// soldiers, citing the PC as origin.
    #[test]
    fn domino_propagates_to_actors_in_flight_path() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let hitter = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let flyer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let mid = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 16.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let far = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 22.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        // 5 frames of +X motion at 1 unit per frame — short enough to
        // stay inside DOMINO_DISTANCE for the front pair.
        give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);

        engine.tick_push_flights(sim, &LevelAssets::default());

        assert_eq!(
            count_domino_hits_for(&engine, mid, hitter),
            1,
            "soldier directly in front should take a domino hit"
        );
        assert_eq!(
            count_domino_hits_for(&engine, far, hitter),
            1,
            "soldier further along the flight axis should also take a domino hit"
        );
        assert_eq!(
            count_domino_hits_for(&engine, hitter, hitter),
            0,
            "hitter must never domino itself"
        );
        assert_eq!(
            count_domino_hits_for(&engine, flyer, hitter),
            0,
            "flyer is not its own domino victim"
        );
    }

    /// ApplyDominoEffect measures the literal world X/Y ground plane. An
    /// elevated victim can therefore be inside the 15-unit world radius even
    /// when projecting elevation into map Y would put it outside the radius.
    #[test]
    fn elevated_domino_uses_world_ground_xy_not_projected_map_y() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let hitter = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 110.0,
                z: 0.0,
            },
            None,
        ));
        let flyer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 10.0,
            },
            None,
        ));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 0.0,
                y: 90.0,
                z: 18.0,
            },
            None,
        ));

        // The generic actor fixture finishes by authoring a map point, which
        // intentionally flattens actors without a level plane. Restore the
        // literal 3D positions needed by this elevated-flight boundary.
        engine
            .get_entity_mut(flyer)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 10.0,
            });
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D {
                x: 0.0,
                y: 90.0,
                z: 18.0,
            });

        give_flight(&mut engine, flyer, hitter, 0.0, -1.0, 5);
        let flight = engine
            .get_entity_mut(flyer)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_flight
            .as_mut()
            .expect("test flight remains active");
        flight.geometry = crate::element::FlightGeometry::World3d;
        flight.increment_z = 1.0;

        engine.tick_push_flights(sim, &LevelAssets::default());

        let flyer_element = engine.get_entity(flyer).unwrap().element_data();
        let victim_element = engine.get_entity(victim).unwrap().element_data();
        let world_y_delta = victim_element.position().y - flyer_element.position().y;
        let map_y_delta = victim_element.position_map().y - flyer_element.position_map().y;
        assert_eq!(world_y_delta, -9.0);
        assert_eq!(map_y_delta, -16.0);

        assert_eq!(
            count_domino_hits_for(&engine, victim, hitter),
            1,
            "world ground delta is 9 units after the flight step; projected map Y would incorrectly measure 16"
        );
    }

    /// Actors behind the flight vector (negative dot product) are
    /// outside the punch arc and must not take damage.
    #[test]
    fn domino_skips_actors_behind_flight_direction() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let hitter = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let flyer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        // Sits "behind" the flyer relative to its +X motion.
        let behind = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 5.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
        engine.tick_push_flights(sim, &LevelAssets::default());

        assert_eq!(
            count_domino_hits_for(&engine, behind, hitter),
            0,
            "actor behind the flyer should not be domino-hit (negative dot product)"
        );
    }

    /// The Chebyshev pre-filter (`MaxNorm < DOMINO_DISTANCE`) and the
    /// Euclidean check both have to fire. Place a candidate just past
    /// the radius and assert it is skipped.
    #[test]
    fn domino_respects_distance_radius() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let hitter = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let flyer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        // 16 map units away on the X axis — outside DOMINO_DISTANCE = 15.
        let far = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 26.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
        engine.tick_push_flights(sim, &LevelAssets::default());

        assert_eq!(
            count_domino_hits_for(&engine, far, hitter),
            0,
            "actor outside DOMINO_DISTANCE must not be domino-hit"
        );
    }

    /// Non-upright actors (lying, dead, etc.) are excluded — they're
    /// already on the ground and the upright-only filter rejects them.
    #[test]
    fn domino_skips_non_upright_actors() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let hitter = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let flyer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let mut lying_entity = make_soldier(
            WorldPoint3D {
                x: 16.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        );
        lying_entity.set_posture(Posture::Lying);
        let lying = engine.add_entity(lying_entity);

        give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
        engine.tick_push_flights(sim, &LevelAssets::default());

        assert_eq!(
            count_domino_hits_for(&engine, lying, hitter),
            0,
            "lying actor must not be domino-hit (filtered by Posture::Upright)"
        );
    }

    /// Rolling and ladder/wall flights set `antagonist = None`, so the
    /// per-frame sweep skips them entirely. Verify by giving the flyer
    /// a None-antagonist flight even though there's a candidate
    /// directly in the flight path.
    #[test]
    fn no_domino_when_flight_has_no_antagonist() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = make_engine();
        let _hitter = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let flyer = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));
        let candidate = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 16.0,
                y: 100.0,
                z: 0.0,
            },
            None,
        ));

        // No antagonist — mirrors the rolling / ladder-wall fall path.
        let flyer_pos = engine
            .get_entity(flyer)
            .unwrap()
            .element_data()
            .position_map();
        if let Some(entity) = engine.world.entities.get_mut(flyer)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.active_flight = Some(ActiveFlight {
                increment_x: 1.0,
                increment_y: 0.0,
                goal_x: flyer_pos.x + 5.0,
                goal_y: flyer_pos.y,
                frames_remaining: 5,
                antagonist: None,
                ..Default::default()
            });
        }

        engine.tick_push_flights(sim, &LevelAssets::default());

        let any_hit = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .any(|e| e.command == Command::ReceiveHitDamage && e.owner == Some(candidate));
        assert!(
            !any_hit,
            "antagonist=None flight (rolling / ladder-fall) must not domino"
        );
    }

    /// Regression: cheat-driven `apply_concussion` on a PC must seed
    /// `concussion_healing_timeout` with the PC profile's `wake_up`,
    /// not the soldier fallback constant.  Before the asset-context
    /// plumbing landed, the cheat path hard-coded
    /// `SOLDIER_CONCUSSION_HEALING_SPEED` because `&LevelAssets`
    /// wasn't reachable from `dispatch_console_command`.
    #[test]
    fn apply_concussion_uses_pc_profile_wake_up() {
        use crate::engine::LevelAssets;
        use crate::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

        const PC_WAKE_UP: u16 = 555;

        let mut engine = make_engine();
        // Forest of Barnsdale/Charnwood/Ashby missions use the forest proto
        // flag too, but only the Sherwood HQ mission grants PC immunity.
        engine.world.weather.is_forest_level = true;

        // PC with profile_index 0 — `make_pc` defaults to that.
        let pc_id = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            None,
        ));
        // Sanity: the helper does default to index 0.
        assert_eq!(
            engine
                .get_entity(pc_id)
                .unwrap()
                .pc_data()
                .unwrap()
                .profile_index,
            CharacterProfileIdx(0)
        );

        // Build a `LevelAssets` whose `ProfileManager` has a single PC
        // profile at index 0 with a distinctive `wake_up`.
        let mut profile_manager = ProfileManager::new();
        profile_manager.characters.push(CharacterProfile {
            wake_up: PC_WAKE_UP,
            ..CharacterProfile::default()
        });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        };

        // Drive the cheat-equivalent call: 100 concussion → KO →
        // healing-timeout init.
        let outcome =
            engine.apply_concussion(&crate::sim_rng::test_context(), &assets, pc_id, 100, false);
        assert_eq!(outcome, combat::ConcussionOutcome::WentUnconscious);

        let timeout = engine
            .get_entity(pc_id)
            .unwrap()
            .human_data()
            .unwrap()
            .concussion_healing_timeout;
        assert_eq!(
            timeout, PC_WAKE_UP,
            "cheat-driven KO on a PC must seed `concussion_healing_timeout` with \
             the PC profile's `wake_up`, not the soldier fallback constant \
             ({SOLDIER_CONCUSSION_HEALING_SPEED})"
        );
    }

    #[test]
    fn concussion_context_uses_campaign_description_identity_not_ui_list_index() {
        let mut engine = make_engine();
        engine.mission_domain.campaign.characters = vec![
            crate::campaign::PcDescription {
                character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
                status: crate::pc_status::PcStatus {
                    in_coma: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            crate::campaign::PcDescription {
                character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
                status: crate::pc_status::PcStatus {
                    in_coma: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];

        let mut pc = make_pc(
            WorldPoint3D {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            None,
        );
        let pc_data = pc.pc_data_mut().unwrap();
        pc_data.list_index = 0;
        pc_data.campaign_description_index = Some(1);

        let ctx = concussion_ctx_full(
            &pc,
            false,
            Some(&engine.mission_domain.campaign),
            engine.control.sim_config.difficulty,
        );
        assert!(
            !ctx.is_in_coma,
            "the UI list index must not borrow another same-profile PC's coma status"
        );

        engine.mission_domain.campaign.characters[1].status.in_coma = true;
        let ctx = concussion_ctx_full(
            &pc,
            false,
            Some(&engine.mission_domain.campaign),
            engine.control.sim_config.difficulty,
        );
        assert!(ctx.is_in_coma);
    }

    #[test]
    fn swordfight_distance_keeps_original_strict_minimum_boundary() {
        use super::evaluate::{
            SwordfightDistanceAdjustment as Adjustment, swordfight_distance_adjustment,
        };

        // Savegame_008/replay-012 reaches this representable distance after
        // one ordinary 12-unit swordfight correction. Original compares it
        // directly with the 45-unit MINIMAL range and requests another move.
        assert_eq!(
            swordfight_distance_adjustment(44.999_71, 45.0, 65.0, 65.0, false),
            Adjustment::Farther,
        );
        assert_eq!(
            swordfight_distance_adjustment(45.0, 45.0, 65.0, 65.0, false),
            Adjustment::None,
        );
    }

    #[test]
    fn evaluated_step_back_aborted_before_motion_terminal_preserves_history() {
        let mut engine = make_engine();
        let owner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        engine
            .get_entity_mut(owner)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .last_motion_was_step_back_in_combat = false;

        engine.launch_evaluated_step_back(owner, crate::coordinates::MapPoint::new(12.0, 0.0), 0);
        let (sequence_id, element_index) = engine
            .orders
            .sequence_manager
            .live_element_for_actor_matching(owner, |element| {
                element.movement_flags_for_test().is_some_and(|flags| {
                    flags.contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT)
                })
            })
            .expect("evaluated step-back movement must be registered");

        engine
            .orders
            .sequence_manager
            .element_impossible(sequence_id, element_index);
        assert!(
            !engine
                .get_entity(owner)
                .unwrap()
                .human_data()
                .unwrap()
                .last_motion_was_step_back_in_combat,
            "requesting and then aborting a step-back before RHMOTION_TERMINATED must not publish completed-step history"
        );
    }

    #[test]
    fn swordfight_distance_keeps_original_strict_maximum_and_step_back_guards() {
        use super::evaluate::{
            SwordfightDistanceAdjustment as Adjustment, swordfight_distance_adjustment,
        };

        assert_eq!(
            swordfight_distance_adjustment(65.000_01, 45.0, 65.0, 60.0, false),
            Adjustment::Closer,
        );
        assert_eq!(
            swordfight_distance_adjustment(65.0, 45.0, 65.0, 60.0, false),
            Adjustment::None,
        );
        assert_eq!(
            swordfight_distance_adjustment(65.000_01, 45.0, 65.0, 60.0, true),
            Adjustment::None,
        );
    }
}
