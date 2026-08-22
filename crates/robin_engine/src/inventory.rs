//! Inventory system — picking up, equipping, dropping, and using items.
//!
//! ## Design
//!
//! Inventory logic is consolidated into pure functions that operate on
//! `PcStatus` + `CharacterProfile` + `DifficultyLevel`, making the logic
//! testable without a full entity hierarchy.
//!
//! Items in the game world are `ElementBonus` entities with an `ObjectData`
//! that carries `associated_action` (which action the item feeds) and
//! `quantity` (how many units the pickup contains).

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::ObjectType;
use crate::pc_status::{LIFEPOINTS_PC, PcStatus};
use crate::player_profile::DifficultyLevel;
use crate::profiles::{Action, CharacterProfile, NUMBER_OF_PC_ACTIONS};

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
//  Action ↔ ObjectType helpers
// ═══════════════════════════════════════════════════════════════════

/// Whether this action consumes ammo (has an associated inventory counter).
pub fn action_uses_ammo(action: Action) -> bool {
    matches!(
        action,
        Action::Bow
            | Action::Stone
            | Action::Apple
            | Action::Ale
            | Action::Eat
            | Action::Guzzle
            | Action::Heal
            | Action::Net
            | Action::WaspNest
            | Action::Purse
    )
}

/// Convert an action to the corresponding **bonus** object type for
/// spawning a dropped pickup on the map.
///
/// Maps every ammo-action to a `Bonus*` variant — the static-pickup
/// sprite set.  Callers that want the in-flight *projectile* variant
/// (mid-throw) should use [`action_to_projectile_type`] instead.
pub fn action_to_object_type(action: Action) -> ObjectType {
    match action {
        Action::Bow => ObjectType::BonusArrow,
        Action::Stone => ObjectType::BonusStone,
        Action::Apple => ObjectType::BonusApple,
        Action::Ale => ObjectType::BonusAle,
        Action::Eat | Action::Guzzle => ObjectType::BonusLambLeg,
        Action::Heal => ObjectType::BonusPlants,
        Action::Net => ObjectType::BonusNet,
        Action::WaspNest => ObjectType::BonusWaspNest,
        Action::Purse => ObjectType::BonusPurse,
        _ => ObjectType::None,
    }
}

