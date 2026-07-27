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
//! 1. `dispatch_sword_strike` sets [`ActiveMelee`] on the attacker.
//! 2. `tick_melee_strikes` counts down the timer; at the hit frame, performs
//!    distance-based hit detection and applies damage.
//! 3. On completion, clears `ActiveMelee` and terminates the sequence element.
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
//!   `MELEE_HIT_FRAME` timer when sprite animation is unavailable.

use super::*;
use crate::combat::{self, ConcussionContext, ConcussionOutcome};
use crate::element::{ActionState, Entity, EntityId, EyeStatus, Posture};
use crate::entities::Entities;
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;
#[cfg(test)]
use crate::{element::Command, sequence::SequenceElementData};

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

/// Small comparison tolerance for swordfight spacing thresholds.  The
/// reference compares floats directly, but the Rust port can arrive at
/// `64.99995` for an intended `65`-unit duel spacing after replayed movement.
const SWORDFIGHT_DISTANCE_EPSILON: f32 = 0.001;

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
    kind: WeaponThrustKind,
    strike: SwordStrike,
    max_distance: f32,
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
        id: I,
    ) -> Option<ConcussionContext> {
        let id = id.into();
        self.get_entity(id).map(|entity| {
            concussion_ctx_full(
                entity,
                self.world.weather.is_forest_level,
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
        let Some(mut ctx) = self.concussion_ctx_for(sim, entity_id) else {
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
                self.world.weather.is_forest_level,
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
            self.apply_scripted_virtual_kill(sim, assets, entity_id);
        }
    }

    /// Run the virtual PC/NPC/Soldier/Human `Kill` chain used by
    /// `RHElementActorHuman::SetLifePoints`, without synthesizing a damage
    /// element. Damage-only animation, roll, attacker attribution, and fight
    /// score belong to `ReceiveDamage`, not to a script setter.
    fn apply_scripted_virtual_kill(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (is_pc, is_npc, allied_soldier) = {
            let entity = self
                .get_entity(entity_id)
                .expect("script-killed actor vanished before virtual Kill");
            (
                entity.is_pc(),
                entity.is_npc(),
                entity.is_soldier() && entity.camp() == crate::element::Camp::Royalists,
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
            npc.inform_my_friends = false;
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
/// on `campaign.characters[list_index].status.in_coma` rather than the
/// entity, so callers that care about the coma override must pass
/// `campaign`.
pub(crate) fn concussion_ctx_full(
    entity: &Entity,
    is_forest_level: bool,
    campaign: Option<&crate::campaign::Campaign>,
    difficulty: crate::player_profile::DifficultyLevel,
) -> ConcussionContext {
    let human = entity.human_data();
    let posture = entity.element_data().posture;
    let is_in_coma = match entity {
        Entity::Pc(pc) => campaign
            .and_then(|c| c.characters.get(pc.pc.list_index as usize))
            .map(|p| p.status.in_coma)
            .unwrap_or(false),
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
        // PCs in Sherwood Forest get knockdown protection (concussion
        // always max instead of kill).
        is_sherwood_pc: is_forest_level && entity.kind().is_pc(),
        is_in_coma,
        // `force_value` is a per-call parameter, not entity state.
        // Default to false; cheats / scripts that need force-wake set
        // it on the ctx returned by `concussion_ctx_for`.
        force_value: false,
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

    // Building sector check.  Matches `IsInsideBuilding` semantics —
    // sector flag OR door-transit, so an actor mid-door-pass also
    // counts as inside a building.
    let sector_a = entity_a.element_data().sector();
    let sector_b = entity_b.element_data().sector();
    let inside_a = is_in_building_sector(sector_a, fast_grid) || entity_a.is_in_door_transit();
    let inside_b = is_in_building_sector(sector_b, fast_grid) || entity_b.is_in_door_transit();
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

fn is_possible_sword_strike_victim_id(
    entities: &crate::entities::Entities,
    attacker: impl Into<EntityId>,
    target_id: impl Into<EntityId>,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
    let attacker = attacker.into();
    let target_id = target_id.into();
    let Some(target_entity) = entities.get(target_id) else {
        return false;
    };
    is_possible_sword_strike_victim(
        entities,
        attacker,
        target_entity,
        target_id,
        profile_manager,
        fast_grid,
        obstacles,
    )
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
        // Use ground position (which includes elevation in the Y
        // coordinate) for MOTION_DONE victim collection.
        let pos = entity.ground_position();
        let dx = pos.x - attacker_pos.0;
        let dy = (pos.y - attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        // Quick reject
        if dx.abs().max(dy.abs()) > 150.0 {
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
        let pos = entity.ground_position();
        let dx = pos.x - attacker_pos.0;
        let dy = (pos.y - attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        if dx.abs().max(dy.abs()) > 150.0 {
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
            let relative = ((enemy_sector + 16 - attacker_direction) % 16) as f32;
            let rotation = rotation_angle_deg.max(1) as f32;
            max_dist += 10.0 + (relative * 5.0 * std::f32::consts::PI) / (8.0 * rotation);
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
    dir_x: f32,
    dir_y: f32,
    min_distance: f32,
    max_distance: f32,
    half_width: f32,
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
        // Use ground position (which includes elevation in Y) for
        // MOTION_DONE victim collection.
        let pos = entity.ground_position();
        let dx = pos.x - attacker_pos.0;
        let dy = (pos.y - attacker_pos.1) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        if dx.abs().max(dy.abs()) > 150.0 {
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
/// sector's begin edge, so the floor-based `angle_to_sector`
/// round-trips back to the same sector.
fn sector_to_angle(sector: i16) -> f32 {
    (sector as f32) * std::f32::consts::PI * 2.0 / 16.0 + 0.1
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
    }
}

/// Original `RHElementActorHuman::GetSwordStrikeFromAnimation`.
///
/// Strike startup may select a replacement animation (for example, a requested
/// right strike can be rendered by the left-strike row). Reactive defenders
/// observe that live animation, not the sequence command that requested it.
fn sword_strike_from_animation(animation: crate::order::OrderType) -> Option<SwordStrike> {
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
/// Floor binning where sector `k` covers `[k·2π/16, (k+1)·2π/16)`.
/// The negative-angle case is handled by normalising the input into
/// `[0, 2π)` first instead of by recursion.
fn angle_to_sector(angle: f32) -> u8 {
    let two_pi = std::f32::consts::PI * 2.0;
    let normalized = ((angle % two_pi) + two_pi) % two_pi;
    ((normalized / two_pi * 16.0).floor() as u32 % 16) as u8
}

/// Get the unit direction vector for a 0-15 sector.
///
/// Computes the unbiased sin/cos directly — must NOT go through
/// `sector_to_angle`, which adds the `+0.1` round-trip nudge that
/// would rotate the resulting vector by ~5.7° relative to the
/// pre-baked per-sector unit vectors used elsewhere.
fn sector_to_direction(sector: i16) -> (f32, f32) {
    let angle = (sector as f32) * std::f32::consts::PI * 2.0 / 16.0;
    (angle.sin(), -angle.cos())
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
fn select_hit_fall_animation(
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
        ActiveFlight, ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, HumanData,
        NpcData, PcData, SoldierData,
    };

    fn make_engine() -> EngineInner {
        EngineInner::new()
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
                ..PcData::default()
            },
        })
    }

    /// Set up an active push-flight on `flyer` so the per-frame
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
        if let Some(entity) = engine.world.entities.get_mut(flyer)
            && let Some(actor) = entity.actor_data_mut()
        {
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
            let position = engine.get_entity_mut(victim).unwrap().position_iface_mut();
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
        assert!(victim_entity.actor_data().unwrap().active_flight.is_none());

        engine.initialize_hit_flight(&LevelAssets::default(), victim, Some(attacker), queued_type);

        assert_ne!(
            engine
                .get_entity(victim)
                .unwrap()
                .element_data()
                .direction(),
            5
        );
    }

    fn assets_with_sword_profile(energy: u16, max_distance: u16) -> LevelAssets {
        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = max_distance;
        weapon.thrusts[SwordStrike::A as usize].energy = energy;
        weapon.thrusts[SwordStrike::A as usize].minimal_distance = 0;
        weapon.thrusts[SwordStrike::A as usize].maximal_distance = max_distance;
        weapon.thrusts[SwordStrike::A as usize].cutting = 4;
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
            target.actor_data_mut().unwrap().old_action = OrderType::Invalid;
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

    fn soldier_life(engine: &EngineInner, soldier_id: EntityId) -> i16 {
        match engine
            .get_entity(soldier_id)
            .expect("test soldier must remain present")
        {
            Entity::Soldier(soldier) => soldier.npc.life_points,
            _ => panic!("test victim must be a soldier"),
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

        if let Some(actor) = engine.get_entity_mut(attacker).unwrap().actor_data_mut() {
            actor.active_melee = crate::movement::ActiveMelee::new(target, SwordStrike::A, None, 0);
            actor.active_melee.frames_remaining = 1;
        }

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
        let mut active = crate::movement::ActiveMelee::new(victim, SwordStrike::D, None, 0);
        active.frames_remaining =
            crate::movement::MELEE_STRIKE_DURATION - crate::movement::MELEE_HIT_FRAME;
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_melee = active;

        let phase = engine.tick_nonstraight_melee_for(sim, &assets, attacker);
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

        let mut active = crate::movement::ActiveMelee::new(victim, SwordStrike::D, None, 0);
        active.hit_applied = true;
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_melee = active;
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

        {
            let entity = engine.get_entity_mut(attacker).unwrap();
            let mut replacement =
                crate::movement::ActiveMelee::new(victim, SwordStrike::E, None, 0);
            replacement.order_id = std::num::NonZeroU32::new(99);
            entity.actor_data_mut().unwrap().active_melee = replacement;

            let sprite = &mut entity.element_data_mut().sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 3,
                frame_ids: vec![0, 1, 2, 3],
                delays: vec![1, 1, 1, 1],
                distances: vec![0, 0, 0, 0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
                sound_ids: vec![0; 4],
                ..Default::default()
            }]);
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
        assert_eq!(retained_on_start.strike, SwordStrike::D);
        assert_eq!(retained_on_start.current_angle, 0.0);
        assert_eq!(soldier_life(&engine, victim), 50);

        engine.tick_melee_strikes(sim, &assets);

        assert!(
            soldier_life(&engine, victim) < 50,
            "the first E IN_PROGRESS must hit using E's left-to-right 90-degree sweep"
        );
        assert_eq!(
            soldier_life(&engine, unreached_victim),
            50,
            "a victim outside E's newly swept sector must remain pending"
        );
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

        engine.tick_sweep_for(sim, &assets, attacker, false);
        assert_eq!(
            soldier_life(&engine, victim),
            50,
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
        assert!(
            soldier_life(&engine, victim) < 50,
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
    fn push_victims_receive_synchronous_damage_in_creation_fifo() {
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
        let mut active = crate::movement::ActiveMelee::new(first_victim, SwordStrike::D, None, 0);
        active.frames_remaining =
            crate::movement::MELEE_STRIKE_DURATION - crate::movement::MELEE_HIT_FRAME;
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .active_melee = active;

        assert_eq!(
            engine.tick_nonstraight_melee_for(sim, &assets, attacker),
            strikes::SweepTickPhase::Dormant
        );

        assert!(
            soldier_life(&engine, first_victim) < 50 && soldier_life(&engine, second_victim) < 50,
            "both push victims must be damaged before the attacker's slot returns"
        );
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
            "push damage launches must retain the original actor-list victim FIFO"
        );
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
}
