//! Weapon system — bows and hand-to-hand weapons.
//!
//! Stores a `profile_idx` referring to profile data loaded by the profile
//! manager; the profile is passed by reference when needed.

use serde::{Deserialize, Serialize};

use crate::profiles::{
    BowProfile, BowShootMode, HtHWeaponProfile, WeaponTarget, WeaponThrustDirection,
    WeaponThrustKind,
};

// ─── Enums ──────────────────────────────────────────────────────

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum WeaponType {
    HandToHand,
    Bow,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum ShootMode {
    Normal,
    Long,
    /// Down-shoot from a leaning-out posture (soldiers only).
    /// Uses the same profile data as `Normal` for damage/hit-chance
    /// but a different animation and arrow mass.
    Down,
}

/// Skill level buckets for bow accuracy interpolation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum SkillLevel {
    Beginner = 0,
    Normal = 1,
    Elite = 2,
}

/// Indices into `HtHWeaponProfile.distance`.
#[repr(usize)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum WeaponDistance {
    Minimal = 0,
    Default = 1,
    Maximal = 2,
    Uber = 3,
}

/// Named sword strikes (A–I plus Charge).
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum SwordStrike {
    #[default]
    A = 0,
    B = 1,
    C = 2,
    D = 3,
    E = 4,
    F = 5,
    G = 6,
    H = 7,
    I = 8,
    Charge = 9,
}

/// Number of normal (non-charge) strikes: A through I.
pub const NUM_NORMAL_SWORD_STRIKES: usize = SwordStrike::Charge as usize; // 9

/// Total real strikes including Charge.
pub const NUM_REAL_STRIKES: usize = 10;

impl SwordStrike {
    /// Convert a live actor command to the corresponding normal sword strike.
    ///
    /// Original combat-learning code reads `GetCommand()` from the attacker
    /// when delayed damage is translated, so this conversion intentionally
    /// does not consult the strike snapshotted in the damage element.
    pub fn from_command(command: crate::element::Command) -> Option<Self> {
        use crate::element::Command;
        match command {
            Command::SwordstrikeThrustA => Some(Self::A),
            Command::SwordstrikeThrustB => Some(Self::B),
            Command::SwordstrikeThrustC => Some(Self::C),
            Command::SwordstrikeThrustD => Some(Self::D),
            Command::SwordstrikeThrustE => Some(Self::E),
            Command::SwordstrikeThrustF => Some(Self::F),
            Command::SwordstrikeThrustG => Some(Self::G),
            Command::SwordstrikeThrustH => Some(Self::H),
            Command::SwordstrikeThrustI => Some(Self::I),
            _ => None,
        }
    }

    /// Convert to the corresponding [`Command`](crate::element::Command) variant.
    pub fn to_command(self) -> crate::element::Command {
        use crate::element::Command;
        match self {
            Self::A => Command::SwordstrikeThrustA,
            Self::B => Command::SwordstrikeThrustB,
            Self::C => Command::SwordstrikeThrustC,
            Self::D => Command::SwordstrikeThrustD,
            Self::E => Command::SwordstrikeThrustE,
            Self::F => Command::SwordstrikeThrustF,
            Self::G => Command::SwordstrikeThrustG,
            Self::H => Command::SwordstrikeThrustH,
            Self::I => Command::SwordstrikeThrustI,
            Self::Charge => Command::SwordstrikeThrustB, // charge uses strong-strike anim
        }
    }
}

