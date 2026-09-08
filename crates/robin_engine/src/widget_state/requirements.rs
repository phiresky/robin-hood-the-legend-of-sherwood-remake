//! Mission-requirements HUD widget state.
//!
//! Displays the icon strip for the next mission's required characters,
//! required actions, and optional PC slots.
//!
//! Immediate-mode: rather than holding cached icon widgets, we recompute
//! the requirements strip every frame from the attached mission +
//! current team via [`build_requirements_state`].

use crate::campaign::Campaign;
use crate::profiles::{Action, CharacterProfileIdx};

/// Status of a single requirement row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementStatus {
    Fulfilled,
    Missing,
}

/// One icon slot in the requirements strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementSlot {
    /// A named PC required for the mission.
    RequiredCharacter {
        character_profile_idx: CharacterProfileIdx,
        status: RequirementStatus,
        /// Whether the selected team contains this PC.
        selected: bool,
    },
    /// A specific action required for the mission.
    RequiredAction {
        action: Action,
        status: RequirementStatus,
        selected: bool,
    },
    /// Optional PC slot — either empty or filled with an optional team
    /// member.
    OptionalCharacter {
        /// `None` when the slot is empty.
        character_profile_idx: Option<CharacterProfileIdx>,
    },
}

/// Snapshot of the requirements bar contents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequirementsState {
    pub slots: Vec<RequirementSlot>,
    /// `true` when every required character + action slot is fulfilled.
    pub all_fulfilled: bool,
}

/// Build the requirements state for the given mission.
///
/// `mission_team_char_profile_indices` is the planned-mission team
/// (indices into `campaign.characters`).  `selected_profile_indices` is
/// the currently selected PC set, used to compute `selected` flags.
/// Both may be empty.
///
/// The per-action `selected` flag is set when a selected PC enables the
/// action overlay because it
///   - has the action directly or contextually, **or**
///   - holds the LittleJohn/Farmer carry contextual alias for the
///     other carry action, **or**
///   - has direct `Eat` / `Guzzle` for the other of the pair.
///
/// Returns `None` when the mission has no profile (e.g. Sherwood or an
/// incompletely set-up mission).
pub fn build_requirements_state(
    campaign: &Campaign,
    profiles: &crate::profiles::ProfileManager,
    mission_idx: usize,
    mission_team_char_profile_indices: &[CharacterProfileIdx],
    selected_profile_indices: &[CharacterProfileIdx],
) -> Option<RequirementsState> {
    let mission = campaign.missions.get(mission_idx)?;
    let profile = mission.profile(profiles);

    let required_characters: Vec<CharacterProfileIdx> = profile
        .required_character_indices
        .iter()
        .copied()
        .map(CharacterProfileIdx)
        .collect();
    let required_actions = purify_actions(&profile.required_actions);
    let num_required_chars = required_characters.len();
    let num_required_actions = required_actions.len();
    let num_beam_mes = profile.number_of_beam_mes as usize;
    let num_optional = num_beam_mes.saturating_sub(num_required_chars);

    let mut slots: Vec<RequirementSlot> =
        Vec::with_capacity(num_required_chars + num_required_actions + num_optional);

    // Track team PCs left after matching required characters.
    let mut team_left: Vec<CharacterProfileIdx> = mission_team_char_profile_indices.to_vec();
    let mut all_fulfilled = !mission_team_char_profile_indices.is_empty();

    // Required characters.
    for &cpi in &required_characters {
        let present = team_left.iter().position(|&c| c == cpi);
        let status = if let Some(pos) = present {
            team_left.remove(pos);
            RequirementStatus::Fulfilled
        } else {
            all_fulfilled = false;
            RequirementStatus::Missing
        };
        let selected = selected_profile_indices.contains(&cpi);
        slots.push(RequirementSlot::RequiredCharacter {
            character_profile_idx: cpi,
            status,
            selected,
        });
    }

    // Required actions.
    for &action in &required_actions {
        let status = if mission_team_has_action(profiles, mission_team_char_profile_indices, action)
        {
            RequirementStatus::Fulfilled
        } else {
            all_fulfilled = false;
            RequirementStatus::Missing
        };
        let selected = selected_pc_enables_action(profiles, selected_profile_indices, action);
        slots.push(RequirementSlot::RequiredAction {
            action,
            status,
            selected,
        });
    }

    // Optional characters — fill from `team_left` front-to-back, or
    // leave empty.
    for _ in 0..num_optional {
        let cpi = if !team_left.is_empty() {
            Some(team_left.remove(0))
        } else {
            None
        };
        slots.push(RequirementSlot::OptionalCharacter {
            character_profile_idx: cpi,
        });
    }

    Some(RequirementsState {
        slots,
        all_fulfilled,
    })
}

