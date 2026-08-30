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

mod animations;
pub(in crate::engine) use animations::select_hit_fall_animation;
use animations::{PushDamageAnimations, select_combat_animations, select_push_damage_animations};

/// Whether a human must be represented in the shared strike-estimation
/// context. Original's straight collector considers the principal opponent
/// without calling `IsPossibleSwordStrikeVictim`, even while that opponent is
/// temporarily inactive (for example, while traversing a door). Other
/// inactive humans must stay absent from every collector.
fn should_collect_strike_estimation_human(
    candidate: EntityId,
    attacker: EntityId,
    principal_opponent: Option<EntityId>,
    active: bool,
) -> bool {
    candidate != attacker && (active || principal_opponent == Some(candidate))
}

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
        | Posture::Cloaked
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
    u32::from(difficulty.rules().special_strike_base_frames)
        .saturating_sub(fighting_ability as u32 / 10)
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
        // Script/native life-point setters reach this virtual Kill boundary
        // without allocating a Damage element. Preserve their exact optional
        // killer instead of inferring responsibility from aggregate totals.
        self.record_achievement_npc_death(entity_id, killer);
        let (is_pc, is_ai_owner, allied_soldier, killer_is_pc) = {
            let entity = self
                .get_entity(entity_id)
                .expect("script-killed actor vanished before virtual Kill");
            (
                entity.is_pc(),
                entity.ai_controller().is_some(),
                entity.is_soldier() && entity.camp() == crate::element::Camp::Royalists,
                killer
                    .and_then(|killer_id| self.get_entity(killer_id))
                    .is_some_and(|killer| killer.is_pc()),
            )
        };

        if is_pc {
            self.apply_pc_kill_cascade(sim, assets, entity_id);
        }
        if is_ai_owner {
            self.delete_detectable_for_all_npc(entity_id, crate::element::DetectableType::Friend);
            self.delete_detectable_for_all_npc(
                entity_id,
                crate::element::DetectableType::MissedFriend,
            );
            let entity = self
                .get_entity_mut(entity_id)
                .expect("script-killed AI owner vanished during virtual Kill");
            let forced_attentive = entity
                .enemy_ai()
                .is_some_and(|enemy| enemy.forced_attentive);
            let ai = entity
                .ai_controller_mut()
                .expect("script-killed AI owner has no AI controller");
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
                .ai_actor_data_mut()
                .expect("script-killed AI owner has no AI actor data");
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
                        .and_then(|entity| entity.ai_actor_data_mut())
                    {
                        npc.clear_all_suspects();
                    }
                    self.dispatch_ai_stimulus(
                        entity_id,
                        crate::ai::Stimulus::new(crate::ai::StimulusType::EventLoseConsciousness),
                    );
                    if let Some(npc) = self
                        .get_entity_mut(entity_id)
                        .and_then(|entity| entity.ai_actor_data_mut())
                    {
                        // Script setters have no who-dunnit actor.
                        npc.inform_my_friends = false;
                    }
                }
                ConcussionOutcome::WokeUp => {
                    let (is_pc, has_ai) = self
                        .get_entity(entity_id)
                        .map(|entity| (entity.is_pc(), entity.ai_controller().is_some()))
                        .unwrap_or((false, false));
                    if has_ai {
                        self.dispatch_ai_stimulus(
                            entity_id,
                            crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
                        );
                    } else if is_pc {
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
        let npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
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
            let npc = entity.ai_actor_data_mut().unwrap_or_else(|| {
                panic!(
                    "AI owner {} lost its AI actor data while queueing wake blink for {}",
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
            if s.soldier
                .cached_camp
                .is_hostile_to(crate::element::Camp::Royalists)
            {
                difficulty.rules().enemy_fighting(base, 100)
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
    if !victim.camp().is_hostile_to(attacker.camp()) {
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

/// Get an entity's allegiance.
fn entity_camp<I: Into<EntityId>>(
    entities: &crate::entities::Entities,
    id: I,
) -> crate::element::Camp {
    let id = id.into();
    entities
        .get(id)
        .filter(|entity| entity.is_human())
        .map(Entity::camp)
        .unwrap_or(crate::element::Camp::Error)
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
        if !sword_strike_distance_is_in_range(distance, min_distance, max_distance) {
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
        if !sword_strike_distance_is_in_range(distance, min_distance, max_distance) {
            continue;
        }

        let sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
        if is_sector_between(sector, begin_sector, end_sector) {
            victims.push(target_id.into());
        }
    }
    victims
}

/// Original full-circle DONE-time victim admission in unprojected 3D space.
///
/// This is deliberately separate from the circle warning collector: warning
/// admission uses map-space distance and different range rules, while
/// `ExecuteFullCircleSwordStrikeEffect` seeds its retained victim list from
/// `GetPosition()` and the inclusive authored strike range.
#[allow(clippy::too_many_arguments)]
fn collect_full_circle_strike_victims(
    entities: &Entities,
    attacker_id: EntityId,
    attacker_position: crate::coordinates::WorldPoint3D,
    min_distance: f32,
    max_distance: f32,
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
        if full_circle_strike_distance_is_in_range(
            attacker_position,
            target_position,
            min_distance,
            max_distance,
        ) {
            victims.push(target_id.into());
        }
    }
    victims
}

/// Original half-circle DONE-time victim admission: 3D range combined with
/// an unprojected ground-space angular sector.
#[allow(clippy::too_many_arguments)]
fn collect_half_circle_strike_victims(
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
        if half_circle_strike_seed_allows(
            attacker_position,
            target_position,
            min_distance,
            max_distance,
            begin_sector,
            end_sector,
        ) {
            victims.push(target_id.into());
        }
    }
    victims
}

/// `SBGeoVector3D::Norm`: GEOTYPE products and additions, followed by the
/// double-precision square root and a narrowing store back to GEOTYPE.
fn full_circle_strike_distance(
    attacker: crate::coordinates::WorldPoint3D,
    victim: crate::coordinates::WorldPoint3D,
) -> f32 {
    let dx = attacker.x - victim.x;
    let dy = attacker.y - victim.y;
    let dz = attacker.z - victim.z;
    let squared_norm = dx * dx + dy * dy + dz * dz;
    f64::from(squared_norm).sqrt() as f32
}

fn full_circle_strike_distance_is_in_range(
    attacker: crate::coordinates::WorldPoint3D,
    victim: crate::coordinates::WorldPoint3D,
    min_distance: f32,
    max_distance: f32,
) -> bool {
    let distance = full_circle_strike_distance(attacker, victim);
    sword_strike_distance_is_in_range(distance, min_distance, max_distance)
}

fn sword_strike_distance_is_in_range(distance: f32, min_distance: f32, max_distance: f32) -> bool {
    // Original's collectors spell this as a positive conjunction. That
    // distinction matters for corrupt-but-loadable legacy actors whose
    // position is NaN: both comparisons reject the actor in C++, while the
    // negated form (`distance < min || distance > max`) admits it.
    distance >= min_distance && distance <= max_distance
}

fn half_circle_strike_seed_allows(
    attacker: crate::coordinates::WorldPoint3D,
    victim: crate::coordinates::WorldPoint3D,
    min_distance: f32,
    max_distance: f32,
    begin_sector: u8,
    end_sector: u8,
) -> bool {
    if !full_circle_strike_distance_is_in_range(attacker, victim, min_distance, max_distance) {
        return false;
    }

    let dx = victim.x - attacker.x;
    let dy = (victim.y - attacker.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
    let sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
    is_sector_between(sector, begin_sector, end_sector)
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
        if !sword_strike_distance_is_in_range(distance, min_distance, max_distance) {
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
    base_max_distance: u16,
    rotation_angle_deg: u16,
    is_walking_with_sword: impl Fn(EntityId) -> bool,
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
        if is_walking_with_sword(target_id.into()) {
            let enemy_sector = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
            let relative = ((enemy_sector + 16 - attacker_direction) % 16) as u16;
            // Original stores both the authored range and this tolerance in
            // UWORD, so the compound assignment wraps before promotion for
            // the floating-point distance comparison.
            max_dist = max_dist.wrapping_add(circle_warning_walking_tolerance(
                relative,
                rotation_angle_deg,
            ));
        }
        if distance <= f32::from(max_dist) {
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
    attacker_direction: i16,
    min_distance: f32,
    max_distance: f32,
    half_width: f32,
}

#[derive(Clone, Copy)]
enum PushStrikePositionSpace {
    Map,
    Ground,
}

/// Whether PushAside's current collector applies its elevation admission.
///
/// Original's warning-time `GetPossibleVictimsOfPushSwordStrike` works only
/// in map space and has no elevation gate. The DONE-time
/// `ExecutePushSwordStrike` collector uses ground space and first truncates
/// the absolute elevation difference to `ULONG` before comparing it with
/// `MAX_ELEVATION_SWORDFIGHT`.
fn push_strike_elevation_allows(
    position_space: PushStrikePositionSpace,
    attacker_elevation: f32,
    victim_elevation: f32,
) -> bool {
    match position_space {
        PushStrikePositionSpace::Map => true,
        PushStrikePositionSpace::Ground => {
            (attacker_elevation - victim_elevation).abs() as u32 <= MAX_ELEVATION_SWORDFIGHT as u32
        }
    }
}

/// Warning-time push collection has Original's `< 150` MaxNorm shortcut;
/// DONE-time push effects do not.
fn push_strike_max_norm_allows(position_space: PushStrikePositionSpace, dx: f32, dy: f32) -> bool {
    match position_space {
        PushStrikePositionSpace::Map => dx.abs().max(dy.abs()) < 150.0,
        PushStrikePositionSpace::Ground => true,
    }
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
        attacker_direction,
        min_distance,
        max_distance,
        half_width,
    } = *params;
    let ((fx, fy), (sx, sy)) = crate::combat::push_strike_basis(attacker_direction);

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
        let victim_elev = entity.position_iface().get_elevation();
        if !push_strike_elevation_allows(position_space, attacker_elevation, victim_elev) {
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
        if !push_strike_max_norm_allows(position_space, dx, dy) {
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
mod tests;
