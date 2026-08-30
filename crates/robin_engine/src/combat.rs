//! Combat system — melee, ranged, knockout, tie-up, damage, and death.
//!
//! ## Design
//!
//! Free functions operate on the existing data structs (`HumanData`,
//! `PcData`, `NpcData`) plus small context structs to pass entity state that
//! lives in other parts of the hierarchy (action state, weapon profiles, etc.).

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::element::{ActionState, Camp, EntityId, HumanData, Posture};
use crate::profiles::{HtHWeaponProfile, WeaponThrustDirection, WeaponThrustKind};
use crate::weapons::{NUM_NORMAL_SWORD_STRIKES, SwordState, SwordStrike};

use std::f32::consts::PI;

// ─── Constants ─────────────────────────────────────────────────────

/// Concussion threshold at which a human goes unconscious (KO).
pub const CONCUSSION_THRESHOLD: u16 = 70;

/// Concussion level below which an unconscious human wakes up.
pub const CONCUSSION_WAKEUP_THRESHOLD: u16 = 30;

/// Maximum possible concussion value.
pub const CONCUSSION_MAX: u16 = 300;

/// Default max life points for PCs.
pub const LIFEPOINTS_PC: i16 = 100;

/// Experience gained for killing with sword.
pub const SWORD_KILL_EXPERIENCE_POINTS: u32 = 20;

/// Experience gained for killing with bow.
pub const BOW_KILL_EXPERIENCE_POINTS: u32 = 20;

/// Concussion healing speed for civilians (frames between -1 concussion ticks).
pub const CIVILIAN_CONCUSSION_HEALING_SPEED: u16 = 500;

/// Isometric aspect ratio used for direction calculations.
/// Re-export of [`crate::position_interface::ASPECT_RATIO`].
pub use crate::position_interface::ASPECT_RATIO;

// ─── Sword damage result flags ─────────────────────────────────────

bitflags! {
    /// Bitfield result from `receive_sword_damage`, indicating which
    /// damage components were actually applied.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct SwordDamageResult: u32 {
        const CUTTING_DAMAGE    = 1;
        const STUNNING_DAMAGE   = 2;
        const NO_DAMAGE_PARRIED = 4;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(SwordDamageResult, u32);

// ─── Damage event ──────────────────────────────────────────────────

/// Type of incoming damage.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum DamageKind {
    /// Generic damage (e.g. falling, environmental).
    Generic,
    /// Sword strike damage.
    Sword,
    /// Arrow damage.
    Arrow,
    /// Stone throw damage.
    Stone,
    /// Fist/club hit (concussion only, no cutting).
    Hit,
    /// Net entanglement (no direct damage).
    Net,
    /// Mobile object collision.
    Mobile,
}

/// Describes incoming damage.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DamageEvent {
    pub kind: DamageKind,
    pub damage: u16,
    pub concussion: u16,
    pub origin: Option<EntityId>,
    /// For sword damage: the strike type used.
    pub sword_strike: Option<SwordStrike>,
    /// For hit damage: whether this was a hard hit.
    pub is_harder_hit: bool,
}

impl DamageEvent {
    pub fn sword(origin: EntityId, strike: SwordStrike) -> Self {
        Self {
            kind: DamageKind::Sword,
            damage: 0, // computed by receive_sword_damage
            concussion: 0,
            origin: Some(origin),
            sword_strike: Some(strike),
            is_harder_hit: false,
        }
    }

    pub fn arrow(origin: EntityId, damage: u16) -> Self {
        Self {
            kind: DamageKind::Arrow,
            damage,
            concussion: 0,
            origin: Some(origin),
            sword_strike: None,
            is_harder_hit: false,
        }
    }

    pub fn stone(damage: u16, concussion: u16) -> Self {
        Self {
            kind: DamageKind::Stone,
            damage,
            concussion,
            origin: None,
            sword_strike: None,
            is_harder_hit: false,
        }
    }

    pub fn hit(origin: EntityId, concussion: u16, hard: bool) -> Self {
        Self {
            kind: DamageKind::Hit,
            damage: 0,
            concussion,
            origin: Some(origin),
            sword_strike: None,
            is_harder_hit: hard,
        }
    }

    pub fn net(origin: EntityId) -> Self {
        Self {
            kind: DamageKind::Net,
            damage: 0,
            concussion: 0,
            origin: Some(origin),
            sword_strike: None,
            is_harder_hit: false,
        }
    }

    pub fn generic(damage: u16, concussion: u16) -> Self {
        Self {
            kind: DamageKind::Generic,
            damage,
            concussion,
            origin: None,
            sword_strike: None,
            is_harder_hit: false,
        }
    }
}

// ─── Context for concussion checks ────────────────────────────────

/// Entity state needed by concussion/KO logic that lives outside `HumanData`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConcussionContext {
    pub difficulty: crate::player_profile::DifficultyLevel,
    pub is_invulnerable: bool,
    pub is_tied: bool,
    pub is_carried: bool,
    pub is_script_locked: bool,
    /// True if this is a PC in Sherwood mode (PCs can't be KO'd).
    pub is_sherwood_pc: bool,
    /// True if this is a PC currently in coma.  When in coma the input
    /// value is overridden to `CONCUSSION_MAX`, so any call that
    /// would lower concussion is a no-op.
    pub is_in_coma: bool,
    /// When true, bypass the `script_locked && old >= WAKEUP_THRESHOLD`
    /// stay-asleep clause so a script can force-wake a script-locked NPC.
    pub force_value: bool,
    /// Civilian carrying an attached scroll.  Such a civilian silently
    /// discards life-point loss and concussion; every other part of the
    /// damage pipeline — including the protection rolls — still runs.
    pub scroll_attached: bool,
}

// ─── Concussion result ─────────────────────────────────────────────

/// Outcome of setting concussion.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum ConcussionOutcome {
    /// No state change.
    NoChange,
    /// Entity just went unconscious (KO).
    WentUnconscious,
    /// Entity just woke up from unconsciousness.
    WokeUp,
}

// ═══════════════════════════════════════════════════════════════════
//  Life points
// ═══════════════════════════════════════════════════════════════════

/// Set life points, clamped to `[0, max]`.
/// Returns `true` if the entity just died (life_points reached 0).
pub fn set_life_points(
    life_points: &mut i16,
    value: i16,
    invulnerable: bool,
    _max_life_points: i16,
    is_sherwood_pc: bool,
) -> bool {
    // Already dead — can only die once.
    if *life_points <= 0 {
        return false;
    }

    // PCs can't be hurt in Sherwood mode.
    if is_sherwood_pc && value < *life_points {
        return false;
    }

    let new_value = if invulnerable {
        // RHElementActorHuman::SetLifePoints uses the literal 100 here,
        // irrespective of the actor profile or difficulty-scaled maximum.
        100
    } else {
        value.max(0)
    };

    let died = new_value <= 0 && *life_points > 0;
    *life_points = new_value;
    died
}

/// Subtract `damage` from life points. Returns `true` if entity died.
pub fn get_wounded(
    life_points: &mut i16,
    damage: u16,
    invulnerable: bool,
    max_life_points: i16,
    is_sherwood_pc: bool,
) -> bool {
    let new_value = *life_points - damage as i16;
    set_life_points(
        life_points,
        new_value,
        invulnerable,
        max_life_points,
        is_sherwood_pc,
    )
}

// ═══════════════════════════════════════════════════════════════════
//  Concussion / Knockout
// ═══════════════════════════════════════════════════════════════════

/// Compute the new concussion value when adding a positive concussion effect.
/// The effect is scaled inversely by life points:
/// `current + (effect * 100 / life_points)`.
pub fn compute_concussion_effect(current: u16, effect: u16, life_points: i16) -> u16 {
    if life_points <= 0 {
        return current;
    }
    current + (effect as u32 * 100 / life_points as u32) as u16
}

/// Set the concussion value with full KO threshold logic.
///
/// Returns a [`ConcussionOutcome`] indicating whether the entity's
/// consciousness state changed.
pub fn set_concussion(
    human: &mut HumanData,
    value: u16,
    ctx: &ConcussionContext,
) -> ConcussionOutcome {
    // PC-in-coma override: any caller trying to lower an in-coma PC's
    // concussion is forced back up to the max so the clamp below leaves
    // the value at max.
    let value = if ctx.is_in_coma {
        CONCUSSION_MAX
    } else {
        value
    };

    // Invulnerable entities can't have concussion increased.
    if ctx.is_invulnerable && value > human.concussion_of_the_brain {
        return ConcussionOutcome::NoChange;
    }

    // Sherwood PCs can't be knocked out.
    if ctx.is_sherwood_pc && value > human.concussion_of_the_brain {
        return ConcussionOutcome::NoChange;
    }

    let old_concussion = human.concussion_of_the_brain;
    human.concussion_of_the_brain = value.min(CONCUSSION_MAX);

    // If tied/carried/script-locked, don't let concussion drop below wakeup
    // threshold. `force_value` lets a script force-wake a script-locked NPC.
    let should_stay_asleep = ctx.is_tied
        || ctx.is_carried
        || (ctx.is_script_locked
            && !ctx.force_value
            && old_concussion >= CONCUSSION_WAKEUP_THRESHOLD);

    if should_stay_asleep && human.concussion_of_the_brain < CONCUSSION_WAKEUP_THRESHOLD {
        human.concussion_of_the_brain = CONCUSSION_WAKEUP_THRESHOLD;
    }

    // State transitions
    if human.unconscious {
        if human.concussion_of_the_brain < CONCUSSION_WAKEUP_THRESHOLD {
            if ctx.is_carried || ctx.is_tied {
                // Can't wake up while carried or tied.
                human.concussion_of_the_brain = CONCUSSION_WAKEUP_THRESHOLD;
                ConcussionOutcome::NoChange
            } else {
                // Wake up!
                human.unconscious = false;
                ConcussionOutcome::WokeUp
            }
        } else {
            ConcussionOutcome::NoChange
        }
    } else if human.concussion_of_the_brain >= CONCUSSION_THRESHOLD {
        // Go unconscious (KO).
        human.unconscious = true;
        // Start the healing timeout.
        if human.concussion_healing_timeout == 0 {
            // Will be set by the caller who knows the healing speed.
        }
        ConcussionOutcome::WentUnconscious
    } else {
        ConcussionOutcome::NoChange
    }
}

/// Add a concussion effect (positive) or healing (negative).
///
/// Positive values are scaled by life points via [`compute_concussion_effect`].
/// Negative values subtract directly (floored at 0).
pub fn add_concussion(
    human: &mut HumanData,
    amount: i16,
    life_points: i16,
    ctx: &ConcussionContext,
) -> ConcussionOutcome {
    if ctx.is_invulnerable || ctx.scroll_attached {
        return ConcussionOutcome::NoChange;
    }

    let new_value = if amount > 0 {
        compute_concussion_effect(human.concussion_of_the_brain, amount as u16, life_points)
    } else {
        let subtract = (-amount) as u16;
        human.concussion_of_the_brain.saturating_sub(subtract)
    };

    set_concussion(human, new_value, ctx)
}

