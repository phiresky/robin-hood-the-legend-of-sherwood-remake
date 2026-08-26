//! Human and player-character status structs.
//!
//! These structs track a character's dynamic state: HP, combat skills,
//! and inventory (ammo counts for each action type). They are the
//! "mutable" counterpart to the static `CharacterProfile` data.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::player_profile::DifficultyLevel;
use crate::profiles::{Action, CharacterProfile, NUMBER_OF_PC_ACTIONS};
use crate::sherwood_stat::MenuTextLookup;

// ─── Constants ───────────────────────────────────────────────────

/// Default life points for a player character.
pub const LIFEPOINTS_PC: i16 = 100;

/// Score bonus awarded to the campaign whenever a PC crosses a 100-XP
/// boundary (i.e. skill capacity increases).
pub const PC_ADDITIONAL_CAPACITY_POINTS: i32 = 100;

// ─── Special-peasant menu-text ids ──────────────────────────────
//
// Used by the `PROP_NAME` script native to overwrite a rescued PC's
// name with the localized peasant string.

/// Menu-text id for SPECIAL_PEASANT_A.
pub const MT_STR_SPECIAL_PEASANT_A: usize = 250;
/// Menu-text id for SPECIAL_PEASANT_B.
pub const MT_STR_SPECIAL_PEASANT_B: usize = 251;
/// Menu-text id for SPECIAL_PEASANT_C.
pub const MT_STR_SPECIAL_PEASANT_C: usize = 252;

/// The amount enum passed to the `PROP_NAME` script native (NAME_A/B/C).
#[repr(u8)]
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
pub enum SpecialPeasantName {
    A = 0,
    B = 1,
    C = 2,
}

impl SpecialPeasantName {
    /// Map the script `iAmount` (0/1/2) to a [`SpecialPeasantName`].
    /// Returns `None` for any other value (the script default arm logs
    /// "invalid name ID" and returns `false`).
    pub fn from_amount(amount: i32) -> Option<Self> {
        match amount {
            0 => Some(Self::A),
            1 => Some(Self::B),
            2 => Some(Self::C),
            _ => None,
        }
    }

    /// Return the menu-text id this name resolves to.
    pub fn menu_text_id(self) -> usize {
        match self {
            Self::A => MT_STR_SPECIAL_PEASANT_A,
            Self::B => MT_STR_SPECIAL_PEASANT_B,
            Self::C => MT_STR_SPECIAL_PEASANT_C,
        }
    }
}

// ─── Skill ──────────────────────────────────────────────────────

/// Names of the trainable combat skills.
#[repr(u32)]
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
pub enum SkillName {
    HandToHand = 0,
    Bow = 1,
}

/// A single combat skill with experience and capacity (0–100 each).
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Skill {
    pub experience: u32,
    pub capacity: u32,
}

// ─── HumanStatus ────────────────────────────────────────────────

/// Dynamic combat stats shared by all human characters.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HumanStatus {
    pub hand_to_hand: Skill,
    pub bow: Skill,
}

impl HumanStatus {
    /// Create from a character profile's fighting/shooting stats.
    pub fn from_profile_stats(fighting: u32, shooting: u32) -> Self {
        HumanStatus {
            hand_to_hand: Skill {
                capacity: fighting,
                experience: 0,
            },
            bow: Skill {
                capacity: shooting,
                experience: 0,
            },
        }
    }

    /// Get a mutable reference to the skill by name.
    pub fn skill_mut(&mut self, name: SkillName) -> &mut Skill {
        match name {
            SkillName::HandToHand => &mut self.hand_to_hand,
            SkillName::Bow => &mut self.bow,
        }
    }

    /// Get an immutable reference to the skill by name.
    pub fn skill(&self, name: SkillName) -> &Skill {
        match name {
            SkillName::HandToHand => &self.hand_to_hand,
            SkillName::Bow => &self.bow,
        }
    }

    /// Set the capacity of a skill directly.
    pub fn set_capacity(&mut self, name: SkillName, capacity: u32) {
        self.skill_mut(name).capacity = capacity;
    }