/// Convert an action to the corresponding **projectile** object type
/// for in-flight throws (arrow in the air, stone flying, etc.).
///
/// Throws spawn the non-bonus variant whose sprite-master is the
/// "ACCESSORIES_*" set — the ambiance variant used for motion.  Landed
/// projectiles that persist as pickups still carry this variant; the
/// dropped-pickup path [`action_to_object_type`] uses the `Bonus*`
/// variant instead.
pub fn action_to_projectile_type(action: Action) -> ObjectType {
    match action {
        Action::Bow => ObjectType::Arrow,
        Action::Stone => ObjectType::Stone,
        Action::Apple => ObjectType::Apple,
        Action::Ale => ObjectType::Ale,
        Action::Net => ObjectType::Net,
        Action::WaspNest => ObjectType::WaspNest,
        Action::Purse => ObjectType::Purse,
        _ => ObjectType::None,
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

/// Number of coins contained in each purse.
pub const COINS_PER_PURSE: u16 = 5;

/// Value of a single coin in ransom currency.
pub const COIN_VALUE: u32 = 10;

/// Base HP restored by a healing plant.
pub const HEAL_AMOUNT: i16 = 25;

/// Base HP restored by eating a food ration.
pub const EAT_HEAL_AMOUNT: i16 = 15;

// ═══════════════════════════════════════════════════════════════════
//  Difficulty-scaled capacity
// ═══════════════════════════════════════════════════════════════════

/// Get the maximum ammo for a given action slot, applying difficulty scaling.
///
/// Reads `action_max_ammo[slot]` from the character profile and scales
/// it based on difficulty:
///   - Easy:   6→8,  12→15  (roughly ×1.33)
///   - Normal: no change
///   - Hard:   6→4,  12→9   (roughly ×0.67)
pub fn max_ammo_for_slot(
    profile: &CharacterProfile,
    slot_index: usize,
    difficulty: DifficultyLevel,
) -> u16 {
    if slot_index >= NUMBER_OF_PC_ACTIONS {
        return 0;
    }
    let base = profile.action_max_ammo[slot_index];
    apply_difficulty_scaling(base, difficulty)
}

/// Find which action slot (0..NUMBER_OF_PC_ACTIONS) in the profile matches
/// the given action. Returns `None` if the character doesn't have that action.
///
/// Includes an `Eat → Guzzle` fallback so a hero whose profile lists only
/// `Guzzle` (e.g. Little John) still resolves an `Action::Eat` lookup to
/// the Guzzle slot. Without this fallback `max_ammo_for_action(profile,
/// Eat, _)` returns 0 for a Guzzle-only profile and breaks the cap check
/// on lamb-leg pickups.
pub fn find_action_slot(profile: &CharacterProfile, action: Action) -> Option<usize> {
    if let Some(idx) = profile.actions.iter().position(|&a| a == action) {
        return Some(idx);
    }
    if action == Action::Eat {
        return profile.actions.iter().position(|&a| a == Action::Guzzle);
    }
    None
}

/// Get the max ammo for a specific action, applying difficulty scaling.
/// Returns 0 if the character's profile doesn't include that action.
pub fn max_ammo_for_action(
    profile: &CharacterProfile,
    action: Action,
    difficulty: DifficultyLevel,
) -> u16 {
    match find_action_slot(profile, action) {
        Some(slot) => max_ammo_for_slot(profile, slot, difficulty),
        None => 0,
    }
}

/// Apply difficulty scaling to a base ammo capacity.
///
/// Easy maps 6→8 / 12→15, Hard maps 6→4 / 12→9, every other base value
/// passes through unchanged. An arithmetic form (e.g. ×1.33 / ×0.67)
/// would rewrite non-tabulated bases (8 → 10 on Easy), so the literal
/// `match` keeps modded / future capacities in step.
fn apply_difficulty_scaling(base: u16, difficulty: DifficultyLevel) -> u16 {
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

// ═══════════════════════════════════════════════════════════════════
//  Action slot state
// ═══════════════════════════════════════════════════════════════════

/// State of a PC's 3 quick-action slots.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ActionSlots {
    /// Currently selected action.
    pub current_action: Action,
    /// Saved action for restoration after temporary switches.
    pub saved_action: Action,
    /// Whether each slot is disabled (e.g., out of ammo).
    pub disabled: [bool; NUMBER_OF_PC_ACTIONS],
    /// Temporary disables (e.g., inside a building).
    pub disabled_temp: [bool; NUMBER_OF_PC_ACTIONS],
}

impl Default for ActionSlots {
    fn default() -> Self {
        Self {
            current_action: Action::NoAction,
            saved_action: Action::NoAction,
            disabled: [false; NUMBER_OF_PC_ACTIONS],
            disabled_temp: [false; NUMBER_OF_PC_ACTIONS],
        }
    }
}

impl ActionSlots {
    /// Create from a character profile, initializing the current action
    /// to the first available action.
    pub fn from_profile(profile: &CharacterProfile) -> Self {
        let current = profile
            .actions
            .iter()
            .copied()
            .find(|a| *a != Action::NoAction)
            .unwrap_or(Action::NoAction);
        Self {
            current_action: current,
            saved_action: Action::NoAction,
            disabled: [false; NUMBER_OF_PC_ACTIONS],
            disabled_temp: [false; NUMBER_OF_PC_ACTIONS],
        }
    }

    /// Check if a given slot is effectively disabled (permanent or temporary).
    pub fn is_slot_disabled(&self, slot: usize) -> bool {
        if slot >= NUMBER_OF_PC_ACTIONS {
            return true;
        }
        self.disabled[slot] || self.disabled_temp[slot]
    }

    /// Disable a slot (e.g., ammo ran out).
    pub fn disable_slot(&mut self, slot: usize) {
        if slot < NUMBER_OF_PC_ACTIONS {
            self.disabled[slot] = true;
        }
    }

    /// Enable a slot (e.g., ammo was added).
    pub fn enable_slot(&mut self, slot: usize) {
        if slot < NUMBER_OF_PC_ACTIONS {
            self.disabled[slot] = false;
        }
    }

    /// Save the current action and switch to a new one.
    pub fn save_and_switch(&mut self, new_action: Action) {
        self.saved_action = self.current_action;
        self.current_action = new_action;
    }

    /// Restore the previously saved action.
    pub fn restore_saved(&mut self) {
        if self.saved_action != Action::NoAction {
            self.current_action = self.saved_action;
            self.saved_action = Action::NoAction;
        }
    }

    /// Update slot enable/disable state based on current ammo.
    /// Called after any ammo change to keep the slot toggles in sync
    /// with the underlying counters.
    pub fn update_from_ammo(&mut self, profile: &CharacterProfile, status: &PcStatus) {
        for (slot, &action) in profile.actions.iter().enumerate() {
            if action == Action::NoAction {
                continue;
            }
            if action_uses_ammo(action) {
                if status.get_ammo(action) == 0 {
                    self.disable_slot(slot);
                } else {
                    self.enable_slot(slot);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Item pickup
// ═══════════════════════════════════════════════════════════════════

/// Result of attempting to pick up a bonus item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupResult {
    /// How many units were actually taken.
    pub taken: u16,
    /// How many units remain on the ground (0 = item fully consumed).
    pub remainder: u16,
    /// Whether the bonus entity should be removed from the world.
    pub remove_from_world: bool,
}

/// Attempt to pick up a bonus item.
///
/// Greedy algorithm:
/// - Calculate available capacity: `max_ammo - current_ammo`
/// - If capacity >= object quantity: take all, remove object from world
/// - If capacity < object quantity: take what fits, leave remainder
///
/// Returns `None` if the action has no ammo counter (shouldn't happen for
/// valid bonus items).
pub fn take_object(
    status: &mut PcStatus,
    profile: &CharacterProfile,
    difficulty: DifficultyLevel,
    action: Action,
    object_quantity: u16,
) -> Option<PickupResult> {
    if !action_uses_ammo(action) {
        return None;
    }

    let max = max_ammo_for_action(profile, action, difficulty);
    // If the character doesn't have this action, max will be 0.
    // They can still pick up if there's any room (e.g., purses always work).
    let effective_max = if max == 0 && action == Action::Purse {
        // Purses are special — everyone can carry them, default capacity
        u16::MAX
    } else {
        max
    };

    let current = status.get_ammo(action);
    // Original computes this subtraction from two UWORD values and stores
    // the result in an ULONG (`RHElementActorPC::TakeObject`).  Integer
    // promotion therefore makes an over-capacity legacy status negative
    // before the ULONG conversion wraps it to a large positive capacity.
    // That quirk consumes the whole world pickup, while the later
    // `IncreaseAmmoAmount` range check rejects the inventory addition.
    let available = (i32::from(effective_max) - i32::from(current)) as u32;

    if available == 0 {
        return Some(PickupResult {
            taken: 0,
            remainder: object_quantity,
            remove_from_world: false,
        });
    }

    let taken = u32::from(object_quantity).min(available) as u16;
    status.increase_ammo(action, taken, effective_max);

    let remainder = object_quantity - taken;
    Some(PickupResult {
        taken,
        remainder,
        remove_from_world: remainder == 0,
    })
}

// ═══════════════════════════════════════════════════════════════════
//  Item drop
// ═══════════════════════════════════════════════════════════════════

/// Description of an item to spawn on the map when dropped.
#[derive(Debug, Clone)]
pub struct DroppedItem {
    pub action: Action,
    pub object_type: ObjectType,
    pub quantity: u16,
    pub position: MapPoint,
}

/// Drop one unit of ammo from inventory onto the map.
///
/// Decreases the ammo counter by `amount` and returns a description of
/// the item to spawn.
///
/// Returns `None` if the character has no ammo of that type.
pub fn drop_item(
    status: &mut PcStatus,
    action: Action,
    amount: u16,
    position: MapPoint,
) -> Option<DroppedItem> {
    let removed = status.decrease_ammo(action, amount);
    if removed == 0 {
        return None;
    }

    let object_type = action_to_object_type(action);
    if object_type == ObjectType::None {
        // Shouldn't happen for valid ammo actions, but defensive
        return None;
    }

    Some(DroppedItem {
        action,
        object_type,
        quantity: removed,
        position,
    })
}

// ═══════════════════════════════════════════════════════════════════
//  Item use / consumption
// ═══════════════════════════════════════════════════════════════════

/// What happened when an item was used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseItemResult {
    /// A healing plant was consumed, restoring HP.
    Healed { hp_restored: i16, new_hp: i16 },
    /// A food ration was consumed, restoring HP.
    Ate { hp_restored: i16, new_hp: i16 },
    /// A throwable item was consumed (projectile should be created by caller).
    Thrown {
        action: Action,
        object_type: ObjectType,
    },
    /// An ale was consumed (may cause drunk state).
    DrankAle,
    /// The item couldn't be used (no ammo, wrong context, etc.).
    Failed,
}

/// Use/consume an item from inventory.
///
/// This handles the inventory side of item usage. The caller is responsible
/// for creating projectile entities, applying combat effects, etc.
pub fn use_item(status: &mut PcStatus, action: Action) -> UseItemResult {
    let current = status.get_ammo(action);
    if current == 0 {
        return UseItemResult::Failed;
    }

    match action {
        Action::Heal => {
            status.decrease_ammo(Action::Heal, 1);
            let old_hp = status.life_points;
            let max_hp = LIFEPOINTS_PC;
            let restored = HEAL_AMOUNT.min(max_hp - old_hp);
            status.life_points = (old_hp + restored).min(max_hp);
            UseItemResult::Healed {
                hp_restored: restored,
                new_hp: status.life_points,
            }
        }
        Action::Eat | Action::Guzzle => {
            status.decrease_ammo(action, 1);
            let old_hp = status.life_points;
            let max_hp = LIFEPOINTS_PC;
            let restored = EAT_HEAL_AMOUNT.min(max_hp - old_hp);
            status.life_points = (old_hp + restored).min(max_hp);
            UseItemResult::Ate {
                hp_restored: restored,
                new_hp: status.life_points,
            }
        }
        Action::Ale => {
            status.decrease_ammo(Action::Ale, 1);
            UseItemResult::DrankAle
        }
        // Throwable items: bow, apple, stone, purse, wasp nest, net
        Action::Bow
        | Action::Apple
        | Action::Stone
        | Action::Purse
        | Action::WaspNest
        | Action::Net => {
            status.decrease_ammo(action, 1);
            UseItemResult::Thrown {
                action,
                // Throws use the non-bonus (projectile) variant —
                // its "ACCESSORIES_*" sprite is the in-flight sprite.
                object_type: action_to_projectile_type(action),
            }
        }
        _ => UseItemResult::Failed,
    }
}

/// Check whether a PC can use a specific action right now.
pub fn can_use_action(status: &PcStatus, action: Action) -> bool {
    if action_uses_ammo(action) {
        status.get_ammo(action) > 0
    } else {
        // Non-ammo actions (hit, strangle, etc.) are always available
        // if the character has them in their profile
        true
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ObjectTypeExt;
    fn test_profile() -> CharacterProfile {
        CharacterProfile {
            actions: [Action::Bow, Action::Apple, Action::Purse],
            action_max_ammo: [12, 6, 6],
            ..CharacterProfile::default()
        }
    }

    // ── Difficulty scaling ──

    #[test]
    fn difficulty_scaling_medium_unchanged() {
        assert_eq!(apply_difficulty_scaling(6, DifficultyLevel::Medium), 6);
        assert_eq!(apply_difficulty_scaling(12, DifficultyLevel::Medium), 12);
    }

    #[test]
    fn difficulty_scaling_easy_increases() {
        assert_eq!(apply_difficulty_scaling(6, DifficultyLevel::Easy), 8);
        assert_eq!(apply_difficulty_scaling(12, DifficultyLevel::Easy), 15);
    }

    #[test]
    fn difficulty_scaling_hard_decreases() {
        assert_eq!(apply_difficulty_scaling(6, DifficultyLevel::Hard), 4);
        assert_eq!(apply_difficulty_scaling(12, DifficultyLevel::Hard), 9);
    }

    // ── Max ammo ──

    #[test]
    fn max_ammo_for_action_found() {
        let profile = test_profile();
        assert_eq!(
            max_ammo_for_action(&profile, Action::Bow, DifficultyLevel::Medium),
            12
        );
        assert_eq!(
            max_ammo_for_action(&profile, Action::Apple, DifficultyLevel::Medium),
            6
        );
    }

    #[test]
    fn max_ammo_for_action_not_found() {
        let profile = test_profile();
        assert_eq!(
            max_ammo_for_action(&profile, Action::Net, DifficultyLevel::Medium),
            0
        );
    }

    #[test]
    fn max_ammo_with_difficulty() {
        let profile = test_profile();
        assert_eq!(
            max_ammo_for_action(&profile, Action::Bow, DifficultyLevel::Easy),
            15
        );
        assert_eq!(
            max_ammo_for_action(&profile, Action::Bow, DifficultyLevel::Hard),
            9
        );
    }

    // ── increase/decrease ammo ──

    #[test]
    fn increase_ammo_rejects_when_overflow() {
        // When `current + amount > max` the add is rejected entirely,
        // not clipped.
        let mut status = PcStatus::default();
        let added = status.increase_ammo(Action::Bow, 20, 12);
        assert_eq!(added, 0);
        assert_eq!(status.get_ammo(Action::Bow), 0);
    }

    #[test]
    fn increase_ammo_rejects_partial_overflow() {
        // All-or-nothing — 10 + 5 = 15 > max(12) → reject.
        let mut status = PcStatus::default();
        status.set_ammo(Action::Bow, 10);
        let added = status.increase_ammo(Action::Bow, 5, 12);
        assert_eq!(added, 0);
        assert_eq!(status.get_ammo(Action::Bow), 10);
    }

    #[test]
    fn increase_ammo_accepts_exact_fit() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Bow, 10);
        let added = status.increase_ammo(Action::Bow, 2, 12);
        assert_eq!(added, 2);
        assert_eq!(status.get_ammo(Action::Bow), 12);
    }

    #[test]
    fn decrease_ammo_partial() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Bow, 3);
        let removed = status.decrease_ammo(Action::Bow, 5);
        assert_eq!(removed, 3);
        assert_eq!(status.get_ammo(Action::Bow), 0);
    }

    // ── Pickup ──

    #[test]
    fn pickup_full() {
        let mut status = PcStatus::default();
        let profile = test_profile();
        let result = take_object(
            &mut status,
            &profile,
            DifficultyLevel::Medium,
            Action::Bow,
            5,
        )
        .unwrap();
        assert_eq!(result.taken, 5);
        assert_eq!(result.remainder, 0);
        assert!(result.remove_from_world);
        assert_eq!(status.get_ammo(Action::Bow), 5);
    }

    #[test]
    fn pickup_partial() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Bow, 10);
        let profile = test_profile();
        let result = take_object(
            &mut status,
            &profile,
            DifficultyLevel::Medium,
            Action::Bow,
            5,
        )
        .unwrap();
        assert_eq!(result.taken, 2);
        assert_eq!(result.remainder, 3);
        assert!(!result.remove_from_world);
        assert_eq!(status.get_ammo(Action::Bow), 12);
    }

    #[test]
    fn pickup_full_inventory() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Bow, 12);
        let profile = test_profile();
        let result = take_object(
            &mut status,
            &profile,
            DifficultyLevel::Medium,
            Action::Bow,
            3,
        )
        .unwrap();
        assert_eq!(result.taken, 0);
        assert_eq!(result.remainder, 3);
        assert!(!result.remove_from_world);
    }

    #[test]
    fn pickup_over_capacity_consumes_world_object_without_adding_ammo() {
        // Original's UWORD subtraction is assigned to ULONG.  A save can
        // retain six units after Hard difficulty lowers this capacity to
        // four; the wrapped capacity removes the pickup, while
        // IncreaseAmmoAmount rejects 6 + 3 > 4.
        let mut status = PcStatus::default();
        status.set_ammo(Action::Apple, 6);
        let profile = CharacterProfile {
            actions: [Action::Apple, Action::NoAction, Action::NoAction],
            action_max_ammo: [6, 0, 0],
            ..CharacterProfile::default()
        };

        let result = take_object(
            &mut status,
            &profile,
            DifficultyLevel::Hard,
            Action::Apple,
            3,
        )
        .expect("Apple has an ammo counter");

        assert_eq!(result.taken, 3);
        assert_eq!(result.remainder, 0);
        assert!(result.remove_from_world);
        assert_eq!(status.get_ammo(Action::Apple), 6);
    }

    // ── Drop ──

    #[test]
    fn drop_item_basic() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Apple, 4);
        let pos = MapPoint { x: 100.0, y: 200.0 };
        let dropped = drop_item(&mut status, Action::Apple, 2, pos).unwrap();
        assert_eq!(dropped.quantity, 2);
        // Dropped pickups spawn as the Bonus* variant — never the
        // projectile variant, which is reserved for in-flight throws.
        assert_eq!(dropped.object_type, ObjectType::BonusApple);
        assert_eq!(status.get_ammo(Action::Apple), 2);
    }

    #[test]
    fn drop_item_no_ammo() {
        let mut status = PcStatus::default();
        let pos = MapPoint { x: 0.0, y: 0.0 };
        assert!(drop_item(&mut status, Action::Apple, 1, pos).is_none());
    }

    // ── Use item ──

    #[test]
    fn use_heal() {
        let mut status = PcStatus {
            life_points: 60,
            ..Default::default()
        };
        status.set_ammo(Action::Heal, 3);
        let result = use_item(&mut status, Action::Heal);
        assert_eq!(
            result,
            UseItemResult::Healed {
                hp_restored: HEAL_AMOUNT,
                new_hp: 85
            }
        );
        assert_eq!(status.get_ammo(Action::Heal), 2);
    }

    #[test]
    fn use_heal_caps_at_max() {
        let mut status = PcStatus {
            life_points: 90,
            ..Default::default()
        };
        status.set_ammo(Action::Heal, 1);
        let result = use_item(&mut status, Action::Heal);
        assert_eq!(
            result,
            UseItemResult::Healed {
                hp_restored: 10,
                new_hp: LIFEPOINTS_PC
            }
        );
    }

    #[test]
    fn use_eat() {
        let mut status = PcStatus {
            life_points: 70,
            ..Default::default()
        };
        status.set_ammo(Action::Eat, 2);
        let result = use_item(&mut status, Action::Eat);
        assert_eq!(
            result,
            UseItemResult::Ate {
                hp_restored: EAT_HEAL_AMOUNT,
                new_hp: 85
            }
        );
        assert_eq!(status.get_ammo(Action::Eat), 1);
    }

    #[test]
    fn use_throwable() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Stone, 5);
        let result = use_item(&mut status, Action::Stone);
        assert_eq!(
            result,
            UseItemResult::Thrown {
                action: Action::Stone,
                object_type: ObjectType::Stone,
            }
        );
        assert_eq!(status.get_ammo(Action::Stone), 4);
    }

    #[test]
    fn use_no_ammo_fails() {
        let mut status = PcStatus::default();
        let result = use_item(&mut status, Action::Bow);
        assert_eq!(result, UseItemResult::Failed);
    }

    #[test]
    fn use_ale() {
        let mut status = PcStatus::default();
        status.set_ammo(Action::Ale, 2);
        let result = use_item(&mut status, Action::Ale);
        assert_eq!(result, UseItemResult::DrankAle);
        assert_eq!(status.get_ammo(Action::Ale), 1);
    }

    // ── Action slots ──

    #[test]
    fn action_slots_from_profile() {
        let profile = test_profile();
        let slots = ActionSlots::from_profile(&profile);
        assert_eq!(slots.current_action, Action::Bow);
        assert!(!slots.is_slot_disabled(0));
    }

    #[test]
    fn action_slots_disable_enable() {
        let mut slots = ActionSlots::default();
        slots.disable_slot(0);
        assert!(slots.is_slot_disabled(0));
        slots.enable_slot(0);
        assert!(!slots.is_slot_disabled(0));
    }

    #[test]
    fn action_slots_update_from_ammo() {
        let profile = test_profile();
        let mut slots = ActionSlots::from_profile(&profile);
        let mut status = PcStatus::default();
        // All ammo at 0 → bow and apple should be disabled, purse too
        slots.update_from_ammo(&profile, &status);
        assert!(slots.is_slot_disabled(0)); // Bow
        assert!(slots.is_slot_disabled(1)); // Apple
        assert!(slots.is_slot_disabled(2)); // Purse

        // Give some arrows → bow slot re-enabled
        status.set_ammo(Action::Bow, 5);
        slots.update_from_ammo(&profile, &status);
        assert!(!slots.is_slot_disabled(0));
        assert!(slots.is_slot_disabled(1));
    }

    #[test]
    fn action_slots_save_restore() {
        let mut slots = ActionSlots {
            current_action: Action::Bow,
            ..Default::default()
        };
        slots.save_and_switch(Action::Heal);
        assert_eq!(slots.current_action, Action::Heal);
        assert_eq!(slots.saved_action, Action::Bow);
        slots.restore_saved();
        assert_eq!(slots.current_action, Action::Bow);
        assert_eq!(slots.saved_action, Action::NoAction);
    }

    // ── Bonus type conversions ──

    #[test]
    fn bonus_to_action_conversions() {
        use crate::element::{BonusItemType, BonusItemTypeExt};
        assert_eq!(BonusItemType::Arrow.to_action(), Action::Bow);
        assert_eq!(BonusItemType::Plant.to_action(), Action::Heal);
        assert_eq!(BonusItemType::Lamb.to_action(), Action::Eat);
        assert_eq!(BonusItemType::Purse.to_action(), Action::Purse);
        // Relics (including Ransom) map to NoAction.
        assert_eq!(BonusItemType::Ransom.to_action(), Action::NoAction);
        assert_eq!(BonusItemType::Ampulla.to_action(), Action::NoAction);
    }

    #[test]
    fn object_type_to_action_conversions() {
        assert_eq!(ObjectType::Arrow.to_action(), Action::Bow);
        assert_eq!(ObjectType::BonusArrow.to_action(), Action::Bow);
        assert_eq!(ObjectType::BonusPlants.to_action(), Action::Heal);
        assert_eq!(ObjectType::Purse.to_action(), Action::Purse);
        assert_eq!(ObjectType::None.to_action(), Action::NoAction);
    }

    #[test]
    fn action_to_object_type_round_trip() {
        // Dropped-pickup variant.
        assert_eq!(action_to_object_type(Action::Bow), ObjectType::BonusArrow);
        assert_eq!(action_to_object_type(Action::Stone), ObjectType::BonusStone);
        assert_eq!(action_to_object_type(Action::Ale), ObjectType::BonusAle);
        assert_eq!(action_to_object_type(Action::Heal), ObjectType::BonusPlants);
        assert_eq!(action_to_object_type(Action::Net), ObjectType::BonusNet);
        assert_eq!(action_to_object_type(Action::Purse), ObjectType::BonusPurse,);
        assert_eq!(action_to_object_type(Action::Hit), ObjectType::None);
    }

    #[test]
    fn action_to_projectile_type_round_trip() {
        // In-flight variant — non-bonus master sprite for the flying
        // object.
        assert_eq!(action_to_projectile_type(Action::Bow), ObjectType::Arrow);
        assert_eq!(action_to_projectile_type(Action::Stone), ObjectType::Stone,);
        assert_eq!(action_to_projectile_type(Action::Net), ObjectType::Net);
        assert_eq!(action_to_projectile_type(Action::Purse), ObjectType::Purse,);
        assert_eq!(action_to_projectile_type(Action::Hit), ObjectType::None);
    }

    // ── can_use_action ──

    #[test]
    fn can_use_action_ammo() {
        let mut status = PcStatus::default();
        assert!(!can_use_action(&status, Action::Bow));
        status.set_ammo(Action::Bow, 1);
        assert!(can_use_action(&status, Action::Bow));
    }

    #[test]
    fn can_use_action_non_ammo() {
        let status = PcStatus::default();
        assert!(can_use_action(&status, Action::Hit));
        assert!(can_use_action(&status, Action::Strangle));
    }
}
