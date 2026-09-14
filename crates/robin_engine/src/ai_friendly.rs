//! Friendly (civilian) AI.
//!
//! This module contains the `FriendlyAi` struct which extends [`AiController`]
//! with civilian-specific state: talking, panic behavior, beggar interactions,
//! and the civilian Think state machine.

use serde::{Deserialize, Serialize};

use crate::ai::*;

// ---------------------------------------------------------------------------
// Civilian-specific constants
// ---------------------------------------------------------------------------

pub const APPLE_CHASE_IDEAL_DISTANCE: i32 = 300;
pub const BEGGAR_NO_RANDOM_TALK_DISTANCE: i32 = 100;

// ---------------------------------------------------------------------------
// FriendlyAi — extends AiController with civilian-specific state
// ---------------------------------------------------------------------------

/// Civilian AI state. Extends [`AiController`] with civilian-specific fields.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FriendlyAi {
    /// Base AI controller (contains all common state).
    pub base: AiController,

    // -- Civilian-specific private fields --
    pub beggar_dont_talk_counter: u16,
    pub fleeing_seen_enemy_counter: u16,
    /// Whether this civilian still accepts the talk interaction.
    pub wants_to_talk: bool,
    /// Last NPC this civilian talked to. The original game initializes this reference to
    /// null and preserves that nullability in saved games.
    // TODO: historically persisted untagged (bare handle), unlike the tagged
    // `optional_ai_handle` fields; kept for save compatibility.
    pub last_talk_partner: Option<AiEntityHandle>,
    /// Script-controlled permission to leave after the current interaction.
    pub can_go_away: bool,
}

impl Default for FriendlyAi {
    fn default() -> Self {
        Self {
            base: AiController {
                current_state: AiState::Default,
                attitude: Attitude::Suspicious,
                ..AiController::default()
            },
            beggar_dont_talk_counter: 0,
            fleeing_seen_enemy_counter: 0,
            wants_to_talk: true,
            last_talk_partner: None,
            can_go_away: true,
        }
    }
}

impl AiRole for FriendlyAi {
    fn base_mut(&mut self) -> &mut AiController {
        &mut self.base
    }

    /// Civilians have no view override; use the base alert setter.
    fn role_set_alert_status(&mut self, level: AlertLevel) {
        self.base.set_alert_status(level);
    }
}

impl FriendlyAi {
    pub fn new(owner: NpcHandle) -> Self {
        Self {
            base: AiController::new(owner),
            ..Default::default()
        }
    }

    // -- Accessors --

    pub fn set_beggar_dont_talk_counter(&mut self, value: u16) {
        self.beggar_dont_talk_counter = value;
    }

    // -- State management --

    /// Prepare the civilian state-change log and alert status before the script callback.
    pub(crate) fn begin_state_change(&mut self, state: AiState, substate: Substate) {
        debug_assert_eq!(
            substate.ai_state_family(),
            Some(state),
            "civilian state change received mismatched state/substate: {state:?}/{substate:?}"
        );

        self.base
            .register_log_line(LogLineType::ChangeState, substate as u16);

        // Set alert status based on state (civilians only have green/yellow)
        match state {
            AiState::Sleeping | AiState::Default | AiState::Wondering => {
                self.base.set_alert_status(AlertLevel::Green);
            }
            AiState::Seeking | AiState::Fleeing => {
                self.base.set_alert_status(AlertLevel::Yellow);
            }
            _ => {
                // Civilians should never be in Attacking or Menacing
                panic!("Civilian AI entered invalid state: {:?}", state);
            }
        }
    }

    // Panic requests resume against live engine state after releasing this borrow.

    /// Panic fleeing from a specific point, tagged with the sector
    /// and level of its origin so the engine's door lookup can
    /// resolve multi-level flee paths correctly.
    pub(crate) fn panic_from_point_at(&mut self, center: Position, runs: u8) {
        // Capture the "new panic" flag before the state transition:
        // the drain's no-door arm uses this to suppress repeated
        // State-change / speech / reach-point self-fires when we're already
        // in panic.
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.panic_center_x = center.x;
        self.base.panic_center_y = center.y;
        self.base.directed_panic = true;
        self.base.outbox.actor.begin_panic = Some(PanicRequest {
            center: Some(center),
            runs,
            alert: AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }

    /// Undirected panic.
    pub(crate) fn panic_undirected(&mut self, runs: u8) {
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.directed_panic = false;
        self.base.outbox.actor.begin_panic = Some(PanicRequest {
            center: None,
            runs,
            alert: AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