    /// Add experience to a skill. When experience reaches 100, it
    /// rolls over into capacity (capacity capped at 100).
    pub fn add_experience(&mut self, name: SkillName, additional: u32) {
        let skill = self.skill_mut(name);
        skill.experience += additional;

        if skill.capacity < 100 && skill.experience >= 100 {
            skill.capacity += skill.experience / 100;
            skill.experience %= 100;

            if skill.capacity > 100 {
                skill.capacity = 100;
            }
        }
    }

    /// Scale experience by a coefficient. Overflow rolls into capacity.
    ///
    /// Windows retail multiplies in floating point and converts the
    /// completed value back to `ULONG`.
    pub fn scale_experience(&mut self, name: SkillName, coefficient: f32) {
        let skill = self.skill_mut(name);
        // Promote the binary32 argument without changing its value; the
        // Windows executable performs the multiplication in x87 extended
        // precision and converts only the product.
        skill.experience = (skill.experience as f64 * coefficient as f64) as u32;

        if skill.capacity < 100 && skill.experience >= 100 {
            skill.capacity += skill.experience / 100;
            skill.experience %= 100;

            if skill.capacity > 100 {
                skill.capacity = 100;
            }
        }
    }
}

// ─── PcStatus ───────────────────────────────────────────────────

/// Full dynamic state for a player character.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcStatus {
    // ── Skills (from HumanStatus base) ──
    pub human_status: HumanStatus,

    // ── Life ──
    pub life_points: i16,
    pub in_coma: bool,

    // ── Inventory / ammo counters ──
    pub num_ales: u16,
    pub num_arrows: u16,
    pub num_apples: u16,
    pub num_rations: u16,
    pub num_stones: u16,
    pub num_wasp_nests: u16,
    pub num_nets: u16,
    pub num_plants: u16,
    pub num_purses: u16,

    // ── Identity ──
    pub name: String,

    /// Localized override applied by the `PROP_NAME` script native —
    /// set when a mission renames a rescued PC to one of the
    /// SPECIAL_PEASANT_A/B/C menu strings.  We store the id rather
    /// than the resolved string so the localized text can be looked
    /// up via [`MenuTextLookup`] at display time.  See
    /// [`PcStatus::display_name`].
    pub name_override: Option<SpecialPeasantName>,

    // ── Sherwood map position (–1 = not placed) ──
    pub beam_me_index_in_sherwood: i16,
}

impl Default for PcStatus {
    fn default() -> Self {
        PcStatus {
            human_status: HumanStatus::default(),
            life_points: LIFEPOINTS_PC,
            in_coma: false,
            num_ales: 0,
            num_arrows: 0,
            num_apples: 0,
            num_rations: 0,
            num_stones: 0,
            num_wasp_nests: 0,
            num_nets: 0,
            num_plants: 0,
            num_purses: 0,
            name: String::new(),
            name_override: None,
            beam_me_index_in_sherwood: -1,
        }
    }
}

/// Apply the difficulty-tiered "full pockets" multiplier to one
/// max-ammo value.  Only the hardcoded max values `6` and `12` are
/// remapped; any other value passes through unchanged.
pub fn scale_full_pockets_ammo(base: u16, difficulty: DifficultyLevel) -> u16 {
    match difficulty {
        DifficultyLevel::Easy => match base {
            6 => 8,
            12 => 15,
            other => other,
        },
        DifficultyLevel::Medium => base,
        DifficultyLevel::Hard => match base {
            6 => 4,
            12 => 9,
            other => other,
        },
    }
}

/// Apply a healing amount in place, with the same guards as the
/// underlying SetLifePoints flow:
///
/// 1. Dead targets stay dead.
/// 2. Sum is capped at [`LIFEPOINTS_PC`].
/// 3. `invulnerable` pins the result at [`LIFEPOINTS_PC`].
///
/// The Sherwood-PC immunity branch (skip when new value < current)
/// can never fire on a positive heal — the sum is always ≥ current —
/// so it is intentionally omitted.
pub fn heal(life_points: &mut i16, amount: i16, invulnerable: bool) {
    if *life_points <= 0 {
        return;
    }
    let summed = (*life_points as i32).saturating_add(amount as i32);
    let capped = summed.min(LIFEPOINTS_PC as i32) as i16;
    *life_points = if invulnerable { LIFEPOINTS_PC } else { capped };
}