/// Per-frame concussion healing tick. Call once per frame.
///
/// When concussion > 0 and `healing_speed > 0`, counts down a timeout.
/// When the timeout reaches zero, removes 1 point of concussion and resets
/// the timeout to `healing_speed`.
pub fn concussion_healing_tick(
    human: &mut HumanData,
    healing_speed: u16,
    life_points: i16,
    ctx: &ConcussionContext,
) {
    if human.concussion_of_the_brain == 0 || healing_speed == 0 {
        return;
    }

    if human.concussion_healing_timeout == 0 {
        // Heal 1 point of concussion.
        add_concussion(human, -1, life_points, ctx);
        human.concussion_healing_timeout = healing_speed;
    } else {
        human.concussion_healing_timeout -= 1;
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Sword damage
// ═══════════════════════════════════════════════════════════════════

/// Context about the defender for sword damage calculations.
#[derive(Debug, Clone, Copy)]
pub struct SwordDefenderContext {
    /// Defender's current action state (for parry detection).
    pub action_state: ActionState,
    /// Defender's direction (0–15 sector).
    pub direction: i16,
    /// Defender's world-Z elevation. Used by `get_sword_protection` to
    /// detect elevated defenders (stairs/balconies) and force `HIT_HEAD`
    /// protection.
    pub elevation: f32,
}

/// Context about the attacker for sword damage calculations.
#[derive(Debug, Clone, Copy)]
pub struct SwordAttackerContext {
    /// Attacker's direction (0–15 sector).
    pub direction: i16,
    /// Sector-vector from defender's ground position to attacker's
    /// (0–15 sector).
    pub direction_to_attacker: i16,
    /// Attacker's world-Z elevation. Paired with the defender's elevation
    /// inside `get_sword_protection` to trigger the elevated-defender branch.
    pub elevation: f32,
    /// Attacker's fighting ability (0–100).
    pub fighting_ability: u16,
    /// True if the attacker is a rank soldier (affects cutting damage scaling).
    pub is_rank_soldier: bool,
}

/// All parameters needed for a sword damage calculation.
#[derive(Debug)]
pub struct SwordDamageParams<'a> {
    pub defender: &'a SwordDefenderContext,
    pub defender_profile: Option<&'a HtHWeaponProfile>,
    pub attacker_profile: &'a HtHWeaponProfile,
    pub strike: SwordStrike,
    pub attacker: &'a SwordAttackerContext,
    pub concussion_ctx: &'a ConcussionContext,
    /// Max life points for the defender.
    pub max_life_points: i16,
}

/// Process incoming sword damage on a human.
///
/// Checks for parry, rolls protection against cutting and stunning,
/// applies damage to life points and concussion accordingly.
///
/// Returns a `SwordDamageResult` indicating which damage components
/// were applied, or empty if no damage was dealt.
pub fn receive_sword_damage(
    sim: &crate::sim_rng::SimulationContext,

    human: &mut HumanData,
    life_points: &mut i16,
    params: &SwordDamageParams<'_>,
) -> (SwordDamageResult, u16) {
    let SwordDamageParams {
        defender,
        defender_profile,
        attacker_profile,
        strike,
        attacker,
        concussion_ctx,
        max_life_points: _,
    } = params;
    let mut result = SwordDamageResult::empty();
    // Raw cutting damage attempted against this victim (before HP clamp).
    // Needed so the floating damage-number titbit displays the attempted
    // damage even for overkill hits, not just the HP actually subtracted.
    let mut cutting_inflicted: u16 = 0;

    // Check if the defender is parrying.
    let is_parrying = defender.action_state == ActionState::ParryingSwordLow
        || defender.action_state == ActionState::ParryingSword;

    if !is_parrying {
        // If the defender has a weapon (armor/protection)...
        if let Some(def_profile) = defender_profile {
            // --- Cutting damage ---
            // Roll against protection by localization.
            let protection = get_sword_protection(
                def_profile,
                defender.direction,
                attacker.direction_to_attacker,
                // ReceiveSwordDamage asks the defender's `mpSword` for both
                // its protection and the strike-direction classification.
                // The damage payload's sword remains authoritative for the
                // cutting and stunning values below.
                get_strike_direction(def_profile, *strike),
                attacker.elevation,
                defender.elevation,
            );
            let roll: u16 =
                crate::sim_rng::u16(sim, crate::sim_rng::RngSite::SwordDamageProtection, 1..=99);
            if roll > protection {
                let cutting = get_strike_cutting_effect(
                    attacker_profile,
                    *strike,
                    attacker.fighting_ability,
                    attacker.is_rank_soldier,
                );
                if cutting > 0 {
                    // A scroll-carrying civilian overrides the wounding
                    // primitive to a no-op, but the strike still counts as
                    // cutting damage for the caller's translation and
                    // sound decisions.
                    if !concussion_ctx.scroll_attached {
                        // RHElementActorPC::GetWounded ignores a lethal hit
                        // when the PC is already in an amulet coma. It still
                        // applies sublethal damage, and ReceiveSwordDamage
                        // still reports the cutting effect in either case.
                        let lethal_hit_in_coma = concussion_ctx.is_in_coma
                            && i32::from(cutting) >= i32::from(*life_points);
                        if !lethal_hit_in_coma {
                            get_wounded(
                                life_points,
                                cutting,
                                concussion_ctx.is_invulnerable,
                                params.max_life_points,
                                concussion_ctx.is_sherwood_pc,
                            );
                        }
                        cutting_inflicted = cutting;
                    }
                    result |= SwordDamageResult::CUTTING_DAMAGE;
                }
            }

            // --- Stunning damage ---
            let bludgeon_prot = def_profile.bludgeon_protection;
            let roll: u16 =
                crate::sim_rng::u16(sim, crate::sim_rng::RngSite::SwordDamageProtection, 1..=99);
            if roll > bludgeon_prot {
                let stunning = get_strike_stunning_effect(attacker_profile, *strike);
                if stunning > 0 {
                    add_concussion(human, stunning as i16, *life_points, concussion_ctx);
                    result |= SwordDamageResult::STUNNING_DAMAGE;
                }
            }
        } else {
            // No armor — take full cutting + stunning damage.
            result |= SwordDamageResult::CUTTING_DAMAGE | SwordDamageResult::STUNNING_DAMAGE;
        }
    } else if defender.action_state == ActionState::ParryingSwordLow || !strike.is_smalltalk() {
        // Parry successful.
        result |= SwordDamageResult::NO_DAMAGE_PARRIED;
    }

    (result, cutting_inflicted)
}

/// Compute sword protection value based on strike direction and defender orientation.
///
/// If the attacker stands at least [`ELEVATED_DEFENDER_THRESHOLD`] world-Z
/// units above the defender (attacker swinging down from stairs / balcony),
/// the direction-quadrant lookup is bypassed and `HIT_HEAD` is returned
/// unconditionally.
pub fn get_sword_protection(
    profile: &HtHWeaponProfile,
    defender_direction: i16,
    defender_to_attacker_direction: i16,
    thrust_direction: WeaponThrustDirection,
    attacker_elevation: f32,
    defender_elevation: f32,
) -> u16 {
    // Elevated-attacker override. Must be checked before the quadrant lookup.
    if attacker_elevation >= defender_elevation + ELEVATED_DEFENDER_THRESHOLD {
        return profile.protection_by_localization[HIT_HEAD];
    }

    // Calculate the strike direction modifier: `defender_to_attacker ± 4`
    // for true-circle / half-circle thrusts, else the raw defender→attacker
    // sector.
    let strike_direction = match thrust_direction {
        WeaponThrustDirection::LeftToRight => defender_to_attacker_direction + 4,
        WeaponThrustDirection::RightToLeft => defender_to_attacker_direction - 4,
        WeaponThrustDirection::NonApplicable => defender_to_attacker_direction,
    };

    let relative = ((strike_direction + 32 - defender_direction) & 15) as u16;

    // Map relative direction to hit localization index
    // (0=HIT_HEAD, 1=HIT_FRONT, 2=HIT_LEFT, 3=HIT_BACK, 4=HIT_RIGHT).
    // Shield protection is NOT applied here — shield blocking is handled
    // via the separate obstacle/bounding-box geometry (see `shield_obstacle`),
    // not as a modifier here.
    let localization = match relative {
        0 | 1 | 15 => HIT_FRONT,
        11..=14 => HIT_LEFT,
        6..=10 => HIT_BACK,
        2..=5 => HIT_RIGHT,
        _ => HIT_FRONT, // unreachable after `& 15`, but keeps the match total
    };

    profile.protection_by_localization[localization]
}

/// Indices into `HtHWeaponProfile::protection_by_localization`.
const HIT_HEAD: usize = 0;
const HIT_FRONT: usize = 1;
const HIT_LEFT: usize = 2;
const HIT_BACK: usize = 3;
const HIT_RIGHT: usize = 4;

/// Minimum world-Z difference (attacker above defender) at which the
/// direction-quadrant lookup is replaced by a forced `HIT_HEAD` return.
pub const ELEVATED_DEFENDER_THRESHOLD: f32 = 20.0;

/// Get the thrust direction for a given strike.
///
/// Lateral/circular strikes have a direction; straight/push/assault don't.
pub fn get_strike_direction(
    profile: &HtHWeaponProfile,
    strike: SwordStrike,
) -> WeaponThrustDirection {
    if strike.is_smalltalk() {
        return WeaponThrustDirection::NonApplicable;
    }
    let thrust = &profile.thrusts[strike as usize];
    match thrust.kind {
        WeaponThrustKind::Lateral
        | WeaponThrustKind::TrueHalfCircle
        | WeaponThrustKind::TrueCircle
        | WeaponThrustKind::FalseHalfCircle
        | WeaponThrustKind::FalseCircle => thrust.direction,
        _ => WeaponThrustDirection::NonApplicable,
    }
}

/// Get the cutting effect for a strike, scaled by attacker's fighting ability
/// if the attacker is a rank soldier.
pub fn get_strike_cutting_effect(
    profile: &HtHWeaponProfile,
    strike: SwordStrike,
    fighting_ability: u16,
    is_rank_soldier: bool,
) -> u16 {
    // Original assigns the playful back-hit a fixed one point of cutting
    // damage, independent of weapon profile and fighting ability.
    if strike.is_smalltalk() {
        return 1;
    }
    let base = profile.thrusts[strike as usize].cutting;

    let factor = if is_rank_soldier {
        1.0 + 0.01 * fighting_ability as f32
    } else {
        1.0
    };

    (base as f32 * factor) as u16
}

/// Get the stunning effect for a strike. Smalltalk hits never stun.
pub fn get_strike_stunning_effect(profile: &HtHWeaponProfile, strike: SwordStrike) -> u16 {
    if strike.is_smalltalk() {
        0
    } else {
        profile.thrusts[strike as usize].stunning
    }
}

/// Returns `true` if the strike has a push effect (push aside, circle, charge).
///
/// Used to decide whether to apply push-back movement vs. sword damage animation.
pub fn strike_has_push_effect(profile: &HtHWeaponProfile, strike: SwordStrike) -> bool {
    if strike.is_smalltalk() {
        return false;
    }
    let kind = profile.thrusts[strike as usize].kind;
    matches!(
        kind,
        WeaponThrustKind::PushAside | WeaponThrustKind::FalseCircle | WeaponThrustKind::TrueCircle
    ) || strike == SwordStrike::Charge
}

// ═══════════════════════════════════════════════════════════════════
//  Piercing damage (arrow / stone)
// ═══════════════════════════════════════════════════════════════════

/// Apply piercing damage (arrows, stones). Applies both wounding damage
/// and concussion.
///
/// Returns `true` if the entity died.
pub fn receive_piercing_damage(
    human: &mut HumanData,
    life_points: &mut i16,
    damage: u16,
    concussion: u16,
    max_life_points: i16,
    ctx: &ConcussionContext,
) -> bool {
    // RHElementActorPC::GetWounded ignores an otherwise-lethal hit while
    // the PC is already in an amulet coma. ReceivePiercingDamage still
    // proceeds to AddConcussionOfTheBrain, and sublethal wounds still use
    // the ordinary base-human implementation.
    let lethal_hit_in_coma = ctx.is_in_coma && i32::from(damage) >= i32::from(*life_points);
    let died = if lethal_hit_in_coma {
        false
    } else {
        get_wounded(
            life_points,
            damage,
            ctx.is_invulnerable,
            max_life_points,
            ctx.is_sherwood_pc,
        )
    };
    add_concussion(human, concussion as i16, *life_points, ctx);
    died
}

// ═══════════════════════════════════════════════════════════════════
//  Hit damage (fist / club)
// ═══════════════════════════════════════════════════════════════════

/// Apply hit damage (fist punch, club). Only concussion, no cutting.
///
/// Returns the concussion outcome.
pub fn receive_hit_damage(
    human: &mut HumanData,
    life_points: i16,
    concussion: u16,
    ctx: &ConcussionContext,
) -> ConcussionOutcome {
    // The damage element already carries the final concussion payload.
    // Original applies Hard's 1.5 multiplier only while a PC authors a
    // HITTING damage element; RHCOMMAND_RECEIVE_HIT_DAMAGE consumes the
    // stored value verbatim. This also matters for NPC and domino-authored
    // elements, which must never be reinterpreted based on the victim camp.
    add_concussion(human, concussion as i16, life_points, ctx)
}

// ═══════════════════════════════════════════════════════════════════
//  Generic damage (falling, environmental)
// ═══════════════════════════════════════════════════════════════════

/// Apply generic damage (concussion + wounding). Used for falls,
/// mobile collisions, etc.
///
/// Returns `true` if the entity died.
pub fn receive_generic_damage(
    human: &mut HumanData,
    life_points: &mut i16,
    damage: u16,
    concussion: u16,
    max_life_points: i16,
    ctx: &ConcussionContext,
) -> bool {
    add_concussion(human, concussion as i16, *life_points, ctx);
    get_wounded(
        life_points,
        damage,
        ctx.is_invulnerable,
        max_life_points,
        ctx.is_sherwood_pc,
    )
}

// ═══════════════════════════════════════════════════════════════════
//  Tie-up mechanics
// ═══════════════════════════════════════════════════════════════════

/// Tie up an unconscious human. Sets posture to `Tied` and ensures
/// concussion stays at wakeup threshold so they don't wake up.
///
/// Panics if the entity is not unconscious.
pub fn tie_up(human: &mut HumanData, posture: &mut Posture) {
    assert!(human.unconscious, "cannot tie up a conscious entity");
    if posture.allows_transition_to(Posture::Tied) {
        *posture = Posture::Tied;
    }
    // Ensure concussion stays high enough to prevent waking.
    if human.concussion_of_the_brain < CONCUSSION_WAKEUP_THRESHOLD {
        human.concussion_of_the_brain = CONCUSSION_WAKEUP_THRESHOLD;
    }
}

/// Release a tied-up human. Resets posture to `Lying`.
/// The entity remains unconscious until concussion heals below wakeup threshold.
pub fn untie(_human: &mut HumanData, posture: &mut Posture) {
    assert_eq!(*posture, Posture::Tied, "cannot untie an untied entity");
    assert!(
        posture.allows_transition_to(Posture::Lying),
        "Tied posture must allow release to Lying"
    );
    *posture = Posture::Lying;
    // Don't alter unconsciousness or concussion — natural healing owns wakeup.
}

/// Increment the stuck-under-nets counter without touching posture.
///
/// The eager counter bump that happens at capture time, before the
/// `Command::ReceiveNet` damage element runs and snaps the posture.
pub fn increment_stuck_under_net(human: &mut HumanData) {
    human.stuck_under_nets_counter += 1;
}

/// Snap posture to `StuckUnderNet`.
///
/// Runs the frame after `apply_net`'s eager counter bump, not eagerly.
pub fn set_posture_stuck_under_net(posture: &mut Posture) {
    if posture.allows_transition_to(Posture::StuckUnderNet) {
        *posture = Posture::StuckUnderNet;
    }
}

/// Apply a net to a human atomically (counter increment + posture snap).
///
/// Convenience wrapper for callers that don't need the eager-counter /
/// next-frame-posture split — direct script natives, tests, and any path
/// that lands a victim under a net without going through the
/// `Command::ReceiveNet` damage pipeline.
pub fn apply_net(human: &mut HumanData, posture: &mut Posture) {
    increment_stuck_under_net(human);
    set_posture_stuck_under_net(posture);
}

/// Remove one net layer from a human. Decrements counter; if zero,
/// reverts posture to `Lying`.
pub fn remove_net(human: &mut HumanData, posture: &mut Posture) {
    if human.stuck_under_nets_counter > 0 {
        human.stuck_under_nets_counter -= 1;
    }
    if human.stuck_under_nets_counter == 0
        && *posture == Posture::StuckUnderNet
        && posture.allows_transition_to(Posture::Lying)
    {
        *posture = Posture::Lying;
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Combat utility
// ═══════════════════════════════════════════════════════════════════

/// Compute relative fighting ability (0–100) of `own_ability` against the
/// sum of all opponents' abilities.
///
/// Returns 50 if equal, >50 if stronger, <50 if weaker.
pub fn compute_relative_fighting_ability(own_ability: u16, opponents_total_ability: u16) -> u16 {
    if own_ability == opponents_total_ability {
        50
    } else {
        let total = own_ability as u32 + opponents_total_ability as u32;
        if total == 0 {
            return 50;
        }
        ((100 * own_ability as u32) / total) as u16
    }
}

/// Check if a target is within melee range of the attacker's weapon.
///
/// `distance` is the ground-plane distance between attacker and target.
/// Returns `true` if `distance` is in `(minimal, maximal]` for the weapon.
pub fn is_in_melee_range(_sword: &SwordState, profile: &HtHWeaponProfile, distance: f32) -> bool {
    let min = profile.distance[0] as f32; // MINIMAL
    let max = profile.distance[2] as f32; // MAXIMAL
    distance > min && distance <= max
}

/// Check if a specific strike can reach a target at the given distance.
pub fn is_strike_in_range(profile: &HtHWeaponProfile, strike: SwordStrike, distance: f32) -> bool {
    let thrust = &profile.thrusts[strike as usize];
    distance >= thrust.minimal_distance as f32 && distance <= thrust.maximal_distance as f32
}

/// Check if a target is within bow range.
///
/// `distance` is the ground-plane distance. Returns `true` if the bow
/// can reach (in either normal or long shoot mode).
pub fn is_in_bow_range(max_bow_range: u16, distance: f32) -> bool {
    distance <= max_bow_range as f32
}

/// Energy cost of performing a sword strike.
/// Returns the tiredness increase from executing the strike.
pub fn strike_energy_cost(profile: &HtHWeaponProfile, strike: SwordStrike) -> u16 {
    profile.thrusts[strike as usize].energy
}

/// Apply a completed strike's energy to Original's serialized `UWORD`
/// tiredness accumulator.
///
/// `muwTiredness += GetStrikeEnergy(...)` performs the addition after integer
/// promotion and then narrows back to the unsigned 16-bit member. Preserve
/// that modulo-2^16 assignment rather than saturating at the Rust type bound.
pub fn add_strike_tiredness(current_tiredness: u16, strike_energy: u16) -> u16 {
    current_tiredness.wrapping_add(strike_energy)
}

/// Rust-side mirror of the Original's `PARITY_DEBUG_ORIGINAL_TIREDNESS` probe
/// (`original-code/RHelementactorhuman.cpp:205-217`).
///
/// `PARITY_DEBUG_TIREDNESS` enables it; the optional
/// `PARITY_DEBUG_TIREDNESS_CREATION_ORDER` narrows it to one actor, matched on
/// the Original `GetCreationOrder()` value so the two trails line up
/// frame-for-frame. Every `HumanData::tiredness` write and the
/// `EvaluateSwordfight` threshold read prints one `RUST_TIREDNESS` line.
pub fn tiredness_debug_enabled() -> bool {
    tiredness_debug_filter().is_some()
}

pub fn tiredness_debug_matches(creation_order: u32) -> bool {
    match tiredness_debug_filter() {
        None => false,
        Some(None) => true,
        Some(Some(wanted)) => *wanted == creation_order,
    }
}

/// `None` when the probe is off, `Some(None)` for every actor, `Some(Some(co))`
/// for a single creation order. Resolved once so the per-entity, per-frame
/// write sites do not pay for an environment scan.
fn tiredness_debug_filter() -> &'static Option<Option<u32>> {
    static FILTER: std::sync::OnceLock<Option<Option<u32>>> = std::sync::OnceLock::new();
    FILTER.get_or_init(|| {
        std::env::var_os("PARITY_DEBUG_TIREDNESS")?;
        Some(
            std::env::var("PARITY_DEBUG_TIREDNESS_CREATION_ORDER")
                .ok()
                .map(|raw| {
                    raw.parse::<u32>().unwrap_or_else(|error| {
                        panic!("invalid PARITY_DEBUG_TIREDNESS_CREATION_ORDER: {error}")
                    })
                }),
        )
    })
}

/// Tiredness recovery per frame when not fighting or moving.
/// Based on endurance stat.
pub fn tiredness_recovery(current_tiredness: u16, endurance: u16) -> u16 {
    let recovery = endurance / 10;
    current_tiredness.saturating_sub(recovery)
}

// ═══════════════════════════════════════════════════════════════════
//  Sword strike selection
// ═══════════════════════════════════════════════════════════════════

/// Minimum fighting ability required per strike (A–I).
const SWORD_STRIKE_MIN_SKILL: [u8; NUM_NORMAL_SWORD_STRIKES] = [
    0,  // A: simple
    40, // B: strong
    95, // C: lethal
    20, // D: lateral left
    20, // E: lateral right
    70, // F: half circle left
    70, // G: half circle right
    80, // H: circle left
    80, // I: circle right
];

/// Maximum blood alcohol level per strike (A–I).
const SWORD_STRIKE_MAX_ALCOHOL: [u8; NUM_NORMAL_SWORD_STRIKES] = [
    80, // A: simple
    50, // B: strong
    0,  // C: lethal
    20, // D: lateral left
    20, // E: lateral right
    0,  // F: half circle left
    0,  // G: half circle right
    80, // H: circle left
    80, // I: circle right
];

const SWORD_STRIKE_BOREDOM_DECREMENTATION: u16 = 10;
const SWORD_STRIKE_BOREDOM_INCREMENTATION: u16 = 50;
const SWORD_STRIKE_BOREDOM_MALUS_FACTOR: f32 = 3.0;
const SWORD_STRIKE_HIT_BONUS: i16 = 30;

/// The 9 normal (non-charge) strikes in index order.
pub const NORMAL_STRIKES: [SwordStrike; NUM_NORMAL_SWORD_STRIKES] = [
    SwordStrike::A,
    SwordStrike::B,
    SwordStrike::C,
    SwordStrike::D,
    SwordStrike::E,
    SwordStrike::F,
    SwordStrike::G,
    SwordStrike::H,
    SwordStrike::I,
];

/// Returns true if the strike is a group/area strike (semiround or round).
/// Group strikes require more than one target to be considered optimal.
fn is_group_strike(strike: SwordStrike) -> bool {
    matches!(
        strike,
        SwordStrike::F | SwordStrike::G | SwordStrike::H | SwordStrike::I
    )
}

// ─── Sector math (shared with engine/melee.rs) ──────────────────────

/// Convert a 0-15 direction sector to angle in radians.
///
/// Original's trailing `+ 0.1` nudges the beginning edge just inside the
/// sector so `AngleToSector` round-trips it to the same sector.
fn sector_to_angle(sector: i16) -> f32 {
    // Original's unsuffixed literals promote the UBYTE sector and f32 PI
    // constant to double for the whole expression, then narrow on return.
    ((f64::from(sector) / 16.0) * 2.0 * f64::from(PI) + 0.1) as f32
}

/// Convert an angle in radians to a 0-15 sector.
///
/// Positive angles use Original's truncating ULONG conversion and modulo.
/// Negative angles use its recursive mirror rule, including its asymmetric
/// treatment of exact negative sector boundaries.
fn angle_to_sector(angle: f32) -> u8 {
    if angle >= 0.0 {
        // As in Original, the unsuffixed constants keep this calculation in
        // double until the truncating ULONG cast.
        ((f64::from(angle) / (2.0 * f64::from(PI)) * 16.0) as u32 % 16) as u8
    } else {
        16 - angle_to_sector(-angle) - 1
    }
}

/// Check if `sector` is between `begin` and `end` (inclusive, wrapping 0-15).
fn is_sector_between(sector: u8, begin: u8, end: u8) -> bool {
    if begin <= end {
        sector >= begin && sector <= end
    } else {
        sector >= begin || sector <= end
    }
}

// ─── Multi-victim strike estimation ────────────────────────────────

/// Build the separately normalized forward and side axes used by Original's
/// push-strike rectangle.
///
/// The side axis is a copied direction vector rotated through
/// `SBGeoVector2D::Rotate(PI / 2.0)`. That method receives a FLOAT angle but
/// evaluates `sin`/`cos` and its component expressions in double precision,
/// so it is not bit-identical to the `(-forward.y, forward.x)` shortcut.
pub(crate) fn push_strike_basis(direction: i16) -> ((f32, f32), (f32, f32)) {
    let (forward_x, base_y) = crate::element_kinds::direction_vector_16(direction);
    let forward_y =
        base_y * ASPECT_RATIO * crate::position_interface::INVERSE_SWORDFIGHT_ASPECT_RATIO;

    let angle = (f64::from(PI) / 2.0) as f32;
    let sine = f64::from(angle).sin();
    let cosine = f64::from(angle).cos();
    let side_x = (f64::from(forward_x) * cosine - f64::from(forward_y) * sine) as f32;
    let side_y = (f64::from(forward_x) * sine + f64::from(forward_y) * cosine) as f32;

    let forward_square_norm = forward_x * forward_x + forward_y * forward_y;
    let forward_norm = f64::from(forward_square_norm).sqrt() as f32;
    let side_square_norm = side_x * side_x + side_y * side_y;
    let side_norm = f64::from(side_square_norm).sqrt() as f32;
    assert!(forward_norm != 0.0 && side_norm != 0.0);

    (
        (forward_x / forward_norm, forward_y / forward_norm),
        (side_x / side_norm, side_y / side_norm),
    )
}

/// A nearby entity that might be hit by a sword strike, pre-collected
/// by the caller from the entity list. Positions are relative to the
/// attacker, with Y stretched by `INVERSE_SWORDFIGHT_ASPECT_RATIO`.
pub struct NearbyVictim<'a> {
    /// Whether the human is currently active. Original's straight-strike
    /// collector bypasses `IsPossibleSwordStrikeVictim` and can therefore
    /// retain an inactive principal opponent, while the lateral collector
    /// explicitly requires active humans.
    pub is_active: bool,
    /// Whether Original's `IsPossibleSwordStrikeVictim` accepts this human.
    /// Lateral strike collection deliberately ignores this predicate and
    /// scans every active human; straight strikes use their separate
    /// principal-opponent collector, while the remaining damaging families
    /// require regular eligibility.
    pub eligible_for_regular_strikes: bool,
    /// Relative X (victim.x - attacker.x).
    pub dx: f32,
    /// Relative Y, stretched for isometric (dy * INVERSE_SWORDFIGHT_ASPECT_RATIO).
    pub dy_stretched: f32,
    /// Euclidean distance in stretched coordinates.
    pub distance: f32,
    /// Direction sector (0-15) from attacker to victim.
    pub direction_sector: u8,
    /// Victim's camp.
    pub camp: Camp,
    /// Victim's facing direction (0-15 sector).
    pub facing_direction: i16,
    /// Victim's world-Z elevation. Feeds the elevated-defender branch of
    /// `get_sword_protection` during strike-damage estimation.
    pub elevation: f32,
    /// Victim's remaining life points.
    pub life_points: i16,
    /// Victim's defensive weapon profile.
    pub defender_profile: Option<&'a HtHWeaponProfile>,
    /// Whether this is the attacker's principal opponent / primary target.
    pub is_primary_target: bool,
    /// Whether the victim is currently walking with a sword (used for
    /// circle-strike approach tolerance in the warn-AI mode).
    pub is_walking_with_sword: bool,
}

/// Check if a victim is within the geometric arc of a given strike.
///
/// Dispatches by thrust kind (straight, lateral, push, half-circle, circle).
///
/// This is the AI-strike-damage-estimation branch (no warn-AI tolerance).
/// The warn-AI tolerance (circle-strike extension for walking enemies) lives
/// in `engine::melee::collect_circle_warn_victims`, which the actual
/// warn-for-strike phase reaches through `execute_multi_target_strike`.
fn is_victim_in_strike_arc(
    profile: &HtHWeaponProfile,
    strike: SwordStrike,
    attacker_direction: i16,
    victim: &NearbyVictim,
    is_swordfighting: bool,
) -> bool {
    let thrust = &profile.thrusts[strike as usize];
    let min_d = thrust.minimal_distance as f32;
    let max_d = thrust.maximal_distance as f32;
    let max_norm = victim.dx.abs().max(victim.dy_stretched.abs());

    match thrust.kind {
        // Straight strikes only hit the principal opponent (when swordfighting).
        WeaponThrustKind::Straight | WeaponThrustKind::Assault => {
            if is_swordfighting {
                // Only the principal opponent.
                victim.is_primary_target && victim.distance >= min_d && victim.distance <= max_d
            } else {
                victim.distance >= min_d && victim.distance <= max_d
            }
        }

        // Lateral: angular arc between initial and final angles.
        //
        // Straddles the facing direction by combining one positive offset
        // with one negative offset, producing a front-facing wedge that
        // *contains* the attacker's facing. R→L: begin = facing - final,
        // end = facing + initial. L→R: begin = facing - initial,
        // end = facing + final.
        WeaponThrustKind::Lateral => {
            if !victim.is_active {
                return false;
            }
            // Original enters the collector only when `MaxNorm() < 150`,
            // then admits the victim only when both inclusive range
            // comparisons succeed.  Keep that positive formulation: a
            // session-restored actor can carry NaN coordinates, for which
            // all three Original comparisons are false.  Negating the
            // rejection predicates instead let NaN actors into the arc.
            if !(max_norm < 150.0 && victim.distance >= min_d && victim.distance <= max_d) {
                return false;
            }
            let dir_angle = sector_to_angle(attacker_direction);
            let initial = degrees_to_radians(thrust.initial_angle);
            let final_a = degrees_to_radians(thrust.final_angle);
            let (begin, end) = match thrust.direction {
                WeaponThrustDirection::RightToLeft => (
                    angle_to_sector(dir_angle - final_a),
                    angle_to_sector(dir_angle + initial),
                ),
                _ => (
                    angle_to_sector(dir_angle - initial),
                    angle_to_sector(dir_angle + final_a),
                ),
            };
            is_sector_between(victim.direction_sector, begin, end)
        }

        // Push: rectangle geometry.
        //
        // Original GetDirectionVector starts from the literal 16-sector
        // table and applies ordinary ASPECT_RATIO to Y; the collector then
        // applies the shipping no-op INVERSE_SWORDFIGHT_ASPECT_RATIO and
        // normalises. Do not use `sector_to_angle` here: its +0.1 radian
        // roundtrip nudge rotates this narrow rectangle enough to admit
        // actors immediately beyond a side boundary.
        WeaponThrustKind::PushAside => {
            if max_norm >= 150.0 {
                return false;
            }
            let ((fx, fy), (sx, sy)) = push_strike_basis(attacker_direction);
            let front_dist = victim.dx * fx + victim.dy_stretched * fy;
            let side_dist = (victim.dx * sx + victim.dy_stretched * sy).abs();
            // Original stores this in an ULONG and divides before comparing
            // against the floating-point side distance, so odd widths truncate.
            let half_width = (thrust.repulsion / 2) as f32;
            front_dist >= min_d && front_dist <= max_d && side_dist <= half_width
        }

        // Half-circle: 180° arc.
        //
        // R→L uses `+initial` for both endpoints offset by ±π so wrap is
        // benign; L→R uses `-initial` and the 180° arc extends in the +π
        // direction from there.
        WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => {
            // GetPossibleVictimsOfHalfCircleSwordStrike uses the same
            // positive MaxNorm/range guards as the lateral collector, with
            // the same unordered-NaN behavior.
            if !(max_norm < 150.0 && victim.distance >= min_d && victim.distance <= max_d) {
                return false;
            }
            let dir_angle = sector_to_angle(attacker_direction);
            let initial = degrees_to_radians(thrust.initial_angle);
            let (begin, end) = match thrust.direction {
                WeaponThrustDirection::RightToLeft => {
                    // initial' = facing + initial, final' = -π + initial';
                    // begin = sector(final'), end = sector(initial').
                    let final_a = -PI + initial;
                    (
                        angle_to_sector(dir_angle + final_a),
                        angle_to_sector(dir_angle + initial),
                    )
                }
                _ => {
                    // initial' = facing - initial, final' = π + initial';
                    // begin = sector(initial'), end = sector(final').
                    let initial_signed = -initial;
                    let final_a = PI + initial_signed;
                    (
                        angle_to_sector(dir_angle + initial_signed),
                        angle_to_sector(dir_angle + final_a),
                    )
                }
            };
            is_sector_between(victim.direction_sector, begin, end)
        }

        // Circle: omnidirectional, distance-only.
        WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => {
            // Original applies a strict MaxNorm < 150 quick reject, then
            // distance <= max_d. Min distance is not checked for circles.
            // The walking-enemy tolerance is not applied in this
            // damage-estimation context.
            max_norm < 150.0 && victim.distance <= max_d
        }
    }
}

/// Convert degrees to radians (profile stores integer degrees).
fn degrees_to_radians(degrees: u16) -> f32 {
    // `degrees / 360.0 * 2.0 * PI` is double-promoted in Original and
    // narrowed only by the FLOAT return value.
    ((f64::from(degrees) / 360.0) * 2.0 * f64::from(PI)) as f32
}

/// Estimate damage of a single strike against a single victim.
///
/// Replicates the original copy-paste bug where the concussion value gets
/// overwritten by the cutting modified value when cutting > 0.
fn estimate_damage_of_this_strike(
    attacker_profile: &HtHWeaponProfile,
    strike: SwordStrike,
    fighting_ability: u16,
    is_rank_soldier: bool,
    victim_to_attacker: i16,
    attacker_elevation: f32,
    victim: &NearbyVictim,
) -> u16 {
    let mut damage =
        get_strike_cutting_effect(attacker_profile, strike, fighting_ability, is_rank_soldier);

    // Cap at victim's remaining HP.
    if (damage as i16) > victim.life_points {
        damage = victim.life_points.max(0) as u16;
    }

    let strike_dir = get_strike_direction(attacker_profile, strike);

    // Apply armor protection.
    let mut modified_cutting: u16 = 0;
    if let Some(def_prof) = victim.defender_profile {
        let protection = get_sword_protection(
            def_prof,
            victim.facing_direction,
            victim_to_attacker,
            strike_dir,
            attacker_elevation,
            victim.elevation,
        );
        modified_cutting = (damage as f32 * 0.01 * (100.0 - protection as f32).max(0.0)) as u16;
        if modified_cutting > 0 {
            damage = modified_cutting;
        }
    }

    // Concussion estimate.
    let mut concussion = attacker_profile.thrusts[strike as usize].stunning;
    if let Some(def_prof) = victim.defender_profile {
        let bludgeon_prot = def_prof.bludgeon_protection;
        concussion = (concussion as f32 * 0.01 * (100.0 - bludgeon_prot as f32).max(0.0)) as u16;
        // BUG (replicated for behavioral fidelity): if cutting-after-protection
        // was nonzero, concussion gets overwritten with that value.
        if modified_cutting > 0 {
            concussion = modified_cutting;
        }
    }

    damage + concussion
}

/// Direction used by `RHSword::GetProtection` during strike estimation.
///
/// Sword hit arcs use projected map coordinates with the shipping
/// `INVERSE_SWORDFIGHT_ASPECT_RATIO` (1.0), but armor localization is a
/// separate calculation in Original: `GetProtection` subtracts the actors'
/// unprojected `GetPositionGround()` points and calls
/// `GetSector0to15(ASPECT_RATIO)`. Reconstruct world Y from map Y + elevation
/// before applying that ordinary isometric sector classifier.
fn protection_direction_to_attacker(attacker_elevation: f32, victim: &NearbyVictim<'_>) -> i16 {
    let victim_to_attacker_x = -victim.dx;
    let victim_to_attacker_world_y = -victim.dy_stretched + attacker_elevation - victim.elevation;
    crate::position_interface::vector_to_sector_0_to_15_iso(
        victim_to_attacker_x,
        victim_to_attacker_world_y,
    )
}

/// Estimate total damage of a strike against all nearby victims.
///
/// Returns `(overall_damage, num_victims)` where `num_victims == -1`
/// means the strike would hit a friendly (abort!).
fn estimate_damage_of_sword_strike(
    ctx: &StrikeSelectionContext,
    strike: SwordStrike,
    is_drunken: bool,
    nearby: &[NearbyVictim],
) -> (i16, i16) {
    let attacker_profile = ctx.attacker_profile;
    let fighting_ability = ctx.fighting_ability;
    let is_rank_soldier = ctx.is_rank_soldier;
    let attacker_direction = ctx.attacker_direction;
    let attacker_elevation = ctx.attacker_elevation;
    let attacker_camp = ctx.attacker_camp;
    let is_swordfighting = ctx.is_swordfighting;
    let mut overall_damage: u16 = 0;
    let mut num_victims: i16 = 0;

    for victim in nearby {
        let thrust_kind = ctx.attacker_profile.thrusts[strike as usize].kind;
        if !matches!(
            thrust_kind,
            WeaponThrustKind::Straight | WeaponThrustKind::Assault | WeaponThrustKind::Lateral
        ) && !victim.eligible_for_regular_strikes
        {
            continue;
        }
        // Strike selection uses the no-warn-AI arc check.
        if !is_victim_in_strike_arc(
            attacker_profile,
            strike,
            attacker_direction,
            victim,
            is_swordfighting,
        ) {
            continue;
        }

        // Friendly fire check.
        if victim.camp == attacker_camp && !is_drunken {
            return (0, -1);
        }

        let victim_to_attacker = protection_direction_to_attacker(attacker_elevation, victim);
        let dmg = estimate_damage_of_this_strike(
            attacker_profile,
            strike,
            fighting_ability,
            is_rank_soldier,
            victim_to_attacker,
            attacker_elevation,
            victim,
        );
        overall_damage += dmg;
        if dmg > 0 {
            num_victims += 1;
            overall_damage += SWORD_STRIKE_HIT_BONUS as u16;
        }
    }

    (overall_damage as i16, num_victims)
}

/// Context for strike selection — describes the attacker.
pub struct StrikeSelectionContext<'a> {
    pub attacker_profile: &'a HtHWeaponProfile,
    pub fighting_ability: u16,
    pub blood_alcohol: u8,
    pub is_rank_soldier: bool,
    pub attacker_direction: i16,
    /// Attacker's world-Z elevation. Feeds per-victim
    /// `get_sword_protection` calls during estimation so the
    /// elevated-defender branch fires consistently with the live
    /// damage path.
    pub attacker_elevation: f32,
    pub attacker_camp: Camp,
    /// Whether the attacker is currently in a swordfight, i.e. holds a
    /// non-empty opponent list. False aborts the whole proposal (after the
    /// skill draw); when true it also narrows straight-strike targeting to
    /// the principal opponent.
    pub is_swordfighting: bool,
    /// Frames remaining until the opponent's current action completes.
    /// Used to reject strikes whose startup animation would be too slow.
    /// Set to `None` (= unlimited) when the sprite system hasn't provided
    /// timing data; defaults to 1000 when the opponent has no strike.
    pub opponent_time_limit: Option<i16>,
    /// Per-strike startup frame counts from the attacker's sprite data
    /// (frames-from-start-till-action-done for each strike animation).
    /// When `None`, falls back to the hardcoded [`STRIKE_STARTUP_FRAMES`]
    /// estimates.
    pub strike_startup_frames: Option<[i16; NUM_NORMAL_SWORD_STRIKES]>,
    /// Startup frames for the waiting→parrying transition animation.
    /// When `None`, falls back to [`PARRY_STARTUP_FRAMES`].
    pub parry_startup_frames: Option<i16>,
    /// Whether the entity calling this is an NPC (soldier).
    /// NPCs always get parade fallback; PCs require a second
    /// `fighting_ability` roll.
    pub is_npc: bool,
}