/// Check if any team member has the given action.
fn mission_team_has_action(
    profiles: &crate::profiles::ProfileManager,
    team_char_profile_indices: &[CharacterProfileIdx],
    action: Action,
) -> bool {
    for &cpi in team_char_profile_indices {
        let Some(character) = profiles.get_character(cpi) else {
            continue;
        };
        if character.actions.contains(&action) {
            return true;
        }
    }
    false
}

/// Whether any currently-selected PC can perform `action_required`.
/// Direct or contextual match on the target action, plus the
/// LittleJohn ↔ Farmer carry contextual alias and the Eat ↔ Guzzle
/// direct-action alias.
fn selected_pc_enables_action(
    profiles: &crate::profiles::ProfileManager,
    selected_profile_indices: &[CharacterProfileIdx],
    action_required: Action,
) -> bool {
    for &cpi in selected_profile_indices {
        let Some(p) = profiles.get_character(cpi) else {
            continue;
        };
        if p.has_action(action_required)
            || p.has_contextual_action(action_required)
            || (action_required == Action::LittleJohnCarry
                && p.has_contextual_action(Action::FarmerCarry))
            || (action_required == Action::FarmerCarry
                && p.has_contextual_action(Action::LittleJohnCarry))
            || (action_required == Action::Eat && p.has_action(Action::Guzzle))
            || (action_required == Action::Guzzle && p.has_action(Action::Eat))
        {
            return true;
        }
    }
    false
}

/// Remove duplicate actions while preserving first-occurrence order.
fn purify_actions(actions: &[Action]) -> Vec<Action> {
    let mut out: Vec<Action> = Vec::with_capacity(actions.len());
    for &a in actions {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{CharacterProfile, ProfileManager};

    fn pm_with_characters(chars: Vec<CharacterProfile>) -> ProfileManager {
        let mut pm = ProfileManager::new();
        pm.characters = chars;
        pm
    }

    fn make_character(actions: &[Action], contextual: &[Action]) -> CharacterProfile {
        let mut p = CharacterProfile::default();
        for (slot, a) in p.actions.iter_mut().zip(actions.iter()) {
            *slot = *a;
        }
        for (slot, a) in p.contextual_actions.iter_mut().zip(contextual.iter()) {
            *slot = *a;
        }
        p
    }

    #[test]
    fn selected_pc_enables_action_direct_match() {
        let c = pm_with_characters(vec![make_character(&[Action::Bow], &[])]);
        assert!(selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::Bow
        ));
        assert!(!selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::Climb
        ));
    }

    #[test]
    fn selected_pc_enables_action_contextual_fallback() {
        // Contextual-only matches count too — e.g. a PC whose Climb is
        // contextual-only.
        let c = pm_with_characters(vec![make_character(&[], &[Action::Climb])]);
        assert!(selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::Climb
        ));
    }

    #[test]
    fn selected_pc_enables_action_lj_farmer_carry_alias_uses_contextual() {
        // A PC with contextual FarmerCarry should enable LittleJohnCarry
        // via the carry alias (and vice versa).
        let c = pm_with_characters(vec![make_character(&[], &[Action::FarmerCarry])]);
        assert!(selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::LittleJohnCarry
        ));

        let c = pm_with_characters(vec![make_character(&[], &[Action::LittleJohnCarry])]);
        assert!(selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::FarmerCarry
        ));
    }

    #[test]
    fn selected_pc_enables_action_eat_guzzle_alias_uses_has_action() {
        // Eat / Guzzle alias is checked against the direct action set,
        // *not* contextual actions.
        let c = pm_with_characters(vec![make_character(&[Action::Guzzle], &[])]);
        assert!(selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::Eat
        ));

        let c = pm_with_characters(vec![make_character(&[Action::Eat], &[])]);
        assert!(selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::Guzzle
        ));

        // The Eat / Guzzle alias does NOT fall through to contextual
        // actions for the other name.
        let c = pm_with_characters(vec![make_character(&[], &[Action::Guzzle])]);
        // Direct contextual-Eat path still matches when the PC has
        // `Eat` in contextual_actions, so only test the *alias* sibling
        // lookup here: contextual Guzzle should NOT enable Eat.
        assert!(!selected_pc_enables_action(
            &c,
            &[CharacterProfileIdx(0)],
            Action::Eat
        ));
    }

    #[test]
    fn selected_pc_enables_action_empty_selection() {
        let c = pm_with_characters(vec![make_character(&[Action::Bow], &[])]);
        assert!(!selected_pc_enables_action(&c, &[], Action::Bow));
    }
}