impl PcStatus {
    /// Build a `PcStatus` for a character profile.
    ///
    /// When `with_full_pockets` is true, each action slot is filled
    /// from `profile.action_max_ammo[i]` scaled by `difficulty` (see
    /// `scale_full_pockets_ammo`); when false, all ammo counters stay
    /// at zero.  Skills are seeded from the profile's fighting/shooting
    /// stats.
    ///
    /// Naming is handled separately — leave `name` empty here and let
    /// callers populate it if they need a display name.
    pub fn from_profile(
        profile: &CharacterProfile,
        with_full_pockets: bool,
        difficulty: DifficultyLevel,
    ) -> Self {
        let mut status = Self {
            human_status: HumanStatus::from_profile_stats(
                profile.fighting as u32,
                profile.shooting as u32,
            ),
            ..Self::default()
        };
        if with_full_pockets {
            for i in 0..NUMBER_OF_PC_ACTIONS {
                let action = profile.actions[i];
                let amount = scale_full_pockets_ammo(profile.action_max_ammo[i], difficulty);
                status.set_ammo(action, amount);
            }
        }
        status
    }

    /// Get a mutable reference to the ammo counter for the given action.
    /// Returns `None` for actions that don't consume ammo.
    pub fn ammo_counter_mut(&mut self, action: Action) -> Option<&mut u16> {
        match action {
            Action::Ale => Some(&mut self.num_ales),
            Action::Apple => Some(&mut self.num_apples),
            Action::Bow => Some(&mut self.num_arrows),
            Action::Eat | Action::Guzzle => Some(&mut self.num_rations),
            Action::Net => Some(&mut self.num_nets),
            Action::Stone => Some(&mut self.num_stones),
            Action::Heal => Some(&mut self.num_plants),
            Action::Purse => Some(&mut self.num_purses),
            Action::WaspNest => Some(&mut self.num_wasp_nests),
            _ => None,
        }
    }

    /// Get the ammo count for the given action.
    ///
    /// Returns `u16::MAX` (`0xFFFF`) for actions that don't have an
    /// ammo counter — callers detect the "no counter" case via this
    /// sentinel.
    pub fn get_ammo(&self, action: Action) -> u16 {
        match action {
            Action::Ale => self.num_ales,
            Action::Apple => self.num_apples,
            Action::Bow => self.num_arrows,
            Action::Eat | Action::Guzzle => self.num_rations,
            Action::Net => self.num_nets,
            Action::Stone => self.num_stones,
            Action::Heal => self.num_plants,
            Action::Purse => self.num_purses,
            Action::WaspNest => self.num_wasp_nests,
            _ => u16::MAX,
        }
    }

    /// Set the ammo amount for a given action.
    /// No-op for actions that don't consume ammo (hit, strangle, etc.).
    pub fn set_ammo(&mut self, action: Action, quantity: u16) {
        match action {
            Action::Ale => self.num_ales = quantity,
            Action::Apple => self.num_apples = quantity,
            Action::Bow => self.num_arrows = quantity,
            Action::Eat | Action::Guzzle => self.num_rations = quantity,
            Action::Net => self.num_nets = quantity,
            Action::Stone => self.num_stones = quantity,
            Action::Heal => self.num_plants = quantity,
            Action::Purse => self.num_purses = quantity,
            Action::WaspNest => self.num_wasp_nests = quantity,
            // Actions without ammo — intentional no-op
            Action::NoAction
            | Action::Hit
            | Action::HitHard
            | Action::Lever
            | Action::HelpToClimb
            | Action::Beggar
            | Action::Shield
            | Action::BigShield
            | Action::Strangle
            | Action::Listen
            | Action::Whistle => {}
            other => {
                tracing::warn!("set_ammo: unhandled action {:?}", other);
            }
        }
    }

    /// Increase ammo for a given action, all-or-nothing.
    ///
    /// The add is rejected outright when `current + amount > max`.
    /// Callers that need clipping must pre-cap the amount.
    ///
    /// Returns the amount actually added (0 if rejected or the action
    /// has no counter).
    pub fn increase_ammo(&mut self, action: Action, amount: u16, max: u16) -> u16 {
        if let Some(counter) = self.ammo_counter_mut(action) {
            let current = *counter;
            if let Some(next) = current.checked_add(amount)
                && next <= max
            {
                *counter = next;
                amount
            } else {
                0
            }
        } else {
            0
        }
    }