/// Rough startup frame count per strike animation.
///
/// Callers read the real values from the sprite system when available
/// (via `StrikeSelectionContext::strike_startup_frames`); these constants
/// serve as a fallback when sprite data is missing.
pub const STRIKE_STARTUP_FRAMES: [i16; NUM_NORMAL_SWORD_STRIKES] = [
    15, // A: straight — fast
    20, // B: strong straight — medium
    25, // C: execution — slow
    18, // D: lateral left — medium
    18, // E: lateral right — medium
    22, // F: semiround left — medium-slow
    22, // G: semiround right — medium-slow
    30, // H: round left — slow
    30, // I: round right — slow
];

/// Result of [`propose_good_sword_strike`] — either an offensive strike
/// or a defensive parry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposedCombatAction {
    Strike(SwordStrike),
    Parry,
}

/// Process-local label for the opt-in reactive sword-selection diagnostic.
/// This is deliberately not part of any serialized simulation structure.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SwordStrikeProposalDebug {
    pub frame: u32,
    pub victim: u32,
    pub victim_creation_order: u32,
    pub attacker: u32,
}

/// Rough startup frames for the waiting→parrying transition animation.
/// Callers read the real value from sprite data when available
/// (via `StrikeSelectionContext::parry_startup_frames`).
const PARRY_STARTUP_FRAMES: i16 = 10;

