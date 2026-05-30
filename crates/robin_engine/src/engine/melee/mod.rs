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
use crate::combat::{self, ConcussionContext};
use crate::element::{ActionState, Entity, EntityId, EyeStatus, Posture};
use crate::entities::Entities;
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;
#[cfg(test)]
use crate::{element::Command, sequence::SequenceElementData};

// ─── Constants ──────────────────────────────────────────────────────

/// Fallback concussion healing speed when a soldier's profile lookup
/// fails. Matches the legacy soldier
/// wake-up default in the NPC concussion-healing path.
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
                tracing::warn!(
                    profile_index = ?pc.pc.profile_index,
                    "PC character profile missing wake_up; falling back to civilian default"
                );
                combat::CIVILIAN_CONCUSSION_HEALING_SPEED
            }),
        Entity::Soldier(s) => profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .map(|p| p.wake_up)
            .unwrap_or_else(|| {
                tracing::warn!(
                    profile_index = ?s.soldier.soldier_profile_index,
                    "Soldier profile missing wake_up; falling back to default"
                );
                SOLDIER_CONCUSSION_HEALING_SPEED
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

fn compute_upright_eye_point_map_space(entity: &crate::element::Entity) -> [f32; 3] {
    let Some(eye) = entity.compute_eyes_point(Some(Posture::Upright)) else {
        let pos = entity.element_data().position();
        let map = entity.element_data().position_map();
        return [map.x, map.y, pos.z];
    };
    let ground_z = entity.element_data().position().z;
    // `compute_eyes_point` returns render-space Y (`map_y + elevation`).
    // C++ `ComputeEyesPoint` / `IsReachable` uses map-space XY with Z
    // separate, so strip the feet elevation before the obstacle raycast.
    [eye.x, eye.y - ground_z, eye.z]
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
fn compute_special_strike_preparation_time(fighting_ability: u16) -> u32 {
    use crate::player_profile::DifficultyLevel;
    match DifficultyLevel::current() {
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

// ─── Remark IDs (NPC speech) ────────────────────────────────────────
// Only the combat-related remarks are listed; values are offsets within
// the full remark enum (REMARK_AWAITS_ORDERS=0, REMARK_WOUNDED=30, etc.).

const REMARK_WOUNDED: u16 = 30;
const REMARK_DIES: u16 = 31;
const CIV_REMARK_WOUNDED: u16 = 86;
const CIV_REMARK_DIES: u16 = 87;
const VIP_REMARK_WOUNDED: u16 = 117;
const VIP_REMARK_DIES: u16 = 118;

// ─── Helpers ────────────────────────────────────────────────────────

impl EngineInner {
    /// Build a [`ConcussionContext`] for a given entity id, reading the
    /// real invulnerable / tied / carried / script-locked / sherwood-pc
    /// flags off the entity instead of defaulting them.  Used by console
    /// cheats that call `set_concussion` (WAKEUP, MORPHEUS, BUD SPENCER)
    /// so the guards — "invulnerable entity refuses concussion
    /// increase", "tied/carried keeps asleep below wakeup threshold" —
    /// still fire through the cheat path.  Returns a default (all-false)
    /// context when the entity is missing (the caller then no-ops on
    /// the cheat target anyway).
    pub(crate) fn concussion_ctx_for(&self, id: EntityId) -> ConcussionContext {
        match self.get_entity(id) {
            Some(entity) => {
                concussion_ctx_full(entity, self.weather.is_forest_level, self.campaign.as_ref())
            }
            None => ConcussionContext::default(),
        }
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
    /// On `WentUnconscious` this:
    ///  - sets the concussion healing timeout
    ///  - clears action_state / active melee / path
    ///  - clears NPC suspects + alerted flag, sets eye-status
    ///  - clears `inform_my_friends` (no PC who-dunnit on cheat path)
    ///  - queues `quit_swordfight` + `add_unconscious_star` +
    ///    `EventLoseConsciousness` for the deferred drain in
    ///    `perform_hourglass` (where `&LevelAssets` is available).
    ///
    /// On `WokeUp` this queues `EventFitAgain` for the deferred drain.
    ///
    /// When `force_value` is true, bypass the script-lock
    /// stay-asleep clause so the call can wake a script-locked NPC.
    ///
    /// Returns the `ConcussionOutcome` from the underlying call.
    pub(crate) fn apply_concussion(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        value: u16,
        force_value: bool,
    ) -> crate::combat::ConcussionOutcome {
        use crate::combat::ConcussionOutcome;

        let mut ctx = self.concussion_ctx_for(entity_id);
        ctx.force_value = force_value;

        // Read the wake-up speed from the PC or soldier profile.  The
        // previous version hard-coded `SOLDIER_CONCUSSION_HEALING_SPEED`
        // because `&LevelAssets` wasn't in scope at the cheat call
        // sites; now it's plumbed through `dispatch_console_command`,
        // so the cheat-driven KO seeds the same per-profile timer the
        // damage-path tick uses.
        let healing_speed = match self.get_entity(entity_id) {
            Some(e) => concussion_healing_speed_for_entity(e, &assets.profile_manager),
            None => return ConcussionOutcome::NoChange,
        };

        let outcome = match self
            .get_entity_mut(entity_id)
            .and_then(|e| e.human_data_mut())
        {
            Some(h) => combat::set_concussion(h, value, &ctx),
            None => return ConcussionOutcome::NoChange,
        };

        match outcome {
            ConcussionOutcome::WentUnconscious => {
                // Healing-timeout init.
                if let Some(h) = self
                    .get_entity_mut(entity_id)
                    .and_then(|e| e.human_data_mut())
                    && h.concussion_healing_timeout == 0
                {
                    h.concussion_healing_timeout = healing_speed;
                }

                // State cleanup that handle_knockout also performs
                // (eye-status + action_state / active_melee / path
                // clears).  Done inline because none of these
                // mutations need `&LevelAssets`.
                if let Some(victim) = self.get_entity_mut(entity_id) {
                    if let Some(actor) = victim.actor_data_mut() {
                        if actor.action_state.is_sword()
                            || actor.action_state == ActionState::Menacing
                        {
                            actor.action_state = ActionState::Waiting;
                        }
                        actor.active_melee.clear();
                        actor.clear_path();
                    }
                    if let Some(npc) = victim.npc_data_mut() {
                        crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
                        npc.alerted = false;
                        npc.clear_all_suspects();
                        // Cheat path has no PC who-dunnit, so leave
                        // the body-detect broadcast off.
                        npc.inform_my_friends = false;
                    }
                }

                self.pending_concussion_side_effects
                    .push((entity_id, outcome));
            }
            ConcussionOutcome::WokeUp => {
                self.pending_concussion_side_effects
                    .push((entity_id, outcome));
            }
            ConcussionOutcome::NoChange => {}
        }

        outcome
    }

    /// Engine-level wrapper around `combat::set_life_points` that runs
    /// the cross-system side-effects the pure `combat::set_life_points`
    /// helper can't reach (Human base behaviour plus the PC override).
    ///
    /// Used by the script native `SetPersistentProperty(LIFEPOINTS, …)`,
    /// which passes the equivalent of `bShowTitbit = false` — so no
    /// damage-number titbit is emitted.
    ///
    /// Side effects:
    /// - Skips when `life_points <= 0` (already-dead branch).
    /// - Applies the clamp / invulnerable / Sherwood-PC guards via
    ///   `combat::set_life_points`.
    /// - Calls `handle_death(assets, …)` synchronously on the kill edge.
    /// - For PCs, fires the HERO_DIE / HERO_HURT cues via `say_ouch`
    ///   on any drop, mirroring the PC override.
    pub(crate) fn apply_scripted_life_points(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        amount: i32,
    ) {
        let entity = match self.get_entity(entity_id) {
            Some(e) => e,
            None => return,
        };
        let is_pc = entity.kind().is_pc();
        let invulnerable = entity.human_data().map(|h| h.invulnerable).unwrap_or(false);
        let max_lp = get_max_life_points(entity);
        let is_sherwood_pc = self.weather.is_forest_level && is_pc;
        let before = get_life_points(entity);

        // Already-dead skip: bypass without invoking the helper so
        // the death pipeline isn't re-entered for a corpse.
        if before <= 0 {
            return;
        }

        let new_value = amount.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let died = match self.get_entity_mut(entity_id) {
            Some(Entity::Pc(e)) => crate::combat::set_life_points(
                &mut e.pc.life_points,
                new_value,
                invulnerable,
                max_lp,
                is_sherwood_pc,
            ),
            Some(Entity::Soldier(e)) => crate::combat::set_life_points(
                &mut e.npc.life_points,
                new_value,
                invulnerable,
                max_lp,
                is_sherwood_pc,
            ),
            Some(Entity::Civilian(e)) => crate::combat::set_life_points(
                &mut e.npc.life_points,
                new_value,
                invulnerable,
                max_lp,
                is_sherwood_pc,
            ),
            _ => return,
        };

        let after = self.get_entity(entity_id).map(get_life_points).unwrap_or(0);
        let damage = (before - after).max(0) as u16;

        // PC override: HERO_DIE / HERO_HURT cues fire on any drop.
        // `say_ouch` reads the post-drop life/dead/unconscious state
        // and selects the right exclamation group, so call after the
        // field is updated.
        if is_pc && damage > 0 {
            self.say_ouch(assets, entity_id, Some(damage));
        }

        if died {
            self.handle_death(assets, entity_id);
        }
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
    pub(crate) fn drain_pending_concussion_side_effects(&mut self, assets: &LevelAssets) {
        use crate::combat::ConcussionOutcome;

        if self.pending_concussion_side_effects.is_empty() {
            return;
        }
        let entries = std::mem::take(&mut self.pending_concussion_side_effects);
        for (entity_id, outcome) in entries {
            match outcome {
                ConcussionOutcome::WentUnconscious => {
                    self.quit_swordfight(assets, entity_id);
                    self.add_unconscious_star(entity_id);
                    self.dispatch_ai_stimulus(
                        entity_id,
                        crate::ai::Stimulus::new(crate::ai::StimulusType::EventLoseConsciousness),
                    );
                }
                ConcussionOutcome::WokeUp => {
                    self.queue_wake_redetection_blinks(entity_id);
                    self.dispatch_ai_stimulus(
                        entity_id,
                        crate::ai::Stimulus::new(crate::ai::StimulusType::EventFitAgain),
                    );
                }
                ConcussionOutcome::NoChange => {}
            }
        }
    }

    /// legacy implementation `SetConcussionOfTheBrain` calls `BlinkEnemy(this)` for
    /// every opposite-camp NPC after a PC wakes, or after a soldier
    /// wakes while NPC-vs-NPC soldier hostility is enabled.
    pub(crate) fn queue_wake_redetection_blinks(&mut self, waker_id: EntityId) {
        let waker = match self.get_entity(waker_id) {
            Some(e) => e,
            None => return,
        };
        let waker_is_pc = waker.is_pc();
        let waker_is_soldier = matches!(waker, Entity::Soldier(_));
        if !(waker_is_pc || (waker_is_soldier && self.ai_global.npcs_can_be_enemies())) {
            return;
        }
        let waker_camp = waker.camp();
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            if npc_id == waker_id {
                continue;
            }
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            if entity.camp() == waker_camp {
                continue;
            }
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            let Some(ai) = npc.ai_brain.base_mut() else {
                continue;
            };
            ai.pending_blink_enemy_specific.push(waker_id);
        }
    }

    /// Drain the deferred `HADES` cheat queue.  Each queued id gets
    /// the full NPC-kill cascade via [`EngineInner::handle_death`]:
    /// alert-green, sleeping-forever state, eye close, friend /
    /// missed-friend detectable removal, emoticon clear, and the
    /// dying animation.
    pub(crate) fn drain_pending_hades_kills(&mut self, assets: &LevelAssets) {
        if self.pending_hades_kills.is_empty() {
            return;
        }
        let victims: Vec<EntityId> = std::mem::take(&mut self.pending_hades_kills);
        for victim_id in victims {
            self.handle_death(assets, victim_id);
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
/// For Lacklandist soldiers, applies difficulty scaling via `modify_capacity`.
fn fighting_ability_from_profile(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
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
                let diff = crate::player_profile::DifficultyLevel::current();
                diff.modify_capacity(
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
fn should_enter_swordfight_after_strike(
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

/// Check if an entity is a shield bearer (has a shield weapon).
fn is_entity_shield_bearer(
    entity: &Entity,
    profile_manager: &crate::profiles::ProfileManager,
) -> bool {
    get_hth_weapon_id_full(entity, profile_manager)
        .and_then(|idx| profile_manager.get_hth_weapon(idx))
        .map(|p| p.shield)
        .unwrap_or(false)
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
fn entity_distance(entities: &crate::entities::Entities, a: EntityId, b: EntityId) -> f32 {
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
fn direction_to(entities: &crate::entities::Entities, from: EntityId, to: EntityId) -> i16 {
    let pos_a = match entities.get(from) {
        Some(e) => e.element_data().position_map(),
        None => return 0,
    };
    let pos_b = match entities.get(to) {
        Some(e) => e.element_data().position_map(),
        None => return 0,
    };
    crate::position_interface::vector_to_sector_0_to_15(pos_b.x - pos_a.x, pos_b.y - pos_a.y)
}

/// Sector to unit vector with isometric Y scaling.  Thin alias over
/// [`crate::position_interface::sector_to_vector_iso`].  Every caller
/// passes `aspect_ratio = ASPECT_RATIO`; the argument is retained for
/// signature stability.
fn sector_to_vector_iso(sector: u16, _aspect_ratio: f32) -> (f32, f32) {
    let [x, y] = crate::position_interface::sector_to_vector_iso(sector as i16);
    (x, y)
}

/// Point-in-quadrilateral test using cross products.
///
/// Tests whether point `(px, py)` is inside the quadrilateral defined by
/// vertices `p0..p3` (assumed to be in consistent winding order).
/// Uses the sign-of-cross-product method.
fn point_in_quad(
    px: f32,
    py: f32,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) -> bool {
    // Cross product of edge vector with point-to-vertex vector.
    fn cross(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
        ax * by - ay * bx
    }
    let vertices = [p0, p1, p2, p3];
    let mut positive = 0;
    let mut negative = 0;
    for i in 0..4 {
        let (x1, y1) = vertices[i];
        let (x2, y2) = vertices[(i + 1) % 4];
        let c = cross(x2 - x1, y2 - y1, px - x1, py - y1);
        if c > 0.0 {
            positive += 1;
        } else if c < 0.0 {
            negative += 1;
        }
    }
    // All cross products same sign → inside.
    positive == 0 || negative == 0
}

/// Get an entity's camp (faction). PCs are always Royalists.
fn entity_camp(entities: &crate::entities::Entities, id: EntityId) -> crate::element::Camp {
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
    pc_id: EntityId,
    victim_id: EntityId,
) -> Option<u32> {
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
fn is_possible_sword_strike_victim(
    entities: &crate::entities::Entities,
    attacker: EntityId,
    target_entity: &Entity,
    target_id: EntityId,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
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
    attacker: EntityId,
    target_id: EntityId,
    profile_manager: &crate::profiles::ProfileManager,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> bool {
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
        let target_id = EntityId::from(target_id);
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
        let target_id = EntityId::from(target_id);
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
            victims.push(target_id);
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
        let target_id = EntityId::from(target_id);
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
            victims.push(target_id);
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
mod dispatch;
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
        if let Some(entity) = engine.entities.get_mut(flyer)
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

    fn assets_with_sword_profile(energy: u16, max_distance: u16) -> LevelAssets {
        let mut profile_manager = crate::profiles::ProfileManager::new();
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
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
                ..crate::profiles::SoldierProfile::default()
            });

        LevelAssets {
            profile_manager: std::sync::Arc::new(profile_manager),
            ..LevelAssets::default()
        }
    }

    #[test]
    fn completed_missed_sword_strike_adds_tiredness_once() {
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

        engine.tick_melee_strikes(&assets);

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

        engine.tick_sweep_strikes(&LevelAssets::default());
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

        engine.tick_sweep_strikes(&LevelAssets::default());
        assert!(
            engine
                .get_entity(attacker)
                .unwrap()
                .actor_data()
                .unwrap()
                .sweep_state
                .is_none(),
            "empty true-circle sweep should clear once the rotation reaches the final angle"
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

        crate::sim_rng::with_seed(1, || {
            engine.launch_sword_damage_now(&assets, victim, attacker, SwordStrike::A, 1);
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

    /// `SwordstrikeThrustA` promotes both principal opponents before
    /// the strike, so clicking a secondary opponent during a
    /// swordfight switches the primary target.
    #[test]
    fn thrust_a_promotes_clicked_secondary_opponent() {
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

        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustA,
            Some(pc),
            Some(clicked),
        ));
        let seq_id = engine.launch_sequence(sequence);

        engine.dispatch_sword_strike(
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

    /// Bud-Spencer-style line of three: PC punches the first soldier,
    /// who is launched along +X into a second soldier directly in
    /// front, and a third soldier behind the second. The flight tick
    /// should fire a domino RECEIVE_HIT_DAMAGE on both downstream
    /// soldiers, citing the PC as origin.
    #[test]
    fn domino_propagates_to_actors_in_flight_path() {
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

        engine.tick_push_flights(&LevelAssets::default());

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
        engine.tick_push_flights(&LevelAssets::default());

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
        engine.tick_push_flights(&LevelAssets::default());

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
        engine.tick_push_flights(&LevelAssets::default());

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
        if let Some(entity) = engine.entities.get_mut(flyer)
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

        engine.tick_push_flights(&LevelAssets::default());

        let any_hit = engine
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
        let outcome = engine.apply_concussion(&assets, pc_id, 100, false);
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