    /// Decrease ammo for a given action, floored at 0.
    /// Returns the actual amount removed (may be less if not enough ammo).
    pub fn decrease_ammo(&mut self, action: Action, amount: u16) -> u16 {
        if let Some(counter) = self.ammo_counter_mut(action) {
            let current = *counter;
            let removed = amount.min(current);
            *counter = current - removed;
            removed
        } else {
            0
        }
    }

    /// Force-set ammo amount, ignoring capacity limits.
    /// For use in testing/scripting only.
    pub fn force_set_ammo(&mut self, action: Action, quantity: u16) {
        if let Some(counter) = self.ammo_counter_mut(action) {
            *counter = quantity;
        }
    }

    /// Resolve the PC's display name.
    ///
    /// When `name_override` is set (via the `PROP_NAME` script
    /// native), returns the localized menu-text string for the
    /// chosen SPECIAL_PEASANT slot.  Otherwise returns
    /// [`Self::name`].  Falls back to the raw `name` if the menu-text
    /// table returns an empty string for the override id — keep the
    /// previous name rather than show "".
    pub fn display_name<'a>(&'a self, menu_text: &dyn MenuTextLookup) -> Cow<'a, str> {
        if let Some(slot) = self.name_override {
            let resolved = menu_text.get(slot.menu_text_id());
            if !resolved.is_empty() {
                return Cow::Owned(resolved);
            }
        }
        Cow::Borrowed(&self.name)
    }

    /// Reset all ammo counters to zero.
    pub fn reset_ammo(&mut self) {
        self.num_ales = 0;
        self.num_arrows = 0;
        self.num_apples = 0;
        self.num_rations = 0;
        self.num_stones = 0;
        self.num_wasp_nests = 0;
        self.num_nets = 0;
        self.num_plants = 0;
        self.num_purses = 0;
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_status_default() {
        let hs = HumanStatus::default();
        assert_eq!(
            hs.hand_to_hand,
            Skill {
                experience: 0,
                capacity: 0
            }
        );
        assert_eq!(
            hs.bow,
            Skill {
                experience: 0,
                capacity: 0
            }
        );
    }

    #[test]
    fn human_status_from_profile() {
        let hs = HumanStatus::from_profile_stats(50, 75);
        assert_eq!(hs.hand_to_hand.capacity, 50);
        assert_eq!(hs.bow.capacity, 75);
        assert_eq!(hs.hand_to_hand.experience, 0);
    }

    #[test]
    fn add_experience_levels_up() {
        let mut hs = HumanStatus::from_profile_stats(10, 0);
        hs.add_experience(SkillName::HandToHand, 250);
        // 250 / 100 = 2 capacity gained, 250 % 100 = 50 leftover
        assert_eq!(hs.hand_to_hand.capacity, 12);
        assert_eq!(hs.hand_to_hand.experience, 50);
    }

    #[test]
    fn add_experience_caps_at_100() {
        let mut hs = HumanStatus::from_profile_stats(95, 0);
        hs.add_experience(SkillName::HandToHand, 800);
        assert_eq!(hs.hand_to_hand.capacity, 100);
    }

    #[test]
    fn add_experience_no_levelup_when_at_max() {
        let mut hs = HumanStatus::from_profile_stats(100, 0);
        hs.add_experience(SkillName::HandToHand, 200);
        // Capacity already 100, so no change to capacity
        assert_eq!(hs.hand_to_hand.capacity, 100);
        assert_eq!(hs.hand_to_hand.experience, 200);
    }

    #[test]
    fn scale_experience() {
        let mut hs = HumanStatus::from_profile_stats(10, 0);
        hs.hand_to_hand.experience = 50;
        hs.scale_experience(SkillName::HandToHand, 3.0);
        // 50 * 3 = 150 => capacity += 1, experience = 50
        assert_eq!(hs.hand_to_hand.capacity, 11);
        assert_eq!(hs.hand_to_hand.experience, 50);
    }

    #[test]
    fn scale_experience_fractional_coefficient_scales_before_conversion() {
        // Windows retail computes 80 * 1.5 = 120 before converting,
        // then rolls the extra 100 into capacity.
        let mut hs = HumanStatus::from_profile_stats(10, 0);
        hs.hand_to_hand.experience = 80;
        hs.scale_experience(SkillName::HandToHand, 1.5);
        assert_eq!(hs.hand_to_hand.experience, 20);
        assert_eq!(hs.hand_to_hand.capacity, 11);
    }

    #[test]
    fn pc_status_default() {
        let ps = PcStatus::default();
        assert_eq!(ps.life_points, LIFEPOINTS_PC);
        assert!(!ps.in_coma);
        assert_eq!(ps.num_arrows, 0);
        assert_eq!(ps.beam_me_index_in_sherwood, -1);
    }

    #[test]
    fn pc_status_ammo_round_trip() {
        let mut ps = PcStatus::default();
        ps.set_ammo(Action::Bow, 12);
        ps.set_ammo(Action::Ale, 3);
        ps.set_ammo(Action::Eat, 5);
        assert_eq!(ps.get_ammo(Action::Bow), 12);
        assert_eq!(ps.get_ammo(Action::Ale), 3);
        assert_eq!(ps.get_ammo(Action::Eat), 5);
        // Guzzle shares the rations counter with Eat
        assert_eq!(ps.get_ammo(Action::Guzzle), 5);
    }

    #[test]
    fn pc_status_increase_ammo_rejects_u16_overflow() {
        let mut ps = PcStatus::default();
        ps.set_ammo(Action::Bow, u16::MAX);

        assert_eq!(ps.increase_ammo(Action::Bow, 1, u16::MAX), 0);
        assert_eq!(ps.get_ammo(Action::Bow), u16::MAX);
    }

    #[test]
    fn pc_status_reset_ammo() {
        let mut ps = PcStatus::default();
        ps.set_ammo(Action::Bow, 12);
        ps.set_ammo(Action::Stone, 6);
        ps.reset_ammo();
        assert_eq!(ps.get_ammo(Action::Bow), 0);
        assert_eq!(ps.get_ammo(Action::Stone), 0);
    }

    #[test]
    fn pc_status_set_ammo_noop_for_melee() {
        let mut ps = PcStatus::default();
        ps.set_ammo(Action::Hit, 99);
        // Hit has no ammo counter — `get_ammo` returns the `0xFFFF` /
        // `u16::MAX` sentinel for actions without a counter.
        assert_eq!(ps.get_ammo(Action::Hit), u16::MAX);
    }

    #[test]
    fn full_pockets_ammo_scaling_medium_passthrough() {
        assert_eq!(scale_full_pockets_ammo(0, DifficultyLevel::Medium), 0);
        assert_eq!(scale_full_pockets_ammo(6, DifficultyLevel::Medium), 6);
        assert_eq!(scale_full_pockets_ammo(12, DifficultyLevel::Medium), 12);
        assert_eq!(scale_full_pockets_ammo(42, DifficultyLevel::Medium), 42);
    }

    #[test]
    fn full_pockets_ammo_scaling_easy_hard() {
        // Only 6 and 12 are remapped.
        assert_eq!(scale_full_pockets_ammo(6, DifficultyLevel::Easy), 8);
        assert_eq!(scale_full_pockets_ammo(12, DifficultyLevel::Easy), 15);
        assert_eq!(scale_full_pockets_ammo(6, DifficultyLevel::Hard), 4);
        assert_eq!(scale_full_pockets_ammo(12, DifficultyLevel::Hard), 9);
        assert_eq!(scale_full_pockets_ammo(3, DifficultyLevel::Easy), 3);
        assert_eq!(scale_full_pockets_ammo(3, DifficultyLevel::Hard), 3);
    }

    #[test]
    fn from_profile_empty_pockets_leaves_zero_ammo() {
        let mut profile = CharacterProfile {
            fighting: 50,
            shooting: 75,
            ..Default::default()
        };
        profile.actions[0] = Action::Bow;
        profile.action_max_ammo[0] = 12;
        let ps = PcStatus::from_profile(&profile, false, DifficultyLevel::Medium);
        assert_eq!(ps.life_points, LIFEPOINTS_PC);
        assert_eq!(ps.get_ammo(Action::Bow), 0);
        assert_eq!(ps.human_status.hand_to_hand.capacity, 50);
        assert_eq!(ps.human_status.bow.capacity, 75);
    }

    #[test]
    fn from_profile_full_pockets_medium_matches_profile_max() {
        let mut profile = CharacterProfile::default();
        profile.actions[0] = Action::Bow;
        profile.action_max_ammo[0] = 12;
        profile.actions[1] = Action::Stone;
        profile.action_max_ammo[1] = 6;
        let ps = PcStatus::from_profile(&profile, true, DifficultyLevel::Medium);
        assert_eq!(ps.get_ammo(Action::Bow), 12);
        assert_eq!(ps.get_ammo(Action::Stone), 6);
    }

    #[test]
    fn from_profile_full_pockets_easy_buffs_known_caps() {
        let mut profile = CharacterProfile::default();
        profile.actions[0] = Action::Bow;
        profile.action_max_ammo[0] = 12;
        profile.actions[1] = Action::Stone;
        profile.action_max_ammo[1] = 6;
        let ps = PcStatus::from_profile(&profile, true, DifficultyLevel::Easy);
        assert_eq!(ps.get_ammo(Action::Bow), 15);
        assert_eq!(ps.get_ammo(Action::Stone), 8);
    }

    #[test]
    fn from_profile_full_pockets_hard_nerfs_known_caps() {
        let mut profile = CharacterProfile::default();
        profile.actions[0] = Action::Bow;
        profile.action_max_ammo[0] = 12;
        profile.actions[1] = Action::Stone;
        profile.action_max_ammo[1] = 6;
        let ps = PcStatus::from_profile(&profile, true, DifficultyLevel::Hard);
        assert_eq!(ps.get_ammo(Action::Bow), 9);
        assert_eq!(ps.get_ammo(Action::Stone), 4);
    }

    #[test]
    fn special_peasant_name_from_amount() {
        assert_eq!(
            SpecialPeasantName::from_amount(0),
            Some(SpecialPeasantName::A)
        );
        assert_eq!(
            SpecialPeasantName::from_amount(1),
            Some(SpecialPeasantName::B)
        );
        assert_eq!(
            SpecialPeasantName::from_amount(2),
            Some(SpecialPeasantName::C)
        );
        assert_eq!(SpecialPeasantName::from_amount(3), None);
        assert_eq!(SpecialPeasantName::from_amount(-1), None);
    }

    #[test]
    fn special_peasant_name_menu_text_id_matches_original_constants() {
        assert_eq!(SpecialPeasantName::A.menu_text_id(), 250);
        assert_eq!(SpecialPeasantName::B.menu_text_id(), 251);
        assert_eq!(SpecialPeasantName::C.menu_text_id(), 252);
    }

    #[test]
    fn display_name_uses_override_when_set() {
        struct Fake;
        impl MenuTextLookup for Fake {
            fn get(&self, id: usize) -> String {
                match id {
                    250 => "Aelfric".into(),
                    251 => "Beornred".into(),
                    252 => "Cuthbert".into(),
                    _ => String::new(),
                }
            }
        }

        let mut ps = PcStatus {
            name: "Default".into(),
            ..Default::default()
        };
        assert_eq!(ps.display_name(&Fake).as_ref(), "Default");

        ps.name_override = Some(SpecialPeasantName::B);
        assert_eq!(ps.display_name(&Fake).as_ref(), "Beornred");

        // Empty menu-text result falls back to the raw `name`.
        struct Empty;
        impl MenuTextLookup for Empty {
            fn get(&self, _id: usize) -> String {
                String::new()
            }
        }
        assert_eq!(ps.display_name(&Empty).as_ref(), "Default");
    }

    #[test]
    fn pc_status_serde_round_trip() {
        let mut ps = PcStatus {
            life_points: 75,
            in_coma: true,
            name: "Little John".into(),
            human_status: HumanStatus::from_profile_stats(80, 60),
            ..Default::default()
        };
        ps.set_ammo(Action::Bow, 15);

        let json = serde_json::to_string(&ps).unwrap();
        let ps2: PcStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(ps, ps2);
        assert_eq!(ps2.life_points, 75);
        assert!(ps2.in_coma);
        assert_eq!(ps2.name, "Little John");
        assert_eq!(ps2.get_ammo(Action::Bow), 15);
        assert_eq!(ps2.human_status.hand_to_hand.capacity, 80);
    }
}