/// Propose the best sword strike (or parry) for an NPC.
///
/// Scans all `nearby` victims per strike to estimate damage and avoid
/// friendly fire. Returns the chosen action, or `None` if nothing viable.
///
/// When `also_parade` is true and no good strike is found (or the skill
/// gate fails), proposes a parry if the parry animation can start within
/// the opponent's time limit.
///
/// `sword_strike_boredom` will be grown to `NUM_NORMAL_SWORD_STRIKES`
/// entries if undersized.
pub fn propose_good_sword_strike(
    sim: &crate::sim_rng::SimulationContext,

    ctx: &StrikeSelectionContext,
    nearby: &[NearbyVictim],
    sword_strike_boredom: &mut Vec<u16>,
    also_parade: bool,
) -> Option<ProposedCombatAction> {
    propose_good_sword_strike_with_debug(
        sim,
        ctx,
        nearby,
        sword_strike_boredom,
        also_parade,
        false,
        None,
        &mut None,
    )
}

/// Out-parameter for the sweep-geometry side effect of strike estimation.
///
/// `RHElementActorHuman::EstimateDamageOfSwordStrike`
/// (`original-code/RHelementactorhuman.cpp:12875`) builds its candidate
/// victim list through `GetPossibleVictimsOfSwordStrike`, and the lateral
/// and half-circle collectors
/// (`original-code/RHelementactorhuman.cpp:10854` and `:10941`) are not
/// side-effect free: both overwrite the human-owned
/// `mfInitialStrikeAngle` / `mfFinalStrikeAngle` / `mfCurrentStrikeAngle`
/// before they scan. Proposing a strike therefore rebases whatever sweep
/// geometry an earlier, interrupted lateral/circle strike left behind, and
/// the retained victim list is later tested against the *proposal's*
/// geometry. `GetPossibleVictimsOfCircleSwordStrike`
/// (`:10781`), the straight collector and the push collector write nothing,
/// so only the last lateral/half-circle candidate that actually reached
/// estimation is reported here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrikeSelectionSweepRebase {
    /// Strike whose collector performed the final angle write.
    pub strike: SwordStrike,
}

