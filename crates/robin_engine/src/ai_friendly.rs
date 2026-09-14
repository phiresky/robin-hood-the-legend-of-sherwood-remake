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

    // -----------------------------------------------------------------------
    // Think — main stimulus dispatcher
    // -----------------------------------------------------------------------

    /// Run only the admitted handler; the engine owns the surrounding call.
    pub(crate) fn think_body(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> AiFlow<bool> {
        let stimulus_type = stimulus.stimulus_type;

        Ok(match stimulus_type {
            // Expected events
            StimulusType::EventReachPoint
            | StimulusType::EventDone
            | StimulusType::EventTimer
            | StimulusType::CallYourTalk1
            | StimulusType::CallYourTalk2
            | StimulusType::CallYourTalk3
            | StimulusType::EventMyTalk1
            | StimulusType::EventMyTalk2
            | StimulusType::EventMyTalk3 => {
                self.think_expected_event(sim, stimulus, ctx, grid, doors)?
            }

            // Unexpected events
            StimulusType::EventOutOfView
            | StimulusType::EventCouldntReachPoint
            | StimulusType::EventFitAgain
            | StimulusType::EventAfterScriptGoOn
            | StimulusType::EventSeesSoldier
            | StimulusType::CallPatrolCoordinate
            | StimulusType::CallYouJustWait
            | StimulusType::EventAppleChaseNear
            | StimulusType::EventNetAway => {
                self.think_unexpected_event(sim, stimulus, global, ctx, grid, doors)?
            }

            // Alerting events
            StimulusType::EventView
            | StimulusType::EventHear
            | StimulusType::EventPcShotAtMe
            | StimulusType::EventSeesBody
            | StimulusType::EventSeesObject
            | StimulusType::EventSeesFriendInTrouble
            | StimulusType::EventGotHit
            | StimulusType::EventLoseConsciousness
            | StimulusType::EventGetArrow
            | StimulusType::EventPanic
            | StimulusType::EventStop => false,

            // Events not handled for civilians.  The original
            // shipping build silently drops the stimulus and returns
            // false; we additionally warn so a misroute is still
            // visible.
            StimulusType::EventObjectAway
            | StimulusType::EventMissesCharly
            | StimulusType::EventSeesCharly
            | StimulusType::EventSyncCharly => {
                tracing::warn!(
                    "FriendlyAi::think: stimulus {:?} not handled for civilians (stale routing?)",
                    stimulus_type
                );
                false
            }

            StimulusType::EventReturnToDuty => {
                // EVENT_RETURN_TO_DUTY runs the duty hand-off but
                // Think returns false.
                return Err(DutyCall::new(DutyFlags::empty(), false));
            }

            // Shadows are ignored by civilians; Think returns false.
            StimulusType::EventSeesShadow => false,

            // Unknown stimuli silently no-op with a return of false.
            _ => {
                tracing::warn!(
                    "FriendlyAi::think: unknown stimulus type {:?}",
                    stimulus_type
                );
                false
            }
        })
    }

    // -----------------------------------------------------------------------
    // Think sub-methods
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Expected-event civilian dispatcher
    // -----------------------------------------------------------------------

    fn think_expected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        _doors: Option<&[crate::gate::Door]>,
    ) -> AiFlow<bool> {
        debug_assert_eq!(
            self.base.current_substate.ai_state_family(),
            Some(self.base.current_state),
            "FriendlyAi expected-event dispatch received mismatched state/substate: {:?}/{:?}",
            self.base.current_state,
            self.base.current_substate
        );

        let stimulus_type = stimulus.stimulus_type;

        match self.base.current_substate {
            // -------- Common stuff for soldiers and civilians --------
            Substate::FleeingPanic => {
                if self.base.lasting_panic_runs == 0 {
                    self.fleeing_seen_enemy_counter = 0;
                }
                // Falls through to the common-stuff dispatcher.
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::DefaultGotoPost
            | Substate::DefaultGotoPostTurn
            | Substate::DefaultGotoRoute
            | Substate::DefaultGotoRouteTurn
            | Substate::DefaultOnPost
            | Substate::DefaultEnroute
            | Substate::DefaultInMacro
            | Substate::DefaultInMacroWaitingForDone
            | Substate::FleeingRunToHide
            | Substate::FleeingRunToDoor
            | Substate::FleeingHiding => {
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
            }

            Substate::DefaultHomeSweetHome => {
                // NOP — stay home
            }

            Substate::DefaultScriptDriven => {
                // NOP — script handles everything
            }

            _ => {
                tracing::warn!(
                    "FriendlyAi::think_expected_event: unhandled substate {:?} \
                     with stimulus {:?}",
                    self.base.current_substate,
                    stimulus_type,
                );
            }
        }
        Ok(false)
    }

    // -----------------------------------------------------------------------
    // Unexpected-event civilian dispatcher
    // -----------------------------------------------------------------------

    pub(crate) fn think_unexpected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        _doors: Option<&[crate::gate::Door]>,
    ) -> AiFlow<bool> {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventCouldntReachPoint => {
                if self.base.current_substate == Substate::FleeingPanic {
                    if self.base.lasting_panic_runs == 0 {
                        self.fleeing_seen_enemy_counter = 0;
                    }
                    self.base
                        .think_expected_event_common_stuff(sim, stimulus, ctx)?;
                } else {
                    return Err(DutyCall::new(DutyFlags::empty(), false));
                }
            }

            StimulusType::EventFitAgain => {
                // Preserve the original game's direct work around return-to-duty
                // state-change callback at the engine borrow boundary.
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::InformResurrection);
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::SetEyeStatus(
                        crate::element::EyeStatus::LookForward,
                    ));
                return Err(DutyCall::new(DutyFlags::empty(), false));
            }

            StimulusType::EventOutOfView => {
                // Lost sight of someone — civilians don't react
            }

            _ => {}
        }

        Ok(false)
    }

    // -----------------------------------------------------------------------
    // Alerting-event civilian dispatcher
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Standard procedures
    // -----------------------------------------------------------------------

    /// Random ambient speech for civilians.
    ///
    /// Called each frame; only acts every 256 frames (`frame_phase == 0`).
    pub fn random_speech(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        frame_phase: u8,
        ctx: &AiContext,
    ) {
        if frame_phase != 0 {
            return;
        }

        self.random_speech_for_owner(
            sim,
            ctx.self_is_beggar,
            ctx.entity_view(self.base.me)
                .map(|view| view.current_animation),
        );
    }

    /// Ambient speech only observes its owner; production need not snapshot
    /// every other entity to supply these two inputs.
    pub(crate) fn random_speech_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        is_beggar: bool,
        animation: Option<crate::order::OrderType>,
    ) {
        // ---- executed only every 256 frames ----

        if is_beggar {
            if self.beggar_dont_talk_counter > 0 {
                self.beggar_dont_talk_counter -= 1;
            } else if self.base.current_remark == Remark::TheSoundOfSilence
                && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CivilianBeggarSpeechGate, 0..3)
                    == 0
            {
                match crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::CivilianBeggarSpeechChoice,
                    0..5,
                ) {
                    0..=2 => self.base.say(Remark::CivBeggarBegging),
                    3 => self.base.say(Remark::CivUnderNet),
                    4 => self.base.say(Remark::CivCries),
                    _ => unreachable!(),
                }
            }
        }

        // If our own current animation is Weeping, say "cries".
        // An owner excluded from the spatial observation has no animation
        // input, matching the full-context entity_view lookup.
        if animation == Some(crate::order::OrderType::Weeping) {
            self.base.say(Remark::CivCries);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