// ─── Core Structs ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Weapon {
    pub weapon_type: WeaponType,
    pub profile_idx: u32,
    pub ammo: u16,
    pub max_ammo: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct BowState {
    pub weapon: Weapon,
    pub long_shoot_available: bool,
    pub current_mode: ShootMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SwordState {
    pub weapon: Weapon,
    pub current_thrust: u8,
}

// ─── Weapon ─────────────────────────────────────────────────────

impl Weapon {
    pub fn new(weapon_type: WeaponType, profile_idx: u32, max_ammo: u16) -> Self {
        Self {
            weapon_type,
            profile_idx,
            ammo: max_ammo,
            max_ammo,
        }
    }

    /// Add ammo, clamped to `max_ammo`.
    pub fn reload(&mut self, amount: u16) {
        self.ammo = self.ammo.saturating_add(amount).min(self.max_ammo);
    }
}

// ─── Bow ────────────────────────────────────────────────────────

impl BowState {
    pub fn new(profile_idx: u32, profile: &BowProfile, max_ammo: u16) -> Self {
        Self {
            weapon: Weapon::new(WeaponType::Bow, profile_idx, max_ammo),
            long_shoot_available: profile.has_long_shoot,
            current_mode: ShootMode::Normal,
        }
    }

    pub fn get_range(&self, profile: &BowProfile, mode: ShootMode) -> u16 {
        self.shoot_data(profile, mode).range
    }

    pub fn get_max_range(&self, profile: &BowProfile) -> u16 {
        if self.long_shoot_available {
            profile.long_shoot.range
        } else {
            profile.normal_shoot.range
        }
    }

    /// Choose shoot mode based on whether `distance` fits the normal range.
    pub fn get_shoot_mode_for_distance(&self, profile: &BowProfile, distance: f32) -> ShootMode {
        if distance <= profile.normal_shoot.range as f32 {
            ShootMode::Normal
        } else {
            ShootMode::Long
        }
    }

    pub fn get_damage(&self, profile: &BowProfile, mode: ShootMode) -> u16 {
        self.shoot_data(profile, mode).damage
    }

    /// Fire one arrow in the current mode.  Panics if out of ammo.
    pub fn fire(&mut self) -> ShootMode {
        assert!(self.weapon.ammo > 0, "cannot fire: out of ammo");
        self.weapon.ammo -= 1;
        self.current_mode
    }

    /// Compute hit chance (0–100) given shooter `ability` (0–100) and
    /// `distance` to target.
    ///
    /// Bilinear interpolation over distance-ratio and skill-level.
    ///
    /// Windows retail multiplies the floating-point distance ratio by
    /// 100 before converting it to the integer percentage used to
    /// select a distance bucket.
    pub fn get_hit_chance(&self, profile: &BowProfile, ability: u32, distance: u32) -> u32 {
        assert!(ability <= 100);

        let shoot_mode = self.get_shoot_mode_for_distance(profile, distance as f32);
        let shoot = self.shoot_data(profile, shoot_mode);
        let range = shoot.range as u32;

        if range == 0 || distance > range {
            return 0;
        }

        // Windows retail keeps the division and scale in x87 extended
        // precision until the integer conversion.
        let distance_ratio = (distance as f64 / range as f64 * 100.0) as u32 as f32;
        let dr = distance_ratio as u32;

        // Distance-key bracket (each bucket spans 20 %-points of range).
        let (lower_key, upper_key, lo_d, hi_d): (usize, usize, f32, f32) = if dr < 20 {
            (0, 1, 0.0, 20.0)
        } else if dr < 40 {
            (1, 2, 20.0, 40.0)
        } else if dr < 60 {
            (2, 3, 40.0, 60.0)
        } else if dr < 80 {
            (3, 4, 60.0, 80.0)
        } else {
            (4, 5, 80.0, 100.0)
        };

        // Skill-level bracket.
        let (lo_skill, hi_skill): (usize, usize) = if ability < 50 {
            (0, 1) // Beginner → Normal
        } else {
            (1, 2) // Normal → Elite
        };

        let lo_hit = interpolate(
            lo_d,
            hi_d,
            shoot.hit_chances[lo_skill].hit_chance[lower_key] as f32,
            shoot.hit_chances[lo_skill].hit_chance[upper_key] as f32,
            distance_ratio,
        );
        let hi_hit = interpolate(
            lo_d,
            hi_d,
            shoot.hit_chances[hi_skill].hit_chance[lower_key] as f32,
            shoot.hit_chances[hi_skill].hit_chance[upper_key] as f32,
            distance_ratio,
        );

        interpolate(0.0, 100.0, lo_hit, hi_hit, ability as f32) as u32
    }

    fn shoot_data<'a>(&self, profile: &'a BowProfile, mode: ShootMode) -> &'a BowShootMode {
        match mode {
            ShootMode::Normal | ShootMode::Down => &profile.normal_shoot,
            ShootMode::Long => &profile.long_shoot,
        }
    }
}