pub(crate) fn propose_good_sword_strike_with_debug(
    sim: &crate::sim_rng::SimulationContext,
    ctx: &StrikeSelectionContext,
    nearby: &[NearbyVictim],
    sword_strike_boredom: &mut Vec<u16>,
    also_parade: bool,
    // Custom-mission AI-controlled heroes use the ordinary strike evaluator but
    // should not spend half their turns failing its initial hesitation roll.
    // Candidate skill requirements and every other combat rule still apply.
    skip_initial_skill_gate: bool,
    debug: Option<SwordStrikeProposalDebug>,
    sweep_rebase: &mut Option<StrikeSelectionSweepRebase>,
) -> Option<ProposedCombatAction> {
    // Ensure boredom array is properly sized.
    if sword_strike_boredom.len() < NUM_NORMAL_SWORD_STRIKES {
        sword_strike_boredom.resize(NUM_NORMAL_SWORD_STRIKES, 0);
    }

    let time_limit = ctx.opponent_time_limit.unwrap_or(1000);

    // Skill gate: the better you fight, the more likely you attempt a
    // special strike. `(rand() % 100) >= max(50, fighting_ability)`.
    let threshold = ctx.fighting_ability.max(50) as u32;
    let mut only_parade = false;
    let skill_roll =
        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SwordStrikeSelection, 0..100);
    if let Some(debug) = debug {
        eprintln!(
            "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=proposal_input skill={} blood={} direction={} elevation_bits={:#010x} camp={:?} swordfighting={} time_limit={} startup={:?} parry_startup={:?} also_parade={} skill_roll={} threshold={} boredom={:?} nearby_count={}]",
            debug.frame,
            debug.victim_creation_order,
            debug.victim,
            debug.attacker,
            ctx.fighting_ability,
            ctx.blood_alcohol,
            ctx.attacker_direction,
            ctx.attacker_elevation.to_bits(),
            ctx.attacker_camp,
            ctx.is_swordfighting,
            time_limit,
            ctx.strike_startup_frames,
            ctx.parry_startup_frames,
            also_parade,
            skill_roll,
            threshold,
            sword_strike_boredom,
            nearby.len(),
        );
        for (index, victim) in nearby.iter().enumerate() {
            eprintln!(
                "[REACTIVE_SWORD frame={} co={} victim={} phase=nearby index={} eligible={} dx_bits={:#010x} dy_bits={:#010x} distance_bits={:#010x} sector={} camp={:?} facing={} elevation_bits={:#010x} life={} primary={} walking_sword={}]",
                debug.frame,
                debug.victim_creation_order,
                debug.victim,
                index,
                victim.eligible_for_regular_strikes,
                victim.dx.to_bits(),
                victim.dy_stretched.to_bits(),
                victim.distance.to_bits(),
                victim.direction_sector,
                victim.camp,
                victim.facing_direction,
                victim.elevation.to_bits(),
                victim.life_points,
                victim.is_primary_target,
                victim.is_walking_with_sword,
            );
        }
    }
    if initial_strike_skill_gate_rejects(skill_roll, threshold, skip_initial_skill_gate) {
        if also_parade {
            // NPCs always get parade fallback. PCs need a second
            // fighting_ability roll — higher skill means they're more likely
            // to retry a strike next time rather than fall back to parry.
            if ctx.is_npc
                || crate::sim_rng::u32(sim, crate::sim_rng::RngSite::SwordStrikeSelection, 0..100)
                    >= ctx.fighting_ability as u32
            {
                only_parade = true;
            } else {
                return None;
            }
        } else {
            return None;
        }
    }

    // An actor with no live opponent has nothing to propose against. The
    // bail-out sits after the skill draw (so the RNG stream advances either
    // way) and before any boredom bookkeeping.
    if !ctx.is_swordfighting {
        return None;
    }

    let mut best_strike: Option<SwordStrike> = None;

    if !only_parade {
        // Decrement boredom for all strikes.
        for boredom in sword_strike_boredom.iter_mut() {
            *boredom = boredom.saturating_sub(SWORD_STRIKE_BOREDOM_DECREMENTATION);
        }

        let is_drunken = ctx.blood_alcohol > 0;

        let mut best_damage: i16 = 0;

        for (i, &strike) in NORMAL_STRIKES.iter().enumerate() {
            // Skill/alcohol gating per strike.
            let (can_strike, drunken_circular_hit) = match strike {
                SwordStrike::H | SwordStrike::I => {
                    if ctx.blood_alcohol == 0 {
                        (
                            ctx.fighting_ability >= SWORD_STRIKE_MIN_SKILL[i] as u16,
                            false,
                        )
                    } else {
                        // Drunken guys love circular strikes!
                        (true, true)
                    }
                }
                _ => {
                    let ok = ctx.fighting_ability >= SWORD_STRIKE_MIN_SKILL[i] as u16
                        && ctx.blood_alcohol <= SWORD_STRIKE_MAX_ALCOHOL[i];
                    (ok, false)
                }
            };

            if !can_strike {
                if let Some(debug) = debug {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} phase=candidate strike={:?} accepted=false reason=skill_or_alcohol boredom={}]",
                        debug.frame,
                        debug.victim_creation_order,
                        debug.victim,
                        strike,
                        sword_strike_boredom[i],
                    );
                }
                continue;
            }

            // Time limit: reject strikes whose startup exceeds the opponent's
            // remaining action frames.
            // `time_limit >= 1000 || startup_frames + 2 < time_limit`.
            if let Some(limit) = ctx.opponent_time_limit {
                let startup = ctx
                    .strike_startup_frames
                    .map(|f| f[i])
                    .unwrap_or(STRIKE_STARTUP_FRAMES[i]);
                if limit < 1000 && startup + 2 >= limit {
                    if let Some(debug) = debug {
                        eprintln!(
                            "[REACTIVE_SWORD frame={} co={} victim={} phase=candidate strike={:?} accepted=false reason=time_limit startup={} time_limit={}]",
                            debug.frame,
                            debug.victim_creation_order,
                            debug.victim,
                            strike,
                            startup,
                            limit,
                        );
                    }
                    continue;
                }
            }

            // Estimate damage against all nearby victims for this strike.
            // EstimateDamageOfSwordStrike always builds its victim list
            // first, so the lateral / half-circle collectors' angle writes
            // land here even when the estimate is later discarded.
            if matches!(
                ctx.attacker_profile.thrusts[strike as usize].kind,
                WeaponThrustKind::Lateral
                    | WeaponThrustKind::TrueHalfCircle
                    | WeaponThrustKind::FalseHalfCircle
            ) {
                *sweep_rebase = Some(StrikeSelectionSweepRebase { strike });
            }
            let (raw_damage, num_victims) =
                estimate_damage_of_sword_strike(ctx, strike, is_drunken, nearby);

            // Friendly fire — skip this strike entirely.
            if num_victims == -1 {
                if let Some(debug) = debug {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} phase=candidate strike={:?} accepted=false reason=friendly_fire raw_damage={}]",
                        debug.frame, debug.victim_creation_order, debug.victim, strike, raw_damage,
                    );
                }
                continue;
            }

            let mut damage = raw_damage;

            // Boredom malus.
            if !drunken_circular_hit {
                damage -=
                    (sword_strike_boredom[i] as f32 * SWORD_STRIKE_BOREDOM_MALUS_FACTOR) as i16;
            } else {
                damage += 500;
            }

            if let Some(debug) = debug {
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} phase=candidate strike={:?} startup={} raw_damage={} victims={} boredom={} drunken={} adjusted_damage={} best_before={}]",
                    debug.frame,
                    debug.victim_creation_order,
                    debug.victim,
                    strike,
                    ctx.strike_startup_frames
                        .map(|frames| frames[i])
                        .unwrap_or(STRIKE_STARTUP_FRAMES[i]),
                    raw_damage,
                    num_victims,
                    sword_strike_boredom[i],
                    drunken_circular_hit,
                    damage,
                    best_damage,
                );
            }

            // Group strikes require > 1 victim.
            if num_victims > 0
                && (!is_group_strike(strike) || num_victims > 1)
                && damage > best_damage
            {
                best_damage = damage;
                best_strike = Some(strike);
            }
        }

        // Increment boredom for the selected strike.
        if let Some(strike) = best_strike {
            sword_strike_boredom[strike as usize] += SWORD_STRIKE_BOREDOM_INCREMENTATION;
        }
    } // if !only_parade

    if let Some(strike) = best_strike {
        let result = Some(ProposedCombatAction::Strike(strike));
        if let Some(debug) = debug {
            eprintln!(
                "[REACTIVE_SWORD frame={} co={} victim={} phase=result action={:?} boredom={:?}]",
                debug.frame,
                debug.victim_creation_order,
                debug.victim,
                result,
                sword_strike_boredom,
            );
        }
        return result;
    }

    // Parade fallback: if no good strike and `also_parade` is true,
    // propose a parry if it can start in time.
    let parry_frames = ctx.parry_startup_frames.unwrap_or(PARRY_STARTUP_FRAMES);
    let result = if also_parade && (time_limit >= 1000 || parry_frames < time_limit) {
        Some(ProposedCombatAction::Parry)
    } else {
        None
    };
    if let Some(debug) = debug {
        eprintln!(
            "[REACTIVE_SWORD frame={} co={} victim={} phase=result action={:?} only_parade={} parry_frames={} boredom={:?}]",
            debug.frame,
            debug.victim_creation_order,
            debug.victim,
            result,
            only_parade,
            parry_frames,
            sword_strike_boredom,
        );
    }
    result
}