// ─── Sword ──────────────────────────────────────────────────────

impl SwordState {
    pub fn new(profile_idx: u32) -> Self {
        Self {
            weapon: Weapon::new(WeaponType::HandToHand, profile_idx, 0),
            current_thrust: 0,
        }
    }

    pub fn get_range(&self, profile: &HtHWeaponProfile, distance: WeaponDistance) -> u16 {
        profile.distance[distance as usize]
    }

    pub fn is_charge_weapon(&self, profile: &HtHWeaponProfile) -> bool {
        profile.charge
    }

    pub fn is_shield_weapon(&self, profile: &HtHWeaponProfile) -> bool {
        profile.shield
    }

    pub fn get_shield_width(&self, profile: &HtHWeaponProfile) -> u16 {
        profile.shield_width
    }

    pub fn get_shield_height(&self, profile: &HtHWeaponProfile) -> u16 {
        profile.shield_height
    }

    // ── Per-strike accessors ────────────────────────────────────

    pub fn get_strike_target(
        &self,
        profile: &HtHWeaponProfile,
        strike: SwordStrike,
    ) -> WeaponTarget {
        profile.thrusts[strike as usize].target
    }

    pub fn get_strike_kind(
        &self,
        profile: &HtHWeaponProfile,
        strike: SwordStrike,
    ) -> WeaponThrustKind {
        profile.thrusts[strike as usize].kind
    }

    /// Returns the thrust direction for lateral / circular strikes;
    /// `NonApplicable` for straight / push-aside / assault.
    pub fn get_strike_direction(
        &self,
        profile: &HtHWeaponProfile,
        strike: SwordStrike,
    ) -> WeaponThrustDirection {
        let t = &profile.thrusts[strike as usize];
        match t.kind {
            WeaponThrustKind::Lateral
            | WeaponThrustKind::TrueHalfCircle
            | WeaponThrustKind::TrueCircle
            | WeaponThrustKind::FalseHalfCircle
            | WeaponThrustKind::FalseCircle => t.direction,
            _ => WeaponThrustDirection::NonApplicable,
        }
    }

    pub fn get_strike_stunning(&self, profile: &HtHWeaponProfile, strike: SwordStrike) -> u16 {
        profile.thrusts[strike as usize].stunning
    }

    pub fn get_strike_cutting(&self, profile: &HtHWeaponProfile, strike: SwordStrike) -> u16 {
        profile.thrusts[strike as usize].cutting
    }

    /// Initial angle in radians (profile stores degrees).
    pub fn get_strike_initial_angle(&self, profile: &HtHWeaponProfile, strike: SwordStrike) -> f32 {
        degrees_to_radians(profile.thrusts[strike as usize].initial_angle)
    }

    /// Final angle in radians.
    pub fn get_strike_final_angle(&self, profile: &HtHWeaponProfile, strike: SwordStrike) -> f32 {
        degrees_to_radians(profile.thrusts[strike as usize].final_angle)
    }

    pub fn get_strike_repulsion(&self, profile: &HtHWeaponProfile, strike: SwordStrike) -> u16 {
        profile.thrusts[strike as usize].repulsion
    }

    pub fn get_strike_stumble_probability(
        &self,
        profile: &HtHWeaponProfile,
        strike: SwordStrike,
    ) -> u16 {
        profile.thrusts[strike as usize].stumble_probability
    }

    pub fn get_strike_energy(&self, profile: &HtHWeaponProfile, strike: SwordStrike) -> u16 {
        // The default-energy fallback only applies to non-real strikes; our
        // SwordStrike enum can only represent real strikes (A-I + Charge), so
        // that branch is unreachable and we use a direct lookup.  A genuinely
        // zero-energy real strike in the profile returns 0.
        profile.thrusts[strike as usize].energy
    }

    /// Advance to the next normal strike in the A–I cycle.
    pub fn strike(&mut self) {
        self.current_thrust = ((self.current_thrust as usize + 1) % NUM_NORMAL_SWORD_STRIKES) as u8;
    }

    /// Returns `true` when `distance` is in the half-open interval
    /// `(min_range, max_range]`.
    pub fn is_distance_between(
        &self,
        profile: &HtHWeaponProfile,
        distance: f32,
        min_range: WeaponDistance,
        max_range: WeaponDistance,
    ) -> bool {
        assert!(min_range < max_range);
        let lo = profile.distance[min_range as usize] as f32;
        let hi = profile.distance[max_range as usize] as f32;
        lo < distance && hi >= distance
    }
}

// ─── Helpers ────────────────────────────────────────────────────

/// Linear interpolation.
fn interpolate(
    lower_bound: f32,
    upper_bound: f32,
    lower_value: f32,
    upper_value: f32,
    balance: f32,
) -> f32 {
    let span = (upper_bound - lower_bound).abs();
    if span < f32::EPSILON {
        return lower_value;
    }
    let grow = (upper_value - lower_value) / span;
    (balance - lower_bound).abs() * grow + lower_value
}

fn degrees_to_radians(deg: u16) -> f32 {
    (deg as f32 / 360.0) * 2.0 * std::f32::consts::PI
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::*;

    fn make_bow_profile() -> BowProfile {
        BowProfile {
            normal_shoot: BowShootMode {
                range: 200,
                damage: 50,
                hit_chances: [
                    BowHitChance {
                        hit_chance: [100, 90, 80, 70, 60, 50],
                    }, // Beginner
                    BowHitChance {
                        hit_chance: [100, 95, 90, 85, 80, 75],
                    }, // Normal
                    BowHitChance {
                        hit_chance: [100, 98, 96, 94, 92, 90],
                    }, // Elite
                ],
            },
            has_long_shoot: true,
            long_shoot: BowShootMode {
                range: 400,
                damage: 30,
                hit_chances: [
                    BowHitChance {
                        hit_chance: [80, 70, 60, 50, 40, 30],
                    },
                    BowHitChance {
                        hit_chance: [90, 80, 70, 60, 50, 40],
                    },
                    BowHitChance {
                        hit_chance: [95, 90, 85, 80, 75, 70],
                    },
                ],
            },
        }
    }

    fn make_hth_profile() -> HtHWeaponProfile {
        let mut profile = HtHWeaponProfile {
            distance: [10, 30, 50, 70],
            protection_by_localization: [5, 10, 8, 3, 8],
            charge: false,
            shield: true,
            shield_width: 20,
            shield_height: 40,
            ..Default::default()
        };
        profile.thrusts[0] = ThrustProfile {
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
        profile.thrusts[1] = ThrustProfile {
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
        profile
    }

    // ── Weapon ──────────────────────────────────────────────────

    #[test]
    fn weapon_new() {
        let w = Weapon::new(WeaponType::Bow, 7, 20);
        assert_eq!(w.weapon_type, WeaponType::Bow);
        assert_eq!(w.profile_idx, 7);
        assert_eq!(w.ammo, 20);
        assert_eq!(w.max_ammo, 20);
    }

    #[test]
    fn weapon_reload_clamps() {
        let mut w = Weapon::new(WeaponType::Bow, 0, 20);
        w.ammo = 5;
        w.reload(10);
        assert_eq!(w.ammo, 15);
        w.reload(100);
        assert_eq!(w.ammo, 20);
    }

    // ── Bow ─────────────────────────────────────────────────────

    #[test]
    fn bow_range() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        assert_eq!(bow.get_range(&p, ShootMode::Normal), 200);
        assert_eq!(bow.get_range(&p, ShootMode::Long), 400);
        assert_eq!(bow.get_max_range(&p), 400);
    }

    #[test]
    fn bow_max_range_no_long_shoot() {
        let mut p = make_bow_profile();
        p.has_long_shoot = false;
        let bow = BowState::new(0, &p, 10);
        assert_eq!(bow.get_max_range(&p), 200);
    }

    #[test]
    fn bow_shoot_mode_selection() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        assert_eq!(
            bow.get_shoot_mode_for_distance(&p, 100.0),
            ShootMode::Normal
        );
        assert_eq!(
            bow.get_shoot_mode_for_distance(&p, 200.0),
            ShootMode::Normal
        );
        assert_eq!(bow.get_shoot_mode_for_distance(&p, 201.0), ShootMode::Long);
    }

    #[test]
    fn bow_damage() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        assert_eq!(bow.get_damage(&p, ShootMode::Normal), 50);
        assert_eq!(bow.get_damage(&p, ShootMode::Long), 30);
    }

    #[test]
    fn bow_fire_consumes_ammo() {
        let p = make_bow_profile();
        let mut bow = BowState::new(0, &p, 3);
        assert_eq!(bow.fire(), ShootMode::Normal);
        assert_eq!(bow.weapon.ammo, 2);
        bow.current_mode = ShootMode::Long;
        assert_eq!(bow.fire(), ShootMode::Long);
        assert_eq!(bow.weapon.ammo, 1);
    }

    #[test]
    #[should_panic(expected = "out of ammo")]
    fn bow_fire_empty_panics() {
        let p = make_bow_profile();
        let mut bow = BowState::new(0, &p, 0);
        bow.fire();
    }

    #[test]
    fn bow_hit_chance_out_of_range() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        assert_eq!(bow.get_hit_chance(&p, 50, 500), 0);
    }

    #[test]
    fn bow_hit_chance_zero_distance() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        // ability=0 (beginner), distance=0 → hit_chance[Beginner][Distance0] = 100
        assert_eq!(bow.get_hit_chance(&p, 0, 0), 100);
    }

    #[test]
    fn bow_hit_chance_uses_fractional_distance_bucket() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        // Half of the 200-unit normal range is the 50% point. For a
        // beginner that interpolates halfway between 80 and 70.
        assert_eq!(bow.get_hit_chance(&p, 0, 100), 75);
    }

    #[test]
    fn bow_hit_chance_increases_with_skill() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 10);
        let low = bow.get_hit_chance(&p, 10, 100);
        let high = bow.get_hit_chance(&p, 90, 100);
        assert!(
            high >= low,
            "elite should hit at least as often as beginner"
        );
    }

    // ── Sword ───────────────────────────────────────────────────

    #[test]
    fn sword_new() {
        let s = SwordState::new(3);
        assert_eq!(s.weapon.weapon_type, WeaponType::HandToHand);
        assert_eq!(s.weapon.profile_idx, 3);
        assert_eq!(s.current_thrust, 0);
    }

    #[test]
    fn sword_strike_cycles() {
        let mut s = SwordState::new(0);
        for i in 0..NUM_NORMAL_SWORD_STRIKES {
            assert_eq!(s.current_thrust, i as u8);
            s.strike();
        }
        assert_eq!(s.current_thrust, 0);
    }

    #[test]
    fn sword_range() {
        let p = make_hth_profile();
        let s = SwordState::new(0);
        assert_eq!(s.get_range(&p, WeaponDistance::Minimal), 10);
        assert_eq!(s.get_range(&p, WeaponDistance::Default), 30);
        assert_eq!(s.get_range(&p, WeaponDistance::Maximal), 50);
        assert_eq!(s.get_range(&p, WeaponDistance::Uber), 70);
    }

    #[test]
    fn sword_charge_and_shield() {
        let p = make_hth_profile();
        let s = SwordState::new(0);
        assert!(!s.is_charge_weapon(&p));
        assert!(s.is_shield_weapon(&p));
        assert_eq!(s.get_shield_width(&p), 20);
        assert_eq!(s.get_shield_height(&p), 40);
    }

    #[test]
    fn sword_strike_properties() {
        let p = make_hth_profile();
        let s = SwordState::new(0);

        // Strike A — straight
        assert_eq!(s.get_strike_target(&p, SwordStrike::A), WeaponTarget::Front);
        assert_eq!(
            s.get_strike_kind(&p, SwordStrike::A),
            WeaponThrustKind::Straight
        );
        assert_eq!(
            s.get_strike_direction(&p, SwordStrike::A),
            WeaponThrustDirection::NonApplicable
        );
        assert_eq!(s.get_strike_stunning(&p, SwordStrike::A), 10);
        assert_eq!(s.get_strike_cutting(&p, SwordStrike::A), 25);
        assert_eq!(s.get_strike_energy(&p, SwordStrike::A), 3);
        assert_eq!(s.get_strike_repulsion(&p, SwordStrike::A), 5);
        assert_eq!(s.get_strike_stumble_probability(&p, SwordStrike::A), 20);

        // Strike B — lateral (direction matters)
        assert_eq!(s.get_strike_target(&p, SwordStrike::B), WeaponTarget::Left);
        assert_eq!(
            s.get_strike_kind(&p, SwordStrike::B),
            WeaponThrustKind::Lateral
        );
        assert_eq!(
            s.get_strike_direction(&p, SwordStrike::B),
            WeaponThrustDirection::LeftToRight
        );
    }

    #[test]
    fn sword_strike_angles() {
        let p = make_hth_profile();
        let s = SwordState::new(0);
        // Strike A: initial=0, final=90 deg
        assert!((s.get_strike_initial_angle(&p, SwordStrike::A)).abs() < f32::EPSILON);
        let quarter_turn = std::f32::consts::FRAC_PI_2;
        assert!((s.get_strike_final_angle(&p, SwordStrike::A) - quarter_turn).abs() < 0.01);
    }

    #[test]
    fn sword_distance_between() {
        let p = make_hth_profile();
        let s = SwordState::new(0);
        // Interval is (Minimal, Default] = (10, 30]
        assert!(!s.is_distance_between(&p, 10.0, WeaponDistance::Minimal, WeaponDistance::Default));
        assert!(s.is_distance_between(&p, 20.0, WeaponDistance::Minimal, WeaponDistance::Default));
        assert!(s.is_distance_between(&p, 30.0, WeaponDistance::Minimal, WeaponDistance::Default));
        assert!(!s.is_distance_between(&p, 31.0, WeaponDistance::Minimal, WeaponDistance::Default));
    }

    #[test]
    fn sword_zero_energy_returns_zero() {
        // get_strike_energy returns the profile value directly for real
        // strikes — zero stays zero.
        let mut p = make_hth_profile();
        p.thrusts[2].energy = 0;
        let s = SwordState::new(0);
        assert_eq!(s.get_strike_energy(&p, SwordStrike::C), 0);
    }

    // ── Interpolation ───────────────────────────────────────────

    #[test]
    fn interpolate_midpoint() {
        assert!((interpolate(0.0, 100.0, 0.0, 100.0, 50.0) - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn interpolate_quarter() {
        assert!((interpolate(0.0, 100.0, 0.0, 200.0, 25.0) - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn interpolate_offset_range() {
        assert!((interpolate(10.0, 20.0, 100.0, 200.0, 15.0) - 150.0).abs() < f32::EPSILON);
    }

    // ── Serde ───────────────────────────────────────────────────

    #[test]
    fn serde_bow_roundtrip() {
        let p = make_bow_profile();
        let bow = BowState::new(0, &p, 15);
        let json = serde_json::to_string(&bow).unwrap();
        let bow2: BowState = serde_json::from_str(&json).unwrap();
        assert_eq!(bow2.weapon.ammo, 15);
        assert!(bow2.long_shoot_available);
        assert_eq!(bow2.current_mode, ShootMode::Normal);
    }

    #[test]
    fn serde_sword_roundtrip() {
        let mut sword = SwordState::new(2);
        sword.strike();
        sword.strike();
        let json = serde_json::to_string(&sword).unwrap();
        let sword2: SwordState = serde_json::from_str(&json).unwrap();
        assert_eq!(sword2.weapon.profile_idx, 2);
        assert_eq!(sword2.current_thrust, 2);
    }
}