fn initial_strike_skill_gate_rejects(
    skill_roll: u32,
    threshold: u32,
    skip_initial_skill_gate: bool,
) -> bool {
    !skip_initial_skill_gate && skill_roll >= threshold
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::*;

    fn default_ctx() -> ConcussionContext {
        ConcussionContext::default()
    }

    fn make_human() -> HumanData {
        HumanData::default()
    }

    fn make_hth_profile() -> HtHWeaponProfile {
        let mut p = HtHWeaponProfile {
            distance: [10, 30, 50, 70],
            protection_by_localization: [5, 10, 8, 3, 8],
            bludgeon_protection: 20,
            piercing_protection: 15,
            ..Default::default()
        };
        p.thrusts[0] = ThrustProfile {
            target: WeaponTarget::Front,
            kind: WeaponThrustKind::Straight,
            direction: WeaponThrustDirection::NonApplicable,
            stunning: 10,
            cutting: 25,
            minimal_distance: 15,
            maximal_distance: 45,
            initial_angle: 0,
            final_angle: 90,
            rotation_angle: 45,
            repulsion: 5,
            stumble_probability: 20,
            energy: 3,
        };
        p.thrusts[1] = ThrustProfile {
            target: WeaponTarget::Left,
            kind: WeaponThrustKind::Lateral,
            direction: WeaponThrustDirection::LeftToRight,
            stunning: 8,
            cutting: 20,
            minimal_distance: 10,
            maximal_distance: 40,
            initial_angle: 180,
            final_angle: 270,
            rotation_angle: 90,
            repulsion: 3,
            stumble_probability: 15,
            energy: 2,
        };
        // Charge strike (index 9) — push type
        p.thrusts[9] = ThrustProfile {
            target: WeaponTarget::Front,
            kind: WeaponThrustKind::PushAside,
            direction: WeaponThrustDirection::NonApplicable,
            stunning: 15,
            cutting: 30,
            minimal_distance: 20,
            maximal_distance: 60,
            initial_angle: 0,
            final_angle: 0,
            rotation_angle: 0,
            repulsion: 10,
            stumble_probability: 50,
            energy: 5,
        };
        p
    }

    #[test]
    fn lateral_estimation_keeps_active_humans_rejected_by_regular_filter() {
        let mut profile = make_hth_profile();
        profile.thrusts[SwordStrike::B as usize] = ThrustProfile {
            kind: WeaponThrustKind::Lateral,
            direction: WeaponThrustDirection::LeftToRight,
            minimal_distance: 0,
            maximal_distance: 100,
            initial_angle: 0,
            final_angle: 0,
            cutting: 10,
            ..Default::default()
        };
        let ctx = StrikeSelectionContext {
            attacker_profile: &profile,
            fighting_ability: 100,
            blood_alcohol: 0,
            is_rank_soldier: true,
            attacker_direction: 0,
            attacker_elevation: 0.0,
            attacker_camp: Camp::Lacklandists,
            is_swordfighting: true,
            opponent_time_limit: None,
            strike_startup_frames: None,
            parry_startup_frames: None,
            is_npc: true,
        };
        let victim = NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: false,
            dx: 20.0,
            dy_stretched: 0.0,
            distance: 20.0,
            direction_sector: 0,
            camp: Camp::Lacklandists,
            facing_direction: 0,
            elevation: 0.0,
            life_points: 0,
            defender_profile: None,
            is_primary_target: false,
            is_walking_with_sword: false,
        };

        assert_eq!(
            estimate_damage_of_sword_strike(&ctx, SwordStrike::B, false, &[victim]),
            (0, -1),
            "the lateral collector's broad active-human scan must still veto friendly corpses"
        );
    }

    #[test]
    fn inactive_principal_is_only_eligible_for_straight_strike_arc() {
        let mut profile = make_hth_profile();
        profile.thrusts[SwordStrike::A as usize] = ThrustProfile {
            kind: WeaponThrustKind::Straight,
            minimal_distance: 0,
            maximal_distance: 100,
            ..Default::default()
        };
        profile.thrusts[SwordStrike::B as usize] = ThrustProfile {
            kind: WeaponThrustKind::Lateral,
            direction: WeaponThrustDirection::LeftToRight,
            minimal_distance: 0,
            maximal_distance: 100,
            initial_angle: 0,
            final_angle: 0,
            ..Default::default()
        };
        let victim = NearbyVictim {
            is_active: false,
            eligible_for_regular_strikes: false,
            dx: 20.0,
            dy_stretched: 0.0,
            distance: 20.0,
            direction_sector: 0,
            camp: Camp::Lacklandists,
            facing_direction: 0,
            elevation: 0.0,
            life_points: 100,
            defender_profile: None,
            is_primary_target: true,
            is_walking_with_sword: false,
        };

        assert!(is_victim_in_strike_arc(
            &profile,
            SwordStrike::A,
            0,
            &victim,
            true,
        ));
        assert!(!is_victim_in_strike_arc(
            &profile,
            SwordStrike::B,
            0,
            &victim,
            true,
        ));
    }

    #[test]
    fn lateral_and_half_circle_collectors_reject_nan_positions() {
        let mut profile = HtHWeaponProfile::default();
        profile.thrusts[SwordStrike::D as usize] = ThrustProfile {
            kind: WeaponThrustKind::Lateral,
            direction: WeaponThrustDirection::RightToLeft,
            minimal_distance: 0,
            maximal_distance: 100,
            initial_angle: 45,
            final_angle: 45,
            ..Default::default()
        };
        profile.thrusts[SwordStrike::F as usize] = ThrustProfile {
            kind: WeaponThrustKind::TrueHalfCircle,
            direction: WeaponThrustDirection::RightToLeft,
            minimal_distance: 0,
            maximal_distance: 100,
            initial_angle: 45,
            final_angle: 45,
            ..Default::default()
        };
        let victim = NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: true,
            dx: f32::NAN,
            dy_stretched: f32::NAN,
            distance: f32::NAN,
            direction_sector: 0,
            camp: Camp::Lacklandists,
            facing_direction: 0,
            elevation: f32::NAN,
            life_points: 100,
            defender_profile: None,
            is_primary_target: false,
            is_walking_with_sword: false,
        };

        // Both Original collectors are guarded by the positive expression
        // `MaxNorm() < 150` followed by positive inclusive range checks.
        // Every comparison with this restored qNaN position is false.
        assert!(!is_victim_in_strike_arc(
            &profile,
            SwordStrike::D,
            0,
            &victim,
            true,
        ));
        assert!(!is_victim_in_strike_arc(
            &profile,
            SwordStrike::F,
            0,
            &victim,
            true,
        ));
    }

    #[test]
    fn push_estimation_keeps_literal_facing_side_boundary() {
        let mut profile = HtHWeaponProfile::default();
        profile.thrusts[SwordStrike::A as usize] = ThrustProfile {
            kind: WeaponThrustKind::PushAside,
            stunning: 10,
            cutting: 5,
            minimal_distance: 0,
            maximal_distance: 45,
            repulsion: 20,
            ..Default::default()
        };
        let primary = NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: true,
            dx: 1.292_724_6,
            dy_stretched: -39.592_773,
            distance: 39.613_873,
            direction_sector: 0,
            camp: Camp::Lacklandists,
            facing_direction: 8,
            elevation: 0.0,
            life_points: 36,
            defender_profile: None,
            is_primary_target: true,
            is_walking_with_sword: false,
        };
        let friend = NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: true,
            dx: 11.060_913,
            dy_stretched: -17.675_049,
            distance: 20.850_687,
            direction_sector: 1,
            camp: Camp::Royalists,
            facing_direction: 7,
            elevation: 0.0,
            life_points: 35,
            defender_profile: None,
            is_primary_target: false,
            is_walking_with_sword: false,
        };
        let ctx = StrikeSelectionContext {
            attacker_profile: &profile,
            fighting_ability: 100,
            blood_alcohol: 0,
            is_rank_soldier: false,
            attacker_direction: 0,
            attacker_elevation: 0.0,
            attacker_camp: Camp::Royalists,
            is_swordfighting: true,
            opponent_time_limit: Some(1000),
            strike_startup_frames: None,
            parry_startup_frames: None,
            is_npc: true,
        };

        let result = propose_good_sword_strike(
            &crate::sim_rng::test_context(),
            &ctx,
            &[primary, friend],
            &mut vec![0; NUM_NORMAL_SWORD_STRIKES],
            false,
        );

        assert_eq!(result, Some(ProposedCombatAction::Strike(SwordStrike::A)));
    }

    // ── Life points ────────────────────────────────────────────────

    #[test]
    fn set_life_points_basic() {
        let mut lp: i16 = 100;
        assert!(!set_life_points(&mut lp, 50, false, 100, false));
        assert_eq!(lp, 50);
    }

    #[test]
    fn set_life_points_clamps_to_zero() {
        let mut lp: i16 = 30;
        assert!(set_life_points(&mut lp, -10, false, 100, false));
        assert_eq!(lp, 0);
    }

    #[test]
    fn set_life_points_already_dead() {
        let mut lp: i16 = 0;
        assert!(!set_life_points(&mut lp, -5, false, 100, false));
        assert_eq!(lp, 0);
    }

    #[test]
    fn set_life_points_invulnerable() {
        let mut lp: i16 = 120;
        assert!(!set_life_points(&mut lp, 20, true, 120, false));
        assert_eq!(lp, 100);
    }

    #[test]
    fn set_life_points_sherwood_pc_cant_be_hurt() {
        let mut lp: i16 = 80;
        assert!(!set_life_points(&mut lp, 50, false, 100, true));
        assert_eq!(lp, 80); // unchanged
    }

    #[test]
    fn get_wounded_kills() {
        let mut lp: i16 = 30;
        assert!(get_wounded(&mut lp, 50, false, 100, false));
        assert_eq!(lp, 0);
    }

    #[test]
    fn get_wounded_survives() {
        let mut lp: i16 = 80;
        assert!(!get_wounded(&mut lp, 20, false, 100, false));
        assert_eq!(lp, 60);
    }

    // ── Concussion / KO ────────────────────────────────────────────

    #[test]
    fn concussion_effect_scales_with_life() {
        // effect=10, life=100 → adds 10*100/100 = 10
        assert_eq!(compute_concussion_effect(0, 10, 100), 10);
        // effect=10, life=50 → adds 10*100/50 = 20
        assert_eq!(compute_concussion_effect(0, 10, 50), 20);
        // effect=10, life=25 → adds 10*100/25 = 40
        assert_eq!(compute_concussion_effect(0, 10, 25), 40);
    }

    #[test]
    fn concussion_ko_threshold() {
        let mut h = make_human();
        let ctx = default_ctx();

        // Set just below threshold — no KO.
        let outcome = set_concussion(&mut h, CONCUSSION_THRESHOLD - 1, &ctx);
        assert_eq!(outcome, ConcussionOutcome::NoChange);
        assert!(!h.unconscious);

        // Set at threshold — KO.
        let outcome = set_concussion(&mut h, CONCUSSION_THRESHOLD, &ctx);
        assert_eq!(outcome, ConcussionOutcome::WentUnconscious);
        assert!(h.unconscious);
    }

    #[test]
    fn concussion_wakeup() {
        let mut h = make_human();
        let ctx = default_ctx();

        // KO the entity.
        set_concussion(&mut h, CONCUSSION_THRESHOLD, &ctx);
        assert!(h.unconscious);

        // Heal below wakeup threshold.
        let outcome = set_concussion(&mut h, CONCUSSION_WAKEUP_THRESHOLD - 1, &ctx);
        assert_eq!(outcome, ConcussionOutcome::WokeUp);
        assert!(!h.unconscious);
    }

    #[test]
    fn concussion_tied_prevents_wakeup() {
        let mut h = make_human();
        let mut ctx = default_ctx();
        ctx.is_tied = true;

        // KO the entity.
        set_concussion(&mut h, CONCUSSION_THRESHOLD, &ctx);
        assert!(h.unconscious);

        // Try to heal below wakeup — should be clamped.
        let outcome = set_concussion(&mut h, 10, &ctx);
        assert_eq!(outcome, ConcussionOutcome::NoChange);
        assert!(h.unconscious);
        assert_eq!(h.concussion_of_the_brain, CONCUSSION_WAKEUP_THRESHOLD);
    }

    #[test]
    fn concussion_max_clamped() {
        let mut h = make_human();
        let ctx = default_ctx();
        set_concussion(&mut h, 500, &ctx);
        assert_eq!(h.concussion_of_the_brain, CONCUSSION_MAX);
    }

    #[test]
    fn concussion_invulnerable_blocks_increase() {
        let mut h = make_human();
        let mut ctx = default_ctx();
        ctx.is_invulnerable = true;

        h.concussion_of_the_brain = 10;
        let outcome = set_concussion(&mut h, 50, &ctx);
        assert_eq!(outcome, ConcussionOutcome::NoChange);
        assert_eq!(h.concussion_of_the_brain, 10);
    }

    #[test]
    fn add_concussion_positive() {
        let mut h = make_human();
        let ctx = default_ctx();
        add_concussion(&mut h, 10, 100, &ctx);
        // 0 + 10*100/100 = 10
        assert_eq!(h.concussion_of_the_brain, 10);
    }

    #[test]
    fn add_concussion_negative() {
        let mut h = make_human();
        let ctx = default_ctx();
        h.concussion_of_the_brain = 50;
        add_concussion(&mut h, -10, 100, &ctx);
        assert_eq!(h.concussion_of_the_brain, 40);
    }

    #[test]
    fn add_concussion_negative_floors_at_zero() {
        let mut h = make_human();
        let ctx = default_ctx();
        h.concussion_of_the_brain = 5;
        add_concussion(&mut h, -10, 100, &ctx);
        assert_eq!(h.concussion_of_the_brain, 0);
    }

    // ── Concussion healing ─────────────────────────────────────────

    #[test]
    fn healing_tick_counts_down() {
        let mut h = make_human();
        let ctx = default_ctx();
        h.concussion_of_the_brain = 50;
        h.concussion_healing_timeout = 3;

        concussion_healing_tick(&mut h, 10, 100, &ctx);
        assert_eq!(h.concussion_healing_timeout, 2);
        assert_eq!(h.concussion_of_the_brain, 50);

        concussion_healing_tick(&mut h, 10, 100, &ctx);
        assert_eq!(h.concussion_healing_timeout, 1);

        concussion_healing_tick(&mut h, 10, 100, &ctx);
        assert_eq!(h.concussion_healing_timeout, 0);

        // Next tick: heals 1 point and resets timeout.
        concussion_healing_tick(&mut h, 10, 100, &ctx);
        assert_eq!(h.concussion_of_the_brain, 49);
        assert_eq!(h.concussion_healing_timeout, 10);
    }

    #[test]
    fn healing_tick_no_concussion_noop() {
        let mut h = make_human();
        let ctx = default_ctx();
        h.concussion_of_the_brain = 0;
        concussion_healing_tick(&mut h, 10, 100, &ctx);
        assert_eq!(h.concussion_of_the_brain, 0);
    }

    // ── Piercing damage ────────────────────────────────────────────

    #[test]
    fn piercing_damage_kills() {
        let mut h = make_human();
        let mut lp: i16 = 30;
        let ctx = default_ctx();
        assert!(receive_piercing_damage(&mut h, &mut lp, 50, 10, 100, &ctx));
        assert_eq!(lp, 0);
        // Concussion not applied when dead (life_points=0 would cause div-by-zero).
    }

    #[test]
    fn piercing_damage_survives() {
        let mut h = make_human();
        let mut lp: i16 = 100;
        let ctx = default_ctx();
        assert!(!receive_piercing_damage(&mut h, &mut lp, 20, 5, 100, &ctx));
        assert_eq!(lp, 80);
    }

    #[test]
    fn repeated_lethal_piercing_hits_preserve_already_comatose_pc() {
        let mut human = make_human();
        human.concussion_of_the_brain = CONCUSSION_MAX;
        human.unconscious = true;
        let mut life_points = 5;
        let ctx = ConcussionContext {
            is_in_coma: true,
            ..default_ctx()
        };

        for _ in 0..2 {
            assert!(!receive_piercing_damage(
                &mut human,
                &mut life_points,
                20,
                0,
                100,
                &ctx,
            ));
            assert_eq!(life_points, 5);
        }
    }

    #[test]
    fn sublethal_piercing_hit_still_wounds_comatose_pc() {
        let mut human = make_human();
        human.concussion_of_the_brain = CONCUSSION_MAX;
        human.unconscious = true;
        let mut life_points = 20;
        let ctx = ConcussionContext {
            is_in_coma: true,
            ..default_ctx()
        };

        assert!(!receive_piercing_damage(
            &mut human,
            &mut life_points,
            5,
            0,
            100,
            &ctx,
        ));
        assert_eq!(life_points, 15);
    }

    // ── Hit damage ─────────────────────────────────────────────────

    #[test]
    fn hit_damage_concussion_only() {
        let mut h = make_human();
        let ctx = default_ctx();
        let outcome = receive_hit_damage(&mut h, 100, 80, &ctx);
        // 80 * 100 / 100 = 80 → exceeds threshold 70 → KO
        assert_eq!(outcome, ConcussionOutcome::WentUnconscious);
        assert!(h.unconscious);
    }

    #[test]
    fn hit_damage_consumes_hard_difficulty_payload_verbatim() {
        let mut h = make_human();
        let mut ctx = default_ctx();
        ctx.difficulty = crate::player_profile::DifficultyLevel::Hard;

        let outcome = receive_hit_damage(&mut h, 53, 3, &ctx);

        assert_eq!(outcome, ConcussionOutcome::NoChange);
        // 3 * 100 / 53 = 5. Scaling the received payload again would yield 7.
        assert_eq!(h.concussion_of_the_brain, 5);
    }

    // ── Generic damage ─────────────────────────────────────────────

    #[test]
    fn generic_damage_applies_both() {
        let mut h = make_human();
        let mut lp: i16 = 100;
        let ctx = default_ctx();
        let died = receive_generic_damage(&mut h, &mut lp, 30, 10, 100, &ctx);
        assert!(!died);
        assert_eq!(lp, 70);
        assert!(h.concussion_of_the_brain > 0);
    }

    // ── Sword damage ───────────────────────────────────────────────

    #[test]
    fn sword_damage_parry_blocks() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut h = make_human();
        let mut lp: i16 = 100;
        let profile = make_hth_profile();
        let defender = SwordDefenderContext {
            action_state: ActionState::ParryingSword,
            direction: 0,
            elevation: 0.0,
        };
        let attacker = SwordAttackerContext {
            direction: 8,
            direction_to_attacker: 0,
            elevation: 0.0,
            fighting_ability: 50,
            is_rank_soldier: false,
        };
        let ctx = default_ctx();

        let (result, cutting) = receive_sword_damage(
            sim,
            &mut h,
            &mut lp,
            &SwordDamageParams {
                defender: &defender,
                defender_profile: Some(&profile),
                attacker_profile: &profile,
                strike: SwordStrike::A,
                attacker: &attacker,
                concussion_ctx: &ctx,
                max_life_points: 100,
            },
        );
        assert!(result.contains(SwordDamageResult::NO_DAMAGE_PARRIED));
        assert_eq!(cutting, 0);
        assert_eq!(lp, 100); // no damage
    }

    #[test]
    fn sword_damage_no_armor_full_damage() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut h = make_human();
        let mut lp: i16 = 100;
        let profile = make_hth_profile();
        let defender = SwordDefenderContext {
            action_state: ActionState::WaitingSword,
            direction: 0,
            elevation: 0.0,
        };
        let attacker = SwordAttackerContext {
            direction: 8,
            direction_to_attacker: 0,
            elevation: 0.0,
            fighting_ability: 50,
            is_rank_soldier: false,
        };
        let ctx = default_ctx();

        let (result, _cutting) = receive_sword_damage(
            sim,
            &mut h,
            &mut lp,
            &SwordDamageParams {
                defender: &defender,
                defender_profile: None,
                attacker_profile: &profile,
                strike: SwordStrike::A,
                attacker: &attacker,
                concussion_ctx: &ctx,
                max_life_points: 100,
            },
        );
        // No defender profile → full damage flags set.
        assert!(result.contains(SwordDamageResult::CUTTING_DAMAGE));
        assert!(result.contains(SwordDamageResult::STUNNING_DAMAGE));
    }

    #[test]
    fn live_sword_damage_uses_defender_weapon_for_protection_direction() {
        let mut attacker_profile = make_hth_profile();
        let mut defender_profile = attacker_profile.clone();
        let strike = SwordStrike::A;
        attacker_profile.thrusts[strike as usize].kind = WeaponThrustKind::Lateral;
        attacker_profile.thrusts[strike as usize].direction = WeaponThrustDirection::LeftToRight;
        attacker_profile.thrusts[strike as usize].cutting = 10;
        attacker_profile.thrusts[strike as usize].stunning = 0;
        defender_profile.thrusts[strike as usize].kind = WeaponThrustKind::Lateral;
        defender_profile.thrusts[strike as usize].direction = WeaponThrustDirection::RightToLeft;
        // Facing north with the attacker due north, LTR selects HIT_RIGHT
        // while RTL selects HIT_LEFT. Extreme protection values make the
        // result independent of the actual 1..=99 rolls.
        attacker_profile.protection_by_localization = [0, 0, 99, 0, 0];
        attacker_profile.bludgeon_protection = 99;
        defender_profile.protection_by_localization = [0, 0, 99, 0, 0];
        defender_profile.bludgeon_protection = 99;

        let defender = SwordDefenderContext {
            action_state: ActionState::WaitingSword,
            direction: 0,
            elevation: 0.0,
        };
        let attacker = SwordAttackerContext {
            direction: 0,
            direction_to_attacker: 0,
            elevation: 0.0,
            fighting_ability: 50,
            is_rank_soldier: false,
        };
        let ctx = default_ctx();

        let apply = |defender_profile: &HtHWeaponProfile| {
            let sim_context = crate::sim_rng::test_context();
            let mut human = make_human();
            let mut life_points = 100;
            let ((result, cutting), trace) = crate::sim_rng::with_draw_trace(|| {
                receive_sword_damage(
                    &sim_context,
                    &mut human,
                    &mut life_points,
                    &SwordDamageParams {
                        defender: &defender,
                        defender_profile: Some(defender_profile),
                        attacker_profile: &attacker_profile,
                        strike,
                        attacker: &attacker,
                        concussion_ctx: &ctx,
                        max_life_points: 100,
                    },
                )
            });
            (result, cutting, life_points, trace)
        };

        let (heterogeneous_result, heterogeneous_cutting, heterogeneous_life, trace) =
            apply(&defender_profile);
        assert!(heterogeneous_result.is_empty());
        assert_eq!(heterogeneous_cutting, 0);
        assert_eq!(heterogeneous_life, 100);
        assert_eq!(
            trace,
            vec![
                crate::sim_rng::RngSite::SwordDamageProtection,
                crate::sim_rng::RngSite::SwordDamageProtection,
            ]
        );

        let (same_profile_result, same_profile_cutting, same_profile_life, trace) =
            apply(&attacker_profile);
        assert!(same_profile_result.contains(SwordDamageResult::CUTTING_DAMAGE));
        assert_eq!(same_profile_cutting, 10);
        assert_eq!(same_profile_life, 90);
        assert_eq!(
            trace,
            vec![
                crate::sim_rng::RngSite::SwordDamageProtection,
                crate::sim_rng::RngSite::SwordDamageProtection,
            ]
        );
    }

    #[test]
    fn sword_damage_preserves_already_comatose_pc_at_lethal_boundary() {
        let sim_context = crate::sim_rng::test_context();
        let mut human = make_human();
        let mut life_points = 5;
        let mut profile = make_hth_profile();
        profile.protection_by_localization = [0; 5];
        profile.thrusts[SwordStrike::A as usize].cutting = 25;
        profile.thrusts[SwordStrike::A as usize].stunning = 0;
        let defender = SwordDefenderContext {
            action_state: ActionState::Waiting,
            direction: 0,
            elevation: 0.0,
        };
        let attacker = SwordAttackerContext {
            direction: 8,
            direction_to_attacker: 0,
            elevation: 0.0,
            fighting_ability: 50,
            is_rank_soldier: false,
        };
        let ctx = ConcussionContext {
            is_in_coma: true,
            ..default_ctx()
        };
        let mut apply = |profile: &HtHWeaponProfile| {
            let (result, cutting) = receive_sword_damage(
                &sim_context,
                &mut human,
                &mut life_points,
                &SwordDamageParams {
                    defender: &defender,
                    defender_profile: Some(profile),
                    attacker_profile: profile,
                    strike: SwordStrike::A,
                    attacker: &attacker,
                    concussion_ctx: &ctx,
                    max_life_points: 100,
                },
            );
            (result, cutting, life_points)
        };

        let (lethal_result, lethal_cutting, life_after_lethal) = apply(&profile);
        profile.thrusts[SwordStrike::A as usize].cutting = 4;
        let (_, sublethal_cutting, life_after_sublethal) = apply(&profile);

        assert_eq!(lethal_cutting, 25);
        assert!(lethal_result.contains(SwordDamageResult::CUTTING_DAMAGE));
        assert_eq!(life_after_lethal, 5);
        assert_eq!(sublethal_cutting, 4);
        assert_eq!(
            life_after_sublethal, 1,
            "sublethal coma damage still applies"
        );
    }

    // ── Protection direction lookup ────────────────────────────────

    /// Baseline: with attacker and defender on the same ground plane, the
    /// direction-quadrant lookup controls which armour slot is rolled
    /// against, so the defender whose front faces the attacker gets
    /// `HIT_FRONT` protection — *not* `HIT_HEAD`.
    ///
    /// Then the same geometry but with the attacker 20 units higher
    /// (attacker swinging down from a stair / balcony) forces `HIT_HEAD`
    /// protection regardless of facing.
    #[test]
    fn get_sword_protection_elevation_overrides_quadrant() {
        // Profile chosen so every slot has a distinct value — we can
        // identify which branch fired just by reading the returned
        // protection number.
        let profile = HtHWeaponProfile {
            // [HEAD, FRONT, LEFT, BACK, RIGHT]
            protection_by_localization: [77, 11, 22, 33, 44],
            ..Default::default()
        };

        // Defender facing north (sector 0); attacker directly north of
        // the defender, so the defender→attacker direction is sector 0
        // (defender looks straight ahead to see the attacker — frontal
        // strike). NonApplicable thrust keeps strike_direction at 0.
        // relative = (0 + 32 - 0) & 15 = 0 → HIT_FRONT (value 11).
        let baseline = get_sword_protection(
            &profile,
            0, // defender_direction
            0, // defender_to_attacker_direction
            WeaponThrustDirection::NonApplicable,
            0.0, // attacker_elevation
            0.0, // defender_elevation
        );
        assert_eq!(
            baseline, 11,
            "same elevation, frontal strike → HIT_FRONT slot"
        );

        // Now place the attacker ≥ 20 units higher — head-protection
        // override must win, regardless of the direction geometry.
        let elevated = get_sword_protection(
            &profile,
            0,
            0,
            WeaponThrustDirection::NonApplicable,
            20.0,
            0.0,
        );
        assert_eq!(
            elevated, 77,
            "attacker 20 units higher → elevated branch returns HIT_HEAD"
        );

        // Just below the threshold → quadrant lookup still applies.
        let just_below = get_sword_protection(
            &profile,
            0,
            0,
            WeaponThrustDirection::NonApplicable,
            19.9,
            0.0,
        );
        assert_eq!(
            just_below, 11,
            "attacker < 20 units higher → baseline lookup still applies"
        );

        // Defender raised instead (attacker below) → quadrant lookup
        // applies; the elevated branch only fires when the *attacker*
        // is higher.
        let defender_higher = get_sword_protection(
            &profile,
            0,
            0,
            WeaponThrustDirection::NonApplicable,
            0.0,
            50.0,
        );
        assert_eq!(
            defender_higher, 11,
            "defender higher than attacker → baseline lookup",
        );
    }

    #[test]
    fn estimated_protection_uses_ground_space_isometric_direction() {
        // Geometry from the Save063 sword-selection frontier. The sword arc
        // sees the un-isometric projected-map direction (sector 1), while
        // RHSword::GetProtection reconstructs ground/world Y and applies the
        // ordinary ASPECT_RATIO classifier (the reverse direction is sector
        // 8, not the arc sector rotated by 8 to sector 9).
        let victim = NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: true,
            dx: 8.706_726,
            dy_stretched: -28.601_807,
            distance: 29.896,
            direction_sector: 1,
            camp: Camp::Lacklandists,
            facing_direction: 8,
            elevation: 90.001_01,
            life_points: 150,
            defender_profile: None,
            is_primary_target: false,
            is_walking_with_sword: false,
        };

        assert_eq!(protection_direction_to_attacker(90.001_01, &victim), 8);
        assert_eq!(((victim.direction_sector as i16) + 8) & 15, 9);
    }

    // ── Tie-up ─────────────────────────────────────────────────────

    #[test]
    fn tie_up_sets_posture() {
        let mut h = make_human();
        h.unconscious = true;
        h.concussion_of_the_brain = CONCUSSION_THRESHOLD;
        let mut posture = Posture::Lying;
        tie_up(&mut h, &mut posture);
        assert_eq!(posture, Posture::Tied);
        assert!(h.concussion_of_the_brain >= CONCUSSION_WAKEUP_THRESHOLD);
    }

    #[test]
    #[should_panic(expected = "cannot tie up a conscious entity")]
    fn tie_up_panics_if_conscious() {
        let mut h = make_human();
        let mut posture = Posture::Upright;
        tie_up(&mut h, &mut posture);
    }

    #[test]
    fn untie_sets_lying() {
        let mut h = make_human();
        h.unconscious = true;
        h.concussion_of_the_brain = CONCUSSION_WAKEUP_THRESHOLD + 17;
        let mut posture = Posture::Tied;
        untie(&mut h, &mut posture);
        assert_eq!(posture, Posture::Lying);
        assert!(h.unconscious);
        assert_eq!(h.concussion_of_the_brain, CONCUSSION_WAKEUP_THRESHOLD + 17);
    }

    #[test]
    fn untied_unconscious_human_wakes_through_normal_concussion_healing() {
        let mut human = make_human();
        human.unconscious = true;
        human.concussion_of_the_brain = CONCUSSION_WAKEUP_THRESHOLD;
        let mut posture = Posture::Tied;

        untie(&mut human, &mut posture);
        concussion_healing_tick(&mut human, 1, 100, &default_ctx());

        assert_eq!(posture, Posture::Lying);
        assert!(!human.unconscious);
        assert_eq!(
            human.concussion_of_the_brain,
            CONCUSSION_WAKEUP_THRESHOLD - 1
        );
    }

    #[test]
    #[should_panic(expected = "cannot untie an untied entity")]
    fn untie_rejects_non_tied_posture() {
        let mut h = make_human();
        let mut posture = Posture::Lying;
        untie(&mut h, &mut posture);
    }

    // ── Net ────────────────────────────────────────────────────────

    #[test]
    fn net_mechanics() {
        let mut h = make_human();
        let mut posture = Posture::Upright;
        apply_net(&mut h, &mut posture);
        assert_eq!(posture, Posture::StuckUnderNet);
        assert_eq!(h.stuck_under_nets_counter, 1);

        apply_net(&mut h, &mut posture);
        assert_eq!(h.stuck_under_nets_counter, 2);

        remove_net(&mut h, &mut posture);
        assert_eq!(h.stuck_under_nets_counter, 1);
        assert_eq!(posture, Posture::StuckUnderNet);

        remove_net(&mut h, &mut posture);
        assert_eq!(h.stuck_under_nets_counter, 0);
        assert_eq!(posture, Posture::Lying);
    }

    // ── Relative fighting ability ──────────────────────────────────

    #[test]
    fn relative_fighting_ability_equal() {
        assert_eq!(compute_relative_fighting_ability(50, 50), 50);
    }

    #[test]
    fn relative_fighting_ability_stronger() {
        let r = compute_relative_fighting_ability(80, 40);
        assert!(r > 50);
    }

    #[test]
    fn relative_fighting_ability_weaker() {
        let r = compute_relative_fighting_ability(30, 90);
        assert!(r < 50);
    }

    #[test]
    fn relative_fighting_ability_both_zero() {
        assert_eq!(compute_relative_fighting_ability(0, 0), 50);
    }

    // ── Range checks ───────────────────────────────────────────────

    #[test]
    fn melee_range_check() {
        let profile = make_hth_profile();
        let sword = SwordState::new(0);
        assert!(!is_in_melee_range(&sword, &profile, 5.0)); // too close
        assert!(is_in_melee_range(&sword, &profile, 30.0)); // in range
        assert!(is_in_melee_range(&sword, &profile, 50.0)); // at max
        assert!(!is_in_melee_range(&sword, &profile, 51.0)); // too far
    }

    #[test]
    fn strike_range_check() {
        let profile = make_hth_profile();
        // Strike A: min=15, max=45
        assert!(!is_strike_in_range(&profile, SwordStrike::A, 10.0));
        assert!(is_strike_in_range(&profile, SwordStrike::A, 30.0));
        assert!(is_strike_in_range(&profile, SwordStrike::A, 45.0));
        assert!(!is_strike_in_range(&profile, SwordStrike::A, 46.0));
    }

    #[test]
    fn bow_range_check() {
        assert!(is_in_bow_range(200, 150.0));
        assert!(is_in_bow_range(200, 200.0));
        assert!(!is_in_bow_range(200, 201.0));
    }

    // ── Strike push detection ──────────────────────────────────────

    #[test]
    fn push_strike_detection() {
        let profile = make_hth_profile();
        assert!(!strike_has_push_effect(&profile, SwordStrike::A)); // Straight
        assert!(strike_has_push_effect(&profile, SwordStrike::Charge)); // Always push
    }

    #[test]
    fn push_strike_width_uses_original_integer_half_width() {
        let mut profile = HtHWeaponProfile::default();
        profile.thrusts[SwordStrike::A as usize] = ThrustProfile {
            kind: WeaponThrustKind::PushAside,
            minimal_distance: 0,
            maximal_distance: 100,
            repulsion: 5,
            ..Default::default()
        };
        // Direction 0 points along -Y, making X the side distance. A width
        // of 5 therefore has Original's ULONG half-width 2, not 2.5.
        let victim = NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: true,
            dx: 2.25,
            dy_stretched: -10.0,
            distance: 10.25,
            direction_sector: 0,
            camp: Camp::Royalists,
            facing_direction: 0,
            elevation: 0.0,
            life_points: 10,
            defender_profile: None,
            is_primary_target: false,
            is_walking_with_sword: false,
        };

        assert!(!is_victim_in_strike_arc(
            &profile,
            SwordStrike::A,
            0,
            &victim,
            false
        ));

        // Even widths are unchanged: width 6 has half-width 3 and admits
        // the same victim.
        profile.thrusts[SwordStrike::A as usize].repulsion = 6;
        assert!(is_victim_in_strike_arc(
            &profile,
            SwordStrike::A,
            0,
            &victim,
            false
        ));
    }

    #[test]
    fn push_strike_side_axis_keeps_original_float_rotate_residual() {
        let ((forward_x, forward_y), (side_x, side_y)) = push_strike_basis(0);
        assert_eq!((forward_x.to_bits(), forward_y.to_bits()), (0, 0xbf80_0000));
        assert_eq!(
            (side_x.to_bits(), side_y.to_bits()),
            (0x3f80_0000, 0x333b_bd2e)
        );

        let mut profile = HtHWeaponProfile::default();
        profile.thrusts[SwordStrike::A as usize] = ThrustProfile {
            kind: WeaponThrustKind::PushAside,
            minimal_distance: 0,
            maximal_distance: 100,
            repulsion: 20,
            ..Default::default()
        };
        let victim = |dx| NearbyVictim {
            is_active: true,
            eligible_for_regular_strikes: true,
            dx,
            dy_stretched: -100.0,
            distance: 101.0,
            direction_sector: 0,
            camp: Camp::Royalists,
            facing_direction: 0,
            elevation: 0.0,
            life_points: 10,
            defender_profile: None,
            is_primary_target: false,
            is_walking_with_sword: false,
        };

        assert!(!is_victim_in_strike_arc(
            &profile,
            SwordStrike::A,
            0,
            &victim(-10.0),
            false
        ));
        assert!(is_victim_in_strike_arc(
            &profile,
            SwordStrike::A,
            0,
            &victim(10.0),
            false
        ));
        assert!(is_victim_in_strike_arc(
            &profile,
            SwordStrike::A,
            0,
            &victim(9.0),
            false
        ));
    }

    #[test]
    fn strike_arc_angles_preserve_original_promotions_and_negative_sectors() {
        let original_sector_angle =
            ((6.0_f64 / 16.0) * 2.0 * f64::from(std::f32::consts::PI) + 0.1) as f32;
        assert_eq!(
            sector_to_angle(6).to_bits(),
            original_sector_angle.to_bits()
        );

        let original_strike_angle =
            ((7.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
        assert_eq!(
            degrees_to_radians(7).to_bits(),
            original_strike_angle.to_bits()
        );

        // Original handles negative angles recursively instead of first
        // normalizing them. Exactly -PI/8 therefore belongs to sector 14,
        // not sector 15 from the normalized-floor formulation.
        assert_eq!(angle_to_sector(-std::f32::consts::PI / 8.0), 14);
    }

    // ── Energy cost ────────────────────────────────────────────────

    #[test]
    fn strike_energy() {
        let profile = make_hth_profile();
        assert_eq!(strike_energy_cost(&profile, SwordStrike::A), 3);
        // RHSword::GetStrikeEnergy returns the authored value directly for a
        // real strike, so zero remains zero.
        let mut p2 = profile.clone();
        p2.thrusts[2].energy = 0;
        assert_eq!(strike_energy_cost(&p2, SwordStrike::C), 0);
    }

    #[test]
    fn completed_strike_tiredness_uses_uword_assignment() {
        assert_eq!(add_strike_tiredness(11, 0), 11);
        assert_eq!(add_strike_tiredness(11, 7), 18);
        assert_eq!(add_strike_tiredness(u16::MAX - 2, 7), 4);
    }

    // ── Tiredness recovery ─────────────────────────────────────────

    #[test]
    fn tiredness_recovers() {
        assert_eq!(tiredness_recovery(50, 100), 40); // 100/10 = 10 recovery
        assert_eq!(tiredness_recovery(5, 100), 0); // recovery exceeds tiredness
        assert_eq!(tiredness_recovery(50, 0), 50); // no endurance = no recovery
    }

    // ── Cutting effect with fighting ability ───────────────────────

    #[test]
    fn cutting_effect_rank_soldier() {
        let profile = make_hth_profile();
        // Non-soldier: base cutting = 25
        let base = get_strike_cutting_effect(&profile, SwordStrike::A, 50, false);
        assert_eq!(base, 25);
        // Rank soldier with ability 50: 25 * (1 + 0.5) = 37
        let soldier = get_strike_cutting_effect(&profile, SwordStrike::A, 50, true);
        assert_eq!(soldier, 37);
    }

    // ── DamageEvent constructors ───────────────────────────────────

    #[test]
    fn damage_event_constructors() {
        let sword = DamageEvent::sword(EntityId::Pc(crate::entity_id::PcId(1)), SwordStrike::A);
        assert_eq!(sword.kind, DamageKind::Sword);
        assert_eq!(sword.origin, Some(EntityId::Pc(crate::entity_id::PcId(1))));
        assert_eq!(sword.sword_strike, Some(SwordStrike::A));

        let arrow = DamageEvent::arrow(EntityId::Pc(crate::entity_id::PcId(2)), 30);
        assert_eq!(arrow.kind, DamageKind::Arrow);
        assert_eq!(arrow.damage, 30);

        let stone = DamageEvent::stone(15, 5);
        assert_eq!(stone.kind, DamageKind::Stone);
        assert_eq!(stone.damage, 15);
        assert_eq!(stone.concussion, 5);

        let hit = DamageEvent::hit(EntityId::Pc(crate::entity_id::PcId(3)), 40, true);
        assert_eq!(hit.kind, DamageKind::Hit);
        assert!(hit.is_harder_hit);

        let net = DamageEvent::net(EntityId::Pc(crate::entity_id::PcId(4)));
        assert_eq!(net.kind, DamageKind::Net);

        let r#gen = DamageEvent::generic(10, 20);
        assert_eq!(r#gen.kind, DamageKind::Generic);
    }
}
